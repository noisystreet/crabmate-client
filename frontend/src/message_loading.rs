//! `/chat/stream` 与消息 UI 共用的 **Loading 占位谓词**（普通助手 / 工具行 / attach 冲突扫描）。

use crate::i18n::{self, Locale};
use crate::storage::{StoredMessage, StoredMessageState};

/// 将仍处 `Loading` 的工具时间线占位收口时的文案/状态语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolLoadingFinalizeKind {
    /// 用户点击「停止」。
    UserStopped,
    /// 流已结束或进程重启后仍残留的僵尸「执行中」。
    OrphanStale,
}

impl ToolLoadingFinalizeKind {
    fn status_label(self, loc: Locale) -> &'static str {
        match self {
            Self::UserStopped => i18n::status_tool_stopped_user(loc),
            Self::OrphanStale => i18n::status_tool_interrupted_stale(loc),
        }
    }

    fn reasoning_status_line(self) -> &'static str {
        match self {
            Self::UserStopped => "status: stopped (user)",
            Self::OrphanStale => "status: interrupted (stale)",
        }
    }
}

/// 收口时替换/补写 reasoning 的 `status: running` 行（state 清空后靠它区分完成与中断）。
fn apply_tool_status_line(reasoning: &mut String, status_line: &str) {
    if reasoning.contains("status: running") {
        *reasoning = reasoning.replace("status: running", status_line);
    } else if !reasoning.contains("status: stopped (user)")
        && !reasoning.contains("status: interrupted (stale)")
    {
        if reasoning.trim().is_empty() {
            *reasoning = status_line.to_string();
        } else {
            *reasoning = format!("{}\n{status_line}", reasoning.trim_end());
        }
    }
}

