//! Web composer：控制面 `/` 命令（不经模型）。密钥从不入聊天记录。

use std::sync::Arc;

use crabmate_client_api::slash_caps::{Surface, supports};
use crabmate_client_api::slash_heads::{Head, head_of};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::client_llm_storage::{
    clear_client_llm_api_key_storage_async, persist_client_llm_to_storage_async,
};
use crate::api::llm_secrets_local::PersistKind;
use crate::api::{
    client_llm_storage_has_api_key, fetch_skills, fetch_status, fetch_workspace,
    load_client_llm_text_fields_from_storage, persist_client_llm_to_storage, post_config_reload,
    post_workspace_set,
};
use crate::app::chat::handles::ComposerStreamShell;
use crate::app::shell_runtime_context::try_chat_shell_ctx;
use crate::app::status_permission_chip::{LevelOutcome, PermTarget, apply_permission_level};
use crate::app::status_session_mode_seg::apply_session_mode_selection;
use crate::chat_session_state::ChatSessionSignals;
use crate::i18n::{self, Locale};
use crate::session_ops::{
    export_session_json_for_id, export_session_markdown_for_id, make_message_id,
    message_created_ms, patch_active_session,
};
use crate::storage::StoredMessage;

/// Web 端**真正独占**的斜杠 head（TUI 没有 handler，故不在 `slash_heads` 里）。
///
/// 只剩三条界面级的：`/api-base` 与 `/api-key` 改的是浏览器 / 壳的本机设置，
/// `/clear` 清的是这段 Web transcript（不碰服务端会话）—— 它们在 TUI 没有等价落点。
///
/// `version` / `skills` / `context` / `config` / `agent` / `export` 曾在这张表里，
/// issue #444 之后三端都有 handler，已升级为共享 `Head`（查 `slash_caps`）：
/// 留着就是第二份名单，`/version` 那种「能敲但 help 不写」的漂移会再来一次。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebOnly {
    ApiBase,
    Clear,
    ApiKey,
}

/// `/<head>` 的 head（去 `/`、取首词、小写）。不是 `/` 开头返回 `None`。
fn slash_head(trimmed: &str) -> Option<String> {
    let s = trimmed.trim();
    s.strip_prefix('/')?
        .split_whitespace()
        .next()
        .map(|h| h.to_ascii_lowercase())
}

fn web_only_head(raw: &str) -> Option<WebOnly> {
    Some(match raw {
        // `apibase` / `apikey` 是无连字符的肌肉记忆写法，与带连字符的一样收。
        "api-base" | "apibase" => WebOnly::ApiBase,
        "clear" => WebOnly::Clear,
        "api-key" | "apikey" => WebOnly::ApiKey,
        _ => return None,
    })
}

/// `/help` 的正文。列全本端拦截集，否则「敲了有用但 help 没写」与「help 写了却没用」
/// 一样难查（`/version` 曾漏在这里）。
const HELP_TEXT: &str = "常用控制命令（不经模型）：\n\
/help · /version · /workspace · /agent · /model\n\
/api-base · /export · /config · /context\n\
/skills · /clear · /api-key（密钥不进对话）\n\
/model set <id> 存本机（TUI 的 /model 只在那个进程有效）；/clear 只清本屏，服务端历史还在\n\
/mode ask|plan|act · /role <id> · /perm <level> · /status（与底栏同一组开关）";

/// 是否为 Web 控制斜杠（应拦截，不发送给模型）。`/<skill-id>` 除外。
///
/// 「这一端支不支持」不再手写：共享 head 查 `slash_caps`（`Surface::Desktop`），
/// 端独占的查 [`web_only_head`]。两边都由测试与分派表夹住 —— 改一处漏一处会有测试失败，
/// 而不是变成「拦下来没人实现」或「实现了却敲不出来」。
#[must_use]
pub(super) fn is_web_control_slash(trimmed: &str) -> bool {
    let Some(head) = slash_head(trimmed) else {
        return false;
    };
    match head_of(&head) {
        Some(h) => supports(Surface::Desktop, h),
        None => web_only_head(&head).is_some(),
    }
}

