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

    on_result(&mut app, "B", None, Err("boom".to_string()), None);

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

    on_result(&mut app, "B", None, Err("gone".to_string()), None);

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

/// `/older` 的门控：五类拒绝理由各一例 + 唯一的放行分支带上游标。
#[test]
fn older_gate_refuses_each_reason_and_enqueues_on_the_happy_path() {
    let page = HistoryPage {
        window_start: 40,
        has_older: true,
        loading: false,
    };
    // ① 回合在跑
    assert!(matches!(
        older_gate(true, false, Some("c1"), Some(&page)),
        OlderGate::Refuse(_)
    ));
    // ② 切换会话的回放还在途：游标还是上一个会话的
    assert!(matches!(
        older_gate(false, true, Some("c1"), Some(&page)),
        OlderGate::Refuse(_)
    ));
    // ③ 没有会话绑定 / 只有空白
    assert!(matches!(
        older_gate(false, false, None, Some(&page)),
        OlderGate::Refuse(_)
    ));
    assert!(matches!(
        older_gate(false, false, Some("   "), Some(&page)),
        OlderGate::Refuse(_)
    ));
    // ④ 没有游标（未回放过）
    assert!(matches!(
        older_gate(false, false, Some("c1"), None),
        OlderGate::Refuse(_)
    ));
    // ⑤ 已有一次在途 / 已到最早一条
    let loading = HistoryPage {
        loading: true,
        ..page
    };
    assert!(matches!(
        older_gate(false, false, Some("c1"), Some(&loading)),
        OlderGate::Refuse(_)
    ));
    let exhausted = HistoryPage {
        has_older: false,
        ..page
    };
    assert!(matches!(
        older_gate(false, false, Some("c1"), Some(&exhausted)),
        OlderGate::Refuse(_)
    ));
    // 放行：`before_index` 取游标，会话 id 去掉首尾空白。
    assert_eq!(
        older_gate(false, false, Some(" c1 "), Some(&page)),
        OlderGate::Enqueue {
            conversation_id: "c1".to_string(),
            before_index: 40,
        }
    );
}

/// 回放在途时 `/older` 必须被拒：此刻 `conversation_id` 已改绑新会话，而 `history_page`
/// 仍是上一个会话的游标，放行就会把别的会话的一页前插进来。这里走完整的 `load_older`，
/// 同时断言**真的没有发出请求**，而不只是拒绝文案。
#[test]
fn load_older_is_refused_while_a_session_replay_is_in_flight() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

    // 模拟 `/conv use B`：已改绑，回包还没落地。
    app.st.conversation_id = Some("B".to_string());
    app.st.history_pending = Some(HistoryRequest {
        conversation_id: "B".to_string(),
        prev_conversation_id: Some("A".to_string()),
        title: "B".to_string(),
    });
    app.st.set_history_page(40, true);

    load_older(&mut app);

    assert!(job_rx.try_recv().is_err(), "回放在途时不该发出翻页请求");
    assert!(
        !app.st.history_page.expect("page").loading,
        "在途标记不该被置起"
    );
    let last = app.st.lines.last().expect("应有一行拒绝提示");
    assert!(
        last.text().contains("正在载入会话历史"),
        "got {}",
        last.text()
    );
}

/// 到顶续翻必须**静默**：没有更早时既不请求，也不往 transcript 写「已经是第一条」——
/// 在顶连按 PgUp 是高频动作，每按一次写一行就成了噪音。
#[test]
fn load_older_on_top_is_silent_when_there_is_nothing_older() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

    app.st.conversation_id = Some("c1".to_string());
    app.st.push_line(LineKind::User, "内容");
    app.st.set_history_page(0, false);
    let before = app.st.lines.len();

    load_older_on_top(&mut app);

    assert!(job_rx.try_recv().is_err(), "没有更早时不该发请求");
    assert_eq!(app.st.lines.len(), before, "拒绝理由不该落进 transcript");
}

/// 有游标时到顶续翻与 `/older` 同源：发出请求并置在途标记。
#[test]
fn load_older_on_top_enqueues_when_a_cursor_is_available() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

    app.st.conversation_id = Some("c1".to_string());
    app.st.push_line(LineKind::User, "内容");
    app.st.set_history_page(40, true);

    load_older_on_top(&mut app);

    assert!(app.st.history_page.expect("page").loading, "在途标记要置起");
    assert!(
        matches!(
            job_rx.try_recv(),
            Ok(WorkerJob::ConversationHistory {
                before_index: Some(40),
                ..
            })
        ),
        "到顶续翻要带同一个游标"
    );
}

/// 前插页回包不得清屏：屏幕上已有的最新页原地保留，只在头部多出更早的行。
#[test]
fn older_result_prepends_without_resetting_the_transcript() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

    app.st.conversation_id = Some("c1".to_string());
    app.st.push_line(LineKind::User, "最新的问题");
    app.st.set_history_page(2, true);
    app.st.set_older_loading(true);

    let resp = response(serde_json::json!({
        "total_count": 3,
        "window_start_index": 0,
        "has_older": false,
        "messages": [{"role": "user", "content": "更早的问题"}]
    }));
    on_older_result(&mut app, "c1", Ok(resp));

    let texts: Vec<&str> = app.st.lines.iter().map(|l| l.text()).collect();
    let older = texts.iter().position(|t| t.contains("更早的问题"));
    let newest = texts.iter().position(|t| t.contains("最新的问题"));
    assert!(older.is_some() && newest.is_some(), "{texts:?}");
    assert!(older < newest, "更早的行必须排在前面：{texts:?}");
    assert!(!app.st.has_older_pages(), "window_start=0 应视为到底");
}

/// 切走之后到达的前插回包必须丢掉（否则旧会话的历史会插进新会话）。
#[test]
fn older_result_is_dropped_after_switching_sessions() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

    app.st.conversation_id = Some("c2".to_string());
    app.st.push_line(LineKind::User, "另一个会话");
    app.st.set_history_page(2, true);
    app.st.set_older_loading(true);

    let resp = response(serde_json::json!({
        "messages": [{"role": "user", "content": "旧会话的历史"}]
    }));
    on_older_result(&mut app, "c1", Ok(resp));

    let texts: Vec<&str> = app.st.lines.iter().map(|l| l.text()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("旧会话的历史")),
        "{texts:?}"
    );
    let page = app.st.history_page.expect("page 仍在");
    assert!(!page.loading, "在途标记要复位，否则 /older 再也发不出去");
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
    assert!(line.contains("/older"), "got {line}");
}
