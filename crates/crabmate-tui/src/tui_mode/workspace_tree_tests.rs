//! `workspace_tree.rs` 的单测（`#[path]` 子模块）：压住单文件行数门禁。

use super::*;

fn ent(name: &str, is_dir: bool) -> WorkspaceDirEntry {
    WorkspaceDirEntry {
        name: name.to_string(),
        is_dir,
    }
}

fn data(path: &str, entries: Vec<WorkspaceDirEntry>) -> WorkspaceDirData {
    WorkspaceDirData {
        path: path.to_string(),
        entries,
        error: None,
    }
}

#[test]
fn ws_rel_helpers() {
    assert_eq!(ws_child_rel("", "src"), "src");
    assert_eq!(ws_child_rel("src", "main.rs"), "src/main.rs");
    assert_eq!(ws_parent_rel("src/lib/mod.rs"), "src/lib");
    assert_eq!(ws_parent_rel("main.rs"), "");
}

#[test]
fn ws_root_replace_flattens_root_rows() {
    let mut s = UiState::new();
    s.ws_root_replace(data(
        "/data/p",
        vec![
            ent("src", true),
            ent("README.md", false),
            ent("target", true),
        ],
    ));
    assert!(s.ws_ready);
    assert_eq!(s.workspace_path.as_deref(), Some("/data/p"));
    assert_eq!(s.ws_rows.len(), 3);
    assert_eq!(s.ws_rows[0].rel, "src");
    assert_eq!(s.ws_rows[0].indent, 0);
    assert!(s.ws_rows[0].is_dir && !s.ws_rows[0].expanded);
    assert_eq!(s.ws_rows[1].name, "README.md");
    // 空树时光标收敛到 0
    s.ws_cursor = 9;
    s.ws_root_replace(data("/empty", Vec::new()));
    assert!(s.ws_rows.is_empty());
    assert_eq!(s.ws_cursor, 0);
}

#[test]
fn ws_expand_fetches_and_inserts_children() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("src".to_string()), "应请求子列表");
    assert!(s.ws_loading.contains("src"));
    // 列表在途时再 toggle 不会重复请求（先折叠再展开）
    assert_eq!(s.ws_toggle_dir(), None, "折叠不请求");
    assert_eq!(s.ws_toggle_dir(), None, "加载中不重复请求");
    s.ws_dir_ok(
        "src",
        data("/p/src", vec![ent("lib", true), ent("main.rs", false)]),
    );
    assert_eq!(s.ws_rows.len(), 3, "src 展开后其后代可见");
    assert_eq!(s.ws_rows[1].rel, "src/lib");
    assert_eq!(s.ws_rows[1].indent, 1);
    assert_eq!(s.ws_rows[2].name, "main.rs");
    // 折叠 src → 后代行消失，光标留在 src
    s.ws_cursor = 0;
    assert_eq!(s.ws_toggle_dir(), None);
    assert_eq!(s.ws_rows.len(), 1);
    assert!(!s.ws_expanded.contains("src"));
}

#[test]
fn ws_dir_ok_caches_when_collapsed_early() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("docs", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("docs".to_string()));
    // 结果到达前用户已折叠：缓存仍写入，再次展开无需请求
    assert_eq!(s.ws_toggle_dir(), None);
    s.ws_dir_ok("docs", data("/p/docs", vec![ent("a.md", false)]));
    assert_eq!(s.ws_rows.len(), 1);
    assert_eq!(s.ws_toggle_dir(), None, "已有缓存不再请求");
    assert_eq!(s.ws_rows.len(), 2);
}

#[test]
fn ws_dir_failed_collapses_and_reports() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("x", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("x".to_string()));
    assert!(s.ws_dir_failed("x"), "展开中的目录失败应收起并上报");
    assert!(!s.ws_expanded.contains("x"));
    assert!(!s.ws_loading.contains("x"));
    assert_eq!(s.ws_rows.len(), 1);
    // 已折叠目录收到迟到的失败结果不应提示
    assert_eq!(s.ws_toggle_dir(), Some("x".to_string()));
    assert_eq!(s.ws_toggle_dir(), None, "先折叠");
    assert!(!s.ws_dir_failed("x"));
}

