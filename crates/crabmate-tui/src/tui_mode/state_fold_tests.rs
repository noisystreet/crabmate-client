use crate::tui_mode::render;

use super::super::{LineKind, UiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{FoldTarget, parse_fold_arg};

/// 造一份最小 transcript（`push_line` 走真实路径，行号与 `search_hits` 才一致）。
fn state_with(rows: &[(LineKind, &str)]) -> UiState {
    let mut st = UiState::new();
    for (kind, text) in rows {
        st.push_line(*kind, text);
    }
    st
}

fn visible(st: &UiState) -> Vec<(LineKind, String)> {
    st.lines
        .iter()
        .filter(|l| !l.is_hidden())
        .map(|l| (l.kind(), l.text().to_string()))
        .collect()
}

fn hidden_count(st: &UiState) -> usize {
    st.lines.iter().filter(|l| l.is_hidden()).count()
}

/// 第 `idx` 条逻辑行在**真实渲染**里的前缀（用户唯一看得见的状态：
/// 折叠 `▸ ` / 展开 `▾ `）。必须走 `build_body_rows`：字段读对了但缓存没失效，
/// 屏幕上照样是旧标记 —— 那种情况只有渲染断言能抓到。
fn summary_prefix(st: &UiState, idx: usize) -> String {
    render::build_body_rows(st, 40)
        .iter()
        .find(|r| r.log_index == Some(idx))
        .and_then(|r| r.line.spans.first().map(|s| s.content.to_string()))
        .unwrap_or_default()
}

/// 一轮的工具行折成一条摘要，原文全部保留。
#[test]
fn a_run_of_tool_rows_folds_into_one_summary() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ read_file — a.rs"),
        (LineKind::Tool, "✓ search_replace — b.rs"),
        (LineKind::Assistant, "答"),
    ]);

    st.fold_process_runs(Some(" · 完成"));

    assert_eq!(
        visible(&st),
        vec![
            (LineKind::User, "问".to_string()),
            (LineKind::Process, "2 个工具 · 完成".to_string()),
            (LineKind::Assistant, "答".to_string()),
        ],
        "一轮只剩摘要一条，答案不动"
    );
    assert_eq!(hidden_count(&st), 2, "原文行仍留在 `lines` 里");
    assert_eq!(
        st.lines
            .iter()
            .filter(|l| l.kind() == LineKind::Process)
            .count(),
        1,
        "摘要行恰好一条"
    );
}

/// 展开 / 折回双向都逐字还原（不是重新概括），且摘要行始终在。
#[test]
fn toggling_restores_the_hidden_rows_verbatim_both_ways() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ read_file — a.rs"),
        (LineKind::Tool, "✓ search_replace — b.rs"),
        (LineKind::Assistant, "答"),
    ]);
    st.fold_process_runs(Some(" · 完成"));
    let summary = st
        .lines
        .iter()
        .position(|l| l.kind() == LineKind::Process)
        .expect("应有摘要行");

    assert_eq!(st.toggle_process_run(summary), Some(true), "第一次是展开");
    assert_eq!(
        visible(&st),
        vec![
            (LineKind::User, "问".to_string()),
            // 摘要行**保留**：它是 toggle 的把手，删掉就再也折不回去。
            (LineKind::Process, "2 个工具 · 完成".to_string()),
            (LineKind::Tool, "✓ read_file — a.rs".to_string()),
            (LineKind::Tool, "✓ search_replace — b.rs".to_string()),
            (LineKind::Assistant, "答".to_string()),
        ],
        "展开后成员与折叠前逐字一致，另加摘要行本身"
    );
    assert_eq!(hidden_count(&st), 0, "成员行全部回到可见");
    assert_eq!(st.lines.len(), 5, "toggle 不插不删行");
    assert!(st.lines[summary].is_run_open(), "摘要行记着自己的开合");
    assert_eq!(summary_prefix(&st, summary), "▾ ", "展开态渲染出 ▾");

    assert_eq!(st.toggle_process_run(summary), Some(false), "第二次是折回");
    assert_eq!(
        visible(&st),
        vec![
            (LineKind::User, "问".to_string()),
            (LineKind::Process, "2 个工具 · 完成".to_string()),
            (LineKind::Assistant, "答".to_string()),
        ],
        "折回后只剩摘要"
    );
    assert_eq!(hidden_count(&st), 2);
    assert_eq!(st.lines.len(), 5, "折回同样不改行数");
    assert!(!st.lines[summary].is_run_open());
    assert_eq!(summary_prefix(&st, summary), "▸ ", "折回态渲染回 ▸");

    // 折回后把 `sticky` 清干净了，所以还能再展开一次（否则这段就锁死了）。
    assert_eq!(st.toggle_process_run(summary), Some(true), "折回后可再展开");
    assert_eq!(hidden_count(&st), 0);
}