#[derive(Clone, Copy)]
pub(super) struct WebSlashControlCtx {
    pub chat: ChatSessionSignals,
    pub locale: RwSignal<Locale>,
    pub draft: RwSignal<String>,
    pub selected_agent_role: RwSignal<Option<String>>,
    pub agent_role_user_override: RwSignal<bool>,
    /// 会话模式与「用户显式选过」标记：与底栏 Ask / Plan / Act 那组是**同一个**信号，
    /// `/mode` 只是把同一件事写成命令形式（issue #444 PR-A）。
    pub selected_session_mode: RwSignal<String>,
    pub session_mode_user_override: RwSignal<bool>,
    pub apply_assistant_display_filters: RwSignal<bool>,
}

/// 尝试处理控制斜杠：返回 `true` 表示已消费（勿走 LLM）。
pub(super) fn try_handle_web_control_slash(
    text: &str,
    shell: &ComposerStreamShell,
    ctx: WebSlashControlCtx,
) -> bool {
    if !is_web_control_slash(text) {
        return false;
    }
    let Some(head) = slash_head(text) else {
        return false;
    };
    let loc = ctx.locale.get_untracked();
    let args: Vec<&str> = text.split_whitespace().skip(1).collect();

    // 共享 head 与端独占 head 分两层分派：前者由 `slash_caps` 决定谁能进来，
    // 后者不在 `Head` 里（TUI 没有 handler），必须单独查表。
    let consumed = match head_of(&head) {
        Some(h) => dispatch_shared(h, &args, shell, ctx, loc),
        None => match web_only_head(&head) {
            Some(which) => {
                dispatch_web_only(which, &args, shell, ctx, loc);
                true
            }
            None => false,
        },
    };
    if consumed {
        ctx.draft.set(String::new());
    }
    consumed
}

/// 共享 head 的分派。`supports(Surface::Desktop, …)` 说支持而这里没臂 = 真 bug：
/// 兜底写一句提示，不要静默吞掉命令（那正是本轮重构要消灭的失效形态）。
fn dispatch_shared(
    head: Head,
    args: &[&str],
    shell: &ComposerStreamShell,
    ctx: WebSlashControlCtx,
    loc: Locale,
) -> bool {
    match head {
        Head::Help => set_ok(ctx.chat, shell, HELP_TEXT),
        Head::Workspace => handle_workspace(args, shell, ctx.chat, loc),
        Head::Model => handle_model(args, shell, ctx.chat, loc),
        // issue #444 PR-A：这四条一直有 UI 入口，命令形式只是第二个入口 ——
        // `/role` 归一到既有 `/agent set`、`/status` 归一到既有 `/config`（无参摘要），
        // `/mode` / `/perm` 写的是状态栏那组同一信号。没有新增真相源。
        Head::Mode => handle_mode(args, shell, ctx),
        Head::Role => handle_role(args, shell, ctx),
        Head::Perm => handle_perm(args, shell, ctx),
        Head::Status => handle_status(shell, ctx),
        // 曾放在 `WebOnly` 里的六条：三端都有 handler 之后归共享分派，
        // 但实现一行没改 —— 变的只是「谁能进来」的判定来源。
        Head::Version => set_ok(
            ctx.chat,
            shell,
            format!("CrabMate Web · {}", env!("CARGO_PKG_VERSION")),
        ),
        Head::Skills => handle_skills(shell, ctx.chat, loc),
        Head::Context => handle_context(shell, ctx.chat, loc),
        Head::Config => handle_config(args, shell, ctx.chat, loc),
        Head::Agent => handle_agent(args, shell, ctx),
        Head::Export => handle_export(args, shell, ctx, loc),
        other => set_ok(
            ctx.chat,
            shell,
            format!("未知控制命令 /{}；输入 /help", other.as_str()),
        ),
    }
    true
}

fn dispatch_web_only(
    which: WebOnly,
    args: &[&str],
    shell: &ComposerStreamShell,
    ctx: WebSlashControlCtx,
    loc: Locale,
) {
    match which {
        WebOnly::ApiBase => handle_api_base(args, shell, ctx.chat, loc),
        WebOnly::Clear => handle_clear(shell, ctx),
        WebOnly::ApiKey => handle_api_key(args, shell, ctx.chat, loc),
    }
}

