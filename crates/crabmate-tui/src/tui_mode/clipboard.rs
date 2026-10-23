//! 把最近一条消息复制到系统剪贴板（`tui_usability.md` W1.4）。
//!
//! 没有做鼠标选区：ratatui 的缓冲是字符格，终端原生选区会把每行两端的填充与滚动
//! 区域一起带走；「复制最近一条助手回复」才是高频诉求，也才有一条能说清的按键语义。
//!
//! 两条投递路径，按环境二选一（[`choose_route`]）：
//! - **OSC52**：剪贴板内容由**本地终端**执行。SSH 到远端跑本程序时，外部工具只会写
//!   远端剪贴板（那里通常根本没有），只有这条路能把正文送回用户手边。
//! - **外部工具**：`wl-copy` / `xclip` / `xsel`。本地会话更可靠，还能拿到退出码。
//!
//! tmux / screen 不转发 OSC52，必须 passthrough 包裹（[`wrap_mux`]），否则序列被复用器吞掉。

use std::io::Write;

use base64::Engine as _;

use super::state::{LineKind, LogLine};

/// 外部剪贴板工具，按优先级排列（Wayland 优先于 X11）。
pub(crate) const TOOLS: [&str; 3] = ["wl-copy", "xclip", "xsel"];

/// 复制投递路径（也用于成功提示，让用户知道内容去了哪）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyRoute {
    /// 终端原生剪贴板（OSC52）。
    Osc52,
    /// 外部剪贴板工具。
    Tool(&'static str),
}

impl CopyRoute {
    #[must_use]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Osc52 => "OSC52",
            Self::Tool(name) => name,
        }
    }
}

/// 最后一条 `kind` 类型、且正文非空的消息；复制目标。
///
/// 只看文本不看折叠态：thinking 折叠与否是显示选择，不该让「复制」拿不到内容。
/// 流式未结束时会复制到当前半截正文——用户在这一刻主动按的键，按最新内容给即可。
pub(crate) fn last_message_text(lines: &[LogLine], kind: LineKind) -> Option<String> {
    lines
        .iter()
        .rev()
        .find(|l| l.kind() == kind && !l.text().trim().is_empty())
        .map(|l| l.text().trim().to_string())
}

/// 选投递路径。
///
/// `override_` 是 `CM_TUI_CLIPBOARD`（`off` / `osc52` / `tool`），认不出的值按未设置处理：
/// 环境变量写错不该让复制直接不可用。
pub(crate) fn choose_route(
    override_: Option<&str>,
    is_ssh: bool,
    tool: Option<&'static str>,
) -> Result<CopyRoute, String> {
    match override_.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("off" | "none" | "0") => Err("CM_TUI_CLIPBOARD 已关闭复制".to_string()),
        Some("osc52") => Ok(CopyRoute::Osc52),
        Some("tool" | "external") => tool
            .map(CopyRoute::Tool)
            .ok_or_else(|| "CM_TUI_CLIPBOARD=tool 但未找到 wl-copy / xclip / xsel".to_string()),
        // 未设置 / 无法识别：SSH 下外部工具写的是远端剪贴板，只能走 OSC52。
        _ if is_ssh => Ok(CopyRoute::Osc52),
        _ => Ok(tool.map_or(CopyRoute::Osc52, CopyRoute::Tool)),
    }
}

/// OSC52 序列：`ESC ] 52 ; c ; <base64> ST`（`c` = clipboard 选择区）。
///
/// 用 ST（`ESC \`）而不是 BEL 终止：BEL 终止的变体在部分终端里会被当成响铃。
pub(crate) fn osc52_sequence(text: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x1b\\")
}

/// 把序列包进 tmux / screen 的 passthrough：`ESC P tmux; <内层> ST`，内层 `ESC` 需双写。
pub(crate) fn wrap_mux(sequence: &str) -> String {
    let escaped = sequence.replace('\x1b', "\x1b\x1b");
    format!("\x1bPtmux;{escaped}\x1b\\")
}

/// 各工具的「从 stdin 读入到剪贴板」参数。
pub(crate) fn tool_args(name: &str) -> &'static [&'static str] {
    match name {
        "xclip" => &["-selection", "clipboard"],
        "xsel" => &["--clipboard", "--input"],
        // wl-copy 默认就读 stdin 写剪贴板。
        _ => &[],
    }
}

/// 按环境选路并真正投递；返回实际用的路径（成功提示要显示它）。
pub(crate) fn deliver(text: &str) -> Result<CopyRoute, String> {
    let override_ = std::env::var("CM_TUI_CLIPBOARD").ok();
    let route = choose_route(override_.as_deref(), in_ssh_session(), find_tool())?;
    match route {
        CopyRoute::Osc52 => write_osc52(text, in_multiplexer())?,
        CopyRoute::Tool(name) => send_to_tool(name, text)?,
    }
    Ok(route)
}

/// `/copy` 的完整流程，返回给用户看的一行结果（成功与失败都是文案）。
///
/// 文案也在这里生成：`mod.rs` 贴着 920 行门禁，事件循环层只负责把它推进 transcript。
/// `arg` 写错时报错而**不**退回默认——打错目标却静默复制了另一种内容，
/// 比直接说「不支持」更容易让人把错东西粘出去。
pub(crate) fn run(lines: &[LogLine], arg: Option<&str>) -> String {
    let kind = match arg {
        None | Some("" | "assistant" | "a" | "last") => LineKind::Assistant,
        Some("user" | "u") => LineKind::User,
        Some(other) => return format!("不支持的 /copy 目标：{other}（支持 assistant / user）"),
    };
    let label = match kind {
        LineKind::User => "你的消息",
        _ => "助手回复",
    };
    let Some(text) = last_message_text(lines, kind) else {
        return format!("没有可复制的{label}");
    };
    let rows = text.lines().count();
    match deliver(&text) {
        Ok(route) => format!("已复制{label}（{rows} 行 → {}）", route.label()),
        Err(err) => format!("复制失败：{err}"),
    }
}

