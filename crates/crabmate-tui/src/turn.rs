//! 单轮对话：生成 approval_session_id 并跑 `/chat/stream`（含传输层中断的自动续流）。

use std::io::{self, Write};
use std::time::Duration;

use crate::serve::{
    ApprovalGate, ChatStreamArgs, ChatStreamOutcome, ClientLlm, ServeClient, StreamResume,
    TermError, new_approval_session_id, run_chat_stream,
};
use anyhow::Result;

/// 传输层中断（[`TermError::InterruptedStream`]）后自动续流的最大次数（不含首次尝试）。
const AUTO_RESUME_MAX_ATTEMPTS: u8 = 2;
/// 自动续流退避基数：第 n 次重试等待 `n × 基数`，给 serve 端 hub 消化断连的时间。
const AUTO_RESUME_BACKOFF: Duration = Duration::from_millis(500);

/// 每轮附加配置：`client_llm` 覆盖 + agent role + 会话模式（由调用方从持久状态派生）。
#[derive(Debug, Clone, Copy)]
pub struct TurnPrefs<'a> {
    pub client_llm: Option<ClientLlm<'a>>,
    pub agent_role: Option<&'a str>,
    pub session_mode: Option<&'a str>,
}

pub async fn run_turn(
    client: &ServeClient,
    message: &str,
    conversation_id: Option<&str>,
    prefs: TurnPrefs<'_>,
    stream_resume: Option<StreamResume>,
    approval: &mut dyn ApprovalGate,
) -> Result<ChatStreamOutcome> {
    let approval_session_id = new_approval_session_id();
    // 不跨 await 长期持有 StdoutLock/StderrLock（审批提示需自行写 stderr）。
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut resume = stream_resume;
    let mut attempts = 0u8;
    loop {
        let outcome = run_chat_stream(
            client,
            ChatStreamArgs {
                message,
                conversation_id,
                approval_session_id: &approval_session_id,
                client_llm: prefs.client_llm,
                agent_role: prefs.agent_role,
                session_mode: prefs.session_mode,
                stream_resume: resume,
            },
            &mut stdout,
            &mut stderr,
            approval,
        )
        .await;
        let err = match outcome {
            Ok(outcome) => return Ok(outcome),
            Err(e) => e,
        };
        // 仅传输层中断且 serve 侧 job 仍可续时重试；其余（用户 Ctrl+C、HTTP 报错）原样上报。
        let Some(point) = auto_resume_point(attempts, &err) else {
            return Err(err.into());
        };
        attempts += 1;
        resume = Some(point);
        let (job_id, after_seq) = (point.job_id, point.after_seq);
        writeln!(
            stderr,
            "\n[crabmate-tui] stream interrupted (job {job_id}, seq {after_seq}); \
             auto-resuming {attempts}/{AUTO_RESUME_MAX_ATTEMPTS} ..."
        )?;
        tokio::time::sleep(AUTO_RESUME_BACKOFF * u32::from(attempts)).await;
    }
}

/// 自动续流策略：已达次数上限、或错误不是「已拿到 job 句柄的传输层中断」时返回 `None`。
fn auto_resume_point(attempts: u8, err: &TermError) -> Option<StreamResume> {
    if attempts >= AUTO_RESUME_MAX_ATTEMPTS {
        return None;
    }
    match err {
        TermError::InterruptedStream {
            job_id, after_seq, ..
        } => Some(StreamResume {
            job_id: *job_id,
            after_seq: *after_seq,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interrupted() -> TermError {
        TermError::InterruptedStream {
            job_id: 7,
            after_seq: 42,
            cause: "connection reset".to_string(),
        }
    }

    #[test]
    fn auto_resume_reuses_job_and_seq() {
        let p = auto_resume_point(0, &interrupted()).expect("可续流");
        assert_eq!((p.job_id, p.after_seq), (7, 42));
    }

    #[test]
    fn auto_resume_stops_at_max_attempts() {
        assert!(auto_resume_point(AUTO_RESUME_MAX_ATTEMPTS, &interrupted()).is_none());
    }

    #[test]
    fn auto_resume_skips_non_transport_errors() {
        assert!(auto_resume_point(0, &TermError::Interrupted).is_none());
        assert!(auto_resume_point(0, &TermError::Stream("bad".to_string())).is_none());
        assert!(
            auto_resume_point(
                0,
                &TermError::Http {
                    status: 410,
                    body: "STREAM_JOB_GONE".to_string(),
                }
            )
            .is_none()
        );
    }
}
