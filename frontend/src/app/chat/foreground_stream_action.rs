//! 回前台时：仅在连接已断时软续传；Idle 仅在后台曾忙时水合。

/// 页面从 hidden → visible 后应对进行中的 `/chat/stream` 采取的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForegroundStreamAction {
    /// 带 `stream_resume` 重新挂接 SSE（仅 abort 槽已空、句柄仍在时）。
    Resume {
        job_id: u64,
        after_seq: u64,
        session_id: String,
    },
    /// 清卡住的 Bound（若有）并水合会话。
    Hydrate,
    /// 未曾进过后台，或健康流仍在跑、或 Idle 且后台无忙态。
    None,
}

/// 回前台决策输入（避免超长形参）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForegroundStreamDecisionInput {
    pub was_hidden: bool,
    /// `(session_id, job_id)`；`None` = Idle。
    pub bound: Option<(String, Option<u64>)>,
    pub after_seq: u64,
    /// 壳上仍登记着 `AbortController`（视为 attach 存活，勿强杀重挂以免清空 scratch）。
    pub abort_present: bool,
    /// 进入后台时曾有进行中流 / Loading 占位；用于 Idle 回前台是否值得水合。
    pub was_busy_when_hidden: bool,
}

/// `bound`：`(session_id, job_id)`；`None` 表示传输车道 Idle。
#[must_use]
pub(crate) fn foreground_stream_action_after_hidden(
    input: ForegroundStreamDecisionInput,
) -> ForegroundStreamAction {
    let ForegroundStreamDecisionInput {
        was_hidden,
        bound,
        after_seq,
        abort_present,
        was_busy_when_hidden,
    } = input;
    if !was_hidden {
        return ForegroundStreamAction::None;
    }
    match bound {
        Some((session_id, Some(job_id))) if !session_id.is_empty() => {
            if abort_present {
                // 健康/半活 attach：保留原 scratch，交由 `send_chat_stream` 内重连。
                ForegroundStreamAction::None
            } else {
                ForegroundStreamAction::Resume {
                    job_id,
                    after_seq,
                    session_id,
                }
            }
        }
        Some(_) => ForegroundStreamAction::Hydrate,
        None if was_busy_when_hidden => ForegroundStreamAction::Hydrate,
        None => ForegroundStreamAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        was_hidden: bool,
        bound: Option<(String, Option<u64>)>,
        after_seq: u64,
        abort_present: bool,
        was_busy_when_hidden: bool,
    ) -> ForegroundStreamDecisionInput {
        ForegroundStreamDecisionInput {
            was_hidden,
            bound,
            after_seq,
            abort_present,
            was_busy_when_hidden,
        }
    }

    #[test]
    fn ignores_when_never_hidden() {
        assert_eq!(
            foreground_stream_action_after_hidden(input(
                false,
                Some(("s1".into(), Some(9))),
                3,
                false,
                true,
            )),
            ForegroundStreamAction::None
        );
    }

    #[test]
    fn skips_resume_when_abort_still_present() {
        assert_eq!(
            foreground_stream_action_after_hidden(input(
                true,
                Some(("s1".into(), Some(42))),
                7,
                true,
                true,
            )),
            ForegroundStreamAction::None
        );
    }

    #[test]
    fn resumes_when_bound_with_job_but_abort_gone() {
        assert_eq!(
            foreground_stream_action_after_hidden(input(
                true,
                Some(("s1".into(), Some(42))),
                7,
                false,
                true,
            )),
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
            foreground_stream_action_after_hidden(input(
                true,
                Some(("s1".into(), None)),
                0,
                false,
                false,
            )),
            ForegroundStreamAction::Hydrate
        );
    }

    #[test]
    fn hydrates_idle_only_when_was_busy() {
        assert_eq!(
            foreground_stream_action_after_hidden(input(true, None, 0, false, true)),
            ForegroundStreamAction::Hydrate
        );
        assert_eq!(
            foreground_stream_action_after_hidden(input(true, None, 0, false, false)),
            ForegroundStreamAction::None
        );
    }
}
