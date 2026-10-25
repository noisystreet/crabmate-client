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

use crabmate_client_api::markdown_normalize::balance_unclosed_fence;
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;
use crabmate_client_api::tool_display::is_placeholder_summary;
use crabmate_client_api::{ConversationMessagesResponse, ReplayRow};
use crabmate_tool_card::parse_tool_envelope;
use unicode_width::UnicodeWidthChar;

use super::state::{HistoryRequest, LineKind};
use super::tool_summary::{flatten_note, tool_end_text};
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
    // 旧会话的用量不能留在状态栏上等新快照覆盖（切换失败时它还会一直在）。
    app.st.clear_usage();
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
    snapshot: Option<TiktokenPromptTokensSnapshot>,
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
    // 用量快照只在这一支应用：回包被判为「不该生效」时（切会话失败 / 回合在途）跳过，
    // 与 Web 端一致 —— 否则会把上一次会话的水位记到当前会话上。
    if let Some(snap) = snapshot {
        app.st.set_prompt_tokens(snap);
    }
    match result {
        Ok(resp) => apply_page(app, &pending, &resp),
        Err(e) => match fail_fallback(pending.prev_conversation_id.as_deref()) {
            ReplayFailFallback::RestorePrevious => {
                // 先换回**原会话**的草稿，再把绑定回滚：`swap_draft_in` 用 `st.conversation_id`
                // 作存回的键，顺序反过来会把目标会话的草稿写进原会话的槽。
                app.swap_draft_in(pending.prev_conversation_id.as_deref());
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
                //
                // 输入框里此刻是**目标会话**的草稿（切走时换进来的），不能让它变成新会话的
                // 开场白：先按目标 id 存回、切到「未绑定」槽，再清绑定。
                app.swap_draft_in(None);
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
    for (kind, text, tool_ok) in rows {
        // `tool_ok` 来自信封的 `ok`：它决定正文颜色（成功绿 / 失败红），不回传就只剩
        // 文案里的 `✓/✗`，与现场的颜色对不上。纯文本结果没有信号，仍是 `None`。
        // 历史行一律记为「已结束」（`push_replayed_line` 内部就是这么落的），
        // 所以不会被画成「进行中」。
        app.st.push_replayed_line(kind, &text, tool_ok);
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

/// 响应 → (行类型, 文本, 工具成败)。抽成纯函数便于单测：`apply_page` 剩下的只是作用到 `UiState`。
/// 第三项只对工具行有值，其余恒 `None`。
///
/// 助手正文先过 [`balance_unclosed_fence`]：回放是**终态**，一条消息里多出的孤儿开符
/// （模型写到一半的 ``` / 服务端展示合并留下的残片）会让 TUI 把余下正文全部涂成代码块
/// （`md::assistant_styled_text` 的 `in_fence` 进去就不出来）。流式期间不能这么做 ——
/// 那时停在未闭合围栏是正常状态，提前「修正」会让代码块在闭合瞬间跳变。
fn rows_to_lines(resp: &ConversationMessagesResponse) -> Vec<(LineKind, String, Option<bool>)> {
    resp.replay_rows()
        .into_iter()
        .map(|row| match row {
            ReplayRow::User(t) => (LineKind::User, t, None),
            ReplayRow::Thinking(t) => (LineKind::Thinking, t, None),
            ReplayRow::Assistant(t) => (LineKind::Assistant, balance_unclosed_fence(&t), None),
            ReplayRow::Tool { name, body } => {
                let (text, ok) = tool_history_row(&name, &body);
                (LineKind::Tool, text, ok)
            }
        })
        .collect()
}

/// 历史工具行：`(文案, 成败)`。
///
/// `role=tool` 落盘的正文有两种形态：纯文本结果，或 `{"crabmate_tool":{…}}` 信封。
/// 后者必须解包——实时路径的 `note` 取的是 serve 的 `metadata.summary`，整段 JSON
/// 直接印出来会让回放和现场差一整条街。信封里的 `ok` 同时决定文案的 `✓`/`✗`
/// **和正文颜色**，所以要一起返回；纯文本结果没有信号 → `None`（不猜成败）。
fn tool_history_row(name: &str, body: &str) -> (String, Option<bool>) {
    if let Some(input) = parse_tool_envelope(body, Some(name)) {
        let note = tool_note(&input.name, input.summary.as_deref(), &input.output);
        return (
            tool_end_text(&input.name, input.ok, note.as_deref()),
            input.ok,
        );
    }
    let note = tool_note(name, None, body);
    (tool_end_text(name, None, note.as_deref()), None)
}

/// 摘要：`summary` 优先，其次结果正文首行；先压成单行，再按显示宽截断。
///
/// 压平必须先做：`summary` 可能是「工具名 \n 工具名: 详情」，只取首行会留下一句纯
/// 工具名（信息全丢），不压平则整段挤进一行、名字出现两次。
fn tool_note(name: &str, summary: Option<&str>, body: &str) -> Option<String> {
    // 占位摘要（Server 摘要推导失败时的 `tool: <name>`）没有信息量，还会把工具行
    // 退回「只有工具名」：丢掉它，宁可回退到结果正文首行。
    let raw = summary
        .filter(|s| !s.trim().is_empty() && !is_placeholder_summary(name, s))
        .or_else(|| body.lines().next())?;
    let flat = flatten_note(name, raw)?;
    Some(head_excerpt(&flat))
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
    use std::sync::mpsc;

    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::*;

    /// 回放时 Server 的占位摘要（`tool: run_command`）必须被丢掉并回退到输出首行：
    /// 否则重开会话的工具行又变回「只有工具名」，正是实时路径刚修掉的那个现象。
    #[test]
    fn replay_drops_placeholder_summary_in_favour_of_output() {
        let note = tool_note(
            "run_command",
            Some("tool: run_command"),
            "git status (exit=0):\n位于分支 main",
        );
        assert_eq!(note.as_deref(), Some("git status (exit=0):"));
    }

    #[test]
    fn replay_prefers_real_summary_over_output() {
        let note = tool_note("run_command", Some("make -C build"), "ignored");
        assert_eq!(note.as_deref(), Some("make -C build"));
    }

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

    /// 回放失败回滚时，草稿必须跟着会话一起退回：屏幕回到 A，输入框也得是 A 的。
    ///
    /// 不修的话，用户对着 A 的 transcript 按回车，发出去的是 B 留下的那段草稿
    /// —— 正是「草稿串会话」这个要消灭的问题，只是入口换到了失败回滚这条路径。
    #[test]
    fn failed_replay_rolls_the_draft_back_with_the_session() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        // 状态：当前在 B，输入框里是 B 的草稿；A 那边也留着半句。
        app.st.conversation_id = Some("B".to_string());
        app.drafts.save(Some("A"), "A 的半句");
        app.drafts.save(Some("B"), "B 的草稿");
        app.st.set_input("B 的草稿");
        app.st.history_pending = Some(HistoryRequest {
            conversation_id: "B".to_string(),
            prev_conversation_id: Some("A".to_string()),
            title: "A".to_string(),
        });

        on_result(&mut app, "B", Err("boom".to_string()), None);

        assert_eq!(
            app.st.conversation_id.as_deref(),
            Some("A"),
            "绑定应回滚到原会话"
        );
        assert_eq!(
            app.st.input_snapshot(),
            "A 的半句",
            "输入框必须跟着回滚，否则这段草稿会发到别的会话里"
        );
        assert_eq!(
            app.drafts.load(Some("B")),
            Some("B 的草稿"),
            "B 的草稿仍留在它自己名下，没有被写坏"
        );
    }

    /// 没有原会话可回（典型：原会话刚被删）：要开一条新会话，但输入框里那条
    /// **目标会话**的草稿不能变成新会话的开场白。
    #[test]
    fn failed_replay_without_a_fallback_keeps_the_draft_off_the_new_session() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.st.conversation_id = Some("B".to_string());
        app.drafts.save(Some("B"), "B 的草稿");
        app.st.set_input("B 的草稿");
        app.st.history_pending = Some(HistoryRequest {
            conversation_id: "B".to_string(),
            prev_conversation_id: None,
            title: "B".to_string(),
        });

        on_result(&mut app, "B", Err("gone".to_string()), None);

        assert_eq!(app.st.conversation_id, None, "应改为新会话");
        assert!(
            app.st.input_snapshot().trim().is_empty(),
            "目标会话的草稿不该被带进新会话：{:?}",
            app.st.input_snapshot()
        );
        assert_eq!(
            app.drafts.load(Some("B")),
            Some("B 的草稿"),
            "草稿仍留在原会话名下"
        );
        assert!(
            app.drafts.load(None).is_none(),
            "「未绑定」槽不该收下这条草稿"
        );
    }

    #[test]
    fn tool_history_row_uses_done_mark_and_truncates_head() {
        let long = format!("{}\ntail", "x".repeat(TOOL_HEAD_WIDTH * 2));
        let (text, ok) = tool_history_row("read_file", &long);
        assert!(text.starts_with("? read_file — "), "got {text}");
        assert!(text.chars().count() < TOOL_HEAD_WIDTH + 32, "got {text}");
        assert_eq!(ok, None, "纯文本结果没有成败信号");
    }

    /// 信封正文必须解包：整段 `{"crabmate_tool":…}` 印出来与实时差一整条街。
    /// `summary` 与实时路径同源（serve `metadata.summary`），`ok` 要一并带出来
    /// —— 它同时决定 `✓` 与正文颜色。
    #[test]
    fn envelope_body_is_unwrapped_like_the_live_path() {
        let body = r#"{"crabmate_tool":{"v":1,"name":"read_file","summary":"读：a.rs","ok":true,"output":"content\nmore"}}"#;
        assert_eq!(
            tool_history_row("read_file", body),
            ("✓ read_file — 读：a.rs".to_string(), Some(true))
        );
    }

    /// 信封没给 `summary` 时退回 output 首行；没给 `ok` 时仍是 `(done)`，不猜成败。
    #[test]
    fn envelope_without_summary_falls_back_to_output_head() {
        let body = r#"{"crabmate_tool":{"v":1,"name":"exec","output":"exit 0\n…"}}"#;
        assert_eq!(
            tool_history_row("exec", body),
            ("? exec — exit 0".to_string(), None)
        );
    }

    #[test]
    fn tool_history_row_without_body_keeps_name_only() {
        assert_eq!(
            tool_history_row("bash", "   "),
            ("? bash".to_string(), None)
        );
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
                (LineKind::User, "问题".to_string(), None),
                (LineKind::Thinking, "先想想".to_string(), None),
                (LineKind::Assistant, "回答".to_string(), None),
                (
                    LineKind::Tool,
                    "? read_file — 1|fn main() {}".to_string(),
                    None,
                ),
            ]
        );
    }

    /// 回放的助手正文先做围栏配平：孤儿开符会让余下正文整段变代码块背景。
    #[test]
    fn replayed_assistant_text_drops_orphan_fence() {
        let resp = response(serde_json::json!({
            "messages": [{
                "role": "assistant",
                "content": "结论\n```rust\n这段正文被孤儿围栏挡住了\n"
            }]
        }));
        let rows = rows_to_lines(&resp);
        let (kind, text, _) = rows.first().expect("应有一行");
        assert_eq!(*kind, LineKind::Assistant);
        assert!(!text.contains("```"), "孤儿开符应被降级：{text:?}");
        assert!(text.contains("结论"), "{text:?}");
        assert!(
            text.contains("这段正文被孤儿围栏挡住了"),
            "正文不能丢：{text:?}"
        );
    }

    /// 信封里的 `ok` 要一路传到渲染层的 `tool_ok`：它决定正文颜色。
    /// 只改文案不改颜色，回放看起来仍与现场不同。
    #[test]
    fn envelope_ok_reaches_the_row_as_tool_ok() {
        let resp = response(serde_json::json!({
            "messages": [{
                "role": "tool",
                "name": "read_file",
                "content": r#"{"crabmate_tool":{"v":1,"name":"read_file","summary":"读：a.rs","ok":false,"output":"denied"}}"#
            }]
        }));
        assert_eq!(
            rows_to_lines(&resp),
            vec![(
                LineKind::Tool,
                "✗ read_file — 读：a.rs".to_string(),
                Some(false)
            )]
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