/// `/clear`：只清本地这段 transcript，**不**动 serve 上的会话记录。
fn handle_clear(shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    let id = ctx.chat.active_id.get_untracked();
    ctx.chat.update_sessions_composer(|list| {
        if let Some(s) = list.iter_mut().find(|s| s.id == id) {
            s.messages.clear();
            s.draft.clear();
            s.history_total = None;
            s.history_window_start = None;
            s.history_has_older = None;
        }
    });
    ctx.draft.set(String::new());
    set_ok(ctx.chat, shell, "已清空当前会话消息。");
}

/// 控制斜杠成功/说明：写入会话 system 气泡（不经模型、导出时跳过），勿塞状态栏以免右下角叠字。
fn set_ok(chat: ChatSessionSignals, shell: &ComposerStreamShell, msg: impl Into<String>) {
    shell.stream.status_err.set(None);
    push_control_slash_notice(chat, msg);
}

fn set_err(shell: &ComposerStreamShell, msg: impl Into<String>) {
    shell.stream.set_error(msg);
}

fn push_control_slash_notice(chat: ChatSessionSignals, msg: impl Into<String>) {
    let text = msg.into();
    let mid = make_message_id();
    let now = message_created_ms();
    patch_active_session(chat.sessions, &chat.active_id.get_untracked(), |s| {
        s.messages.push(StoredMessage {
            id: mid,
            role: "system".into(),
            text,
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: now,
        });
    });
}

fn handle_workspace(
    args: &[&str],
    shell: &ComposerStreamShell,
    chat: ChatSessionSignals,
    loc: Locale,
) {
    let stream = shell.stream.clone();
    if args.is_empty() {
        spawn_local(async move {
            match fetch_workspace(None, loc).await {
                Ok(w) => {
                    stream.status_err.set(None);
                    push_control_slash_notice(chat, format!("工作区: {}", w.path));
                }
                Err(e) => stream.set_error(format!("工作区查询失败: {e}")),
            }
        });
        return;
    }
    let path = args.join(" ");
    let refresh = Arc::clone(&shell.refresh_workspace);
    spawn_local(async move {
        match post_workspace_set(Some(path), loc).await {
            Ok(p) => {
                refresh();
                stream.status_err.set(None);
                push_control_slash_notice(chat, format!("工作区已切换: {p}"));
            }
            Err(e) => stream.set_error(format!("切换失败: {e}")),
        }
    });
}

fn handle_agent(args: &[&str], shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    let sub = args.first().copied().unwrap_or("list");
    match sub {
        "list" => {
            let stream = shell.stream.clone();
            let chat = ctx.chat;
            let loc = ctx.locale.get_untracked();
            let cur = ctx.selected_agent_role.get_untracked();
            spawn_local(async move {
                match fetch_status(loc).await {
                    Ok(s) => {
                        let ids = if s.agent_role_ids.is_empty() {
                            "（未配置多角色；可用 default）".to_string()
                        } else {
                            s.agent_role_ids.join(", ")
                        };
                        let cur_disp = cur
                            .filter(|c| !c.trim().is_empty())
                            .or(s.default_agent_role_id)
                            .unwrap_or_else(|| "default".into());
                        stream.status_err.set(None);
                        push_control_slash_notice(chat, format!("角色: {ids} · 当前 {cur_disp}"));
                    }
                    Err(e) => stream.set_error(format!("获取角色失败: {e}")),
                }
            });
        }
        "set" => {
            let id = args.get(1).copied().unwrap_or("").trim();
            if id.is_empty() {
                set_ok(
                    ctx.chat,
                    shell,
                    "用法: /agent set <id> | /agent set default",
                );
                return;
            }
            if id.eq_ignore_ascii_case("default") {
                ctx.selected_agent_role.set(None);
                ctx.agent_role_user_override.set(true);
                set_ok(ctx.chat, shell, "已切到 default（清除显式角色）");
            } else {
                ctx.selected_agent_role.set(Some(id.to_string()));
                ctx.agent_role_user_override.set(true);
                set_ok(ctx.chat, shell, format!("已设角色: {id}"));
            }
        }
        _ if args.is_empty() => handle_agent(&["list"], shell, ctx),
        _ => set_ok(
            ctx.chat,
            shell,
            "用法: /agent · /agent list · /agent set <id>",
        ),
    }
}