#[test]
fn ws_cursor_moves_and_left_goes_parent() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("a", data("/p/a", vec![ent("a1.txt", false)]));
    // rows: a(0) a1.txt(1) b(2)
    s.ws_cursor = 1;
    s.ws_left();
    assert_eq!(s.ws_cursor, 0, "文件行向左跳到父目录");
    s.ws_toggle_dir(); // 折叠 a
    s.ws_move(false); // 到 b
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
    s.ws_left(); // b 未展开 → 父是根，不动作
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
    s.ws_move(true);
    s.ws_move(true); // 顶部不动
    assert_eq!(s.ws_cursor, 0);
}

#[test]
fn ws_root_failed_keeps_old_tree() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("keep", true)]));
    assert!(!s.ws_root_failed("busy"), "已有数据时失败不提示、保留旧树");
    assert!(s.ws_ready);
    assert_eq!(s.ws_rows.len(), 1);
    // 首次加载失败：占位错误 + 后续可重试
    let mut t = UiState::new();
    assert!(t.ws_root_failed("down"));
    assert_eq!(t.ws_root_err.as_deref(), Some("down"));
    t.ws_begin_root_fetch();
    t.ws_root_replace(data("/p", vec![]));
    assert_eq!(t.ws_root_err, None);
}

#[test]
fn ws_expand_shows_loading_placeholder_until_loaded() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
    assert_eq!(s.ws_rows.len(), 2, "在途时目录下方应有占位行");
    assert!(s.ws_rows[1].loading);
    assert_eq!(s.ws_rows[1].name, "\u{22ef}");
    // 空目录到达：占位变「（空）」，仍保持展开
    s.ws_dir_ok("a", data("/p/a", Vec::new()));
    assert_eq!(s.ws_rows.len(), 2, "目录 +「（空）」占位");
    assert!(!s.ws_rows[0].loading);
    assert!(s.ws_rows[1].placeholder);
    assert_eq!(s.ws_rows[1].name, EMPTY_DIR_TEXT);
}

#[test]
fn ws_dir_ok_shifts_cursor_past_inserted_children() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", false)]));
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
    // 占位行不可选中，光标直接定位到 b
    s.ws_cursor = 2;
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
    // 两个子行顶开占位：b 应后移到索引 3，光标保持指向 b
    s.ws_dir_ok("a", data("/p/a", vec![ent("a1", false), ent("a2", false)]));
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
    assert_eq!(s.ws_rows.len(), 4);
}

#[test]
fn ws_dir_failed_keeps_cursor_on_following_row() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", false)]));
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
    s.ws_cursor = 2; // b（占位行不可选中）
    assert!(s.ws_dir_failed("a"));
    assert_eq!(s.ws_rows.len(), 2);
    assert_eq!(
        s.ws_rows[s.ws_cursor].rel, "b",
        "收起顶开下行时保持指向原行"
    );
}

#[test]
fn ws_dir_failed_anchors_cursor_on_dir_row_when_inside() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()));
    s.ws_move(false); // 占位行不可选中，光标仍在目录行
    assert!(s.ws_dir_failed("a"));
    assert_eq!(s.ws_rows.len(), 1);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "a", "占位行消失后回到目录行");
}

fn pool(enabled: bool, projects: Vec<&str>) -> WorkspaceProjectsData {
    WorkspaceProjectsData {
        enabled,
        pool_path: None,
        projects: projects.into_iter().map(str::to_string).collect(),
    }
}

#[test]
fn ws_pick_open_close_and_projects() {
    let mut s = UiState::new();
    assert!(s.ws_open_pick(), "首次进入应拉项目池");
    assert!(s.ws_pick.as_ref().unwrap().loading);
    assert!(!s.ws_open_pick(), "已在选择中不重复进入");
    s.ws_pick_projects(Ok(pool(true, vec!["proj-a", "proj-b"])));
    let st = s.ws_pick.as_ref().unwrap();
    assert!(!st.loading);
    assert_eq!(
        st.projects,
        vec!["proj-a".to_string(), "proj-b".to_string()]
    );
    assert!(st.enabled);
    s.ws_close_pick();
    assert!(s.ws_pick.is_none());
}

