//! 澄清问卷浮层的用例：光标按**字符**走（中文一个字三字节）、intro 的行预算，
//! 以及「回合在跑 / 新回合开始时问卷怎么办」这几条状态转换。
//!
//! 独立成文件与 `state_tests` / `status_tests` 同一手法：`lizard` 的 Rust 词法器对
//! 含花括号的模块内联测试会算错函数区间（见 `chat_classify_tests.rs` 文件头）。

use std::sync::mpsc;

use crabmate_client_api::sse_dispatch::ClarificationFormField;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::test_support::{test_app, test_client, test_prefs};
use crate::serve::ClarificationQuestionnaireInfo;
use crate::tui_mode::state::ClarifyOverlay;

fn info(labels: &[&str]) -> ClarificationQuestionnaireInfo {
    let fields = labels
        .iter()
        .map(|l| ClarificationFormField {
            id: (*l).to_string(),
            label: (*l).to_string(),
            hint: None,
            required: false,
            kind: Default::default(),
        })
        .collect();
    ClarificationQuestionnaireInfo {
        questionnaire_id: "q1".into(),
        intro: "说明".into(),
        fields,
    }
}

fn overlay(labels: &[&str]) -> ClarifyOverlay {
    ClarifyOverlay::new(info(labels))
}

// 状态转换类的用例各自搭 lane（`LaneApp` 借用 `client` / `prefs`，套不进一个
// 返回它的 helper）：四行夹具换来的是「测的就是真的 `LaneApp`」。

/// 插中文后光标是「几个字」而不是「几个字节」：否则第二次退格会删掉半个字，
/// 而 `String::remove` 对非字符边界会 panic。
#[test]
fn chinese_editing_walks_by_chars() {
    let mut ov = overlay(&["名字"]);
    ov.insert_char('中');
    ov.insert_char('文');
    assert_eq!(ov.form.values[0], "中文");
    ov.backspace();
    assert_eq!(ov.form.values[0], "中", "退格只删一个字");
    ov.insert_char('e');
    assert_eq!(ov.form.values[0], "中e");
}

/// 光标左移后插入：插在中间而不是末尾。
#[test]
fn cursor_move_inserts_in_the_middle() {
    let mut ov = overlay(&["名字"]);
    for c in "abcd".chars() {
        ov.insert_char(c);
    }
    ov.move_cursor(false);
    ov.move_cursor(false);
    ov.insert_char('X');
    assert_eq!(ov.form.values[0], "abXcd");
}

/// 到底 / 到头再移动不越界（越界会让 `insert` 的字节偏移算错）。
#[test]
fn cursor_clamps_at_both_ends() {
    let mut ov = overlay(&["名字"]);
    ov.move_cursor(false);
    ov.insert_char('a');
    assert_eq!(ov.form.values[0], "a", "最左再左移仍在 0");
    for _ in 0..5 {
        ov.move_cursor(true);
    }
    ov.insert_char('b');
    assert_eq!(ov.form.values[0], "ab", "最右再右移仍在末尾");
}

/// 换题只在题间循环，且清掉上一次的必答提示（用户已经在改了）。
#[test]
fn focus_wraps_and_clears_error() {
    let mut ov = overlay(&["a", "b"]);
    ov.error = Some(0);
    assert!(ov.focus_next());
    assert_eq!(ov.focus_index(), 1);
    assert_eq!(ov.error, None, "切换题目后旧的必答提示不再适用");
    assert!(ov.focus_next());
    assert_eq!(ov.focus_index(), 0, "末题再往后回到首题");
    assert!(ov.focus_prev());
    assert_eq!(ov.focus_index(), 1, "首题往前回到末题");
}

/// 单题时换题是 no-op（不能让 focus 跳成 1 再被夹回，静默改状态）。
#[test]
fn single_question_does_not_move() {
    let mut ov = overlay(&["only"]);
    assert!(!ov.focus_next());
    assert!(!ov.focus_prev());
    assert_eq!(ov.focus_index(), 0);
}

/// 各题草稿互不串：编辑第 2 题不会动第 1 题。
#[test]
fn drafts_are_per_question() {
    let mut ov = overlay(&["a", "b"]);
    ov.insert_char('x');
    ov.focus_next();
    ov.insert_char('y');
    assert_eq!(ov.form.values[0], "x");
    assert_eq!(ov.form.values[1], "y");
}

/// 回合在跑时按 `Enter` 必须被拒：`dispatch_turn` 会顶掉 `cancel` / `sent_turn`，
/// 两个 SSE 流并发写同一条 lane，先结束的 `TurnDone` 还会把 `running` 清成空闲。
#[test]
fn submit_is_refused_while_a_turn_runs() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    let mut lane = test_app(&client, &mut prefs, rx, tx, job_tx);
    lane.begin_clarify(info(&["名字"]));
    lane.st.clarify_open = true;
    lane.st.running = true;

    lane.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(lane.st.clarify.is_some(), "忙时不能把问卷提交掉");
    assert!(job_rx.try_recv().is_err(), "忙时不能发出第二轮");
}

