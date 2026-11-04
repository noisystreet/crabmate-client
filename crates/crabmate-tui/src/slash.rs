//! repl 控制斜杠（不经模型）：`/help` `/workspace` `/conv` `/status` 等。

use std::io::{self, Write};

use crate::conv_export;
use crate::serve::{
    ServeClient, conversation_id_for_resume, fetch_all_pages, fetch_web_sessions, fetch_workspace,
    set_workspace,
};
use anyhow::Result;
use crabmate_client_api::chat_export::{ExportFormat, export_messages};
use crabmate_client_api::slash_heads::{self, Head};
use crabmate_client_api::{paths, slash};
use serde_json::Value;

/// 是否为本端控制斜杠（应拦截，不发送给模型）。
#[must_use]
pub fn is_control_slash(trimmed: &str) -> bool {
    let Some(head) = slash_head(trimmed) else {
        return false;
    };
    matches!(
        head.as_str(),
        slash::HELP
            | slash::HELP_Q
            | slash::WORKSPACE
            | slash::CD
            | slash::CONV
            | slash::STATUS
            | slash::MODE
            | slash::ROLE
            | slash::PERM
            | slash::QUIT
            | slash::QUIT_EXIT
            | slash::QUIT_Q
            // `/background` 只对全屏 tui 有意义；repl 里也拦住，免得被当成普通消息发给模型。
            | "background"
            | "bg"
            // 同理：`/editor` / `/chat` 切的是全屏 tui 的主视图，repl 里没有可切的东西。
            | "editor"
            | "view"
            | "chat"
    )
}

/// repl 的**前置层**拦截：这几个命令要拿 `SessionPrefs` / 断点状态，走不到
/// [`handle_control_slash`]（那里只有 serve 请求与打印）。
///
/// 返回规范 head（由 `slash_heads::head_of` 归一，别名如 `/exit` 也认），
/// 于是调用方只需对主命令写臂，新增别名不必再改这里。
///
/// 注意 `is_control_slash` 的拦截集**大于** `slash_caps::supported(Surface::Repl)`：
/// `/background` `/editor` `/chat` 在 repl 里没有 handler，但会被拦下来回一句
/// 「只在全屏可用」—— 拦与做是两件事，别把两者当成同一个集合。
#[must_use]
pub fn repl_pre_turn_head(text: &str) -> Option<Head> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?;
    let raw = rest.split_whitespace().next().unwrap_or("");
    match slash_heads::head_of(raw)? {
        head @ (Head::Resume | Head::Model | Head::Mode | Head::Role | Head::Perm) => Some(head),
        _ => None,
    }
}

/// 处理控制斜杠。返回 `Ok(false)` 表示用户请求退出 repl。
pub async fn handle_control_slash(
    client: &ServeClient,
    line: &str,
    conversation_id: &mut Option<String>,
) -> Result<bool> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let head = slash_head(line).unwrap_or_default();
    let args: Vec<&str> = parts.into_iter().skip(1).collect();
    match head.as_str() {
        slash::QUIT | slash::QUIT_EXIT | slash::QUIT_Q => Ok(false),
        slash::HELP | slash::HELP_Q => {
            print_help();
            Ok(true)
        }
        slash::STATUS => {
            handle_status(client).await?;
            Ok(true)
        }
        slash::WORKSPACE | slash::CD => {
            handle_workspace(client, &args, conversation_id).await?;
            Ok(true)
        }
        slash::CONV => {
            handle_conv(client, &args, conversation_id).await?;
            Ok(true)
        }
        // 只对全屏 tui 有意义：给明确提示，别落进下面的 "unknown"。
        "background" | "bg" => {
            eprintln!(
                "/background 只在全屏 tui 里可用：`crabmate-tui tui` 后执行 /background <图片路径>（off 关闭）"
            );
            Ok(true)
        }
        "editor" | "view" | "chat" => {
            eprintln!(
                "/editor 与 /chat 只在全屏 tui 里可用：`crabmate-tui tui` 后执行（前者打开只读文件视图，后者切回聊天）"
            );
            Ok(true)
        }
        other => {
            eprintln!("unknown control slash /{other}; try /help");
            Ok(true)
        }
    }
}