fn env_present(keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

fn in_ssh_session() -> bool {
    env_present(&["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"])
}

fn in_multiplexer() -> bool {
    env_present(&["TMUX", "STY"])
}

fn find_tool() -> Option<&'static str> {
    let path = std::env::var_os("PATH")?;
    TOOLS
        .into_iter()
        .find(|tool| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// 写 OSC52：序列不移动光标，因此可以直接写 stdout，下一帧 ratatui 重绘不受影响。
fn write_osc52(text: &str, multiplexer: bool) -> Result<(), String> {
    let sequence = osc52_sequence(text);
    let payload = if multiplexer {
        wrap_mux(&sequence)
    } else {
        sequence
    };
    let mut stdout = std::io::stdout();
    stdout
        .write_all(payload.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|e| format!("写终端失败：{e}"))
}

fn send_to_tool(name: &str, text: &str) -> Result<(), String> {
    let mut child = std::process::Command::new(name)
        .args(tool_args(name))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{name} 启动失败：{e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("{name} 写入失败：{e}"))?;
    }
    let status = child.wait().map_err(|e| format!("{name} 等待失败：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} 退出码 {}", status.code().unwrap_or(-1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, text: &str) -> LogLine {
        LogLine::replayed(kind, text, false, None)
    }

    #[test]
    fn last_assistant_text_skips_trailing_other_kinds() {
        let lines = vec![
            line(LineKind::User, "问题"),
            line(LineKind::Assistant, "第一版回答"),
            line(LineKind::System, "系统提示"),
            line(LineKind::Tool, "工具行"),
            line(LineKind::Assistant, "最终回答"),
        ];
        assert_eq!(
            last_message_text(&lines, LineKind::Assistant).as_deref(),
            Some("最终回答")
        );
    }

    #[test]
    fn last_message_text_can_target_user_lines() {
        let lines = vec![
            line(LineKind::User, "旧问题"),
            line(LineKind::Assistant, "回答"),
            line(LineKind::User, "新问题"),
        ];
        assert_eq!(
            last_message_text(&lines, LineKind::User).as_deref(),
            Some("新问题")
        );
    }

    #[test]
    fn blank_and_missing_lines_yield_none() {
        assert_eq!(last_message_text(&[], LineKind::Assistant), None);
        let lines = vec![line(LineKind::Assistant, "   \n  ")];
        assert_eq!(last_message_text(&lines, LineKind::Assistant), None);
    }

    #[test]
    fn copies_trimmed_body() {
        let lines = vec![line(LineKind::Assistant, "\n  正文  \n\n")];
        assert_eq!(
            last_message_text(&lines, LineKind::Assistant).as_deref(),
            Some("正文")
        );
    }

    #[test]
    fn osc52_sequence_round_trips_utf8() {
        let text = "多行\n正文 with emoji 🚀";
        let seq = osc52_sequence(text);
        assert!(seq.starts_with("\x1b]52;c;"));
        assert!(seq.ends_with("\x1b\\"));
        let payload = seq
            .trim_start_matches("\x1b]52;c;")
            .trim_end_matches("\x1b\\");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("valid base64");
        assert_eq!(String::from_utf8(decoded).expect("utf8"), text);
    }

    #[test]
    fn mux_wrapper_doubles_inner_escapes() {
        let wrapped = wrap_mux("\x1b]52;c;QQ==\x1b\\");
        assert!(wrapped.starts_with("\x1bPtmux;"));
        assert!(wrapped.ends_with("\x1b\\"));
        // 外层 2 个（`ESC Ptmux;` 前缀 + 终止 `ESC \`）+ 内层原有的 2 个各双写 = 共 6 个。
        assert_eq!(wrapped.matches('\x1b').count(), 2 + 4);
    }

    #[test]
    fn route_off_is_an_error() {
        assert!(choose_route(Some("off"), false, Some("wl-copy")).is_err());
        assert!(choose_route(Some("none"), true, None).is_err());
    }

    #[test]
    fn route_explicit_values_win() {
        assert_eq!(
            choose_route(Some("osc52"), false, Some("wl-copy")),
            Ok(CopyRoute::Osc52)
        );
        assert_eq!(
            choose_route(Some("tool"), false, Some("xclip")),
            Ok(CopyRoute::Tool("xclip"))
        );
        assert!(choose_route(Some("tool"), false, None).is_err());
    }

    #[test]
    fn ssh_prefers_osc52_over_local_tools() {
        // 远端没有可用剪贴板：外部工具即使存在也不该被选。
        assert_eq!(
            choose_route(None, true, Some("wl-copy")),
            Ok(CopyRoute::Osc52)
        );
    }

    #[test]
    fn local_session_prefers_external_tool_but_falls_back_to_osc52() {
        assert_eq!(
            choose_route(None, false, Some("wl-copy")),
            Ok(CopyRoute::Tool("wl-copy"))
        );
        assert_eq!(choose_route(None, false, None), Ok(CopyRoute::Osc52));
    }

    #[test]
    fn unknown_override_falls_back_to_auto() {
        assert_eq!(
            choose_route(Some("whatever"), false, Some("xsel")),
            Ok(CopyRoute::Tool("xsel"))
        );
    }

    #[test]
    fn tool_args_match_each_tool_contract() {
        assert!(tool_args("wl-copy").is_empty());
        assert_eq!(tool_args("xclip"), &["-selection", "clipboard"]);
        assert_eq!(tool_args("xsel"), &["--clipboard", "--input"]);
    }
}