/// `/fold` 的参数解析：无参 = 最近一条，数字从底部数，`all` / `close` 两个方向。
#[test]
fn fold_arg_parsing() {
    assert_eq!(parse_fold_arg(None), Ok(FoldTarget::Last));
    assert_eq!(parse_fold_arg(Some("   ")), Ok(FoldTarget::Last));
    assert_eq!(parse_fold_arg(Some("ALL")), Ok(FoldTarget::All));
    assert_eq!(parse_fold_arg(Some("close")), Ok(FoldTarget::Close));
    assert_eq!(parse_fold_arg(Some("off")), Ok(FoldTarget::Close));
    assert_eq!(parse_fold_arg(Some("2")), Ok(FoldTarget::Nth(2)));
    // `0` 与 `1` 是两种错法：0 越界，1 等价无参 —— 所以只认 `>= 1`。
    assert!(parse_fold_arg(Some("0")).is_err());
    assert!(parse_fold_arg(Some("x")).is_err(), "非数字给用法提示");
}

/// `/fold <n>` 从底部数：`2` 命中的是**较早**那一段，不是最近那段。
#[test]
fn fold_with_an_index_unfolds_the_nth_run_from_the_bottom() {
    let mut st = state_with(&[
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "中间结论"),
        (LineKind::Tool, "✓ c"),
        (LineKind::Tool, "✓ d"),
    ]);
    st.fold_process_runs(Some(" · 完成"));
    assert_eq!(hidden_count(&st), 4, "两段都折上了");

    st.fold_command(Some("2"));

    let open: Vec<bool> = st
        .lines
        .iter()
        .filter(|l| l.kind() == LineKind::Process)
        .map(|l| l.is_run_open())
        .collect();
    assert_eq!(open, vec![true, false], "只有倒数第 2 条（较早那段）展开");
    assert_eq!(hidden_count(&st), 2, "最近那段仍折叠");
}

/// 越界的 `n` 只给提示，不动任何一段。
#[test]
fn fold_with_a_too_large_index_only_explains_itself() {
    let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
    st.fold_process_runs(None);

    st.fold_command(Some("3"));

    assert_eq!(hidden_count(&st), 2, "越界不该展开任何段");
    let last = st.lines.last().expect("应有提示行");
    assert_eq!(last.kind(), LineKind::System);
    assert!(last.text().contains("只有 1 条"), "{}", last.text());
}