fn handle_export(args: &[&str], shell: &ComposerStreamShell, ctx: WebSlashControlCtx, loc: Locale) {
    let kind = args.first().copied().unwrap_or("json").to_ascii_lowercase();
    let id = ctx.chat.active_id.get_untracked();
    let filters = ctx.apply_assistant_display_filters.get_untracked();
    let include_inject = shell.show_turn_context_inject.get_untracked();
    match kind.as_str() {
        "json" | "" => {
            export_session_json_for_id(ctx.chat.sessions, &id, loc, filters, include_inject);
            set_ok(ctx.chat, shell, "已触发 JSON 导出下载");
        }
        "markdown" | "md" => {
            export_session_markdown_for_id(ctx.chat.sessions, &id, loc, filters, include_inject);
            set_ok(ctx.chat, shell, "已触发 Markdown 导出下载");
        }
        "both" => {
            export_session_json_for_id(ctx.chat.sessions, &id, loc, filters, include_inject);
            export_session_markdown_for_id(ctx.chat.sessions, &id, loc, filters, include_inject);
            set_ok(ctx.chat, shell, "已触发 JSON + Markdown 导出下载");
        }
        _ => set_ok(ctx.chat, shell, "用法: /export [json|markdown|both]"),
    }
}

/// `/mode [ask|plan|act]`：与底栏 mode 段同一条写入路径。
///
/// 它自己会往会话里写一条 system 提示，所以这里**不再补一句**（两条会重复）。
/// 空参只显示当前档；`off` 不做 —— Web 的默认档来自服务端 prefs，清 override 得先
/// 重新拉一次才算「恢复默认」，那是设置页的活，命令层不假装支持。
fn handle_mode(args: &[&str], shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    let cur = ctx.selected_session_mode.get_untracked();
    let Some(mode) = args
        .first()
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        set_ok(
            ctx.chat,
            shell,
            format!("当前 mode={cur}（/mode ask|plan|act 切换；恢复默认用设置页）"),
        );
        return;
    };
    if !apply_session_mode_selection(
        ctx.chat,
        ctx.selected_session_mode,
        ctx.session_mode_user_override,
        ctx.locale.get_untracked(),
        mode,
    ) {
        set_ok(
            ctx.chat,
            shell,
            "用法: /mode ask|plan|act（恢复默认用设置页）",
        );
    }
}

/// `/role <id>`：`/agent set <id>` 的别名 —— 同一个写路径、同一句提示。
fn handle_role(args: &[&str], shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    let Some(id) = args
        .first()
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        set_ok(
            ctx.chat,
            shell,
            "用法: /role <id>（列表见 /agent list；default 清除）",
        );
        return;
    };
    handle_agent(&["set", id], shell, ctx);
}

/// `/perm [level]`：与底栏权限 chip 同一份守卫与落盘（`apply_permission_level`）。
///
/// 回合进行中会被同一条守卫拒绝且**不改意图** —— 命令形式不该比点选宽松。
fn handle_perm(args: &[&str], shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    let loc = ctx.locale.get_untracked();
    let level = args
        .first()
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(shell_ctx) = try_chat_shell_ctx() else {
        set_ok(ctx.chat, shell, "/perm 需要聊天壳层上下文：请在应用内使用");
        return;
    };
    let Some(level) = level else {
        let cur = shell_ctx.selected_permission_level.get_untracked();
        let shown = cur.unwrap_or_else(|| "（未选择，按 serve 生效档）".to_string());
        set_ok(
            ctx.chat,
            shell,
            format!("当前权限意图: {shown}（/perm <level> 切换）"),
        );
        return;
    };
    shell.stream.status_err.set(None);
    let chat = ctx.chat;
    apply_permission_level(
        level,
        PermTarget {
            selected: shell_ctx.selected_permission_level,
            owner: shell_ctx.permission_intent_owner,
            chat: Some(chat),
            turn_in_flight: Some(shell_ctx.perm_turn_in_flight),
            // 命令层不维护 chip 那份「丢弃过期响应」的代次：连打两次时后一次本来就该赢。
            generation: None,
        },
        loc,
        move |outcome| push_control_slash_notice(chat, perm_outcome_text(outcome, loc)),
    );
}

