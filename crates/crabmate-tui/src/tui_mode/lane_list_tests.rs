use super::*;
use crate::tui_mode::inbound::Inbound;
use crate::tui_mode::test_support::{test_client, test_prefs, test_shell_with_jobs};

fn pal() -> Palette {
    Palette::default()
}

/// 测试里的 lane 区行：默认没有待就绪行（那一条路径单独测：`pending_rows_*`）。
fn rows_of(lanes: &[LaneEntry], width: usize, height: u16) -> Vec<Line<'static>> {
    lane_list_rows(lanes, &[], width, height, &pal())
}

/// 只有名字、第一条是当前 lane —— 大多数测试只关心这个。
fn lanes(names: &[&str]) -> Vec<LaneEntry> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| LaneEntry {
            label: (*n).to_string(),
            active: i == 0,
            ..LaneEntry::default()
        })
        .collect()
}

/// 一行的可见文本。`lane_row` 拆成两个 span（头部要单独呼吸换色），
/// 断言「这一行写了什么」必须拼起来看 —— 只读 `spans[0]` 现在只覆盖那 2 列。
fn row_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// 视图标题里的 lane 名：**多 lane 才有**，单 lane 返回 `None`。
///
/// 单 lane 时这个名字恒等于「当前这条」，写进标题只占宽度；多 lane 时它是「这是哪条
/// 连接」的唯一线索（同一个相对路径在两条 lane 里是两个不同的文件）。
#[test]
fn the_current_label_appears_only_with_several_lanes() {
    assert_eq!(current_label(&lanes(&["local"])), None, "单 lane 不必写");
    assert_eq!(current_label(&lanes(&["local", "staging"])), Some("local"));

    // 当前不在第一条时，取的是**当前那条**的名字，不是第一条。
    let mut two = lanes(&["local", "staging"]);
    two[0].active = false;
    two[1].active = true;
    assert_eq!(current_label(&two), Some("staging"));
}

/// 单 lane：标题 1 行 + 1 行底色条，其余行留空（版面高度固定，不随条数拉伸）。
#[test]
fn one_lane_takes_one_row_under_the_title() {
    let rows = rows_of(&lanes(&["local"]), 20, 20);
    // 1（标题）+ 1（那条 lane）= 2 行有内容，其余交给调用方的底色。
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [1/8]");
    // 唯一那条就是当前 lane，故整行都是选中底色。
    assert_eq!(rows[1].spans[0].style.bg, Some(pal().selection_bg));
    assert!(row_text(&rows[1]).contains("local"), "名字要写在这一行里");
}

/// 当前 lane 用选中底色 + 竖条标记；其它 lane 用一档抬升底色，每条各占一行。
#[test]
fn active_lane_is_marked_and_tinted() {
    let rows = rows_of(&lanes(&["a", "b"]), 20, 20);
    // 一条一行：a = rows[1]，b = rows[2]。
    let a = &rows[1];
    let b = &rows[2];
    assert!(a.spans[0].content.starts_with('▎'));
    assert_eq!(a.spans[0].style.bg, Some(pal().selection_bg));
    assert!(!b.spans[0].content.starts_with('▎'));
    assert_eq!(b.spans[0].style.bg, Some(pal().overlay_bg));
}

/// 待就绪行排在真连接之后，各占一行；失败那行带 `✗` 与原因原文。
///
/// 位置是刻意的：`layout::lane_index_at` 只把前 `lanes.len()` 行映射成下标，
/// 所以这些行点不动 —— 点一行却切到某条真 lane，比点不动糟得多。
#[test]
fn pending_rows_follow_the_connections() {
    use crate::tui_mode::pending_lane::{PendingLane, PendingState};

    let pending = vec![
        PendingLane {
            name: "proj-c".to_string(),
            state: PendingState::Starting,
        },
        PendingLane {
            name: "proj-d".to_string(),
            state: PendingState::Failed("工作区不在白名单".to_string()),
        },
    ];
    let rows = lane_list_rows(&lanes(&["local"]), &pending, 40, 20, &pal());
    let texts: Vec<String> = rows.iter().map(row_text).collect();
    assert_eq!(texts.len(), 4, "标题 + 1 条真连接 + 2 条待就绪：{texts:?}");
    assert!(texts[1].contains("local"), "{texts:?}");
    assert!(
        texts[2].contains('◌') && texts[2].contains("proj-c"),
        "启动中那行要带 ◌ 与名字：{texts:?}"
    );
    assert!(
        texts[3].contains('✗') && texts[3].contains("proj-d"),
        "{texts:?}"
    );
    assert!(
        texts[3].contains("工作区不在白名单"),
        "失败原因要露出来：{texts:?}"
    );
    assert!(
        !texts[2].contains('▎'),
        "待就绪行没有聚焦标记（它不在 lanes 里）：{texts:?}"
    );
    // 标题行不铺底色（不补宽），所以只查各行条。
    for row in &texts[1..] {
        assert_eq!(UnicodeWidthStr::width(row.as_str()), 40, "{row:?}");
    }
}