/// `/fold all` 全展开、`/fold close` 全折回，两个方向分别回显改了几段。
#[test]
fn fold_all_and_close_move_every_run() {
    let mut st = state_with(&[
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "中间结论"),
        (LineKind::Tool, "✓ c"),
        (LineKind::Tool, "✓ d"),
    ]);
    st.fold_process_runs(None);

    st.fold_command(Some("all"));
    assert_eq!(hidden_count(&st), 0, "全部展开");
    assert!(
        st.lines
            .last()
            .expect("回显")
            .text()
            .contains("已展开 2 段")
    );
    // 再按一次 `all`：一段都没动，就不能照抄上一次的「已展开 2 段」——
    // 用户会据此以为画面该变了。
    st.fold_command(Some("all"));
    assert_eq!(hidden_count(&st), 0, "重复 all 不该改变任何东西");
    assert!(
        st.lines
            .last()
            .expect("回显")
            .text()
            .contains("都已是展开态"),
        "{}",
        st.lines.last().expect("回显").text()
    );

    st.fold_command(Some("close"));
    assert_eq!(hidden_count(&st), 4, "全部折回");
    assert!(
        st.lines
            .last()
            .expect("回显")
            .text()
            .contains("已折回 2 段")
    );
    // `close` 同理：都已经收起时说「都已是收起态」，而不是再报一次「已折回 2 段」，
    // 更不是「没有可折回的执行过程」（那读起来像「这里没有执行过程」）。
    st.fold_command(Some("close"));
    assert!(
        st.lines
            .last()
            .expect("回显")
            .text()
            .contains("都已是收起态"),
        "{}",
        st.lines.last().expect("回显").text()
    );
    assert!(
        st.lines
            .iter()
            .filter(|l| l.kind() == LineKind::Process)
            .all(|l| !l.is_run_open()),
        "折回后没有一条仍记着展开"
    );
}

/// 一条摘要都没有时，`/fold` 说明「没有可展开的」而不是静默。
#[test]
fn fold_without_any_summary_says_so() {
    let mut st = state_with(&[(LineKind::User, "问"), (LineKind::Assistant, "答")]);

    st.fold_command(None);

    let last = st.lines.last().expect("应有提示行");
    assert_eq!(last.text(), "没有可展开的执行过程");
}

/// 非法参数给用法，且不碰 transcript 里已有的过程段。
#[test]
fn fold_with_a_bad_argument_prints_usage() {
    let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
    st.fold_process_runs(None);

    st.fold_command(Some("bogus"));

    assert_eq!(hidden_count(&st), 2, "参数错不该展开任何段");
    let last = st.lines.last().expect("应有用法行");
    assert!(last.text().contains("用法：/fold"), "{}", last.text());
}

/// 一条过程行不值得折：折了只是换个说法。
#[test]
fn a_single_process_row_is_left_alone() {
    let mut st = state_with(&[(LineKind::User, "问"), (LineKind::Tool, "✓ a")]);

    st.fold_process_runs(None);

    assert_eq!(visible(&st).len(), 2, "只有一条工具行时不折");
    assert_eq!(hidden_count(&st), 0);
}

/// 连续的过程行才算一段：`Assistant` 会把它们切开（答案是结果，不能折）。
#[test]
fn assistant_rows_split_the_runs() {
    let mut st = state_with(&[
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "中间结论"),
        (LineKind::Tool, "✓ c"),
        (LineKind::Tool, "✓ d"),
    ]);

    st.fold_process_runs(None);

    assert_eq!(
        visible(&st),
        vec![
            (LineKind::Process, "2 个工具".to_string()),
            (LineKind::Assistant, "中间结论".to_string()),
            (LineKind::Process, "2 个工具".to_string()),
        ],
        "答案两侧各折一段，答案本身永不被折"
    );
}

/// 混合时把「步」和「工具」分开说。
#[test]
fn a_mixed_run_reports_steps_and_tools() {
    let mut st = state_with(&[
        (LineKind::Thinking, "先看看"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
    ]);

    st.fold_process_runs(Some(" · 完成"));

    assert_eq!(visible(&st)[0].1, "3 步 · 2 个工具 · 完成");
}

/// 回看中折叠：记下视口顶端那条逻辑行，供下一帧钉回视线。
#[test]
fn folding_while_scrolled_back_records_an_anchor() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
    ]);
    st.view_offset = 7;
    st.body_top_log = Some(1);

    st.fold_process_runs(None);

    assert_eq!(st.reanchor_log, Some(1), "非贴底时必须钉回原视线");
}