/// `GET /status?view=shell`：打印 serve 当前模型 / 端点 / 会话模式等。
async fn handle_status(client: &ServeClient) -> Result<()> {
    let v: Value = client.get_json(paths::STATUS_SHELL).await?;
    let str_field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("-")
            .to_string()
    };
    let num_field = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .map_or_else(|| "-".to_string(), |n| n.to_string())
    };
    println!("status:               {}", str_field("status"));
    println!("model:                {}", str_field("model"));
    println!("api_base:             {}", str_field("api_base"));
    println!(
        "default_agent_role:   {}",
        str_field("default_agent_role_id")
    );
    println!(
        "default_session_mode: {}",
        str_field("default_session_mode")
    );
    println!("executor_model:       {}", str_field("executor_model"));
    println!("llm_context_tokens:   {}", num_field("llm_context_tokens"));
    println!("context_char_budget:  {}", num_field("context_char_budget"));
    Ok(())
}

fn slash_head(trimmed: &str) -> Option<String> {
    let s = trimmed.trim();
    if !s.starts_with('/') {
        return None;
    }
    s[1..]
        .split_whitespace()
        .next()
        .map(|h| h.to_ascii_lowercase())
}

fn print_help() {
    eprintln!(
        "control slashes (not sent to the model):\n\
  /help                 this text\n\
  /status               show serve model / endpoint / session mode (GET /status)\n\
  /model [name]         set client-side model override for later turns (off to clear)\n\
  /mode ask|plan|act    set session mode for later turns (off to clear)\n\
  /role <id>            set agent role for later turns (off to clear; list via /status)\n\
  /perm <level>         set permission-level intent for later turns (off to clear; ceiling still applies)\n\
  settings panel        full-screen settings panel: run `crabmate-tui tui` then /settings or F2\n\
  /background <path>    full-screen background (tui only; kitty pixel tier, or per-cell colours elsewhere; off clears)\n\
  /resume               resume the last interrupted run (network drop)\n\
  /workspace [path]     show or set serve workspace root (off=serve default; ~/ expands via local HOME; /cd alias)\n\
  /conv                 show current conversation_id\n\
  /conv list            list Web sessions (user-data)\n\
  /conv new             clear conversation_id (next turn starts fresh)\n\
  /conv use <id>        set conversation_id for continuation\n\
  /conv delete [<id>]   delete a server-persisted conversation (idempotent 204)\n\
  /conv export [...]    export a whole conversation: [json|markdown|both] [--out <path>] [--id <id>]\n\
                        (same md/json shape as Desktop; no --out prints to stdout)\n\
  /quit /exit /q        exit repl\n\
model-side slash (sent to serve; serve short-circuits, nothing is persisted):\n\
  /btw <question>       off-the-record side question (no tools, not added to history)"
    );
}

/// `/workspace` 的参数 → `POST` 载荷。
///
/// 清除词（`off` / `none` / `clear` / `default`）给空串 —— serve 用**空路径**表示「恢复默认
/// 根」，而空参已被「查询当前根」占用，所以 repl 一直缺这个能力；其余走 [`crate::workspace_path`]
/// 的 `~` 展开，展不开就在这里拦下，不把字面量 `~` 发给 serve（那边只会报「路径无效」，
/// 看不出真实原因）。抽成纯函数是为了两个分支都能直接测。
fn workspace_set_payload(raw: &str) -> Result<String, String> {
    if crate::workspace_path::is_default_word(raw) {
        return Ok(String::new());
    }
    crate::workspace_path::resolve(raw)
}

async fn handle_workspace(
    client: &ServeClient,
    args: &[&str],
    conversation_id: &Option<String>,
) -> Result<()> {
    if args.is_empty() {
        let info = fetch_workspace(client).await?;
        println!("workspace: {}", info.path);
        return Ok(());
    }
    let raw = args.join(" ");
    let path = match workspace_set_payload(&raw) {
        Ok(p) => p,
        Err(e) => anyhow::bail!("{e}"),
    };
    let set = set_workspace(client, &path).await?;
    if set.is_empty() {
        println!("workspace: (serve default)");
    } else {
        println!("workspace: {set}");
    }
    if conversation_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        eprintln!(
            "note: conversation_id still set; tools now use the new workspace. /conv new if you want a fresh chat."
        );
    }
    Ok(())
}

async fn handle_conv(
    client: &ServeClient,
    args: &[&str],
    conversation_id: &mut Option<String>,
) -> Result<()> {
    let sub = args.first().copied().unwrap_or("show");
    match sub {
        "show" | "status" => {
            print_current_conv(conversation_id);
            Ok(())
        }
        "new" | "clear" => {
            *conversation_id = None;
            println!("conversation_id: (cleared)");
            Ok(())
        }
        "use" | "switch" => {
            let id = args.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
            let Some(id) = id else {
                anyhow::bail!("usage: /conv use <conversation_id>");
            };
            *conversation_id = Some(id.to_string());
            println!("conversation_id={id}");
            Ok(())
        }
        "list" | "ls" => list_web_sessions(client, conversation_id).await,
        "delete" | "rm" => delete_conversation(client, args.get(1).copied(), conversation_id).await,
        "export" => export_conversation(client, &args[1..], conversation_id).await,
        other => anyhow::bail!("unknown /conv subcommand '{other}'; try /help"),
    }
}

