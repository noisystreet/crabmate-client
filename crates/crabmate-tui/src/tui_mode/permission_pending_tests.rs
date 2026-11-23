//! `perm` 段的 **pending** 口径：意图已提交、服务端尚未确认时不得并列。
//!
//! 回归场景（用户报告）：`/perm unrestricted` 之后状态行出现 `unrestricted→approved`
//! —— 左侧是刚选的意图（本地立刻生效），右侧是**上一次** `/permission` 的读数。
//! 请求在途（worker 串行时更久）或写失败时都会挂着这个箭头，读起来像「服务端把我钳制了」。
//!
//! 口径落点：`strategy` 见共享层 `permission::perm_display_with`，本文件只钉 TUI 侧的
//! 「什么时候算 pending / 失败后旧读数怎么处置」。

use std::sync::mpsc;

use super::super::serve_defaults::ServeDefaults;
use super::super::status_info::perm_value;
use super::super::test_support::{test_app, test_client, test_prefs};
use super::super::worker::WorkerJob;
use super::LaneApp;

/// 只带权限投影的 `ServeDefaults`（走 JSON 反序列化，不在本仓再抄一遍契约枚举）。
fn defaults_with_permission(level: &str) -> ServeDefaults {
    let json = format!(
        r#"{{"level":"{level}","level_source":"persisted","ceiling":"unrestricted",
             "ceiling_source":"derived_loopback",
             "available":["chat","read","workspace","approved","unrestricted"],
             "clamped_by":[],
             "caps":{{"tool_visibility":"full","external_tools":"allowed",
                      "path_scope":"any","command_scope":"any","net_scope":"any",
                      "approval":"audit_only"}}}}"#
    );
    ServeDefaults {
        permission_view: Some(serde_json::from_str(&json).expect("permission view")),
        ..Default::default()
    }
}

fn app<'a>(
    client: &'a super::super::super::serve::ServeClient,
    prefs: &'a mut crate::SessionPrefs,
    job_tx: mpsc::Sender<WorkerJob>,
) -> LaneApp<'a> {
    let (ui_tx, ui_rx) = mpsc::channel();
    test_app(client, prefs, ui_rx, ui_tx, job_tx)
}

/// 状态行 `perm` 段的最终文本（与渲染同一条路径：`status_info` + 共享层 `perm_value`）。
fn perm_text(app: &LaneApp<'_>) -> Option<String> {
    let info = app.status_info();
    perm_value(
        info.permission_intent.as_deref(),
        info.permission.as_deref(),
        info.permission_pending,
    )
}

/// 写请求在途：只显示意图并标 `~`，**不**并列。
#[test]
fn an_in_flight_write_never_pairs_intent_with_a_stale_level() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".to_string());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));

    app.apply_permission_level("unrestricted");

    // 请求已出站（等待回包），生效侧仍是上一次的 approved。
    assert!(matches!(
        job_rx.try_recv(),
        Ok(WorkerJob::WritePermission { level, .. }) if level == "unrestricted"
    ));
    assert_eq!(
        perm_text(&app),
        Some("~unrestricted".to_string()),
        "pending 期间不得并列成 `unrestricted→approved`"
    );
}

/// 写失败：旧读数已经不代表此刻生效 → 折掉，箭头不许留着。
#[test]
fn a_failed_write_drops_the_stale_effective_level() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".to_string());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    app.apply_permission_level("unrestricted");
    let generation = app.st.permission_generation;

    app.on_permission_written("unrestricted", Err("boom".into()), generation);

    assert!(
        app.st
            .serve_defaults
            .as_ref()
            .expect("defaults")
            .permission_view
            .is_none(),
        "写失败后旧投影必须折掉（否则箭头永挂）"
    );
    assert_eq!(perm_text(&app), Some("~unrestricted".to_string()));
}

/// 换会话：pending 跟着作废（旧会话的回包不再合法，见 `forget_permission_view`）。
#[test]
fn switching_sessions_clears_pending() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".to_string());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    app.apply_permission_level("unrestricted");
    assert!(app.st.permission_pending);

    app.forget_permission_view();
    assert!(!app.st.permission_pending);
}

/// 拿到**同会话的权威读数**就等于意图已被确认：pending 必须清，否则一次写失败会让状态行
/// 永远停在 `~intent`（写请求还可能被读请求推进的代数判成 stale，永远收不到成功回包）。
#[test]
fn an_authoritative_projection_clears_pending() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".to_string());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    app.apply_permission_level("unrestricted");
    let generation = app.st.permission_generation;
    // 写失败：pending 保持，状态行 `~unrestricted`。
    app.on_permission_written("unrestricted", Err("boom".into()), generation);
    assert!(app.st.permission_pending);

    // 服务端随后给出同会话的权威投影（意图已生效）：确认到达 → 清 pending 并折叠成单值。
    app.on_serve_status(
        Some("A".to_string()),
        Ok(defaults_with_permission("unrestricted")),
    );
    assert!(!app.st.permission_pending, "权威读数到手即确认");
    assert_eq!(perm_text(&app), Some("unrestricted".to_string()));
}

/// 读失败（投影为 `None`）不算确认：pending 不清，旧读数也不会被抹掉。
#[test]
fn a_failed_status_read_is_not_a_confirmation() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".to_string());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    app.apply_permission_level("unrestricted");
    let generation = app.st.permission_generation;
    app.on_permission_written("unrestricted", Err("boom".into()), generation);

    // 空投影（`/permission` 失败被归成 None）：`keep_known_view` 保旧值，pending 保持。
    app.on_serve_status(Some("A".to_string()), Ok(ServeDefaults::default()));
    assert!(app.st.permission_pending);
    assert_eq!(perm_text(&app), Some("~unrestricted".to_string()));
}