/// 贴底折叠不动视口；贴顶则继续贴顶（与 `/older` 前插同一条规矩）。
#[test]
fn folding_at_the_bottom_leaves_the_viewport_alone() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
    ]);

    st.fold_process_runs(None);

    assert_eq!(st.reanchor_log, None, "贴底时画面本来就不动");
    assert_eq!(st.view_offset, 0);
}

/// 隐藏段不该在屏幕上留下一条孤立分隔线：块间距按「最后一条可见行」算。
#[test]
fn a_hidden_run_leaves_no_orphan_separator() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "答"),
    ]);
    st.fold_process_runs(None);

    let rows = render::build_body_rows(&st, 40);
    let is_sep: Vec<bool> = rows.iter().map(|r| r.log_index.is_none()).collect();
    assert!(
        !is_sep.windows(2).any(|w| w[0] && w[1]),
        "相邻两条分隔行：{is_sep:?}"
    );
    assert_eq!(
        rows.iter().filter(|r| r.log_index.is_some()).count(),
        3,
        "可见行 = 问 / 摘要 / 答"
    );
}

/// 回看中折叠后，锚点必须指向折叠前那条逻辑行：它上面的段各插了一条摘要行，
/// 旧下标已经偏了。上一版只断言「记了锚点」，记到错行也能通过。
#[test]
fn the_anchor_follows_the_rows_inserted_above_it() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "答"),
        (LineKind::Tool, "✓ c"),
        (LineKind::Tool, "✓ d"),
    ]);
    st.view_offset = 5;
    st.body_top_log = Some(3); // 折叠前「答」所在的行

    st.fold_process_runs(None);

    let anchor = st.reanchor_log.expect("回看中应记锚点");
    assert_eq!(anchor, 4, "中间插进一条摘要行，锚点要跟着 +1");
    assert_eq!(st.lines[anchor].text(), "答", "锚点仍指向原来那一行");
}

/// 锚点落在被折的段里时，指向段首那条摘要行 —— 被隐藏的成员行根本不产出物理行。
#[test]
fn an_anchor_inside_a_folded_run_lands_on_its_summary() {
    let mut st = state_with(&[
        (LineKind::User, "问"),
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
    ]);
    st.view_offset = 3;
    st.body_top_log = Some(2); // 落在工具行里

    st.fold_process_runs(None);

    let anchor = st.reanchor_log.expect("回看中应记锚点");
    assert_eq!(
        st.lines[anchor].kind(),
        LineKind::Process,
        "被折掉的行没有物理行，锚点只能落在摘要行上"
    );
}

// ── 折叠光标（`Ctrl+G`） ─────────────────────────────

fn nav_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// 两段过程摘要的 transcript：`[摘要A, 工具A×2, 中间结论, 摘要B, 工具B×2]` 的折叠态。
fn two_folded_runs() -> UiState {
    let mut st = state_with(&[
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "中间结论"),
        (LineKind::Tool, "✓ c"),
        (LineKind::Tool, "✓ d"),
    ]);
    st.fold_process_runs(None);
    st
}

/// 摘要行的下标（自上而下）。
fn summary_indices(st: &UiState) -> Vec<usize> {
    st.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind() == LineKind::Process)
        .map(|(idx, _)| idx)
        .collect()
}

/// `Ctrl+G` 落在**最近一条**摘要上（与 `/fold` 的手感一致）。
#[test]
fn fold_nav_starts_on_the_most_recent_summary() {
    let mut st = two_folded_runs();
    let idxs = summary_indices(&st);
    assert_eq!(idxs.len(), 2);

    st.begin_fold_nav();

    assert_eq!(st.fold_cursor, Some(idxs[1]), "落点是最下面那条");
}