/// 超时仍画 `◌` —— 实例可能还在起，只是加一句「超时」。
#[test]
fn timed_out_row_still_reads_as_starting() {
    use crate::tui_mode::pending_lane::{PendingLane, PendingState};

    let pending = vec![PendingLane {
        name: "proj-c".to_string(),
        state: PendingState::TimedOut,
    }];
    let rows = lane_list_rows(&lanes(&["local"]), &pending, 40, 20, &pal());
    let text = row_text(&rows[2]);
    assert!(text.contains('◌'), "{text}");
    assert!(text.contains("超时"), "{text}");
    assert!(!text.contains('✗'), "超时不是失败结论：{text}");
}

/// 每行宽度恒等于内容区宽度：色块必须是矩形，不是「文字那么宽」的底色。
#[test]
fn every_row_fills_the_width() {
    let rows = rows_of(&lanes(&["local", "很长很长的连接名字"]), 16, 20);
    for r in &rows {
        let w: usize = r
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        assert!(w <= 16, "行宽 {w} 超出 16 列");
    }
    // 色块行必须**正好**铺满（不足部分由 pad_display 补空格）。
    for r in &rows[1..] {
        let w: usize = r
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        assert_eq!(w, 16);
    }
}

/// 装不下时标题行给出被截掉的条数，而不是静默丢弃。
#[test]
fn overflow_is_reported_in_the_title() {
    let names: Vec<String> = (0..30).map(|i| format!("lane{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let rows = rows_of(&lanes(&refs), 20, 10);
    // 可用 9 行、块高 1 → 画 9 条，标题报 ↓21。
    assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [30/8] ↓21");
    assert_eq!(rows.len(), 10);
}

/// 只有标题行放得下时（高度 1）不 panic，且标题如实报出「全部被截掉」。
#[test]
fn one_row_only_shows_the_title() {
    let rows = rows_of(&lanes(&["a", "b"]), 20, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [2/8] ↓2");
}

/// 有人等审批时，左栏标题承担原先状态栈 `lanes ▲k` 那一段的职责
/// （右下角那一行已撤，计数只能在这里）。
#[test]
fn title_reports_the_pending_approval_count() {
    let mut entries = lanes(&["a", "b", "c"]);
    entries[1].status = LaneStatus::WaitingApproval;
    entries[2].status = LaneStatus::WaitingApproval;
    let rows = rows_of(&entries, 20, 20);
    assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [3/8] ▲2");
}

#[test]
fn zero_height_or_width_draws_nothing() {
    assert!(rows_of(&lanes(&["a"]), 0, 10).is_empty());
    assert!(rows_of(&lanes(&["a"]), 10, 0).is_empty());
}

/// 一实例一工作区，但那一行只写名字：工作区在右栏目录树标题与 `/lane` 里，
/// 挤进这条窄条只会把名字吃掉。
#[test]
fn lane_row_carries_the_label_only() {
    let entries = lanes(&["lane-a", "lane-b"]);
    let rows = rows_of(&entries, 40, 20);
    let a = row_text(&rows[1]);
    let b = row_text(&rows[2]);
    assert!(a.contains("lane-a"), "{a}");
    assert!(b.contains("lane-b"), "{b}");
    // 没有分隔符残留：不带工作区就只是一个名字，不是「名字 + 空尾巴」。
    assert!(!a.contains(" · "), "不该留分隔符：{a}");
}

/// 一行只写「标记 + 符号 + 名字」，没有尾巴：窄条里多一个字都是挤出来的。
#[test]
fn row_text_is_mark_plus_label() {
    let rows = rows_of(&lanes(&["local"]), 20, 20);
    assert_eq!(row_text(&rows[1]).trim(), "▎· local");
}

/// 状态符号必须各不相同、且**都是单列**：双列符号会把整行撑破。
#[test]
fn status_marks_are_distinct_and_single_column() {
    for status in [
        LaneStatus::Idle,
        LaneStatus::Running,
        LaneStatus::WaitingApproval,
        LaneStatus::Quitting,
    ] {
        let mark = status.mark();
        assert_eq!(UnicodeWidthStr::width(mark), 1, "`{mark}` 必须是单列");
    }
    let mut marks = vec![
        LaneStatus::Idle.mark(),
        LaneStatus::Running.mark(),
        LaneStatus::WaitingApproval.mark(),
        LaneStatus::Quitting.mark(),
    ];
    marks.sort_unstable();
    marks.dedup();
    assert_eq!(
        marks.len(),
        4,
        "四种状态不能共用符号：否则分不出「在跑」「等审批」与「正在退出」"
    );
}

/// 等审批的**后台** lane 必须一眼能看见：符号 + 告警色，两样都要。
///
/// 后台 lane 的浮层画不到屏幕上，这一行是唯一线索（issue #337）。
#[test]
fn waiting_approval_is_marked_and_tinted() {
    let mut entries = lanes(&["lane-a", "lane-b"]);
    entries[1].status = LaneStatus::WaitingApproval;
    let rows = rows_of(&entries, 30, 20);
    // 前两列拆成两个 span（`▎`/空格 + 状态符号），符号因此落在第二格。
    let mark = rows[2].spans[1].content.as_ref();
    assert!(mark.contains('▲'), "等审批要有状态符号：{mark}");
    assert_eq!(
        rows[2].spans[1].style.fg,
        Some(pal().err),
        "非当前 lane 的等审批走告警色"
    );
}

/// 状态栏的两组计数与左栏同源：`connected` 是 entries 条数，`pending_approvals`
/// 只数 `▲` 那一种状态 —— 两处判定不同就会「左栏亮 `▲`、状态栏说 0」。
#[test]
fn tally_counts_entries_and_waiting_lanes() {
    let mut entries = lanes(&["lane-a", "lane-b", "lane-c"]);
    entries[1].status = LaneStatus::WaitingApproval;
    entries[2].status = LaneStatus::WaitingApproval;
    let tally = LaneTally::from_entries(&entries);
    assert_eq!(tally.connected, 3);
    assert_eq!(tally.pending_approvals, 2);
    // 没人等审批时是 0（状态栏据此**不画** ▲ 后缀）。
    assert_eq!(LaneTally::from_entries(&entries[..1]).pending_approvals, 0);
}

/// 当前 lane 的 `▎` 在任何状态下都要保留：否则「哪条是我正在看的」就没了。
#[test]
fn active_lane_keeps_its_cursor_mark_in_every_status() {
    for status in [
        LaneStatus::Idle,
        LaneStatus::Running,
        LaneStatus::WaitingApproval,
    ] {
        let mut entries = lanes(&["lane-a"]);
        entries[0].status = status;
        let rows = rows_of(&entries, 30, 20);
        assert!(
            rows[1].spans[0].content.starts_with('▎'),
            "{status:?} 时仍要标出当前 lane"
        );
    }
}

/// 尾部读数：上下文占比、耗时、未读点同时出现，未读点在最右。
#[test]
fn tail_shows_context_busy_and_unread_right_aligned() {
    let mut entries = lanes(&["lane-a"]);
    entries[0].context = Some("62%".to_string());
    entries[0].busy_secs = Some(12);
    entries[0].unread = true;
    let rows = rows_of(&entries, 40, 20);
    let line = row_text(&rows[1]);
    assert!(line.ends_with('•'), "未读点必须在最右：{line}");
    assert!(line.contains("62%"), "{line}");
    assert!(line.contains("12s"), "{line}");
    // 读数不许把身份挤掉：这一行首先是「哪条 lane」。
    assert!(line.contains("lane-a"), "{line}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 40);
}

/// 窄行先丢 ctx、再丢耗时：留到最后的必须是未读点。
#[test]
fn tail_drops_readings_before_the_unread_dot() {
    let mut entries = lanes(&["lane-a"]);
    entries[0].context = Some("100%".to_string());
    entries[0].busy_secs = Some(125);
    entries[0].unread = true;
    // 16 列：尾部最多占 8 列，装不下三个读数。
    let rows = rows_of(&entries, 16, 20);
    let line = row_text(&rows[1]);
    assert!(line.ends_with('•'), "{line}");
    assert!(!line.contains("100%"), "窄行先丢 ctx：{line}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 16);

    // 更窄（12 列）：耗时也让位，只剩那个点。
    let rows = rows_of(&entries, 12, 20);
    let line = row_text(&rows[1]);
    assert!(line.ends_with('•'), "{line}");
    assert!(!line.contains("m0"), "耗时也要让位：{line}");
    assert_eq!(UnicodeWidthStr::width(line.as_str()), 12);
}

/// 生成中不换字形：`●` 原地做亮度呼吸，行宽不变（不必为动画多占一列）。
#[test]
fn running_lane_breathes_without_changing_the_glyph() {
    let mut entries = lanes(&["lane-a"]);
    entries[0].status = LaneStatus::Running;
    entries[0].pulse = Some(pulse::STEPS - 1);
    let rows = rows_of(&entries, 30, 20);
    let line = row_text(&rows[1]);
    assert!(line.contains('●'), "字形仍是 `●`：{line}");
    assert_eq!(
        UnicodeWidthStr::width(line.as_str()),
        30,
        "呼吸不改行宽：{line}"
    );
    // 竖条（`▎`）与符号（`●`）各成一列：呼吸只落在符号上。
    assert_eq!(rows[1].spans[0].content.as_ref(), "▎");
    assert_eq!(rows[1].spans[1].content.as_ref(), "●");
    assert_eq!(
        rows[1].spans[1].style.fg,
        Some(pulse::color_at(pal().accent, pulse::STEPS - 1)),
        "呼吸只把状态符号的前景压暗"
    );
    assert_ne!(
        rows[1].spans[1].style.fg,
        Some(pal().accent),
        "最暗档不该等于原色，否则呼吸看不出来"
    );
    assert_eq!(
        rows[1].spans[0].style.fg,
        Some(pal().accent),
        "左栏竖条是「我在哪条 lane」的静态标记，不跟着呼吸"
    );
}

/// 后台 lane 的呼吸相位跟着墙钟走：它不进 `draw_frame`，也不该因此冻结。
#[test]
fn background_lane_spinner_follows_the_clock() {
    let client = test_client();
    let mut prefs = [test_prefs(), test_prefs()];
    let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
    // 两条都在跑：当前 lane 刚发出，后台那条已经跑了 0.25s。
    app.lanes[0].st.running = true;
    app.lanes[0].turn_started = Some(std::time::Instant::now());
    let behind = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_millis(250))
        .expect("系统时钟不支持回退 Instant");
    app.lanes[1].st.running = true;
    app.lanes[1].turn_started = Some(behind);

    let entries = app.lane_entries();
    let front = entries[0].pulse.expect("当前 lane 在跑就该有呼吸相位");
    let back = entries[1].pulse.expect("后台 lane 在跑就该有呼吸相位");
    assert_ne!(
        front, back,
        "后台 lane 的相位必须跟着墙钟走，不能停在切走时那一档"
    );
}

/// 取消中不呼吸：状态行已经在说「取消中」，lane 行再动就是在骗人。
#[test]
fn cancelling_lane_does_not_spin() {
    let client = test_client();
    let mut prefs = [test_prefs()];
    let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
    app.lanes[0].st.running = true;
    app.lanes[0].st.cancel_sent = true;
    app.lanes[0].turn_started = Some(std::time::Instant::now());
    assert!(app.lane_entries()[0].pulse.is_none(), "取消中不该继续呼吸");
}

/// 耗时换成定宽单位：`s` → `m` → `h`，不让秒数无限长。
#[test]
fn busy_duration_switches_units_to_stay_narrow() {
    assert_eq!(duration_text(0), "0s");
    assert_eq!(duration_text(59), "59s");
    assert_eq!(duration_text(60), "1m00s");
    assert_eq!(duration_text(3599), "59m59s");
    assert_eq!(duration_text(3600), "1h00m");
    assert_eq!(duration_text(7325), "2h02m");
}

/// 后台 lane 的新输出点亮未读点，当前 lane 的不点亮；切过去即熄。
#[test]
fn background_output_marks_unread_but_the_active_lane_does_not() {
    let client = test_client();
    let mut prefs = [test_prefs(), test_prefs()];
    let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
    let (first, second) = (app.lanes[0].id, app.lanes[1].id);

    app.on_msg(Inbound::FromLane(second, UiEvent::Text("hi".into())));
    assert!(!app.lane_entries()[0].unread, "当前 lane 不该亮未读");
    assert!(app.lane_entries()[1].unread, "后台 lane 有输出要亮");

    // 切过去即熄（帧循环每帧调用 `clear_active_unread`）。
    app.active = 1;
    app.clear_active_unread();
    assert!(!app.lane_entries()[1].unread);

    // 回到第一条：它自己说话不点亮自己，否则那个点永远亮着。
    app.active = 0;
    app.clear_active_unread();
    app.on_msg(Inbound::FromLane(first, UiEvent::Text("x".into())));
    assert!(!app.lane_entries()[0].unread);
}
