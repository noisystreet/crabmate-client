//! repl 控制斜杠（不经模型）：`/help` `/workspace` `/conv` 等。

use std::io::{self, Write};

use anyhow::Result;
use crabmate_tui_core::{
    ServeClient, conversation_id_for_resume, fetch_web_sessions, fetch_workspace, set_workspace,
};

/// 是否为本端控制斜杠（应拦截，不发送给模型）。
#[must_use]
pub fn is_control_slash(trimmed: &str) -> bool {
    let Some(head) = slash_head(trimmed) else {
        return false;
    };
    matches!(
        head.as_str(),
        "help" | "?" | "workspace" | "cd" | "conv" | "quit" | "exit" | "q"
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
        "quit" | "exit" | "q" => Ok(false),
        "help" | "?" => {
            print_help();
            Ok(true)
        }
        "workspace" | "cd" => {
            handle_workspace(client, &args, conversation_id).await?;
            Ok(true)
        }
        "conv" => {
            handle_conv(client, &args, conversation_id).await?;
            Ok(true)
        }
        other => {
            eprintln!("unknown control slash /{other}; try /help");
            Ok(true)
        }
    }
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
  /resume               resume the last interrupted run (network drop)\n\
  /workspace [path]     show or set serve workspace root (/cd alias)\n\
  /conv                 show current conversation_id\n\
  /conv list            list Web sessions (user-data)\n\
  /conv new             clear conversation_id (next turn starts fresh)\n\
  /conv use <id>        set conversation_id for continuation\n\
  /quit                 exit repl"
    );
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
    let path = args.join(" ");
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
        other => anyhow::bail!("unknown /conv subcommand '{other}'; try /help"),
    }
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
        assert!(is_control_slash("/Workspace /tmp"));
        assert!(is_control_slash("/conv list"));
        assert!(!is_control_slash("/my-skill"));
        assert!(!is_control_slash("hello"));
    }
}
