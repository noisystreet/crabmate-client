//! worker 上跑的**非回合**任务：会话 / 工作区 / 状态 / 设置的 HTTP 拉取与写入。
//!
//! 从 `worker.rs` 拆出来有两个原因：
//!
//! 1. `fn-nloc-ratchet` 的单文件 920 行上限 —— job 级取消（issue #348）前 `worker.rs`
//!    已 852 行，改动必然超；
//! 2. job 级取消把「执行 + `select!` 上取消」收敛到 [`super::worker::run_cancellable`]
//!    之后，这里就只剩「一串请求 → 一个 [`UiEvent`]」的搬砖函数。
//!
//! 因此本文件的函数**一律是 `async fn` 且不碰 runtime**：`block_on` 只出现在调用方
//! `worker::spawn_worker` 那一层。一个 job 内部若被中断，**已发出的**事件保留、
//! 后面的不再发 —— 与「某一步请求失败」的现状同构，UI 侧本来就要容忍半截结果。

use crabmate_client_api::chat_branch::{BranchFailure, user_ordinal_of_last_turn};
use crabmate_client_api::chat_export::{ExportFormat, export_messages};
use crabmate_client_api::paths;
use crabmate_client_api::permission::PermissionView;

use super::serve_defaults::ServeDefaults;
use super::settings::{
    LlmSave, McpSave, PersistedSettings, PrefsSave, merge_llm_save, merge_mcp_save,
    merge_prefs_save,
};
use super::slash_menu::SkillsPayload;
use super::worker::{LaneTx, UiEvent};
use crate::conv_export::{self, ExportDone};
use crate::serve::{
    ServeClient, SessionIndexOp, TermError, apply_web_session_op, fetch_all_pages,
    fetch_conversation_messages, fetch_llm_overrides, fetch_mcp_servers, fetch_user_data_prefs,
    fetch_web_sessions, fetch_workspace, fetch_workspace_dir, fetch_workspace_projects,
    post_chat_branch, put_llm_overrides, put_mcp_servers, put_user_data_prefs,
    set_session_conversation_store, set_workspace, switch_workspace_project,
    write_permission_level,
};

/// 写会话索引（`GET` 最新 → 改 → 全量 `PUT`）：新建空白行或把 `conversation_id` 绑到指定行。
///
/// 这是本文件里唯一会**写** serve 状态的 job（除设置那三条）：`apply_web_session_op` 是
/// 「读全量索引 → 改 → 写全量」，中途被取消会留下未落地的改动（不写坏数据，只是这次
/// 操作没发生）。取消优先于「让它跑完」的理由见 issue #348：lane 已经摘了，再写一次
/// 全量索引只会和刚 attach 进来的新 lane 撞车。
pub(super) async fn job_session_index(client: &ServeClient, op: SessionIndexOp, out: &LaneTx) {
    let result = apply_web_session_op(client, &op)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::SessionIndex { result, op });
}

/// `POST /chat/branch`（issue #422）：把会话截到「最后一轮之前」，供 `/conv retry|edit` 重跑。
///
/// 序号与 revision **都现算**：`fetch_all_pages` → 数普通用户消息 → 用这一份回包里的
/// revision。这就是 Desktop 的 PATH A′（未知 revision 时先补拉），只是这里一次往返做完 ——
/// 客户端不维护 revision，也就没有「拿着旧 revision 撞 409」这条路径。
///
/// 归档会话算不准序号 → 直接失败（见 `user_ordinal_of_last_turn`）：宁可拒绝，也不能把
/// 会话截到别处。
pub(super) async fn job_conv_branch(client: &ServeClient, conversation_id: &str, out: &LaneTx) {
    let result = branch_to_last_turn(client, conversation_id).await;
    let _ = out.send(UiEvent::TurnBranched(result));
}

async fn branch_to_last_turn(
    client: &ServeClient,
    conversation_id: &str,
) -> Result<u64, BranchFailure> {
    let pages = fetch_all_pages(client, conversation_id)
        .await
        .map_err(|e| BranchFailure::Other(e.to_string()))?;
    let ordinal = user_ordinal_of_last_turn(&pages)
        .map_err(|e| BranchFailure::Other(e.message().to_string()))?;
    let revision = pages
        .first()
        .map(|p| p.revision)
        .ok_or_else(|| BranchFailure::Other("回放没有返回 revision，无法安全截断".to_string()))?;
    post_chat_branch(client, conversation_id, ordinal, revision).await
}