/// 去掉渲染层不该出现的**机器状态行**。
///
/// - `status: running`：只在非 Loading 态剥 —— Loading 时它还是真的；非 Loading 时它是
///   「没走到收口」的残留（崩溃 / 旧版本写入的会话）。
/// - `status: stopped (user)` / `status: interrupted (stale)`：这两个是收口写进
///   `reasoning_text` 的**数据层标记**（见 [`apply_tool_status_line`] /
///   [`ToolLoadingFinalizeKind`]），行首状态符与 `status_label` 已经表达了同一件事；
///   展开区再显示一行未本地化的英文，会让中文卡片中英混杂。
pub(crate) fn strip_machine_status_lines(detail: &str, loading: bool) -> String {
    let has_machine_line = detail.contains("status: running")
        || detail.contains("status: stopped (user)")
        || detail.contains("status: interrupted (stale)");
    if !has_machine_line {
        return detail.to_string();
    }
    detail
        .lines()
        .filter(|line| {
            let t = line.trim();
            let running = t == "status: running";
            let terminal = t == "status: stopped (user)" || t == "status: interrupted (stale)";
            !((running && !loading) || terminal)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// 将仍处 `Loading` 的工具时间线占位收口（清 `Loading`、替换「执行中」文案与 `status: running`）。
pub fn finalize_loading_tool_placeholders(
    messages: &mut [StoredMessage],
    loc: Locale,
    kind: ToolLoadingFinalizeKind,
) {
    let running_label = i18n::status_tool_running(loc);
    let replacement = kind.status_label(loc);
    let status_line = kind.reasoning_status_line();
    for m in messages.iter_mut() {
        if !is_loading_tool_message(m) {
            continue;
        }
        m.state = None;
        apply_tool_status_line(&mut m.reasoning_text, status_line);
        if m.text.contains(running_label) {
            m.text = m.text.replacen(running_label, replacement, 1);
        } else if m.text.trim().is_empty() {
            m.text = i18n::stream_stopped_inline(loc).to_string();
        } else if kind == ToolLoadingFinalizeKind::UserStopped {
            m.text.push_str(i18n::stream_stopped_suffix(loc));
        } else if !m.text.contains(replacement) {
            // 重启/孤儿收口：摘要行常见为「工具：http_fetch」，补短标签避免仍像进行中。
            m.text = format!("{replacement} · {}", m.text.trim());
        }
    }
}

#[inline]
#[must_use]
pub fn stored_message_is_loading(m: &StoredMessage) -> bool {
    m.state.as_ref().is_some_and(StoredMessageState::is_loading)
}

#[inline]
#[must_use]
pub fn is_plain_assistant_message(m: &StoredMessage) -> bool {
    m.role == "assistant" && !m.is_tool
}

#[inline]
#[must_use]
pub fn is_loading_plain_assistant(m: &StoredMessage) -> bool {
    is_plain_assistant_message(m) && stored_message_is_loading(m)
}

#[inline]
#[must_use]
pub fn is_loading_tool_message(m: &StoredMessage) -> bool {
    m.is_tool && stored_message_is_loading(m)
}

/// 任意工具 Loading，或 **非** `except_plain_assistant_id` 的普通助手 Loading（截断再生 attach 门闩）。
#[must_use]
pub fn is_stream_attach_loading_conflict(
    m: &StoredMessage,
    except_plain_assistant_id: &str,
) -> bool {
    if !stored_message_is_loading(m) {
        return false;
    }
    if m.is_tool {
        return true;
    }
    is_plain_assistant_message(m) && m.id != except_plain_assistant_id
}

#[must_use]
pub fn messages_have_any_loading(messages: &[StoredMessage]) -> bool {
    messages.iter().any(stored_message_is_loading)
}

#[must_use]
pub fn messages_have_loading_tool(messages: &[StoredMessage]) -> bool {
    messages.iter().any(is_loading_tool_message)
}

#[must_use]
pub fn is_loading_streaming_assistant_id(m: &StoredMessage, streaming_assistant_id: &str) -> bool {
    m.id == streaming_assistant_id && is_loading_plain_assistant(m)
}

/// post-tool 尾泡已去掉 `Loading` 但仍为普通助手行（peel / 过早 finalize 判定）。
#[must_use]
pub fn is_finalized_plain_assistant(m: &StoredMessage) -> bool {
    is_plain_assistant_message(m) && !stored_message_is_loading(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use crate::storage::StoredMessageState;

    fn msg(role: &str, is_tool: bool, state: Option<StoredMessageState>) -> StoredMessage {
        StoredMessage {
            id: "m1".into(),
            role: role.into(),
            text: String::new(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state,
            is_tool,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    #[test]
    fn plain_assistant_loading_and_conflict_except() {
        let loading = msg("assistant", false, Some(StoredMessageState::Loading));
        assert!(is_loading_plain_assistant(&loading));
        assert!(!is_stream_attach_loading_conflict(&loading, "m1"));
        assert!(is_stream_attach_loading_conflict(&loading, "other"));
    }

    #[test]
    fn tool_loading_conflicts_regardless_of_except() {
        let tool = msg("system", true, Some(StoredMessageState::Loading));
        assert!(is_loading_tool_message(&tool));
        assert!(is_stream_attach_loading_conflict(&tool, "any"));
    }

    #[test]
    fn finalize_orphan_writes_status_line_when_missing_running() {
        let mut messages = vec![StoredMessage {
            id: "t".into(),
            role: "system".into(),
            text: "工具：http_fetch".into(),
            reasoning_text: "tool: http_fetch".into(),
            image_urls: vec![],
            state: Some(StoredMessageState::Loading),
            is_tool: true,
            tool_call_id: None,
            tool_name: Some("http_fetch".into()),
            created_at: 0,
        }];
        finalize_loading_tool_placeholders(
            &mut messages,
            Locale::ZhHans,
            ToolLoadingFinalizeKind::OrphanStale,
        );
        assert!(
            messages[0]
                .reasoning_text
                .contains("status: interrupted (stale)"),
            "{}",
            messages[0].reasoning_text
        );
    }
}
