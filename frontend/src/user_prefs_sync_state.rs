//! `/user-data/prefs` 同步显式状态机：区分「加载尝试已结束」与「允许写入服务端」。

/// `GET /user-data/prefs` 与防抖 `PUT` 的生命周期。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UserPrefsSyncPhase {
    /// 首启或重试 GET 进行中。
    #[default]
    Loading,
    /// GET 成功；允许防抖 PUT。
    Ready,
    /// GET 失败；禁止 PUT（避免默认值覆盖服务端偏好）。
    LoadFailed,
    /// 最近一次 PUT 失败；仍允许后续防抖重试。
    SaveFailed,
}

impl UserPrefsSyncPhase {
    #[must_use]
    pub const fn allows_persist(self) -> bool {
        matches!(self, Self::Ready | Self::SaveFailed)
    }

    #[must_use]
    pub const fn load_attempt_finished(self) -> bool {
        !matches!(self, Self::Loading)
    }
}

#[must_use]
pub const fn user_prefs_allows_persist(phase: UserPrefsSyncPhase) -> bool {
    phase.allows_persist()
}

#[must_use]
pub const fn user_prefs_load_attempt_finished(phase: UserPrefsSyncPhase) -> bool {
    phase.load_attempt_finished()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_failed_blocks_persist() {
        assert!(!UserPrefsSyncPhase::LoadFailed.allows_persist());
        assert!(!user_prefs_allows_persist(UserPrefsSyncPhase::Loading));
    }

    #[test]
    fn ready_and_save_failed_allow_persist() {
        assert!(UserPrefsSyncPhase::Ready.allows_persist());
        assert!(UserPrefsSyncPhase::SaveFailed.allows_persist());
    }

    #[test]
    fn only_loading_blocks_initialized_gate() {
        assert!(!user_prefs_load_attempt_finished(
            UserPrefsSyncPhase::Loading
        ));
        assert!(user_prefs_load_attempt_finished(UserPrefsSyncPhase::Ready));
        assert!(user_prefs_load_attempt_finished(
            UserPrefsSyncPhase::LoadFailed
        ));
        assert!(user_prefs_load_attempt_finished(
            UserPrefsSyncPhase::SaveFailed
        ));
    }

    #[test]
    fn persist_gate_by_phase() {
        for (phase, persist) in [
            (UserPrefsSyncPhase::Loading, false),
            (UserPrefsSyncPhase::LoadFailed, false),
            (UserPrefsSyncPhase::Ready, true),
            (UserPrefsSyncPhase::SaveFailed, true),
        ] {
            assert_eq!(user_prefs_allows_persist(phase), persist, "phase {phase:?}");
        }
    }
}
