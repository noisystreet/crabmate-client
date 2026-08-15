//! 截断再生与失败助手重试的**单一待发队列**（显式状态机）。
//!
//! 由 **`composer_wires::follow_up`** 侧单一 `Effect` 消费，取代两个互不协调的 `Option` 信号，避免双 `Effect` 对 `attach` 的隐式顺序依赖。

/// 合成器在「用户未点发送」前提下、待 `/chat/stream` `attach` 的后续动作。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum ComposerStreamFollowUp {
    /// 无待发动作。
    #[default]
    Idle,
    /// 重试当前失败助手气泡：由 [`crate::session_ops::prepare_retry_failed_assistant_turn`] 解析后再 `attach`。
    RetryFailedAssistant { failed_asst_id: String },
    /// 用户线截断后再生：已备好 `(user_text, imgs, loading_asst_id)`，条件满足即 `attach`。
    RegenerateAfterTruncate {
        user_text: String,
        user_imgs: Vec<String>,
        asst_id: String,
    },
    /// 流式进行中排队的下一句（当前轮结束后 `attach`；切走会话则写回该会话草稿）。
    QueuedUserMessage {
        session_id: String,
        user_text: String,
        user_imgs: Vec<String>,
    },
}

impl ComposerStreamFollowUp {
    /// 再生 / 失败重试尚未 `attach` 时，主发送不得覆盖队列。
    #[must_use]
    pub(crate) const fn blocks_user_queue(&self) -> bool {
        matches!(
            self,
            Self::RetryFailedAssistant { .. } | Self::RegenerateAfterTruncate { .. }
        )
    }

    /// 排队正文预览（芯片）；其它变体为 `None`。
    #[must_use]
    pub(crate) fn queued_user_text(&self) -> Option<&str> {
        match self {
            Self::QueuedUserMessage { user_text, .. } => Some(user_text.as_str()),
            _ => None,
        }
    }

    /// 放弃排队时应写回草稿的 `(session_id, user_text)`。
    #[must_use]
    pub(crate) fn queued_draft_to_park(&self) -> Option<(&str, &str)> {
        match self {
            Self::QueuedUserMessage {
                session_id,
                user_text,
                ..
            } if !user_text.is_empty() => Some((session_id.as_str(), user_text.as_str())),
            _ => None,
        }
    }

    /// 就地编辑保存会 `set` 再生队列，非 `Idle` 时不得覆盖。
    #[must_use]
    pub(crate) const fn blocks_user_edit_save(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::ComposerStreamFollowUp;

    #[test]
    fn regen_and_retry_block_queue() {
        assert!(
            ComposerStreamFollowUp::RetryFailedAssistant {
                failed_asst_id: "a".into(),
            }
            .blocks_user_queue()
        );
        assert!(
            ComposerStreamFollowUp::RegenerateAfterTruncate {
                user_text: "x".into(),
                user_imgs: vec![],
                asst_id: "b".into(),
            }
            .blocks_user_queue()
        );
        assert!(!ComposerStreamFollowUp::Idle.blocks_user_queue());
        assert!(
            !ComposerStreamFollowUp::QueuedUserMessage {
                session_id: "s".into(),
                user_text: "n".into(),
                user_imgs: vec![],
            }
            .blocks_user_queue()
        );
    }

    #[test]
    fn queued_draft_parks_when_leaving_that_session() {
        let q = ComposerStreamFollowUp::QueuedUserMessage {
            session_id: "a".into(),
            user_text: "hello".into(),
            user_imgs: vec![],
        };
        assert_eq!(q.queued_draft_to_park(), Some(("a", "hello")));
        assert!(q.queued_draft_to_park().is_some_and(|(sid, _)| sid != "b"));
        assert!(q.queued_draft_to_park().is_some_and(|(sid, _)| sid == "a"));
        assert!(q.blocks_user_edit_save());
        assert!(!ComposerStreamFollowUp::Idle.blocks_user_edit_save());
    }
}
