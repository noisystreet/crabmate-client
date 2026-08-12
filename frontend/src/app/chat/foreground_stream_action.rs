//! 回前台时：有 `job_id` 则软续传 SSE，否则拉会话快照。

/// 页面从 hidden → visible 后应对进行中的 `/chat/stream` 采取的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForegroundStreamAction {
    /// 带 `stream_resume` 重新挂接 SSE。
    Resume {
        job_id: u64,
        after_seq: u64,
        session_id: String,
    },
    /// 无可用 job 句柄：清绑定（若有）并水合会话。
    Hydrate,
    /// 未曾进过后台，或无需处理。
    None,
}

/// `bound`：`(session_id, job_id)`；`None` 表示传输车道 Idle。
#[must_use]
pub(crate) fn foreground_stream_action_after_hidden(
    was_hidden: bool,
    bound: Option<(String, Option<u64>)>,
    after_seq: u64,
) -> ForegroundStreamAction {
    if !was_hidden {
        return ForegroundStreamAction::None;
    }
    match bound {
        Some((session_id, Some(job_id))) if !session_id.is_empty() => {
            ForegroundStreamAction::Resume {
                job_id,
                after_seq,
                session_id,
            }
        }
        Some(_) | None => ForegroundStreamAction::Hydrate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_when_never_hidden() {
        assert_eq!(
            foreground_stream_action_after_hidden(false, Some(("s1".into(), Some(9))), 3),
            ForegroundStreamAction::None
        );
    }

    #[test]
    fn resumes_when_bound_with_job() {
        assert_eq!(
            foreground_stream_action_after_hidden(true, Some(("s1".into(), Some(42))), 7),
            ForegroundStreamAction::Resume {
                job_id: 42,
                after_seq: 7,
                session_id: "s1".into(),
            }
        );
    }

    #[test]
    fn hydrates_when_bound_without_job() {
        assert_eq!(
            foreground_stream_action_after_hidden(true, Some(("s1".into(), None)), 0),
            ForegroundStreamAction::Hydrate
        );
    }

    #[test]
    fn hydrates_when_idle_after_hidden() {
        assert_eq!(
            foreground_stream_action_after_hidden(true, None, 0),
            ForegroundStreamAction::Hydrate
        );
    }
}
