//! `main.rs` 的单元测试：从顶着 920 行门禁的 `main.rs` 拆出（`#[path]` 挂回去）。

use super::{
    ResumePoint, SessionPrefs, TermError, after_turn_outcome, cancel_failed_keeps_resume,
    capture_resume_point, handle_mode_slash, handle_model_slash, handle_role_slash,
};
use crate::serve::ChatStreamOutcome;
use anyhow::anyhow;

#[test]
fn interrupted_stream_error_records_resume_point() {
    let err = anyhow!(TermError::InterruptedStream {
        job_id: 11,
        after_seq: 5,
        cause: "connection reset".into(),
    });
    let mut resume = None;
    assert!(capture_resume_point(&mut resume, "hi", Some("c1"), &err));
    let p = resume.expect("point");
    assert_eq!(p.job_id, 11);
    assert_eq!(p.after_seq, 5);
    assert_eq!(p.message, "hi");
    assert_eq!(p.conversation_id.as_deref(), Some("c1"));
}

#[test]
fn plain_stream_error_does_not_record_resume_point() {
    let err = anyhow!(TermError::Stream("boom".into()));
    let mut resume = None;
    assert!(!capture_resume_point(&mut resume, "hi", None, &err));
    assert!(resume.is_none());
}

#[test]
fn cancel_failure_keeps_resume_flag() {
    assert!(cancel_failed_keeps_resume(true, false));
    assert!(!cancel_failed_keeps_resume(true, true));
    assert!(!cancel_failed_keeps_resume(false, false));
    assert!(!cancel_failed_keeps_resume(false, true));
}

#[test]
fn after_cancel_failure_records_current_run() {
    let outcome = ChatStreamOutcome {
        cancelled_by_user: true,
        cancel_acknowledged: false,
        job_id: Some(5),
        last_event_id: 2,
        ..ChatStreamOutcome::default()
    };
    let mut resume = None;
    let conv = Some("c9".to_string());
    after_turn_outcome(&outcome, "hello", &conv, &mut resume);
    let p = resume.expect("resume point kept on cancel failure");
    assert_eq!(p.job_id, 5);
    assert_eq!(p.after_seq, 2);
    assert_eq!(p.message, "hello");
    assert_eq!(p.conversation_id.as_deref(), Some("c9"));
}

#[test]
fn after_clean_or_acked_finish_clears_resume() {
    for (cancelled, acked) in [(false, false), (true, true)] {
        let outcome = ChatStreamOutcome {
            cancelled_by_user: cancelled,
            cancel_acknowledged: acked,
            job_id: Some(5),
            ..ChatStreamOutcome::default()
        };
        let mut resume = Some(ResumePoint {
            message: "old".into(),
            conversation_id: None,
            job_id: 1,
            after_seq: 0,
        });
        after_turn_outcome(&outcome, "hi", &None, &mut resume);
        assert!(resume.is_none(), "case {cancelled}/{acked} should clear");
    }
}

#[test]
fn llm_overrides_empty_yields_no_client_llm() {
    let o = SessionPrefs::from_options(None, None, None);
    assert!(o.client_llm().is_none());
}

#[test]
fn model_slash_sets_and_clears_override() {
    let mut o = SessionPrefs::from_options(None, None, None);
    handle_model_slash("/model deepseek-chat", &mut o);
    assert_eq!(o.model.as_deref(), Some("deepseek-chat"));
    assert!(o.client_llm().is_some());
    handle_model_slash("/model off", &mut o);
    assert!(o.model.is_none());
    assert!(o.client_llm().is_none());
}

#[test]
fn mode_slash_validates_sets_and_clears() {
    let mut p = SessionPrefs::from_options(None, None, None);
    handle_mode_slash("/mode act", &mut p);
    assert_eq!(p.session_mode.as_deref(), Some("act"));
    handle_mode_slash("/mode plan", &mut p);
    assert_eq!(p.session_mode.as_deref(), Some("plan"));
    handle_mode_slash("/mode bogus", &mut p);
    assert_eq!(
        p.session_mode.as_deref(),
        Some("plan"),
        "invalid mode ignored"
    );
    handle_mode_slash("/mode off", &mut p);
    assert!(p.session_mode.is_none());
}

#[test]
fn role_slash_sets_and_clears() {
    let mut p = SessionPrefs::from_options(None, None, None);
    handle_role_slash("/role coder", &mut p);
    assert_eq!(p.agent_role.as_deref(), Some("coder"));
    handle_role_slash("/role clear", &mut p);
    assert!(p.agent_role.is_none());
}

#[test]
fn prefs_carry_role_and_mode() {
    let mut p = SessionPrefs::from_options(Some("k".into()), None, None);
    handle_role_slash("/role coder", &mut p);
    handle_mode_slash("/mode ask", &mut p);
    let t = p.prefs();
    assert_eq!(t.agent_role, Some("coder"));
    assert_eq!(t.session_mode, Some("ask"));
    assert!(t.client_llm.is_some());
}