#[test]
fn ws_pick_error_and_refresh_gating() {
    let mut s = UiState::new();
    s.ws_open_pick();
    s.ws_pick_projects(Err("boom".to_string()));
    assert_eq!(s.ws_pick.as_ref().unwrap().error.as_deref(), Some("boom"));
    assert!(s.ws_pick_refresh());
    assert!(!s.ws_pick_refresh(), "加载中不重复拉取");
    s.ws_pick_projects(Ok(pool(true, Vec::new())));
    assert_eq!(s.ws_pick.as_ref().unwrap().error, None);
}

#[test]
fn ws_pick_switch_success_closes_failure_keeps() {
    let mut s = UiState::new();
    s.ws_open_pick();
    s.ws_pick_projects(Ok(pool(true, vec!["proj-a", "proj-b"])));
    s.ws_pick_move(false);
    assert_eq!(s.ws_cursor, 1);
    assert_eq!(s.ws_pick_begin_switch(), Some("proj-b".to_string()));
    assert!(s.ws_pick_begin_switch().is_none(), "切换在途不重复");
    s.ws_pick_switch_result(Ok("/pool/proj-b".to_string()));
    assert!(s.ws_pick.is_none(), "成功关闭选择视图");

    s.ws_open_pick();
    s.ws_pick_projects(Ok(pool(true, vec!["proj-x"])));
    assert_eq!(s.ws_pick_begin_switch(), Some("proj-x".to_string()));
    s.ws_pick_switch_result(Err("denied".to_string()));
    assert!(s.ws_pick.is_some(), "失败留在选择视图");
    assert_eq!(s.ws_pick.as_ref().unwrap().error.as_deref(), Some("denied"));
    assert!(!s.ws_pick.as_ref().unwrap().busy);
}

#[test]
fn ws_pick_move_clamps_to_project_list() {
    let mut s = UiState::new();
    s.ws_open_pick();
    s.ws_pick_move(false); // 空列表不越界
    s.ws_pick_projects(Ok(pool(true, vec!["only"])));
    s.ws_pick_move(false);
    assert_eq!(s.ws_cursor, 0, "单项目不越界");
}

#[test]
fn ws_enter_on_file_returns_path() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true), ent("README.md", false)]));
    s.ws_cursor = 1;
    assert_eq!(
        s.ws_activate(),
        Some(WsActivate::File("README.md".to_string()))
    );
    assert_eq!(s.ws_cursor, 1, "文件行不移动光标");
}

#[test]
fn ws_enter_on_dir_toggles_and_reports_fetch() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true)]));
    assert_eq!(
        s.ws_activate(),
        Some(WsActivate::Dir {
            rel: "src".to_string(),
            fetch: true
        })
    );
    assert!(s.ws_expanded.contains("src"));
    assert_eq!(
        s.ws_activate(),
        Some(WsActivate::Dir {
            rel: "src".to_string(),
            fetch: false
        }),
        "第二次 Enter 收起且不再请求"
    );
    assert!(!s.ws_expanded.contains("src"));
}

#[test]
fn ws_right_expands_but_never_collapses() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true)]));
    assert_eq!(s.ws_expand_dir(), Some("src".to_string()));
    assert!(s.ws_expanded.contains("src"));
    assert_eq!(s.ws_expand_dir(), None, "已展开时 → 不收起");
    s.ws_dir_ok("src", data("/p/src", vec![ent("main.rs", false)]));
    assert_eq!(s.ws_rows.len(), 2);
    assert_eq!(s.ws_expand_dir(), None, "已展开目录 → 不重复请求");
    s.ws_cursor = 1;
    assert_eq!(s.ws_expand_dir(), None, "文件行 → 不动作");
}

#[test]
fn ws_empty_dir_shows_placeholder_and_skips_it() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("empty", true), ent("b.txt", false)]));
    assert_eq!(s.ws_toggle_dir(), Some("empty".to_string()));
    s.ws_dir_ok("empty", data("/p/empty", Vec::new()));
    assert_eq!(s.ws_rows.len(), 3, "空目录下补「（空）」占位");
    assert!(s.ws_rows[1].placeholder);
    assert_eq!(s.ws_rows[1].name, EMPTY_DIR_TEXT);
    s.ws_cursor = 0;
    s.ws_move(false);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b.txt", "Down 跳过占位行");
    s.ws_move(true);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "empty");
    s.ws_move(true);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "empty", "顶部不再动");
}

