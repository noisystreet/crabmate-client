//! 历史回放：`GET /conversation/messages` → transcript 行。
//!
//! 单独成模块而不是写进 `mod.rs`：那里已贴着 920 行门禁（同 `body_memo.rs` 的理由）。
//!
//! 回放是**只读**的：不写会话索引、不做 revision 合并、不产生任何副作用。
//! Desktop 的「切换会话 = 换一份 transcript」在这里同样成立，但**先不清屏**：
//! 屏幕只在拿到内容后才被替换，请求期间保留原内容加一行进度提示。这样请求失败时
//! 用户（连同绑定回滚）还在原来的位置，而不是对着一块空白。
//!
//! 回包是异步的，所以「这次回包还该不该落到屏幕上」必须显式判断（[`should_apply_reply`]）：
//! 等它的这段时间里用户可能已经切走、新建会话或直接开聊。

use crabmate_client_api::{ConversationMessagesResponse, ReplayRow};
use unicode_width::UnicodeWidthChar;

use super::state::{HistoryRequest, LineKind};
use super::tool_summary::tool_end_text;
use super::{TuiApp, WorkerJob};

/// 历史工具行正文只留首行的前若干**显示列宽**。
///
/// 服务端存的是完整输出，一次 `read_file` 就可能上百行；实时路径本来也只显示一行摘要。
/// 按列宽而不是字符数截断：96 个汉字约 192 列，折行之后就不再是「一行摘要」了。
const TOOL_HEAD_WIDTH: usize = 96;

/// 请求回放：记下在途状态 → 绑定会话 → 清空 transcript → 发 worker job。
pub(super) fn replay_into(app: &mut TuiApp<'_>, conversation_id: String, title: Option<&str>) {
    let label = title.unwrap_or("(by id)").to_string();
    // 先记 pending 再绑定：回包到达时要靠它判断「是不是我这次请求」。
    app.st.history_pending = Some(HistoryRequest {
        conversation_id: conversation_id.clone(),
        prev_conversation_id: app.st.conversation_id.clone(),
        title: label.clone(),
    });
    app.st.conversation_id = Some(conversation_id.clone());
    // 先不清屏：拿到内容后才在 `apply_page` 里替换。请求失败时原来的对话还在。
    app.st
        .push_line(LineKind::System, &format!("正在载入会话「{label}」的历史…"));
    let sent = app
        .job_tx
        .send(WorkerJob::ConversationHistory {
            conversation_id,
            before_index: None,
        })
        .is_ok();
    if !sent {
        app.st.history_pending = None;
        app.st
            .push_line(LineKind::System, "载入历史失败：worker 已退出");
    }
}

/// 回灌一页历史；过期回包直接丢弃。
pub(super) fn on_result(
    app: &mut TuiApp<'_>,
    conversation_id: &str,
    result: Result<ConversationMessagesResponse, String>,
) {
    let Some(pending) = app.st.history_pending.as_ref() else {
        return;
    };
    let pending_matches = pending.conversation_id == conversation_id;
    let applies = should_apply_reply(
        app.st.running,
        pending_matches,
        app.st.conversation_id.as_deref() == Some(conversation_id),
    );
    if !applies {
        // 只清「就是这次请求」的 pending：否则会把一个更新的在途请求一起清掉。
        if pending_matches {
            app.st.history_pending = None;
        }
        return;
    }
    let Some(pending) = app.st.history_pending.take() else {
        return;
    };
    match result {
        Ok(resp) => apply_page(app, &pending, &resp),
        Err(e) => match fail_fallback(pending.prev_conversation_id.as_deref()) {
            ReplayFailFallback::RestorePrevious => {
                app.st.conversation_id = pending.prev_conversation_id.clone();
                app.st.push_line(
                    LineKind::System,
                    &format!(
                        "载入会话「{}」的历史失败：{e}（已保持原会话，可直接输入）",
                        pending.title
                    ),
                );
            }
            ReplayFailFallback::StartNewSession => {
                // 没有原会话可回（典型：刚删掉当前会话，删除路径先把绑定清空了）。
                // 就此空着的话，下一条消息会去开一个连索引都不知道的 `conversation_id`
                // —— 左栏、Web、Desktop 都看不见它。所以直接开一条新会话顶上。
                app.st.conversation_id = None;
                app.st.push_line(
                    LineKind::System,
                    &format!(
                        "载入会话「{}」的历史失败：{e}（无原会话可回，正在开新会话）",
                        pending.title
                    ),
                );
                app.new_session();
            }
        },
    }
}

/// 回放失败后往哪儿落。
///
/// 决定了「失败提示怎么说」和「要不要顺势开一条新会话」，抽成纯函数便于单测：
/// [`on_result`] 本身绑着 worker 通道，测不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayFailFallback {
    /// 回到 `prev_conversation_id`：屏幕上原来那段对话还在，位置不动。
    RestorePrevious,
    /// 没有原会话可回：必须开一条新会话，否则下一条消息没有索引行可绑。
    StartNewSession,
}

