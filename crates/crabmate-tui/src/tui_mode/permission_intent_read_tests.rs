//! 「一次 `/permission` 读失败不能抹掉已知的安全读数」的回归用例。
//!
//! 线上症状（本组用例的来源）：**首轮会话还没结束时按 Ctrl+C**，右下角状态栈的 `perm`
//! 行消失，而且再也回不来。链条是：`Ctrl+C` 本身不动权限状态 → 回合收尾拿到
//! `conversation_id` → `adopt_turn_conversation` 按会话重拉 `/permission` → 这次读取失败
//! （404 / 超时 / 空 level 都归成 `None`）→ `on_serve_status` 整体替换 `serve_defaults`，
//! 把上一份投影一起冲掉 → `status_values` 两侧都空 → 整段不画。
//!
//! 两条修复各守一半：`keep_known_view` 让「这次没读到」保留旧值；`adopt_turn_conversation`
//! 在投影缺失时同样重拉（原先只在 `conversation_id` 变化时才发），让失败能自愈。

use std::sync::mpsc;

use super::super::LaneApp;
use super::super::serve_defaults::ServeDefaults;
use super::super::test_support::{test_app, test_client, test_prefs};
use super::super::worker::WorkerJob;

/// 造一份只带权限投影的 `ServeDefaults`（走 JSON 反序列化，不把契约里的枚举名在本仓再抄一遍）。
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

/// 线上那条路径：首轮 + Ctrl+C + 拿到 cid 后的按会话读取失败 → `perm` 段仍在。
#[test]
fn a_cancelled_first_turn_keeps_the_perm_segment() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    // 首轮进行中：还没有会话 id，状态行靠裸 `/permission` 的估算值撑住（`~approved`）。
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    app.st.running = true;
    assert_eq!(
        app.status_info().permission.as_deref(),
        Some("~approved"),
        "未绑会话时显示的是估算档"
    );

    app.on_ctrl_c();
    let outcome = crate::serve::ChatStreamOutcome {
        conversation_id: Some("c-1".into()),
        cancelled_by_user: true,
        cancel_acknowledged: true,
        ..Default::default()
    };
    app.on_turn_done(outcome, None, None);

    // serve 这次没读到按会话的投影（404 / 超时 / 空 level 都落到这个形态）。
    app.on_serve_status(Some("c-1".into()), Ok(ServeDefaults::default()));

    assert_eq!(
        app.status_info().permission.as_deref(),
        Some("approved"),
        "一次读失败不该让 perm 段消失"
    );
}

/// 同上，但刷新回包连 `/status` 那一半也失败：同样不许抹掉已知读数。
#[test]
fn a_failed_refresh_keeps_the_known_level() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("c-1".into());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));

    app.on_serve_status(Some("c-1".into()), Err("timeout".into()));

    assert_eq!(
        app.st
            .serve_defaults
            .as_ref()
            .expect("defaults")
            .effective_level(),
        Some("approved"),
        "失败分支本来就不替换 serve_defaults"
    );
}

/// 保旧值**不**等于取消跨会话清空：切会话 / `/conv new` 走 `forget_permission_view`，
/// 清完之后没有旧值可保，后面的失败回包也不许把它填回来。
#[test]
fn switching_conversations_still_clears_the_projection() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, _job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("A".into());
    app.st.serve_defaults = Some(defaults_with_permission("unrestricted"));

    app.forget_permission_view();
    app.on_serve_status(Some("A".into()), Ok(ServeDefaults::default()));

    assert!(
        app.st
            .serve_defaults
            .as_ref()
            .expect("defaults")
            .permission_view
            .is_none(),
        "切会话清掉的投影不能被一次失败回包保回来"
    );
}

/// 同一个会话、投影缺失：回合结束要重拉一次（原先只在 cid 变化时发，失败就永远补不回来）。
#[test]
fn an_unknown_projection_is_refetched_on_the_next_turn() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("c-1".into());
    app.st.serve_defaults = Some(ServeDefaults::default());
    while job_rx.try_recv().is_ok() {}

    app.adopt_turn_conversation("c-1");

    assert!(
        matches!(
            job_rx.try_recv(),
            Ok(WorkerJob::RefreshStatus { conversation_id }) if conversation_id.as_deref() == Some("c-1")
        ),
        "投影缺失时同一会话也要重拉"
    );
}

/// 投影在、cid 没变：照旧一句请求都不多打（普通回合不额外付 `/status` + `/permission`）。
#[test]
fn an_ordinary_turn_end_sends_nothing_extra() {
    let client = test_client();
    let mut prefs = test_prefs();
    let (job_tx, job_rx) = mpsc::channel();
    let mut app = app(&client, &mut prefs, job_tx);

    app.st.conversation_id = Some("c-1".into());
    app.st.serve_defaults = Some(defaults_with_permission("approved"));
    while job_rx.try_recv().is_ok() {}

    app.adopt_turn_conversation("c-1");

    assert!(
        job_rx.try_recv().is_err(),
        "投影在、cid 没变 → 不该多打请求"
    );
}