#[test]
fn ws_move_to_edge_and_page_skip_placeholders() {
    let mut s = UiState::new();
    s.ws_root_replace(data(
        "/p",
        vec![ent("a", true), ent("b.txt", false), ent("c.txt", false)],
    ));
    s.ws_toggle_dir();
    // rows: a, …, b.txt, c.txt
    s.ws_move_to_edge(true);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "c.txt");
    s.ws_move_to_edge(false);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "a");
    s.ws_page(true, 1);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b.txt", "翻页跳过占位");
}

#[test]
fn ws_begin_refresh_dedupes_until_result() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    assert!(s.ws_begin_refresh());
    assert!(!s.ws_begin_refresh(), "在途不重复入队");
    s.ws_root_failed("busy");
    assert!(s.ws_begin_refresh(), "失败后放行重试");
}

#[test]
fn ws_refresh_keep_restores_expansion_and_cursor() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true), ent("b.txt", false)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("a", data("/p/a", vec![ent("a1.txt", false)]));
    // rows: a, a1.txt, b.txt；光标停在 b.txt
    s.ws_cursor = 2;
    let refetch = s.ws_root_replace_keep(data("/p", vec![ent("a", true), ent("b.txt", false)]));
    assert_eq!(refetch, vec!["a".to_string()], "已展开目录需重拉");
    assert!(s.ws_expanded.contains("a"), "刷新保留展开态");
    assert_eq!(
        s.ws_rows[1].name, "a1.txt",
        "重拉前保留旧子列表，不退回占位"
    );
    assert!(!s.ws_rows[1].placeholder);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b.txt", "光标按 rel 还原");
    assert!(s.ws_loading.contains("a"));
}

#[test]
fn ws_refresh_keep_falls_back_to_nearest_ancestor_when_row_is_gone() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("a", data("/p/a", vec![ent("b", true)]));
    s.ws_cursor = 1; // a/b
    // 刷新不清缓存，所以正常情况下 a/b 一直可见、光标原地不动（见上一个测试）。
    // 这条用例构造的是缓存里那条子行真的没了（例如目录在别处被删）：
    // 撤掉子列表后刷新，a/b 不在树上，才需要落回最近存活祖先并记住目标 rel。
    s.ws_dir_cache.remove("a");
    let refetch = s.ws_root_replace_keep(data("/p", vec![ent("a", true)]));
    assert_eq!(refetch, vec!["a".to_string()]);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "a", "子行不可见时落回祖先");
    assert_eq!(s.ws_pending_cursor.as_deref(), Some("a/b"));
    // 子列表重载到位后光标回到原行
    s.ws_dir_ok("a", data("/p/a", vec![ent("b", true)]));
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "a/b", "重载后光标归位");
    assert!(s.ws_pending_cursor.is_none());
}

/// 刷新时子行仍可见 —— 光标不该为了「重拉」而先跳走再跳回。
#[test]
fn ws_refresh_keep_keeps_cursor_on_the_same_row() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("a", data("/p/a", vec![ent("b", true)]));
    s.ws_cursor = 1; // a/b
    let refetch = s.ws_root_replace_keep(data("/p", vec![ent("a", true)]));
    assert_eq!(refetch, vec!["a".to_string()]);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "a/b", "旧子行还在，光标不动");
    assert!(s.ws_pending_cursor.is_none(), "无需等重载归位");
}

#[test]
fn ws_refresh_keep_resets_when_root_changed() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("a", data("/p/a", vec![ent("x", false)]));
    s.ws_cursor = 1;
    let refetch = s.ws_root_replace_keep(data("/other", vec![ent("b", true)]));
    assert!(refetch.is_empty(), "根变了不保留旧展开集");
    assert!(!s.ws_expanded.contains("a"));
    assert_eq!(s.workspace_path.as_deref(), Some("/other"));
    assert_eq!(s.ws_rows.len(), 1);
    assert_eq!(s.ws_rows[s.ws_cursor].rel, "b");
    assert!(s.ws_pending_cursor.is_none());
}