fn fail_fallback(prev: Option<&str>) -> ReplayFailFallback {
    match prev {
        Some(_) => ReplayFailFallback::RestorePrevious,
        None => ReplayFailFallback::StartNewSession,
    }
}

/// 回包是否还能落到屏幕上。
///
/// 四种情况必须丢弃：
/// - 没有配套的在途请求（`replay_into` 之外来的，或已被用户的动作清掉）；
/// - 不是这次请求（连切两次会话，先发的那个回包晚到）；
/// - 当前绑定已经不是它（用户切走 / `/conv new`）；
/// - 回合在跑——屏幕上已经是新的对话内容，清屏就是丢用户的东西。
fn should_apply_reply(running: bool, pending_matches: bool, current_matches: bool) -> bool {
    !running && pending_matches && current_matches
}

/// 把一页历史写进 transcript。
fn apply_page(app: &mut TuiApp<'_>, pending: &HistoryRequest, resp: &ConversationMessagesResponse) {
    let rows = rows_to_lines(resp);
    app.st.reset_transcript();
    if rows.is_empty() {
        // 空会话（刚建、还没发过消息）：说清可以直接输入，别报「载入了 0 行」。
        app.st.push_line(
            LineKind::System,
            &format!("会话「{}」暂无历史消息（可直接输入）", pending.title),
        );
        return;
    }
    for (kind, text) in rows {
        // 工具结果没有成败信号，统一按 `None`（渲染层的 `replayed` 标记保证
        // 不会被画成「进行中」的进度占位）。
        app.st.push_replayed_line(kind, &text, None);
    }
    app.st
        .push_line(LineKind::System, &summary_line(pending, resp));
}

/// 载入完成后的汇总行。
///
/// 明说「已续接」：启动时自动回放会绑定该会话，之后输入的消息续进它而不是新开会话
/// —— 这一点必须让用户看得见，否则会以为自己在一个新对话里。
fn summary_line(pending: &HistoryRequest, resp: &ConversationMessagesResponse) -> String {
    let mut facts = vec![format!("{} 条消息", resp.total_count)];
    // 服务端没给 revision 时是 0：别打出「revision 0」，那看起来像 bug。
    if resp.revision > 0 {
        facts.push(format!("revision {}", resp.revision));
    }
    if resp.has_older {
        facts.push("更早的消息暂不回放".to_string());
    }
    format!(
        "已续接会话「{}」（{}；要开新会话用 `/conv new`）",
        pending.title,
        facts.join("，")
    )
}

/// 启动后第一次拿到会话列表：把「上次在用的会话」回放出来。
///
/// 只尝试一次：否则每次 `r` 刷新左栏都会重放并冲掉当前 transcript。之后只有用户的
/// 显式动作（左栏 Enter / `/conv use`）才触发回放。
pub(super) fn auto_replay_if_active(app: &mut TuiApp<'_>) {
    if app.index.history_auto_replayed {
        return;
    }
    app.index.history_auto_replayed = true;
    // 屏幕上已经有内容（索引首次拉取失败、用户先聊起来了，之后刷新才成功）：
    // 此时回放会把正在进行的对话冲掉，宁可不回放。
    if !app.st.lines.is_empty() {
        return;
    }
    // CLI `--conversation-id` 显式指定（启动时 transcript 必为空）→ 回放它。
    if let Some(existing) = app.st.conversation_id.clone() {
        replay_into(app, existing, None);
        return;
    }
    let Some(active_id) = app.st.active_session_id.clone() else {
        return;
    };
    let Some(pos) = app.st.sessions.iter().position(|r| r.id == active_id) else {
        return;
    };
    app.st.selected = pos;
    let Some(resume) = app.st.selected_resume() else {
        return;
    };
    let title = app.st.sessions[pos].title.trim().to_string();
    let title = (!title.is_empty()).then_some(title);
    replay_into(app, resume, title.as_deref());
}

/// 响应 → (行类型, 文本)。抽成纯函数便于单测：`apply_page` 剩下的只是作用到 `UiState`。
fn rows_to_lines(resp: &ConversationMessagesResponse) -> Vec<(LineKind, String)> {
    resp.replay_rows()
        .into_iter()
        .map(|row| match row {
            ReplayRow::User(t) => (LineKind::User, t),
            ReplayRow::Thinking(t) => (LineKind::Thinking, t),
            ReplayRow::Assistant(t) => (LineKind::Assistant, t),
            ReplayRow::Tool { name, body } => (LineKind::Tool, tool_history_text(&name, &body)),
        })
        .collect()
}