/// 会话列表 + 工作区路径刷新（静默；发送失败忽略，worker 随 UI 退出而停）。
///
/// 两次请求串行：`REQUEST_TIMEOUT` 是 30 s/请求，全部跑完最坏 60 s —— 这也是 job 级
/// 取消最值得覆盖的一条（detach 一条正在刷新的 lane 不必等两轮超时）。
pub(super) async fn job_refresh_sessions(client: &ServeClient, out: &LaneTx) {
    let result = fetch_web_sessions(client).await.map_err(|e| e.to_string());
    let _ = out.send(UiEvent::Sessions(result));
    let path = fetch_workspace(client).await.ok().and_then(|w| {
        let p = w.path.trim();
        (!p.is_empty()).then(|| p.to_string())
    });
    let _ = out.send(UiEvent::WorkspacePath(path));
}

/// 拉一页会话历史（只读回放）。失败如实回传，提示文案由 UI 决定。
pub(super) async fn job_conversation_history(
    client: &ServeClient,
    conversation_id: &str,
    before_index: Option<u32>,
    out: &LaneTx,
) {
    let fetched = fetch_conversation_messages(client, conversation_id, before_index).await;
    // 水合响应自带当前上下文快照：新会话 / 切换会话即可显示初始用量（与 Desktop 同口径），
    // 不必等回合末的 `RUN_FINISHED`。
    let snapshot = fetched
        .as_ref()
        .ok()
        .and_then(|r| r.tiktoken_prompt_tokens.clone());
    let result = fetched.map_err(|e| e.to_string());
    let _ = out.send(UiEvent::ConversationHistory {
        conversation_id: conversation_id.to_string(),
        result,
        snapshot,
        before_index,
    });
}

/// 回合末补用量快照：只留 `tiktoken_prompt_tokens`，消息正文丢弃。
///
/// 失败静默：状态行的用量段不是关键路径，拉不到就维持现状（占位 `—`），
/// 不必往 transcript 里写一条与用户操作无关的报错。
pub(super) async fn job_refresh_usage(client: &ServeClient, conversation_id: &str, out: &LaneTx) {
    let snapshot = fetch_conversation_messages(client, conversation_id, None)
        .await
        .ok()
        .and_then(|r| r.tiktoken_prompt_tokens);
    let _ = out.send(UiEvent::UsageHydrated {
        conversation_id: conversation_id.to_string(),
        snapshot,
    });
}

/// 导出整段会话（`/conv export`）：翻完所有页 → 渲染正文 → [`UiEvent::Exported`]。
///
/// 渲染是纯 CPU，留在 worker；写盘交给 UI 线程 —— 只有那边能判断这份回包是否仍属于
/// 当前绑定（拉取期间用户可能已经切走，见 `tui_mode::export::on_result`）。
pub(super) async fn job_export_conversation(
    client: &ServeClient,
    conversation_id: &str,
    format: ExportFormat,
    out_path: Option<String>,
    tx: &LaneTx,
) {
    let fetched = fetch_all_pages(client, conversation_id).await;
    let (result, pages, messages) = match fetched {
        Ok(fetched) => {
            let messages = export_messages(&fetched);
            (
                conv_export::render_artifacts(format, &messages),
                u32::try_from(fetched.len()).unwrap_or(u32::MAX),
                messages.len(),
            )
        }
        Err(e) => (Err(e.to_string()), 0, 0),
    };
    let _ = tx.send(UiEvent::Exported(Box::new(ExportDone {
        conversation_id: conversation_id.to_string(),
        out: out_path,
        pages,
        messages,
        result,
    })));
}

/// 工作区根目录列表（工作区路径 + 目录树数据）。
pub(super) async fn job_refresh_workspace_root(client: &ServeClient, out: &LaneTx, keep: bool) {
    let result = fetch_workspace_dir(client, None)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::Workspace { result, keep });
}

/// 展开目录的子列表。
pub(super) async fn job_workspace_dir(
    client: &ServeClient,
    rel: &str,
    generation: u64,
    out: &LaneTx,
) {
    let result = fetch_workspace_dir(client, Some(rel))
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::WorkspaceDir {
        rel: rel.to_string(),
        generation,
        result,
    });
}

