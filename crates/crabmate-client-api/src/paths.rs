//! 跨端共享的 HTTP API 端点路径常量与构造器（纯逻辑，无 IO）。
//!
//! 只收「跨端实际共用」的端点：单端专属端点（如 frontend 的
//! `/workspace/file*`、`/user-data/mcp-servers*`）仍留在端内，避免
//! 名义共享、实际单消费。`/uploads/*` 为静态资产路径，不属于 API 端点。

/// 健康检查（tui / connect）。
pub const HEALTH: &str = "/health";
/// Shell 视图状态（frontend / tui）。
pub const STATUS_SHELL: &str = "/status?view=shell";
/// 文件上传（frontend）。
pub const UPLOAD: &str = "/upload";
/// 发起 chat 流（tui / frontend）。
pub const CHAT_STREAM: &str = "/chat/stream";
/// 审批决议（tui / frontend）。
pub const CHAT_APPROVAL: &str = "/chat/approval";
/// 会话分叉（frontend）。
pub const CHAT_BRANCH: &str = "/chat/branch";
/// 工作区根（tui / frontend）。
pub const WORKSPACE: &str = "/workspace";
/// 工作区项目列表（tui / frontend）。
pub const WORKSPACE_PROJECTS: &str = "/workspace/projects";
/// 工作区克隆流（frontend）。
pub const WORKSPACE_CLONE_STREAM: &str = "/workspace/clone/stream";
/// 当前工作区会话列表（tui / frontend）。
pub const USER_DATA_WORKSPACE_SESSIONS: &str = "/user-data/workspaces/current/sessions";
/// 用户偏好（tui / connect / frontend）。
pub const USER_DATA_PREFS: &str = "/user-data/prefs";
/// LLM 覆盖配置（tui / frontend）。
pub const USER_DATA_LLM_OVERRIDES: &str = "/user-data/llm-overrides";
/// 会话存储配置（frontend）。
pub const CONFIG_SESSION_CONVERSATION_STORE: &str = "/config/session/conversation-store";
/// 可调用的 skill 目录（tui / frontend）。
///
/// 响应形如 `{enabled, skills_dir, skills_user_dir, skills_system_dir, skills[], error?}`；
/// 客户端只用 `enabled` 与 `skills[].{id,description,path}` 来喂 `/` 补全菜单。
pub const SKILLS: &str = "/skills";
/// 权限等级投影（frontend / tui）。
///
/// 服务端 `protected_api` 上的只读策略投影；**老 serve 没有这个路由（404）**，
/// 所以客户端同时拿它当「该 serve 是否支持权限等级」的探测端点——
/// 不要用 `/status` 探测（它对新老 serve 都会成功，无法区分能力）。
pub const PERMISSION: &str = "/permission";

/// 取消指定 job 的 chat 流（tui / frontend）。
#[must_use]
pub fn chat_stream_cancel(job_id: &str) -> String {
    format!("/chat/stream/{job_id}/cancel")
}

/// 已持久化会话的消息分页（`GET`；tui / frontend）。
///
/// `limit` / `before_index` 为 `None` 时走服务端默认页；`before_index` 是「本页第一条在
/// 过滤后数组中的下标」，往旧翻页时回传。只有给了 `limit` 才带 `before_index`——与服务端
/// 「不给 limit 就按默认页返回」的语义一致。`conversation_id` 按查询参数做百分号编码。
#[must_use]
pub fn conversation_messages(
    conversation_id: &str,
    limit: Option<u32>,
    before_index: Option<u32>,
) -> String {
    let mut out = format!(
        "/conversation/messages?conversation_id={}",
        crate::handoff::percent_encode_unreserved(conversation_id)
    );
    if let Some(limit) = limit.filter(|&n| n > 0) {
        out.push_str(&format!("&limit={limit}"));
        if let Some(before) = before_index {
            out.push_str(&format!("&before_index={before}"));
        }
    }
    out
}

/// 删除服务端已持久化会话（`DELETE`；tui / frontend）。
///
/// 服务端幂等（不存在 / 已过期也返回 204），非法 id 返回 400
/// `INVALID_CONVERSATION_ID`；只删服务端记录，**不**级联 Client 侧栏索引。
/// `conversation_id` 按路径段做百分号编码。
#[must_use]
pub fn conversation_delete(conversation_id: &str) -> String {
    format!(
        "/conversation/{}",
        crate::handoff::percent_encode_unreserved(conversation_id)
    )
}

/// `GET /permission` 的查询串（`agent_role` / `conversation_id` 都可省）。
///
/// 两个参数都按查询参数做百分号编码；都不给（或只有空白）时返回裸路径——
/// 服务端按配置默认等级 + 无会话意图处理。
#[must_use]
pub fn permission(agent_role: Option<&str>, conversation_id: Option<&str>) -> String {
    let mut out = PERMISSION.to_string();
    let mut sep = '?';
    for (key, val) in [
        ("agent_role", agent_role),
        ("conversation_id", conversation_id),
    ] {
        let Some(val) = val.map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(sep);
        sep = '&';
        out.push_str(key);
        out.push('=');
        out.push_str(&crate::handoff::percent_encode_unreserved(val));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CHAT_STREAM, HEALTH, SKILLS, WORKSPACE, chat_stream_cancel, conversation_delete,
        conversation_messages, permission,
    };

    #[test]
    fn constants_are_stable() {
        assert_eq!(HEALTH, "/health");
        assert_eq!(CHAT_STREAM, "/chat/stream");
        assert_eq!(WORKSPACE, "/workspace");
        assert_eq!(SKILLS, "/skills");
    }

    #[test]
    fn cancel_path_embeds_job_id() {
        assert_eq!(chat_stream_cancel("j1"), "/chat/stream/j1/cancel");
    }

    #[test]
    fn conversation_delete_path_encodes_id() {
        assert_eq!(conversation_delete("c1"), "/conversation/c1");
        assert_eq!(conversation_delete("ws:a/b"), "/conversation/ws%3Aa%2Fb");
    }

    #[test]
    fn conversation_messages_path_carries_paging() {
        assert_eq!(
            conversation_messages("c1", None, None),
            "/conversation/messages?conversation_id=c1"
        );
        assert_eq!(
            conversation_messages("c1", Some(80), None),
            "/conversation/messages?conversation_id=c1&limit=80"
        );
        assert_eq!(
            conversation_messages("c1", Some(80), Some(40)),
            "/conversation/messages?conversation_id=c1&limit=80&before_index=40"
        );
    }

    #[test]
    fn conversation_messages_path_encodes_id_and_drops_zero_limit() {
        assert_eq!(
            conversation_messages("ws:a/b", None, None),
            "/conversation/messages?conversation_id=ws%3Aa%2Fb"
        );
        // limit=0 视为「不指定」，否则服务端会拒或返回空页。
        assert_eq!(
            conversation_messages("c1", Some(0), Some(40)),
            "/conversation/messages?conversation_id=c1"
        );
    }

    #[test]
    fn permission_path_carries_optional_query() {
        assert_eq!(permission(None, None), "/permission");
        assert_eq!(
            permission(Some("coder"), None),
            "/permission?agent_role=coder"
        );
        assert_eq!(
            permission(None, Some("ws:a/b")),
            "/permission?conversation_id=ws%3Aa%2Fb"
        );
        assert_eq!(
            permission(Some("coder"), Some("c1")),
            "/permission?agent_role=coder&conversation_id=c1"
        );
        // 空白等同未指定，避免发出 `agent_role=` 这种空值。
        assert_eq!(permission(Some("  "), Some("")), "/permission");
    }
}