/// `/conv delete [<id>|current]`：删除服务端已持久化会话（幂等 204）。
///
/// 缺省作用于当前 repl 的 `conversation_id`；只删服务端记录，**不**触碰
/// Web 侧栏索引（那是 Client 本地数据）。
async fn delete_conversation(
    client: &ServeClient,
    arg: Option<&str>,
    conversation_id: &mut Option<String>,
) -> Result<()> {
    // `current` / 缺省都落到当前 repl 的 conversation_id。
    let explicit = arg
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "current")
        .map(str::to_string);
    let cid = match explicit.or_else(|| {
        conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }) {
        Some(cid) => cid,
        None => anyhow::bail!(
            "usage: /conv delete <conversation_id> (or /conv use <id> first, or 'current')"
        ),
    };
    client
        .delete_no_content(&paths::conversation_delete(&cid))
        .await?;
    if conversation_id.as_deref().map(str::trim) == Some(cid.as_str()) {
        *conversation_id = None;
    }
    println!("deleted server conversation {cid} (idempotent); local sidebar index untouched");
    Ok(())
}

/// `/conv export [json|markdown|both] [--out <路径>] [--id <会话 id>]`：把整段会话导出成
/// Desktop 同形的 Markdown / JSON。
///
/// 不给 `--out` 时写 stdout —— 只在行模式里成立：全屏 `tui` 用 alt-screen，把几万字打进
/// UI 既没人读、也会刷掉当前对话，那边默认落一个文件（见 `tui_mode::export`）。
///
/// 翻页 / 合并 / 渲染与全屏共用（`serve::fetch_all_pages` +
/// `crabmate_client_api::chat_export` + [`crate::conv_export`]），这里只决定出口。
async fn export_conversation(
    client: &ServeClient,
    args: &[&str],
    conversation_id: &Option<String>,
) -> Result<()> {
    let cmd = conv_export::parse_args(&args.join(" ")).map_err(|e| anyhow::anyhow!("{e}"))?;
    let bound = cmd
        .id
        .clone()
        .or_else(|| conversation_id.clone())
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let Some(cid) = bound else {
        anyhow::bail!(
            "{}（当前会话未绑定：先 /conv use <id>，或给 --id）",
            ExportFormat::USAGE
        );
    };
    let pages = fetch_all_pages(client, &cid).await?;
    let messages = export_messages(&pages);
    let artifacts =
        conv_export::render_artifacts(cmd.format, &messages).map_err(|e| anyhow::anyhow!("{e}"))?;
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    match cmd.out.as_deref() {
        None => {
            conv_export::print_artifacts(&artifacts)?;
            eprintln!(
                "exported {page_count} page(s) / {} message(s) to stdout",
                messages.len()
            );
        }
        Some(out) => {
            let targets =
                conv_export::resolve_targets(&artifacts, Some(out), &conv_export::now_stamp())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            let written = conv_export::write_artifacts(&artifacts, &targets)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            for line in conv_export::report_lines(&artifacts, &written, page_count, messages.len())
            {
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn print_current_conv(conversation_id: &Option<String>) {
    match conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => println!("conversation_id={cid}"),
        None => println!("conversation_id: (none — next chat starts a new server conversation)"),
    }
}

async fn list_web_sessions(client: &ServeClient, conversation_id: &Option<String>) -> Result<()> {
    let list = fetch_web_sessions(client).await?;
    if list.sessions.is_empty() {
        println!("(no web sessions in user-data; use /conv new|use after chatting)");
        print_current_conv(conversation_id);
        return Ok(());
    }
    let current = conversation_id.as_deref().unwrap_or("");
    let mut usable = 0usize;
    for (i, s) in list.sessions.iter().enumerate() {
        let resume = conversation_id_for_resume(s);
        let active = list.active_session_id.as_deref().is_some_and(|a| a == s.id);
        let in_use = resume.is_some_and(|r| !current.is_empty() && current == r);
        let mark = if in_use {
            ">"
        } else if active {
            "*"
        } else {
            " "
        };
        let title = if s.title.trim().is_empty() {
            "(untitled)"
        } else {
            s.title.trim()
        };
        println!("{mark} [{i}] {title}");
        match resume {
            Some(rid) => {
                usable += 1;
                println!("    local_id={}  resume_id={rid}", s.id);
            }
            None => {
                println!(
                    "    local_id={}  resume_id=(none — chat in Web/tui first to bind server id)",
                    s.id
                );
            }
        }
    }
    if usable == 0 {
        println!("hint: no server conversation_id yet; chat once, then /conv list again");
    } else {
        println!("hint: /conv use <resume_id>   (> current repl, * web active)");
    }
    let _ = io::stdout().flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// repl 分成两层拦：`repl_pre_turn_head`（要 SessionPrefs / 断点）与
    /// `handle_control_slash`（serve 请求）。能力表里每一条都必须被其中一层接住，
    /// 否则用户敲了只会原样发给模型 —— 那正是 `/perm` 曾经的形态。
    #[test]
    fn repl_capability_table_is_reachable_from_both_dispatchers() {
        use crabmate_client_api::slash_caps::{Surface, supported};
        for head in supported(Surface::Repl) {
            let line = format!("/{}", head.as_str());
            assert!(
                is_control_slash(&line) || repl_pre_turn_head(&line) == Some(*head),
                "{line} 在 caps 里但两层分派都不认"
            );
        }
    }

    /// 不带 `/` 的行即使首词是 `model` / `resume` 也要发给模型 —— 前置层只认 `/<head>`。
    /// （原在 `main_tests.rs`，随 `repl_pre_turn_head` 一起搬到这里。）
    #[test]
    fn plain_messages_are_not_swallowed_as_pre_turn_control() {
        assert_eq!(repl_pre_turn_head("model gpt-5 please"), None);
        assert_eq!(repl_pre_turn_head("resume where we left"), None);
        assert_eq!(repl_pre_turn_head("  model  "), None);
        assert_eq!(repl_pre_turn_head("mode act now"), None);
        assert_eq!(repl_pre_turn_head("role coder please"), None);
        // 带 `/` 的归一成规范 head（大小写不敏感，别名也认）。
        assert_eq!(repl_pre_turn_head("/model gpt-5"), Some(Head::Model));
        assert_eq!(repl_pre_turn_head("/mode plan"), Some(Head::Mode));
        assert_eq!(repl_pre_turn_head("/ROLE coder"), Some(Head::Role));
        assert_eq!(repl_pre_turn_head("/RESUME"), Some(Head::Resume));
        assert_eq!(repl_pre_turn_head("/perm read"), Some(Head::Perm));
        // 前缀不同、或归给 `handle_control_slash` 的命令都不该被这一层拿走。
        assert_eq!(repl_pre_turn_head("/models"), None);
        assert_eq!(repl_pre_turn_head("/status"), None);
    }

    #[test]
    fn detects_control_slashes() {
        assert!(is_control_slash("/help"));
        assert!(is_control_slash("/HELP"));
        // 退出：`/quit` 与别名 `/exit`、`/q`（大小写不敏感）都要拦截，别发给模型。
        assert!(is_control_slash("/quit"));
        assert!(is_control_slash("/exit"));
        assert!(is_control_slash("/EXIT"));
        assert!(is_control_slash("/q"));
        assert!(is_control_slash("/Workspace /tmp"));
        assert!(is_control_slash("/conv list"));
        assert!(is_control_slash("/conv delete c1"));
        // `/perm` 与 `/mode` `/role` 同类：必须拦截，否则会被当成普通消息发给模型。
        assert!(is_control_slash("/perm read"));
        assert!(is_control_slash("/perm off"));
        // `/btw` 是服务端短路的模型面命令：**不**拦截，原样发给 serve。
        assert!(!is_control_slash("/btw why"));
        assert!(!is_control_slash("/my-skill"));
        assert!(!is_control_slash("hello"));
    }

    /// repl 的 `/workspace` 载荷：清除词给空串（真去恢复默认根），普通路径原样透传。
    ///
    /// `~` 分支挂在 `CM_TUI_EXPAND_TILDE` 与 `$HOME` 上，属 `workspace_path` 自己的单测范围，
    /// 这里只钉住「repl 不再把 `off` 当路径发出去」。
    #[test]
    fn workspace_payload_handles_clear_words_and_plain_paths() {
        for w in ["off", "none", "clear", "default", "  off "] {
            assert_eq!(
                workspace_set_payload(w),
                Ok(String::new()),
                "{w} 应是默认根"
            );
        }
        assert_eq!(workspace_set_payload("/data/proj"), Ok("/data/proj".into()));
        // 近似词照旧当路径：`defaults` 不是清除词。
        assert_eq!(workspace_set_payload("defaults"), Ok("defaults".into()));
    }
}