/// 浮层只在「回合结束且没有新回合接上」时展开。
///
/// 排队消息会在同一个 `TurnDone` 里立刻发下一轮（`running` 又为真），此时开浮层会
/// 盖住新一段 transcript，而其上的 `Enter` 是一条能再发一轮的模态。
#[test]
fn overlay_waits_while_a_turn_is_running() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    let mut lane = test_app(&client, &mut prefs, rx, tx, job_tx);
    lane.begin_clarify(info(&["名字"]));
    lane.st.running = true;

    lane.open_clarify_after_turn();
    assert!(!lane.st.clarify_open, "回合在跑时不能展开浮层");

    lane.st.running = false;
    lane.open_clarify_after_turn();
    assert!(lane.st.clarify_open, "回合结束才展开");
    drop(job_rx);
}

/// 普通回合一旦发出，上一份未提交的问卷就作废。
///
/// 这与 `Esc` 收起时的提示（「这一轮等于没回答」）同一口径：答案只能随**紧接着的
/// 下一条**请求回去。留着它只会在之后每轮结束反复弹出，提交还会带上过期的 id。
#[test]
fn a_new_turn_discards_the_pending_form() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    let mut lane = test_app(&client, &mut prefs, rx, tx, job_tx);
    lane.begin_clarify(info(&["名字"]));
    lane.st.clarify_open = true;
    lane.st.set_input("换个话题");

    lane.on_submit();

    assert!(lane.st.clarify.is_none(), "新回合作废未提交的问卷");
    assert!(!lane.st.clarify_open);
    drop(job_rx);
}

/// 0 题问卷不建表单：否则开出来的是一张「0/0、无题、按 `Enter` 发空 answers」的空表。
#[test]
fn empty_questionnaire_is_ignored() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    let mut lane = test_app(&client, &mut prefs, rx, tx, job_tx);

    lane.begin_clarify(info(&[]));

    assert!(lane.st.clarify.is_none());
    assert!(!lane.st.clarify_open);
    drop(job_rx);
}

/// 长 intro 只占 `INTRO_MAX_ROWS` 行：高度预算按实际行数算，多出来的行会把最后一题
/// （含「必答题还没填」）挤出可视区。
#[test]
fn a_long_intro_stays_within_the_row_budget() {
    let rows = super::wrap_intro(&"说明".repeat(200), 30);
    assert_eq!(rows.len(), super::INTRO_MAX_ROWS, "最多两行");
    assert!(
        rows.last().is_some_and(|r| r.ends_with('…')),
        "截断要留省略号：{rows:?}"
    );
}

/// 短 intro 既不折行也不加省略号；空 intro 仍占一行（预算里就是这么算的）。
#[test]
fn short_and_empty_intro_keep_their_row_count() {
    assert_eq!(super::wrap_intro("请补充", 30), vec!["请补充".to_string()]);
    assert_eq!(super::wrap_intro("", 30), vec![String::new()]);
}

// `Delete` 这两个用例放在**文件末尾**：`lizard` 的 Rust 词法器会把本文件中间那几个
// `#[test]` 的区间一路算到后方的某个 `}`（`fn-nloc` 报告里那条 163 行的
// `chinese_editing_walks_by_chars` 就是它的产物），插在中间会把那条**误算**的 nloc
// 顶过 200 门禁；追加在末尾只增文件行数，不动那些区间。

/// 向后删：删掉光标处那**一个**字（中文按字符算），且光标不动。
#[test]
fn delete_forward_removes_one_char_and_keeps_the_cursor() {
    let mut ov = overlay(&["名字"]);
    for c in "中文".chars() {
        ov.insert_char(c);
    }
    ov.move_cursor(false); // 光标落到「文」前面
    ov.delete_forward();
    assert_eq!(
        ov.form.values[0], "中",
        "删的是光标处那一个字，不是右边全部"
    );
    ov.delete_forward();
    assert_eq!(ov.form.values[0], "中", "光标已在末尾：无事发生");
    ov.move_cursor(false);
    ov.delete_forward();
    assert_eq!(ov.form.values[0], "", "行首也能继续向后删");
}

/// 走真实按键分派：浮层开着时 `Delete` 落在**答案草稿**上。
///
/// 与 `Backspace` 同一层，漏了臂的话这里会红，而不是等用户在终端里发现「没反应」。
#[test]
fn delete_key_edits_the_open_overlay() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut lane = test_app(&client, &mut prefs, rx, tx, job_tx);
    lane.begin_clarify(info(&["名字"]));
    lane.st.clarify_open = true;

    for ch in "ab".chars() {
        lane.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    lane.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    lane.on_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

    let ov = lane.st.clarify.as_ref().expect("问卷还在");
    assert_eq!(ov.form.values[0], "a", "Delete 删掉了光标处的 b");
}