/// 历史工具行文本：`name (done) — 首行摘要`。
///
/// 服务端只存结果正文、没有 `✓/✗`，所以走 `tool_end_text` 的 `(done)` 形态，
/// 而不是猜一个成败（猜错会让用户以为某次失败的调用成功了）。
fn tool_history_text(name: &str, body: &str) -> String {
    let head = head_excerpt(body.lines().next().unwrap_or(""));
    let note = (!head.is_empty()).then_some(head.as_str());
    tool_end_text(name, None, note)
}

/// 首行按**显示列宽**截断（宽字符按 2 列算）。
fn head_excerpt(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in line.trim().chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > TOOL_HEAD_WIDTH {
            break;
        }
        width += w;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(title: &str) -> HistoryRequest {
        HistoryRequest {
            conversation_id: "c1".to_string(),
            prev_conversation_id: None,
            title: title.to_string(),
        }
    }

    fn response(v: serde_json::Value) -> ConversationMessagesResponse {
        serde_json::from_value(v).expect("response parses")
    }

    #[test]
    fn tool_history_text_uses_done_mark_and_truncates_head() {
        let long = format!("{}\ntail", "x".repeat(TOOL_HEAD_WIDTH * 2));
        let text = tool_history_text("read_file", &long);
        assert!(text.starts_with("read_file (done) — "), "got {text}");
        assert!(text.chars().count() < TOOL_HEAD_WIDTH + 32, "got {text}");
    }

    #[test]
    fn tool_history_text_without_body_keeps_name_only() {
        assert_eq!(tool_history_text("bash", "   "), "bash (done)");
    }

    /// 按列宽而不是字符数：96 个汉字会折两行，必须在折行前截住。
    #[test]
    fn head_excerpt_counts_display_width() {
        let cjk = "汉".repeat(TOOL_HEAD_WIDTH);
        let head = head_excerpt(&cjk);
        assert_eq!(head.chars().count(), TOOL_HEAD_WIDTH / 2, "got {head}");
        // ASCII 一格一列，能放满。
        let ascii = "x".repeat(TOOL_HEAD_WIDTH + 10);
        assert_eq!(head_excerpt(&ascii).chars().count(), TOOL_HEAD_WIDTH);
    }

    #[test]
    fn rows_to_lines_maps_roles_and_folds_tool_body() {
        let resp = response(serde_json::json!({
            "messages": [
                {"role": "user", "content": "问题"},
                {"role": "assistant", "content": "回答", "reasoning_content": "先想想"},
                {"role": "tool", "name": "read_file", "content": "1|fn main() {}\n2|}"}
            ]
        }));
        assert_eq!(
            rows_to_lines(&resp),
            vec![
                (LineKind::User, "问题".to_string()),
                (LineKind::Thinking, "先想想".to_string()),
                (LineKind::Assistant, "回答".to_string()),
                (
                    LineKind::Tool,
                    "read_file (done) — 1|fn main() {}".to_string()
                ),
            ]
        );
    }

    /// 回放失败后的落点：有原会话就回滚，没有就必须开一条新会话（否则下一条消息
    /// 会开一个连索引都不知道的 `conversation_id`，左栏与其它端都看不见它）。
    #[test]
    fn fail_fallback_keeps_previous_or_starts_a_new_session() {
        assert_eq!(
            fail_fallback(Some("c1")),
            ReplayFailFallback::RestorePrevious
        );
        assert_eq!(fail_fallback(None), ReplayFailFallback::StartNewSession);
    }

    /// 四种丢弃条件各自都要挡住；只有全绿才落地。
    #[test]
    fn reply_guard_drops_on_any_stale_signal() {
        // running, pending_matches, current_matches
        let cases = [
            ((true, true, true), false),
            ((false, false, true), false),
            ((false, true, false), false),
            ((false, false, false), false),
            ((false, true, true), true),
        ];
        for ((running, pending, current), want) in cases {
            assert_eq!(
                should_apply_reply(running, pending, current),
                want,
                "running={running} pending={pending} current={current}"
            );
        }
    }

    #[test]
    fn summary_line_omits_zero_revision_and_says_resumed() {
        let resp = response(serde_json::json!({"total_count": 3}));
        let line = summary_line(&pending("调试 read_file"), &resp);
        assert!(line.contains("已续接会话「调试 read_file」"), "got {line}");
        assert!(line.contains("3 条消息"), "got {line}");
        assert!(line.contains("/conv new"), "got {line}");
        assert!(!line.contains("revision"), "revision 0 不该出现：{line}");
        assert!(!line.contains("更早"), "got {line}");
    }

    #[test]
    fn summary_line_reports_revision_and_older_pages() {
        let resp = response(serde_json::json!({
            "total_count": 120,
            "revision": 9,
            "has_older": true
        }));
        let line = summary_line(&pending("x"), &resp);
        assert!(line.contains("revision 9"), "got {line}");
        assert!(line.contains("更早的消息暂不回放"), "got {line}");
    }
}