/// 一条摘要都没有时不进导航，只留一句提示（与 `/fold` 同一个说法）。
#[test]
fn fold_nav_without_any_summary_only_explains_itself() {
    let mut st = state_with(&[(LineKind::User, "问"), (LineKind::Assistant, "答")]);

    st.begin_fold_nav();

    assert_eq!(st.fold_cursor, None, "没东西可导航");
    assert_eq!(
        st.lines.last().expect("提示行").text(),
        "没有可展开的执行过程"
    );
}

/// `↑/↓` 在摘要之间跳，到两端就停（不 wrap：与两侧列表的导航一致）。
#[test]
fn arrows_walk_between_summaries_and_stop_at_the_ends() {
    let mut st = two_folded_runs();
    let idxs = summary_indices(&st);
    st.begin_fold_nav();
    assert_eq!(st.fold_cursor, Some(idxs[1]));

    assert!(st.fold_nav_key(&nav_key(KeyCode::Up)), "↑ 归导航");
    assert_eq!(st.fold_cursor, Some(idxs[0]), "↑ 跳到更早那段");
    assert!(st.fold_nav_key(&nav_key(KeyCode::Up)));
    assert_eq!(st.fold_cursor, Some(idxs[0]), "到顶就停");

    assert!(st.fold_nav_key(&nav_key(KeyCode::Down)));
    assert_eq!(st.fold_cursor, Some(idxs[1]), "↓ 回到最近那条");
    assert!(st.fold_nav_key(&nav_key(KeyCode::Down)));
    assert_eq!(st.fold_cursor, Some(idxs[1]), "到底也停");
}

/// `Enter` / `Space` 切换光标那条，**光标不挪**：第二下按它是为了折回。
#[test]
fn enter_toggles_the_cursor_summary_and_keeps_the_cursor() {
    let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
    st.fold_process_runs(None);
    st.begin_fold_nav();
    let idx = st.fold_cursor.expect("进了导航");
    assert_eq!(hidden_count(&st), 2);

    assert!(st.fold_nav_key(&nav_key(KeyCode::Enter)));
    assert_eq!(hidden_count(&st), 0, "Enter 展开");
    assert_eq!(st.fold_cursor, Some(idx), "光标留在原地");
    assert_eq!(summary_prefix(&st, idx), "▾ ");

    assert!(st.fold_nav_key(&nav_key(KeyCode::Enter)));
    assert_eq!(hidden_count(&st), 2, "再按一下 Enter 折回");
    assert_eq!(summary_prefix(&st, idx), "▸ ");
    assert_eq!(st.fold_cursor, Some(idx), "光标仍留在原地");
}

/// 空格与 `Alt+Enter` 都不归导航。
///
/// 空格是**可打印字符** —— 占了它就等于「导航期间写不出 `a b`」，而用户无从预料；
/// `Alt+Enter` 与输入框的「换行」是同一个键，抢走它就等于让导航悄悄变成模态。
#[test]
fn space_and_alt_enter_pass_through_while_navigating() {
    let mut st = two_folded_runs();
    st.begin_fold_nav();
    let before = hidden_count(&st);

    assert!(
        !st.fold_nav_key(&nav_key(KeyCode::Char(' '))),
        "空格要交回输入框"
    );
    let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
    assert!(!st.fold_nav_key(&alt_enter), "Alt+Enter 是换行");

    assert_eq!(hidden_count(&st), before, "两者都不该切换");
    assert!(st.fold_cursor.is_some(), "导航还开着");
}

/// 进导航 = 接管方向键**和视口**：`/find` 的锚点与 `/` 补全菜单都必须让位。
///
/// 搜索锚点优先于折叠光标，留着它就会出现「↑/↓ 在动、画面却钉在命中行上」；补全菜单
/// 则和导航抢同一组键 —— 两个宿主同时在线，用户没法判断 ↑/↓ 会动谁。
#[test]
fn beginning_navigation_takes_over_the_viewport_and_the_menu() {
    let mut st = state_with(&[
        (LineKind::Tool, "✓ a"),
        (LineKind::Tool, "✓ b"),
        (LineKind::Assistant, "含关键词的答案"),
    ]);
    st.fold_process_runs(None);
    assert!(st.start_search("关键词") > 0);
    assert!(st.search_cursor.is_some(), "先有搜索锚点");
    st.slash.dismissed = false;

    st.begin_fold_nav();

    assert_eq!(st.search_cursor, None, "搜索锚点让位给折叠光标");
    assert!(st.slash.dismissed, "补全菜单一并关掉");
    assert!(st.fold_cursor.is_some());
}

