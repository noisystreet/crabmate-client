//! `mouse.rs` 的单测（`#[path]` 子模块）：压住单文件行数门禁。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::super::state::{Focus, LineKind};
use super::super::test_support::{
    narrow_layout, test_app, test_client, test_prefs, visible_layout,
};
use super::super::ws_sidebar::{sessions_content, workspace_content};
use super::{BodyHit, ClickTracker, DOUBLE_CLICK_WINDOW, LaneApp, body_hit, item_index_at, panes};
use crate::serve::SessionListRow;
use ratatui::layout::Rect;

fn row(id: &str, cid: Option<&str>) -> SessionListRow {
    SessionListRow {
        id: id.to_string(),
        title: format!("t-{id}"),
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: cid.map(str::to_string),
        server_revision: None,
        workspace_root: None,
    }
}

/// 宽屏（200×40、两栏可见）下的最小 `LaneApp`。
/// 通道接收端留在作用域内：切换会话会入队历史回放，没人收就会被当成「worker 已退出」。
fn with_wide_app(f: impl FnOnce(&mut LaneApp<'_>)) {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
    app.last_size = (200, 40);
    app.st.sidebar_visible = true;
    f(&mut app);
}

fn click(app: &mut LaneApp<'_>, column: u16, row: u16) {
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn scroll(app: &mut LaneApp<'_>, kind: MouseEventKind, column: u16, row: u16) {
    app.on_mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn drag(app: &mut LaneApp<'_>, column: u16, row: u16) {
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn release(app: &mut LaneApp<'_>, column: u16, row: u16) {
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

/// 宽屏下两栏的矩形（拖拽命中与断言都按它算，不另抄一份尺寸）。
fn side_panes(app: &LaneApp<'_>) -> (Rect, Rect) {
    let p = panes(
        Rect::new(0, 0, 200, 40),
        app.st.pane_layout(),
        app.status_stack_rows(),
    );
    (p.left.expect("宽屏有左栏"), p.right.expect("宽屏有右栏"))
}

/// 按住左栏分隔线拖动：改宽度，但**不**顺带把会话切走（issue #407 P0.5）。
///
/// 分隔线那一列落在左栏矩形里，所以「先判拖拽、再判点行」的顺序错了，表现就是
/// 一次拖宽手势里会话被换掉。
#[test]
fn dragging_the_left_divider_resizes_without_switching_sessions() {
    with_wide_app(|app| {
        app.st
            .replace_sessions(vec![row("s1", None), row("s2", None)]);
        let (left, _) = side_panes(app);
        let divider = left.x + left.width - 1;
        let selected = app.st.selected;

        click(app, divider, left.y);
        assert_eq!(app.st.selected, selected, "按在分隔线上不该切会话");
        assert_eq!(app.st.focus, Focus::Input, "也不该把焦点抢到侧栏");

        drag(app, 40, left.y);
        assert_eq!(
            app.st.pane.left, 41,
            "指针停在第 40 列 → 分隔线跟着到第 40 列"
        );
        release(app, 40, left.y);
    });
}

/// 右栏分隔线是它的**左**边缘：宽度 = 终端宽 - 指针列。
#[test]
fn dragging_the_right_divider_moves_its_own_edge() {
    with_wide_app(|app| {
        let (_, right) = side_panes(app);
        click(app, right.x, right.y);
        drag(app, 150, right.y);
        assert_eq!(app.st.pane.right, 50, "指针在第 150 列 → 右栏宽 50");
        release(app, 150, right.y);
    });
}

/// 起点不在分隔线上（例如在正文里按住左键划一下）不该改宽。
#[test]
fn dragging_from_the_chat_column_leaves_the_panes_alone() {
    with_wide_app(|app| {
        let before = app.st.pane;
        click(app, 90, 10);
        drag(app, 40, 10);
        assert_eq!(app.st.pane, before, "起点不是分隔线就不该改宽");
        release(app, 40, 10);
    });
}

/// 松开之后再拖不该继续改宽：拖拽态在 `Up` 时就结束了。
#[test]
fn dragging_after_release_does_not_resize() {
    with_wide_app(|app| {
        let (left, _) = side_panes(app);
        let divider = left.x + left.width - 1;
        click(app, divider, left.y);
        drag(app, 40, left.y);
        release(app, 40, left.y);
        let settled = app.st.pane;
        drag(app, 100, left.y);
        assert_eq!(app.st.pane, settled, "没按住就不该跟着指针变");
    });
}

#[test]
fn title_row_is_not_an_item() {
    assert_eq!(item_index_at(0, 0, 5), None);
}

#[test]
fn rows_map_to_items_from_window_start() {
    // 窗口裁到条目 3..6，可视行 1 → 条目 3。
    assert_eq!(item_index_at(1, 3, 6), Some(3));
    assert_eq!(item_index_at(3, 3, 6), Some(5));
}

#[test]
fn rows_past_window_end_are_none() {
    assert_eq!(item_index_at(4, 3, 6), None, "列表末尾空白不选中");
}

/// 正文区点击：物理行 → 逻辑行只做一次减法（窗口起点与矩形都来自渲染层的回写）。
///
/// 断言里带上「同一条逻辑行的第二个物理行」与「分隔行」：折行与块间分隔线正是
/// 在 `mouse.rs` 里重算排版时最容易错开一格的两处。
#[test]
fn body_hit_maps_window_rows_to_log_indices() {
    // 区域高 5 行、窗口内容只有 4 行：最后一行是「窗口末尾的空白」。
    let area = Rect::new(1, 5, 20, 5);
    let rows = vec![None, Some(7), Some(7), Some(9)];

    assert!(
        matches!(body_hit(area, &rows, 1, 5), BodyHit::Gap),
        "分隔行不对应任何逻辑行"
    );
    assert!(matches!(body_hit(area, &rows, 1, 6), BodyHit::Log(7)));
    assert!(
        matches!(body_hit(area, &rows, 1, 7), BodyHit::Log(7)),
        "同一条的第二个物理行仍是 7"
    );
    assert!(matches!(body_hit(area, &rows, 1, 8), BodyHit::Log(9)));
    assert!(
        matches!(body_hit(area, &rows, 1, 9), BodyHit::Gap),
        "窗口末尾没有内容"
    );
    assert!(
        matches!(body_hit(area, &rows, 0, 6), BodyHit::Outside),
        "正文区左缘外"
    );
}

/// 点侧栏 = 焦点由鼠标带来：底部按键提示行不画；按一下键就恢复（`on_key` 清标记）。
#[test]
fn clicking_a_pane_hides_the_key_hint_until_a_key_arrives() {
    with_wide_app(|app| {
        app.st.replace_sessions(vec![row("s1", Some("c1"))]);
        let p = panes(
            Rect::new(0, 0, 200, 40),
            app.st.pane_layout(),
            app.status_stack_rows(),
        );

        let left = p.left.expect("宽屏应有左栏");
        click(app, left.x + 1, left.y); // 标题行：只切焦点
        assert_eq!(app.st.focus, Focus::Sidebar);
        assert!(app.st.focus_by_pointer, "鼠标点出来的焦点要打标记");
        assert!(
            sessions_content(&app.st, 20).2.is_none(),
            "鼠标点进会话栏时不显示按键提示"
        );

        let right = p.right.expect("宽屏应有右栏");
        click(app, right.x + 1, right.y);
        assert_eq!(app.st.focus, Focus::Workspace);
        assert!(
            workspace_content(&app.st, 20).2.is_none(),
            "鼠标点进工作区时不显示按键提示"
        );

        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(!app.st.focus_by_pointer, "键盘接管要清掉鼠标标记");
        assert!(
            workspace_content(&app.st, 20).2.is_some(),
            "键盘带来的焦点照旧显示提示"
        );
    });
}

/// 点摘要 = 切换 + 把键盘一起接过来。
///
/// 「一起接过来」正是曾经的漏洞：命中后 `click_body` 直接 `return true`，跳过了兜底的
/// 「焦点收回输入框」，于是焦点留在侧栏时按 ↑/↓ 动的是会话选择 —— 用户看到的是
/// 「点完摘要，方向键在乱动左边的列表」。
#[test]
fn clicking_a_summary_toggles_it_and_takes_the_keyboard_from_the_sidebar() {
    with_wide_app(|app| {
        app.st.push_line(LineKind::Tool, "✓ a");
        app.st.push_line(LineKind::Tool, "✓ b");
        app.st.fold_process_runs(None);
        let summary = app
            .st
            .lines
            .iter()
            .position(|l| l.kind() == LineKind::Process)
            .expect("应有摘要行");
        // 模拟一帧渲染的回写：正文区自 y=5 起，窗口第 0 行就是那条摘要。
        app.st.body_area = Some(Rect::new(1, 5, 20, 6));
        app.st.body_rows_log = vec![Some(summary)];
        app.st.focus = Focus::Sidebar;
        app.st.confirm_delete = Some("s-1".to_string());

        assert!(app.click_body(2, 5), "点中摘要要消费这一下");

        assert_eq!(app.st.fold_cursor, Some(summary));
        assert_eq!(app.st.focus, Focus::Input, "键路必须跟光标一起过来");
        assert!(app.st.confirm_delete.is_none(), "离开侧栏就作废待确认删除");
        assert_eq!(
            app.st.lines.iter().filter(|l| l.is_hidden()).count(),
            0,
            "摘要下的成员应已展开"
        );
    });
}

/// 点正文里的非摘要行：不消费（焦点照旧收回输入框），只把导航作废。
#[test]
fn clicking_a_plain_body_line_only_drops_the_cursor() {
    with_wide_app(|app| {
        app.st.push_line(LineKind::Assistant, "答案");
        app.st.body_area = Some(Rect::new(1, 5, 20, 6));
        app.st.body_rows_log = vec![Some(0)];
        app.st.fold_cursor = Some(0);

        assert!(!app.click_body(2, 5), "非摘要行不该吃掉这一下");
        assert_eq!(app.st.fold_cursor, None, "导航作废");
    });
}

#[test]
fn empty_window_maps_to_nothing() {
    assert_eq!(item_index_at(1, 0, 0), None);
}

/// 点左栏第 2 条会话：选中并**直接切过去**（单击即切换），待确认删除随之作废。
#[test]
fn click_session_row_selects_and_switches() {
    with_wide_app(|app| {
        app.st
            .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
        app.st.conversation_id = Some("c1".to_string());
        app.st.confirm_delete = Some("a".to_string());
        // 列内左留白 1 列；第 0 行是标题，故屏幕第 2 行 = 第 2 条会话。
        click(app, 2, 2);
        assert_eq!(app.st.selected, 1, "点第 2 行应选中第 2 条会话");
        assert_eq!(app.st.conversation_id.as_deref(), Some("c2"), "单击即切换");
        assert!(app.st.confirm_delete.is_none(), "点击作废待确认删除");
    });
}

/// 点标题行只切焦点：不选会话、不切换。
#[test]
fn click_title_row_only_takes_focus() {
    with_wide_app(|app| {
        app.st
            .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
        app.st.conversation_id = Some("c1".to_string());
        click(app, 2, 0);
        assert!(matches!(app.st.focus, Focus::Sidebar), "点左栏应聚焦会话栏");
        assert_eq!(app.st.selected, 0, "标题行不改选中");
        assert_eq!(
            app.st.conversation_id.as_deref(),
            Some("c1"),
            "不该切换会话"
        );
    });
}

/// 点底部提示行（列内最后一行）同样只切焦点。
#[test]
fn click_hint_row_only_takes_focus() {
    with_wide_app(|app| {
        app.st
            .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
        click(app, 2, 39); // 40 行终端：最后一行留给按键提示
        assert_eq!(app.st.selected, 0);
        assert!(app.st.conversation_id.is_none(), "提示行不该切换会话");
    });
}

/// 空列表的「（空）发一条消息后出现」占位行不能被当成会话 0 号。
#[test]
fn click_placeholder_row_of_empty_list_does_not_switch() {
    with_wide_app(|app| {
        app.st.replace_sessions(Vec::new());
        click(app, 2, 1);
        assert_eq!(app.st.selected, 0);
        assert!(
            app.st.conversation_id.is_none(),
            "占位行不该切换会话（否则会提示「该会话还没有 conversation_id」）"
        );
    });
}

/// 左栏滚轮**不**碰会话列表：选中与当前会话都不动（它滚的是 transcript）。
#[test]
fn left_column_scroll_does_not_touch_session_selection() {
    with_wide_app(|app| {
        app.st
            .replace_sessions(vec![row("a", Some("c1")), row("b", Some("c2"))]);
        app.st.conversation_id = Some("c1".to_string());
        scroll(app, MouseEventKind::ScrollDown, 2, 1);
        scroll(app, MouseEventKind::ScrollUp, 2, 1);
        assert_eq!(app.st.selected, 0, "滚轮不改会话选中");
        assert_eq!(
            app.st.conversation_id.as_deref(),
            Some("c1"),
            "滚轮不该切换会话"
        );
    });
}

/// 长列表 + 矮终端：命中必须跟着窗口偏移走（`sidebar_window` 的 `start`），
/// 不能当成「可视第 1 行 = 条目 0」——这正是「点到的行与反色行错开一行」的来源。
#[test]
fn click_on_a_scrolled_list_maps_through_the_window_offset() {
    with_wide_app(|app| {
        let rows: Vec<_> = (0..20)
            .map(|i| row(&format!("s{i}"), Some(&format!("c{i}"))))
            .collect();
        app.st.replace_sessions(rows);
        app.st.selected = 15;
        app.st.conversation_id = Some("c15".to_string());
        // 6 行终端：左栏内容区 5 行 = 标题 + 4 条；选中 15 → 窗口条目 12..16。
        app.last_size = (200, 6);
        click(app, 2, 2); // 可视第 2 行 = 窗口内第 1 条 = 条目 13
        assert_eq!(app.st.selected, 13, "命中应加上窗口起点 start=12");
        assert_eq!(
            app.st.conversation_id.as_deref(),
            Some("c13"),
            "切到的应是条目 13，而不是第 2 条会话"
        );
    });
}

/// 窄屏没有左栏：左栏位置的点击什么都不做。
#[test]
fn narrow_terminal_ignores_left_column_clicks() {
    with_wide_app(|app| {
        app.st.sidebar_visible = false;
        app.last_size = (80, 24);
        app.st.replace_sessions(vec![row("a", Some("c1"))]);
        click(app, 2, 2);
        assert!(matches!(app.st.focus, Focus::Input), "窄屏没有左栏");
        assert!(app.st.conversation_id.is_none(), "窄屏不该切换会话");
    });
}

/// 造一份只带权限投影的 `ServeDefaults`（其余字段走 `Default`）。
///
/// 用 JSON 反序列化而不是手写结构体：投影里的枚举/六轴命名以服务端契约为准，测试跟着
/// 契约形状走，就不必在本仓再抄一遍变体名（抄了就会在契约升级时悄然漂移）。
fn defaults_with_permission(level: &str) -> super::super::serve_defaults::ServeDefaults {
    let json = format!(
        r#"{{"level":"{level}","level_source":"persisted","ceiling":"unrestricted",
             "ceiling_source":"derived_loopback",
             "available":["chat","read","workspace","approved","unrestricted"],
             "clamped_by":[],
             "caps":{{"tool_visibility":"full","external_tools":"allowed",
                      "path_scope":"any","command_scope":"any","net_scope":"any",
                      "approval":"audit_only"}}}}"#
    );
    super::super::serve_defaults::ServeDefaults {
        permission_view: Some(serde_json::from_str(&json).expect("permission view")),
        ..Default::default()
    }
}

/// 宽屏：`perm` 段可点，且锚点就是右栏状态栈里那一行、内宽与渲染一致（左右各留 1 列）。
///
/// 用「逐行点一遍找出可点的那一行」而不是先算行号：行号来自 `status_stack::segment_row_index`，
/// 拿它当期望值等于两边同错同对。这里要验的是**点击到的那一行**和**渲染用的内宽**对得上。
#[test]
fn clicking_the_perm_segment_opens_a_menu_anchored_to_that_row() {
    with_wide_app(|app| {
        app.st.serve_defaults = Some(defaults_with_permission("approved"));
        let (w, h) = app.last_size;
        let p = panes(
            Rect::new(0, 0, w, h),
            visible_layout(),
            app.status_stack_rows(),
        );
        let rs = p.right_status.expect("宽屏有状态栈");
        let mut anchor = None;
        for row in rs.y..rs.y + rs.height {
            click(app, rs.x + 1, row);
            if let Some(menu) = app.st.permission_menu.as_ref() {
                anchor = Some(menu.anchor);
                break;
            }
        }
        let anchor = anchor.expect("状态栈里应有可点的 perm 段");
        assert_eq!(anchor.x, rs.x + 1, "内容列内缩 1 列");
        assert_eq!(anchor.width, rs.width - 2, "与渲染共用内宽");
        assert_eq!(anchor.height, 1, "只有一行");
        // 菜单锚在这一行上，并向上展开（它是屏幕最底那几行之一）。
        let menu = app.st.permission_menu.as_ref().expect("菜单已开");
        let rect = crate::tui_mode::render::overlay::permission_menu_rect(
            Rect::new(0, 0, w, h),
            anchor,
            menu.options.len(),
            10,
        );
        assert_eq!(rect.y + rect.height, anchor.y, "下边贴 anchor 上沿");
        assert_eq!(rect.x + rect.width, anchor.x + anchor.width, "右缘对齐");
    });
}

/// 窄屏：`perm` 段在底栏那一行，锚点整段落在底栏内。
#[test]
fn clicking_the_perm_segment_opens_a_menu_on_the_bottom_bar() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
    app.last_size = (60, 20);
    app.st.sidebar_visible = false;
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    let (w, h) = app.last_size;
    let p = panes(
        Rect::new(0, 0, w, h),
        narrow_layout(),
        app.status_stack_rows(),
    );
    let bar = p.status.expect("窄屏有底栏");
    let mut anchor = None;
    for col in bar.x..bar.x + bar.width {
        click(&mut app, col, bar.y);
        if let Some(menu) = app.st.permission_menu.as_ref() {
            anchor = Some(menu.anchor);
            break;
        }
    }
    let anchor = anchor.expect("底栏应有可点的 perm 段");
    assert_eq!(anchor.y, bar.y, "命中在底栏那一行");
    assert!(anchor.width > 0, "命中段非空");
    assert!(
        anchor.x >= bar.x && anchor.x + anchor.width <= bar.x + bar.width,
        "命中段不能越出底栏：{anchor:?} vs {bar:?}"
    );
}

/// 宽屏：`view` 段可点 —— 点一下切到文件视图，再点同一行切回聊天。
///
/// 断言走 `st.editor.open`（「点了有反应」的定义），与 `/editor` / `/chat` / `Ctrl+T`
/// 是同一份状态：段上的字已由 status 的单测覆盖，这里只压「点得到、点得动」。
#[test]
fn clicking_the_view_segment_toggles_the_main_pane() {
    with_wide_app(|app| {
        let (w, h) = app.last_size;
        let rs = panes(
            Rect::new(0, 0, w, h),
            visible_layout(),
            app.status_stack_rows(),
        )
        .right_status
        .expect("宽屏有状态栈");
        assert!(!app.st.editor.open, "起始在聊天视图");
        let mut view_row = None;
        for row in rs.y..rs.y + rs.height {
            click(app, rs.x + 1, row);
            if app.st.editor.open {
                view_row = Some(row);
                break;
            }
        }
        let view_row = view_row.expect("状态栈里应有可点的 view 段");
        // 段的位置不随开关变化（段数恒定、顺序恒定），所以同一行再点一次就该切回去。
        click(app, rs.x + 1, view_row);
        assert!(!app.st.editor.open, "再点同一行应切回聊天");
    });
}

/// 宽屏：`think` 段可点 —— 点一下把意图翻到另一端，并派一发 `SaveLlm` 落盘。
///
/// 断言走 **override**（`overrides.thinking`）而不是画出来的字：那正是「点了有反应」
/// 的定义（状态行与下一轮 `chat` 都读它），而 render 的字串已由 status 的单测覆盖。
#[test]
fn clicking_the_think_segment_flips_the_switch_and_saves() {
    use crate::tui_mode::settings::{FieldAction, PersistedSettings};
    use crate::tui_mode::worker::WorkerJob;

    let client = test_client();
    let mut prefs = test_prefs();
    let (tx, rx) = mpsc::channel();
    let (job_tx, job_rx) = mpsc::channel();
    let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
    app.last_size = (200, 40);
    app.st.sidebar_visible = true;
    // user-data 到手且当前是「关」：状态栈里因此有可点的 think 行。
    app.persisted = Some(PersistedSettings {
        thinking: Some("off".into()),
        ..Default::default()
    });
    let (w, h) = app.last_size;
    let rs = panes(
        Rect::new(0, 0, w, h),
        visible_layout(),
        app.status_stack_rows(),
    )
    .right_status
    .expect("宽屏有状态栈");
    let mut flipped = false;
    for row in rs.y..rs.y + rs.height {
        click(&mut app, rs.x + 1, row);
        if app.overrides.thinking.is_some() {
            flipped = true;
            break;
        }
    }
    assert!(flipped, "状态栈里应有可点的 think 段");
    assert_eq!(app.overrides.thinking.as_deref(), Some("on"), "关 → 开");
    match job_rx.try_recv().expect("点开关要派发 SaveLlm") {
        WorkerJob::SaveLlm(save) => assert_eq!(
            save.thinking,
            FieldAction::Write(Some("on".into())),
            "落盘值与屏上一致"
        ),
        _ => panic!("点开关应派发 SaveLlm"),
    }
    // 再点一下（同一个矩形）：回到「关」，而不是重复写同一个值。
    for r in rs.y..rs.y + rs.height {
        click(&mut app, rs.x + 1, r);
        if app.overrides.thinking.as_deref() == Some("off") {
            break;
        }
    }
    assert_eq!(app.overrides.thinking.as_deref(), Some("off"), "开 → 关");
}

/// 右栏树的夹具：根下只有一个文件 `README.md`（够验「点文件行」）。
///
/// 直接塞 `ws_root_replace` 而不是走 HTTP：这里要验的是点击落点与视图切换，
/// 拉目录那一段已有 `workspace_tree` 的单测。
fn ws_root_with_one_file(app: &mut LaneApp<'_>) {
    use crate::serve::{WorkspaceDirData, WorkspaceDirEntry};
    app.st.ws_root_replace(WorkspaceDirData {
        path: "/p".to_string(),
        entries: vec![WorkspaceDirEntry {
            name: "README.md".to_string(),
            is_dir: false,
        }],
        error: None,
    });
}

/// 编辑器视图开着时双击文件行 = 打开它（与树上 Enter 同一入口），焦点交回输入框。
#[test]
fn double_click_on_a_file_row_opens_it_while_the_editor_view_is_open() {
    with_wide_app(|app| {
        ws_root_with_one_file(app);
        app.st.editor.open_file("src/lib.rs", 0);
        let (_, right) = side_panes(app);
        // 第 0 行是标题，第 1 行是唯一的树行；列内左留白 1 列。
        app.apply_workspace_click(right.x + 2, right.y + 1, true);
        assert_eq!(
            app.st.editor.rel.as_deref(),
            Some("README.md"),
            "双击该打开这一行指向的文件"
        );
        assert!(
            matches!(app.st.focus, Focus::Input),
            "打开后焦点交回输入框（接着按 PgUp 滚的是文件，不是树）"
        );
    });
}

/// 单击仍只是落光标：打开需要双击（或 Enter），与聊天视图里的单击口径一致。
#[test]
fn a_single_click_on_a_file_row_only_moves_the_cursor() {
    with_wide_app(|app| {
        ws_root_with_one_file(app);
        app.st.editor.open_file("src/lib.rs", 0);
        let (_, right) = side_panes(app);
        app.apply_workspace_click(right.x + 2, right.y + 1, false);
        assert_eq!(
            app.st.editor.rel.as_deref(),
            Some("src/lib.rs"),
            "单击不该换文件"
        );
        assert_eq!(app.st.ws_cursor, 0, "只落光标");
    });
}

/// 聊天视图下的双击没有新语义：那里 Enter 是「插路径」，双击插两次只会顶掉草稿。
#[test]
fn double_click_in_chat_view_does_not_open_the_editor() {
    with_wide_app(|app| {
        ws_root_with_one_file(app);
        let (_, right) = side_panes(app);
        app.apply_workspace_click(right.x + 2, right.y + 1, true);
        assert!(!app.st.editor.open, "聊天视图下不切视图");
        assert_eq!(app.st.ws_cursor, 0, "仍然只落光标");
    });
}

/// 真实的事件序列（`Down` → `Up` → `Down` → `Up`）必须能打开文件。
///
/// 上面那条用例把 `double` 直接喂给 `apply_workspace_click`，绕过了 `ClickTracker`
/// 与 `on_mouse`：真机上「双击没反应」恰好只可能坏在这一段，所以这里走整条路。
#[test]
fn a_real_down_up_down_sequence_opens_the_file() {
    with_wide_app(|app| {
        ws_root_with_one_file(app);
        app.st.editor.open_file("src/lib.rs", 0);
        let (_, right) = side_panes(app);
        let (c, r) = (right.x + 2, right.y + 1);
        click(app, c, r);
        release(app, c, r);
        click(app, c, r);
        release(app, c, r);
        assert_eq!(
            app.st.editor.rel.as_deref(),
            Some("README.md"),
            "真实序列下双击该打开这一行指向的文件"
        );
    });
}

/// 双击识别：同一行、窗口内的第二次按下才算；换行 / 超窗口都是两次单击。
#[test]
fn click_tracker_needs_the_same_row_within_the_window() {
    let t0 = Instant::now();
    let mut t = ClickTracker::default();
    assert!(!t.register(t0, 5, 7), "第一下不是双击");
    assert!(
        !t.register(t0 + Duration::from_millis(50), 5, 8),
        "换一行不算双击"
    );
    assert!(
        !t.register(t0 + Duration::from_millis(120), 6, 7),
        "换一列也不算双击"
    );
    assert!(
        !t.register(t0 + Duration::from_millis(200), 5, 7),
        "前面那次已被上面两下顶掉，这里只是新的第一下"
    );
    assert!(
        t.register(t0 + Duration::from_millis(260), 5, 7),
        "同一行、窗口内 = 双击"
    );
    assert!(
        !t.register(t0 + Duration::from_millis(300), 5, 7),
        "双击之后不再算一次（三连击只读一次文件）"
    );
}

/// 视图开着时，聊天列上的滚轮滚的是**文件**，不是被它整片遮住的 transcript。
///
/// 滚错对象在屏幕上表现为「滚轮坏了」：内容不动，而 transcript 在看不见的地方跑远了。
#[test]
fn the_wheel_over_the_chat_column_scrolls_the_open_file_view() {
    with_wide_app(|app| {
        let body: String = (1..=40).map(|i| format!("l{i}\n")).collect();
        app.st.editor.open_file("src/lib.rs", 0);
        app.st.editor.accept(&body);
        let (left, right) = side_panes(app);
        let column = left.x + left.width + (right.x - left.x - left.width) / 2;
        scroll(app, MouseEventKind::ScrollDown, column, 2);
        assert_eq!(app.st.editor.scroll, 3, "一格 3 行，与聊天区滚轮同一步长");
        scroll(app, MouseEventKind::ScrollUp, column, 2);
        assert_eq!(app.st.editor.scroll, 0, "往回滚回到顶部");
    });
}

/// 超出窗口就是两次单击：慢慢点两下不该打开文件。
#[test]
fn click_tracker_expires_after_the_window() {
    let t0 = Instant::now();
    let mut t = ClickTracker::default();
    assert!(!t.register(t0, 3, 4));
    assert!(
        !t.register(t0 + DOUBLE_CLICK_WINDOW + Duration::from_millis(1), 3, 4),
        "超过窗口的第二次点击是新的一次单击"
    );
}
