//! 左栏**过滤态**的渲染断言（issue #423）：标题条数、过滤词、无匹配占位行。
//!
//! 单独成文件与 `color_tests` / `system_tests` 同一手法：`render_tests.rs` 已经贴着
//! 920 行门禁，再挤这一支就顶过去。

use super::*;
use crate::serve::SessionListRow;

fn row(id: &str, title: &str, conv: Option<&str>) -> SessionListRow {
    SessionListRow {
        id: id.to_string(),
        title: title.to_string(),
        updated_at: 0,
        pinned: false,
        starred: false,
        server_conversation_id: conv.map(str::to_string),
        server_revision: None,
        workspace_root: None,
    }
}

/// 过滤态标题写 `[可见/总数]` 并把过滤词挂上去；无匹配给占位行而不是空列表。
#[test]
fn sidebar_rows_report_filtered_counts_and_the_term() {
    let mut st = UiState::new();
    st.replace_sessions(vec![
        row("a", "修复登录闪退", Some("c1")),
        row("b", "Add TUI filter", Some("c2")),
        row("c", "重写登录页", None),
    ]);
    st.set_session_filter(Some("登录".into()));

    let rows = sidebar_rows(&st, 40);
    let title = rows[0].to_string();
    assert!(title.contains("会话 [2/3]"), "过滤态要报可见/总数：{title}");
    assert!(title.contains("filter: 登录"), "过滤词必须看得见：{title}");
    assert_eq!(rows.len(), 3, "标题 + 两条可见行");
    assert!(rows[1].to_string().contains("修复登录闪退"));
    assert!(rows[2].to_string().contains("重写登录页"));

    st.set_session_filter(Some("压根没有这个词".into()));
    let rows = sidebar_rows(&st, 40);
    assert!(rows[0].to_string().contains("会话 [0/3]"));
    assert!(
        rows[1].to_string().contains("（无匹配）"),
        "空结果要有占位行，不能只留一个标题"
    );
}

/// 提示行：过滤态换成「怎么清过滤」，删除确认态仍然优先。
#[test]
fn hint_line_switches_to_the_clear_command_while_filtered() {
    let mut st = UiState::new();
    st.set_palette(Palette::default());
    st.replace_sessions(vec![row("a", "会话 A", Some("c1"))]);
    st.focus = Focus::Sidebar;
    let (_, _, hint) = sessions_content(&st, 25);
    assert_eq!(
        hint.map(|h| h.to_string()).unwrap_or_default(),
        SIDEBAR_HINT,
        "不过滤时是普通键位提示"
    );

    st.set_session_filter(Some("会话".into()));
    let (_, _, hint) = sessions_content(&st, 25);
    assert_eq!(
        hint.map(|h| h.to_string()).unwrap_or_default(),
        SIDEBAR_FILTER_HINT,
        "过滤态要告诉人怎么清"
    );
}