/// `/perm` 的落点：把与 chip 相同的那份 [`LevelOutcome`] 写成一行会话提示。
fn perm_outcome_text(outcome: LevelOutcome, loc: Locale) -> String {
    match outcome {
        LevelOutcome::Applied(view) => format!("权限等级已写入：{}", view.level),
        LevelOutcome::IntentOnly => i18n::settings_permission_send_to_apply(loc).to_string(),
        LevelOutcome::UnknownLevel => i18n::settings_permission_unknown_level(loc).to_string(),
        LevelOutcome::TurnRunning => i18n::settings_permission_turn_running(loc).to_string(),
        LevelOutcome::ConversationGone => {
            i18n::settings_permission_conversation_gone(loc).to_string()
        }
        LevelOutcome::Unsupported => i18n::settings_permission_unsupported(loc).to_string(),
        LevelOutcome::WriteFailed { status, code } => {
            let base = if status == 500 {
                i18n::settings_permission_write_retry(loc)
            } else {
                i18n::settings_permission_write_failed(loc)
            };
            let detail = code.unwrap_or_else(|| format!("HTTP {status}"));
            format!("{base} ({detail})")
        }
        LevelOutcome::IoError(e) => {
            format!("{} {e}", i18n::settings_permission_write_failed(loc))
        }
    }
}

/// `/status`：`/config`（无参）的别名 —— 同一份只读摘要，不是第二次读取。
fn handle_status(shell: &ComposerStreamShell, ctx: WebSlashControlCtx) {
    handle_config(&[], shell, ctx.chat, ctx.locale.get_untracked());
}

fn handle_config(
    args: &[&str],
    shell: &ComposerStreamShell,
    chat: ChatSessionSignals,
    loc: Locale,
) {
    let sub = args.first().copied().unwrap_or("");
    if sub.eq_ignore_ascii_case("reload") {
        let stream = shell.stream.clone();
        spawn_local(async move {
            match post_config_reload(loc).await {
                Ok(msg) => {
                    stream.status_err.set(None);
                    push_control_slash_notice(chat, msg);
                }
                Err(e) => stream.set_error(format!("热重载失败: {e}")),
            }
        });
        return;
    }
    if !sub.is_empty() {
        set_ok(chat, shell, "用法: /config · /config reload");
        return;
    }
    let stream = shell.stream.clone();
    spawn_local(async move {
        match fetch_status(loc).await {
            Ok(s) => {
                stream.status_err.set(None);
                push_control_slash_notice(
                    chat,
                    format!(
                        "model={} · api_base={} · ctx_tokens={} · sqlite_active={}",
                        s.model,
                        s.api_base,
                        s.llm_context_tokens,
                        s.conversation_store_sqlite_active
                    ),
                );
            }
            Err(e) => stream.set_error(format!("/config 失败: {e}")),
        }
    });
}

fn handle_model(args: &[&str], shell: &ComposerStreamShell, chat: ChatSessionSignals, loc: Locale) {
    if args.is_empty() {
        let (base, model, temp, ctx_tok, think) = load_client_llm_text_fields_from_storage();
        let model_disp = if model.trim().is_empty() {
            "(server default)".to_string()
        } else {
            model
        };
        set_ok(
            chat,
            shell,
            format!(
                "model={model_disp} · api_base={base} · temp={temp} · ctx={ctx_tok} · think={think}"
            ),
        );
        return;
    }
    if !args
        .first()
        .copied()
        .unwrap_or("")
        .eq_ignore_ascii_case("set")
    {
        set_ok(chat, shell, "用法: /model · /model set <id>");
        return;
    }
    let name = args.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
    let name = name.trim();
    if name.is_empty() {
        set_ok(chat, shell, "用法: /model set <id>");
        return;
    }
    let (base, _old, temp, ctx_tok, think) = load_client_llm_text_fields_from_storage();
    match persist_client_llm_to_storage(&base, name, &temp, &ctx_tok, &think, None, loc) {
        Ok(()) => set_ok(
            chat,
            shell,
            format!("已设 model={name}（已同步 user-data）"),
        ),
        Err(e) => set_err(shell, format!("写入失败: {e}")),
    }
}