/// `/fold` 在导航开着时把光标带到它刚切换的那条。
///
/// 两套入口说的是同一件事：不联动的话「刚展开的不是光标这条」，接着按 Enter 就折错段。
#[test]
fn folder_command_moves_the_cursor_when_navigating() {
    let mut st = two_folded_runs();
    st.begin_fold_nav();
    let recent = st.fold_cursor.expect("进了导航");

    // `2` = 自底部数第 2 段，也就是更早那条。
    st.fold_command(Some("2"));

    let older = st.fold_cursor.expect("还在导航");
    assert_ne!(older, recent, "光标跟着 /fold 走到更早那段");
    assert_eq!(st.lines[older].kind(), LineKind::Process);
    assert_eq!(summary_prefix(&st, older), "▾ ", "刚展开的就是光标这条");
}

/// 导航没开时 `/fold` 不顺手把人拖进模式。
#[test]
fn folder_command_leaves_the_cursor_alone_when_not_navigating() {
    let mut st = two_folded_runs();

    st.fold_command(Some("2"));

    assert_eq!(st.fold_cursor, None, "键盘模式不该被命令悄悄改掉");
}

/// `Esc` 退出导航；别的键不归它 —— 字母照常交回输入框，导航还开着。
#[test]
fn esc_leaves_navigation_and_other_keys_pass_through() {
    let mut st = two_folded_runs();
    st.begin_fold_nav();

    assert!(
        !st.fold_nav_key(&nav_key(KeyCode::Char('x'))),
        "字母不归导航"
    );
    assert!(st.fold_cursor.is_some(), "按字母不该退出导航");

    assert!(st.fold_nav_key(&nav_key(KeyCode::Esc)));
    assert_eq!(st.fold_cursor, None);
    assert!(
        !st.fold_nav_key(&nav_key(KeyCode::Up)),
        "退出后 ↑ 还给输入框"
    );
}

/// `/older` 前插后再折叠：旧页里的段折成摘要**插在光标上方**，光标必须跟着 +N。
///
/// 这条链上有两处平移（`prepend_replayed` 的 ② 与 `remap_after_fold`），少任一处，
/// 光标都会指向别的段 —— 而那个段此刻可能正好是折叠态，看起来像「↑↓ 卡住了」。
#[test]
fn folding_after_a_prepend_keeps_the_cursor_on_its_own_run() {
    let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
    st.fold_process_runs(None);
    st.begin_fold_nav();
    let own = st.fold_cursor.expect("进了导航");

    // 更早一页：两行连续工具行，前插后会被折成一条摘要插在上方。
    st.prepend_replayed(vec![
        (LineKind::Tool, "✓ old1".to_string(), None),
        (LineKind::Tool, "✓ old2".to_string(), None),
    ]);

    let moved = st.fold_cursor.expect("还在导航");
    assert_eq!(st.lines[moved].kind(), LineKind::Process, "仍指向一条摘要");
    assert_eq!(moved, own + 3, "前插 2 行 + 上方新摘要 1 条");
    assert!(st.lines[moved].text().contains("2 个工具"), "指向自己那段");
}