/// 刷新根目录时，已展开目录的旧子列表必须留在树上 —— 不能退回 `…` 占位。
/// 这道测试挡的是「刷新排在慢请求后面 → 整棵树突然空掉」那条路径。
#[test]
fn root_refresh_keeps_children_until_reload() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("src", data("/p/src", vec![ent("main.rs", false)]));
    assert_eq!(s.ws_rows.len(), 2, "src 展开后有子行");

    // 刷新根：src 进入重拉，但旧子列表仍在
    let refetch = s.ws_root_replace_keep(data("/p", vec![ent("src", true)]));
    assert_eq!(refetch, vec!["src".to_string()]);
    assert!(s.ws_loading.contains("src"), "标记为在途，防重复请求");
    assert_eq!(s.ws_rows.len(), 2, "刷新在途时旧子列表仍渲染");
    assert!(
        s.ws_rows.iter().all(|r| !r.placeholder),
        "不该出现 `…` / `（空）` 占位行"
    );
    assert_eq!(s.ws_rows[1].name, "main.rs");

    // 重拉到达：按 rel 覆盖旧值
    s.ws_dir_ok("src", data("/p/src", vec![ent("lib.rs", false)]));
    assert_eq!(s.ws_rows[1].name, "lib.rs");
    assert!(!s.ws_loading.contains("src"));
}

/// 根路径变了（外部切走工作区）：旧缓存没有意义，必须清掉重来。
#[test]
fn root_refresh_to_a_new_path_drops_old_cache() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("src", true)]));
    s.ws_toggle_dir();
    s.ws_dir_ok("src", data("/p/src", vec![ent("main.rs", false)]));

    let refetch = s.ws_root_replace_keep(data("/other", vec![ent("docs", true)]));
    assert!(refetch.is_empty());
    assert!(s.ws_expanded.is_empty());
    assert!(!s.ws_dir_cache.contains_key("src"));
    assert_eq!(s.ws_rows.len(), 1);
    assert_eq!(s.ws_rows[0].name, "docs");
}

/// 重置树（首次/切换工作区）必须换代数：并发读请求回来后据此丢弃旧工作区的结果。
#[test]
fn root_replace_bumps_generation_but_refresh_does_not() {
    let mut s = UiState::new();
    let g0 = s.ws_generation;
    s.ws_root_replace(data("/a", vec![ent("x", true)]));
    let g1 = s.ws_generation;
    assert_ne!(g0, g1, "重置树必须换代数");
    // 同一工作区的手动刷新不算换工作区：在途的子目录重拉仍然有效。
    s.ws_root_replace_keep(data("/a", vec![ent("x", true)]));
    assert_eq!(s.ws_generation, g1, "刷新不该递增代数");
}

/// 刷新要把**折叠**目录也标成过期：否则 `r` 永远刷不到当时没展开的那些，
/// 用户折叠后重开会看到刷新前的旧内容，且没有任何机制纠正。
#[test]
fn root_refresh_marks_collapsed_dirs_stale() {
    let mut s = UiState::new();
    s.ws_root_replace(data("/p", vec![ent("a", true), ent("b", true)]));
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()), "首次展开要拉");
    s.ws_dir_ok("a", data("/p/a", vec![ent("a1", false)]));
    s.ws_toggle_dir(); // 折叠 a
    assert!(!s.ws_expanded.contains("a"));
    assert!(!s.ws_dir_stale.contains("a"));

    s.ws_root_replace_keep(data("/p", vec![ent("a", true), ent("b", true)]));
    assert!(s.ws_dir_stale.contains("a"), "刷新后旧缓存标记为过期");

    // 再展开：过期缓存要补拉，但**不**退回占位 —— 旧值继续显示。
    assert_eq!(s.ws_toggle_dir(), Some("a".to_string()), "过期缓存也要重拉");
    assert!(!s.ws_loading.contains("a"), "有缓存就不该显示 `…`");
    assert_eq!(s.ws_rows.len(), 3, "旧子行仍在树上");

    // 补拉到位后过期标记清除，之后的展开不再重复请求。
    s.ws_dir_ok("a", data("/p/a", vec![ent("a2", false)]));
    assert!(!s.ws_dir_stale.contains("a"));
    s.ws_toggle_dir(); // 折叠
    assert_eq!(s.ws_toggle_dir(), None, "已经新鲜了，不再重拉");
}