fn handle_api_base(
    args: &[&str],
    shell: &ComposerStreamShell,
    chat: ChatSessionSignals,
    loc: Locale,
) {
    if args.is_empty() {
        let (base, model, ..) = load_client_llm_text_fields_from_storage();
        set_ok(chat, shell, format!("api_base={base} · model={model}"));
        return;
    }
    if !args
        .first()
        .copied()
        .unwrap_or("")
        .eq_ignore_ascii_case("set")
    {
        set_ok(chat, shell, "用法: /api-base · /api-base set <url>");
        return;
    }
    let url = args.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
    let url = url.trim();
    if url.is_empty() {
        set_ok(chat, shell, "用法: /api-base set <url>");
        return;
    }
    let (_old, model, temp, ctx_tok, think) = load_client_llm_text_fields_from_storage();
    match persist_client_llm_to_storage(url, &model, &temp, &ctx_tok, &think, None, loc) {
        Ok(()) => set_ok(
            chat,
            shell,
            format!("已设 api_base={url}（已同步 user-data）"),
        ),
        Err(e) => set_err(shell, format!("写入失败: {e}")),
    }
}

fn handle_context(shell: &ComposerStreamShell, chat: ChatSessionSignals, loc: Locale) {
    let stream = shell.stream.clone();
    spawn_local(async move {
        match fetch_status(loc).await {
            Ok(s) => {
                stream.status_err.set(None);
                push_control_slash_notice(
                    chat,
                    format!(
                        "llm_context_tokens={} · effective_char_budget={} · counting_model={}",
                        s.llm_context_tokens,
                        s.effective_context_char_budget,
                        s.tiktoken_prompt_counting_model
                    ),
                );
            }
            Err(e) => stream.set_error(format!("/context 失败: {e}")),
        }
    });
}

fn handle_skills(shell: &ComposerStreamShell, chat: ChatSessionSignals, loc: Locale) {
    let stream = shell.stream.clone();
    spawn_local(async move {
        match fetch_skills(loc).await {
            Ok(d) if !d.enabled => {
                stream.status_err.set(None);
                push_control_slash_notice(chat, "skills 已关闭（skills_enabled=false）");
            }
            Ok(d) if d.skills.is_empty() => {
                stream.status_err.set(None);
                push_control_slash_notice(
                    chat,
                    format!("当前未发现 skills（dir={}）", d.skills_dir),
                );
            }
            Ok(d) => {
                let mut lines = vec![format!("skills {} 个：", d.skills.len())];
                for s in d.skills.iter().take(24) {
                    lines.push(format!("  /{} — {}", s.id, s.description));
                }
                stream.status_err.set(None);
                push_control_slash_notice(chat, lines.join("\n"));
            }
            Err(e) => stream.set_error(format!("列出 skills 失败: {e}")),
        }
    });
}