/// serve 默认偏好（`/status?view=shell`）+ 当前生效权限投影（`/permission`）。
///
/// 两个端点合进同一个任务：状态行要的是**同一时刻**的 serve 侧事实，分两次任务到达顺序随机，
/// 会短暂拼出"旧 model + 新等级"这种没发生过的组合。
pub(super) async fn job_refresh_status(
    client: &ServeClient,
    conversation_id: Option<&str>,
    out: &LaneTx,
) {
    let mut result = fetch_serve_defaults(client).await;
    let view = fetch_permission_view(client, conversation_id).await;
    if let Ok(defaults) = result.as_mut() {
        defaults.permission_view = view;
    }
    let _ = out.send(UiEvent::Status {
        conversation_id: conversation_id.map(str::to_string),
        result,
    });
}

async fn fetch_serve_defaults(client: &ServeClient) -> Result<ServeDefaults, String> {
    let v: serde_json::Value = client
        .get_json(paths::STATUS_SHELL)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ServeDefaults::from_status(&v))
}

/// `GET /permission` 的完整投影：**404（老 serve）与任何失败都返回 `None`**。
///
/// 状态行少一段，而不是显示一个猜来的档位——等级是策略事实，猜错的方向可能是危险的
/// （把 `chat` 显示成 `unrestricted` 才最糟）。需要细节时看 Desktop 设置页或 `/status`
/// 里的 `permission_level_default` / `permission_level_max`。
///
/// 存**整个** view（不再只取 `level`）：切换菜单的选项集就是 `available`，与 Desktop chip
/// 吃同一份投影，避免「菜单列五档、服务端只让选三档」这种自造的分歧。
///
/// **带 `conversation_id`**：这是按会话算的投影（会话列里的意图 + min 链）。裸请求回的是
/// 「配置默认档 + 无会话意图」，与状态行要回答的「此刻这个会话是哪一档」是两个问题 ——
/// 混用会让状态行在刷新后跳回默认档，看起来像刚选的档自己丢掉了。
async fn fetch_permission_view(
    client: &ServeClient,
    conversation_id: Option<&str>,
) -> Option<PermissionView> {
    let view = client
        .get_json_optional::<PermissionView>(&paths::permission(None, conversation_id))
        .await
        .ok()
        .flatten()?;
    (!view.level.trim().is_empty()).then_some(view)
}

/// 权限等级意图落盘（`POST /conversation/state`）。
///
/// 只发 `permission_level` 一列：服务端是部分更新，顺带回传 `session_mode` / `agent_role`
/// 会把过期值写回去（Desktop `apply_level` 同一条约束）。
///
/// 结局**不在这一层判定**：拿到响应就交给共享层
/// `classify_conversation_state_response` —— 这条路由的 404 有「老 serve 没路由」与
/// 「会话不存在」两种含义，两处各写一份判别就是新的漂移点。
pub(super) async fn job_write_permission(
    client: &ServeClient,
    conversation_id: &str,
    level: &str,
    generation: u64,
    out: &LaneTx,
) {
    let outcome = write_permission_level(client, conversation_id, level)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::PermissionWritten {
        level: level.to_string(),
        outcome,
        generation,
    });
}

/// skill 目录（`GET /skills`）：只给 `/` 补全菜单的 skill 候选，失败如实回传。
pub(super) async fn job_skills(client: &ServeClient, out: &LaneTx) {
    let result = client
        .get_json::<SkillsPayload>(paths::SKILLS)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::Skills(result));
}

/// `POST /config/reload`：让 serve 重读磁盘配置（`/config reload`）。
///
/// 空 JSON 体：服务端把「重载」表达成一次无参 POST，客户端不猜任何字段。
/// 成功信息是**空响应体**（前端 `post_config_reload` 同口径），所以这里只看 `Err`，
/// 不解析 body —— 拿 body 猜成败会让「换了一版 serve 的返回格式」变成假失败。
pub(super) async fn job_config_reload(client: &ServeClient, out: &LaneTx) {
    let result = client
        .post_json(paths::CONFIG_RELOAD, &serde_json::json!({}))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::ConfigReloaded(result));
}

/// 项目池列表（`GET /workspace/projects`）。
pub(super) async fn job_ws_projects(client: &ServeClient, out: &LaneTx) {
    let result = fetch_workspace_projects(client)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::WsProjects(result));
}