/// 纯思考段自己看不出成败，结局词听整轮的；且取消优先于「出错」。
///
/// `failed` 把 `interrupted.is_some()` 也算进去，而 Ctrl+C 的常见结局正是「cancel 未送达 /
/// 传输中断」—— 先判 `failed` 会把用户主动取消的回合写成「出错」（见
/// `resume::record_turn_outcome` 对这两个来源的区分）。
#[test]
fn cancel_wins_over_error_for_a_thinking_only_run() {
    let mut st = state_with(&[
        (LineKind::Thinking, "…想想…"),
        (LineKind::Thinking, "…再想想…"),
    ]);

    st.fold_after_turn(true, true);

    assert_eq!(visible(&st)[0].1, "2 段思考 · 已取消");
}

/// 取消只标它真正打断的那一段。
///
/// 一轮里出现多段很常见（模型的中途答案会把过程行断开），而 Ctrl+C 只打断最后一段。
/// 以前把整轮的 `cancelled` 贴到每一段上，于是「工具全 ✓、答案也给了」的段也写着
/// 「已取消」—— 展开一看全是绿的，摘要却在说取消。
#[test]
fn cancel_only_marks_the_run_it_actually_interrupted() {
    let mut st = state_with(&[]);
    // 第一段：两个工具都拿到了结果。
    st.tool_start("c1", "read", Some("a.rs"));
    st.tool_end("c1", "read", Some(true), None);
    st.tool_start("c2", "read", Some("b.rs"));
    st.tool_end("c2", "read", Some(true), None);
    // 中途答案把过程行断开 → 两段。
    st.push_line(LineKind::Assistant, "先看这两个文件");
    // 第二段：一个跑完、一个还在跑的时候被取消。
    st.tool_start("c3", "write", Some("c.rs"));
    st.tool_end("c3", "write", Some(true), None);
    st.tool_start("c4", "write", Some("d.rs"));

    st.fold_after_turn(true, false);

    let shown = visible(&st);
    assert_eq!(shown[0].1, "2 个工具 · 完成", "前一段跑完了：{shown:?}");
    assert_eq!(shown[2].1, "2 个工具 · 已取消", "后一段被打断：{shown:?}");
}

/// 段里有工具判了失败（`✗`）：说「有失败」，不跟着整轮说「完成」。
#[test]
fn a_run_with_a_failed_tool_says_so() {
    let mut st = state_with(&[]);
    st.tool_start("c1", "read", Some("a.rs"));
    st.tool_end("c1", "read", Some(false), None);
    st.tool_start("c2", "read", Some("b.rs"));
    st.tool_end("c2", "read", Some(true), None);

    st.fold_after_turn(false, true);

    assert_eq!(visible(&st)[0].1, "2 个工具 · 有失败");
}

/// 整轮后来出错（比如最后一次模型调用挂了），但这一段工具全成功 —— 这一段仍是「完成」。
#[test]
fn a_finished_run_stays_done_even_if_the_turn_failed_later() {
    let mut st = state_with(&[]);
    st.tool_start("c1", "read", Some("a.rs"));
    st.tool_end("c1", "read", Some(true), None);
    st.tool_start("c2", "read", Some("b.rs"));
    st.tool_end("c2", "read", Some(true), None);

    st.fold_after_turn(false, true);

    assert_eq!(visible(&st)[0].1, "2 个工具 · 完成");
}

/// `/fold` 展开的原文，下一轮结束时不被重新折回去；新的一轮单独成段。
#[test]
fn an_expanded_run_is_not_refolded_on_the_next_turn() {
    let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
    st.fold_process_runs(Some(" · 完成"));
    st.fold_command(None); // 等价 `/fold`
    assert_eq!(visible(&st).len(), 3, "摘要 + 两条原文");

    st.push_line(LineKind::Tool, "✓ c");
    st.push_line(LineKind::Tool, "✓ d");
    st.fold_after_turn(false, false);

    let shown = visible(&st);
    assert_eq!(
        shown
            .iter()
            .filter(|(k, _)| *k == LineKind::Process)
            .count(),
        2,
        "旧段仍展开、新段单独折：{shown:?}"
    );
    assert_eq!(shown.len(), 4, "摘要A + 两条旧原文 + 摘要B：{shown:?}");
}
