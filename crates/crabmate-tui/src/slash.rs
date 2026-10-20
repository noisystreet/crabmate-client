//! repl 控制斜杠（不经模型）：`/help` `/workspace` `/conv` `/status` 等。

use std::io::{self, Write};

use crate::serve::{
    ServeClient, conversation_id_for_resume, fetch_web_sessions, fetch_workspace, set_workspace,
};
use anyhow::Result;
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
            | slash::QUIT
            | slash::QUIT_EXIT
            | slash::QUIT_Q
    )
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
  settings panel        full-screen settings panel: run `crabmate-tui tui` then /settings or F2\n\
  /resume               resume the last interrupted run (network drop)\n\
  /workspace [path]     show or set serve workspace root (off=serve default; ~/ expands via local HOME; /cd alias)\n\
  /conv                 show current conversation_id\n\
  /conv list            list Web sessions (user-data)\n\
  /conv new             clear conversation_id (next turn starts fresh)\n\
  /conv use <id>        set conversation_id for continuation\n\
  /conv delete [<id>]   delete a server-persisted conversation (idempotent 204)\n\
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