/// 切换项目池内项目（`POST /workspace/projects`）。
pub(super) async fn job_ws_switch_project(client: &ServeClient, name: &str, out: &LaneTx) {
    let result = switch_workspace_project(client, name)
        .await
        .map_err(|e| e.to_string());
    let _ = out.send(UiEvent::WsProjectSwitch(result));
}

/// `/workspace`（空参）：读 serve 当前工作区根（`GET /workspace`）。
///
/// `fetch_workspace` 不像 `fetch_workspace_dir` 那样检查体内 `error`（2xx 也可能带
/// 语义错误），这里补上：否则用户会看到「当前工作区：」空一行而不是失败原因。
pub(super) async fn job_workspace_query(client: &ServeClient, out: &LaneTx) {
    let result = match fetch_workspace(client).await {
        Ok(w) => match w.error.as_deref().map(str::trim) {
            Some(e) if !e.is_empty() => Err(e.to_string()),
            _ => Ok(w.path),
        },
        Err(e) => Err(e.to_string()),
    };
    let _ = out.send(UiEvent::WorkspaceQuery(result));
}

/// `/workspace <路径>`：按路径设置 serve 工作区根（`POST /workspace`）。
///
/// 与项目池那条（`POST /workspace/projects`）是同一个后端状态的两种入口：项目池只能选
/// 池内已有项目，这里把路径原样发出去（客户端不做白名单，能否落地由 serve 决定；
/// 空串＝恢复 serve 默认）。
///
/// 失败**不压成字符串**：`WorkspaceSetError` 带 `kind`，UI 层才能把 409
/// `WORKSPACE_ALREADY_SET` 提示成「另起 lane」而不是「重试」。
pub(super) async fn job_workspace_set(client: &ServeClient, path: &str, out: &LaneTx) {
    let result = set_workspace(client, path).await;
    let _ = out.send(UiEvent::WorkspaceSet(result));
}

/// 启动时拉取 user-data 设置快照（`GET /user-data/prefs` + `GET /user-data/llm-overrides`）
/// 并合成持久层；任一失败整体报错（面板/合成回退 override 与 serve 默认）。
pub(super) async fn job_load_user_settings(client: &ServeClient, out: &LaneTx) {
    let result: Result<PersistedSettings, String> = async {
        let prefs = fetch_user_data_prefs(client)
            .await
            .map_err(|e| e.to_string())?;
        let llm = fetch_llm_overrides(client)
            .await
            .map_err(|e| e.to_string())?;
        // MCP 是可选端点：失败只让那一栏显示「未提供」，不拖垮 prefs/llm 的加载。
        // 但要分清「404 = 这个 serve 没有该端点」与「这次没拉到」—— 后者由面板重开时
        // 重拉（`open_settings` 认 `mcp_fetch_failed`）。压成同一个 `None` 就再也没有
        // 第二次机会了，而提示还会指错方向（「版本过旧」）。
        let (mcp, mcp_fetch_failed) = match fetch_mcp_servers(client).await {
            Ok(v) => (v, false),
            Err(e) => {
                eprintln!("[crabmate-tui] GET /user-data/mcp-servers failed: {e}");
                (None, true)
            }
        };
        let mut persisted = PersistedSettings::from_snapshot(&prefs, &llm, mcp.as_ref());
        persisted.mcp_fetch_failed = mcp_fetch_failed;
        Ok(persisted)
    }
    .await;
    let _ = out.send(UiEvent::UserData(result));
}

/// 保存 llm-overrides：先 GET 现值，只改 `client_llm.{model,api_base,temperature,
/// llm_thinking_mode,llm_context_tokens}` 再全量 PUT
/// （合并保真，不覆盖 `executor_llm/saved_models/execution_mode`）。
pub(super) async fn job_save_llm(client: &ServeClient, save: LlmSave, out: &LaneTx) {
    let result: Result<LlmSave, String> = async {
        let dto = fetch_llm_overrides(client)
            .await
            .map_err(|e| e.to_string())?;
        put_llm_overrides(client, &merge_llm_save(dto, &save))
            .await
            .map_err(|e| e.to_string())?;
        Ok(save)
    }
    .await;
    let _ = out.send(UiEvent::SavedLlm(result));
}