fn handle_api_key(
    args: &[&str],
    shell: &ComposerStreamShell,
    chat: ChatSessionSignals,
    loc: Locale,
) {
    let sub = args.first().copied().unwrap_or("").to_ascii_lowercase();
    match sub.as_str() {
        "" | "help" | "?" => set_ok(
            chat,
            shell,
            "用法: /api-key status · /api-key set <密钥> · /api-key clear（密钥不进对话记录）",
        ),
        "status" => {
            let has = client_llm_storage_has_api_key();
            set_ok(
                chat,
                shell,
                if has {
                    "本机已设置 client_llm API 密钥（值已隐藏）"
                } else {
                    "本机尚未设置 client_llm API 密钥"
                },
            );
        }
        "clear" => {
            let shell = shell.clone();
            spawn_local(async move {
                match clear_client_llm_api_key_storage_async(loc).await {
                    Ok(_) => set_ok(chat, &shell, "已清除 client_llm API 密钥"),
                    Err(e) => set_err(&shell, format!("清除失败: {e}")),
                }
            });
        }
        "set" => {
            let secret = args.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
            let secret = secret.trim().to_string();
            if secret.is_empty() {
                set_ok(chat, shell, "用法: /api-key set <密钥>");
                return;
            }
            let (base, model, temp, ctx_tok, think) = load_client_llm_text_fields_from_storage();
            let shell = shell.clone();
            spawn_local(async move {
                match persist_client_llm_to_storage_async(
                    &base,
                    &model,
                    &temp,
                    &ctx_tok,
                    &think,
                    Some(&secret),
                    loc,
                )
                .await
                {
                    Ok(Some(PersistKind::BrowserInsecure)) => set_ok(
                        chat,
                        &shell,
                        "已写入 API 密钥（不进对话；浏览器弱持久化，建议使用壳）",
                    ),
                    Ok(_) => set_ok(chat, &shell, "已写入 API 密钥（不进对话；已存本机钥匙串）"),
                    Err(e) => set_err(&shell, format!("写入失败: {e}")),
                }
            });
        }
        _ => set_ok(
            chat,
            shell,
            "用法: /api-key status · /api-key set <密钥> · /api-key clear",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crabmate_client_api::slash_caps::supported;

    /// 拦截判定与能力表必须互为镜像：表里有 → 拦得住；表里没有 → 原样发给模型。
    ///
    /// 这条以前是「两份手写名单」（拦截一份、分派一份），漏一处就是
    /// 「拦下来没人实现」或「实现了却敲不出来」—— 现在两侧都指向 `slash_caps`。
    #[test]
    fn caps_and_the_intercept_list_cannot_drift() {
        for head in supported(Surface::Desktop) {
            let line = format!("/{}", head.as_str());
            assert!(is_web_control_slash(&line), "{line} 在 caps 里却没被拦截");
        }
        for head in Head::ALL {
            if supports(Surface::Desktop, *head) {
                continue;
            }
            let line = format!("/{}", head.as_str());
            assert!(
                !is_web_control_slash(&line),
                "{line} 不在 caps 里却被拦截 —— 会变成「拦下来没人实现」"
            );
        }
    }

    /// issue #444 之后这六条从「Web 独占」升级为共享 head：判定来源换了，
    /// 「敲出来不发模型」的契约没变。
    #[test]
    fn shared_info_heads_are_intercepted() {
        for line in [
            "/agent", "/export", "/config", "/context", "/skills", "/version",
        ] {
            assert!(is_web_control_slash(line), "{line} 是共享命令却没被拦截");
        }
        // 真正 Web 独占的三条（含无连字符的肌肉记忆写法）。
        for line in ["/api-base", "/apibase", "/clear", "/api-key", "/apikey"] {
            assert!(is_web_control_slash(line), "{line} 是端独占命令却没被拦截");
        }
        // 大小写与前缀空白同样认（`slash_head` 负责归一）。
        assert!(is_web_control_slash("  /VERSION  "));
        assert!(is_web_control_slash("/Agent list"));
    }

    #[test]
    fn unknown_and_skill_slashes_pass_through() {
        // `/my-skill` 是运行期才知道的 skill；`/data/proj` 是路径。都不该被拦。
        for line in ["/my-skill", "/data/proj", "hello", "/", ""] {
            assert!(!is_web_control_slash(line), "{line} 不该被当成控制命令");
        }
    }

    /// `/help` 正文必须覆盖本端拦截集：`/version` 曾经「能敲但 help 不写」。
    #[test]
    fn help_text_lists_every_intercepted_head() {
        for head in supported(Surface::Desktop) {
            let needle = format!("/{}", head.as_str());
            assert!(HELP_TEXT.contains(&needle), "help 漏了 {needle}");
        }
        for name in [
            "/agent",
            "/export",
            "/config",
            "/api-base",
            "/context",
            "/clear",
            "/api-key",
            "/skills",
            "/version",
            "/mode",
            "/role",
            "/perm",
            "/status",
        ] {
            assert!(HELP_TEXT.contains(name), "help 漏了 {name}");
        }
    }
}