/// 保存 prefs：先 GET 现值，只改 `cm_role/session_mode/disable_readonly_tool_ttl_cache`
/// 再全量 PUT（合并保真，不覆盖 `locale/theme/布局/IDE` 等字段）。
pub(super) async fn job_save_prefs(client: &ServeClient, save: PrefsSave, out: &LaneTx) {
    let result: Result<PrefsSave, String> = async {
        let dto = fetch_user_data_prefs(client)
            .await
            .map_err(|e| e.to_string())?;
        put_user_data_prefs(client, &merge_prefs_save(dto, &save))
            .await
            .map_err(|e| e.to_string())?;
        Ok(save)
    }
    .await;
    let _ = out.send(UiEvent::SavedPrefs(result));
}

/// 切换会话存储（`POST /config/session/conversation-store`）。
///
/// 不做「先 GET 再合并」：这个端点是一次动作而不是全量 DTO，没有别的键会被覆盖。
pub(super) async fn job_save_store(client: &ServeClient, sqlite: bool, out: &LaneTx) {
    let result = set_session_conversation_store(client, sqlite)
        .await
        .map(|()| sqlite)
        .map_err(|e| save_error_text(&e));
    let _ = out.send(UiEvent::SavedStore(result));
}

/// 保存 MCP 标量：先 GET 现值，只改 `global_enabled` / `tool_timeout_secs` 再全量 PUT
/// （服务器条目与 `schema_version` 原样带回，见 [`merge_mcp_save`]）。
pub(super) async fn job_save_mcp(client: &ServeClient, save: McpSave, out: &LaneTx) {
    let result: Result<McpSave, String> = async {
        let dto = fetch_mcp_servers(client)
            .await
            .map_err(|e| save_error_text(&e))?
            .ok_or_else(|| "该 serve 没有 /user-data/mcp-servers（版本过旧）".to_string())?;
        put_mcp_servers(client, &merge_mcp_save(dto, &save))
            .await
            .map_err(|e| save_error_text(&e))?;
        Ok(save)
    }
    .await;
    let _ = out.send(UiEvent::SavedMcp(result));
}

/// 保存失败 → 给用户看的文案：**区分「serve 已经表态」与「这次没拿到」**。
///
/// `TermError::Message` 在这条路径上几乎都是 serve 的 JSON 错误体（`code: message`）：
/// 同一个请求体再发一次结果不会变，所以不能像网络错误那样写「可按 S 重试」。
/// 反过来 5xx / 网关 HTML / 传输错误值得重试，也要把话说出来。
fn save_error_text(e: &TermError) -> String {
    match e {
        // serve 明确拒绝：文案里已经带了原因，原样给人读，不再套一层。
        TermError::Message(m) => m.clone(),
        TermError::Http { status, .. } if (400..500u16).contains(status) => {
            format!("{e}（serve 拒绝：改值后再试）")
        }
        _ => format!("{e}（网络/传输失败，可按 S 重试）"),
    }
}

#[cfg(test)]
mod tests {
    use super::save_error_text;
    use crate::serve::TermError;

    /// 业务拒绝（带错误码）原样呈现，且不承诺重试 —— 重试同一个值永远不会成功。
    #[test]
    fn server_rejection_is_passed_through_without_a_retry_hint() {
        let e = TermError::Message("MCP_TIMEOUT_INVALID: 超时需 1..=3600 秒".to_string());
        let text = save_error_text(&e);
        assert_eq!(text, "MCP_TIMEOUT_INVALID: 超时需 1..=3600 秒");
        assert!(!text.contains("重试"), "业务拒绝不该承诺重试：{text}");
    }

    /// 5xx / 非 JSON 错误体：这类重试可能成功，提示要说出来。
    #[test]
    fn transport_failures_say_retry_is_fine() {
        let e = TermError::Http {
            status: 502,
            body: "<html>bad gateway</html>".to_string(),
        };
        assert!(save_error_text(&e).contains("可按 S 重试"));
    }

    /// 4xx 但响应体不成形（网关 / 反向代理）同样算 serve 拒绝。
    #[test]
    fn bare_4xx_counts_as_a_rejection() {
        let e = TermError::Http {
            status: 409,
            body: String::new(),
        };
        assert!(save_error_text(&e).contains("serve 拒绝"));
    }
}
