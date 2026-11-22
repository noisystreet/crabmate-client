//! 顶栏工作区标题旁的 git 徽标文案（纯映射，便于单测）。
//!
//! 技术标识（`main ●5` / `HEAD@1a2b3c4`）由 `crabmate-client-api` 的 `workspace_git`
//! 统一产出；这里只补两种 UI 侧的词：**三态降级文案**与**计数明细提示**。
//!
//! 口径提醒：`dirty` 是「**当前工作区根下**有未提交改动」，不是「仓库干净」的补集；
//! 与会话变更集（`GET /workspace/changelog`，本会话改了什么）是两个概念，措辞不得混用。

use crabmate::cm_api_contract::workspace::{WorkspaceGitState, WorkspaceGitStatusResponse};

use crate::i18n::{self, Locale};

/// 徽标一行文本：优先技术标识（`main ●5`），`state` 非 `ok` 时给对应的降级文案。
pub fn git_badge_line(resp: &WorkspaceGitStatusResponse, loc: Locale) -> String {
    if let Some(text) = crabmate_client_api::badge_text(resp) {
        return text;
    }
    match resp.state {
        // `ok` 但没有 `head`：契约不该出现；给一句中性文案而不是空白。
        WorkspaceGitState::Ok => i18n::ws_git_not_a_repo(loc).to_string(),
        WorkspaceGitState::NotARepo => i18n::ws_git_not_a_repo(loc).to_string(),
        WorkspaceGitState::GitUnavailable => i18n::ws_git_unavailable(loc).to_string(),
        WorkspaceGitState::Failed => i18n::ws_git_failed(loc).to_string(),
    }
}

/// 悬停提示：有改动给分类计数，干净给「无未提交改动」。
///
/// 计数按分类给（同一文件可同时 staged 与 unstaged），与服务端 `counts` 口径一致；
/// 明细里不展开文件名——文件列表留给将来独立端点。
pub fn git_badge_title(resp: &WorkspaceGitStatusResponse, loc: Locale) -> String {
    match crabmate_client_api::counts(resp) {
        Some(counts) if counts.total > 0 => i18n::ws_git_counts_title(
            loc,
            counts.staged,
            counts.unstaged,
            counts.untracked,
            counts.conflicted,
        ),
        _ => i18n::ws_git_clean_title(loc).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{git_badge_line, git_badge_title};
    use crate::i18n::Locale;
    use crabmate::cm_api_contract::workspace::{
        WorkspaceGitCounts, WorkspaceGitHead, WorkspaceGitSource, WorkspaceGitState,
        WorkspaceGitStatusResponse,
    };

    fn base(state: WorkspaceGitState) -> WorkspaceGitStatusResponse {
        WorkspaceGitStatusResponse {
            state,
            reason_code: None,
            message: None,
            head: Some(WorkspaceGitHead::Branch {
                name: "main".into(),
                oid: Some("1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9012".into()),
            }),
            upstream: None,
            ahead: None,
            behind: None,
            dirty: Some(false),
            counts: Some(WorkspaceGitCounts::default()),
            computed_at_unix: 1_735_689_600,
            source: WorkspaceGitSource::Probe,
        }
    }

    #[test]
    fn ok_state_uses_the_technical_label() {
        let mut r = base(WorkspaceGitState::Ok);
        assert_eq!(git_badge_line(&r, Locale::En), "main");
        r.dirty = Some(true);
        r.counts = Some(WorkspaceGitCounts {
            staged: 1,
            unstaged: 2,
            untracked: 0,
            conflicted: 0,
            total: 3,
        });
        assert_eq!(git_badge_line(&r, Locale::En), "main ●3");
    }

    #[test]
    fn detached_head_is_not_blank() {
        let mut r = base(WorkspaceGitState::Ok);
        r.head = Some(WorkspaceGitHead::Detached {
            oid: Some("1a2b3c4d5e6f".into()),
        });
        assert_eq!(git_badge_line(&r, Locale::En), "HEAD@1a2b3c4");
    }

    #[test]
    fn degraded_states_are_distinct_in_both_locales() {
        let not_repo = base(WorkspaceGitState::NotARepo);
        let unavailable = base(WorkspaceGitState::GitUnavailable);
        let mut failed = base(WorkspaceGitState::Failed);
        failed.reason_code = Some("GIT_PROBE_TIMEOUT".into());

        for loc in [Locale::ZhHans, Locale::En] {
            let a = git_badge_line(&not_repo, loc);
            let b = git_badge_line(&unavailable, loc);
            let c = git_badge_line(&failed, loc);
            // 三态文案必须两两不同（否则 UI 分不清该说哪句）。
            assert_ne!(a, b, "{loc:?}");
            assert_ne!(b, c, "{loc:?}");
            assert_ne!(a, c, "{loc:?}");
            assert!(!a.is_empty() && !b.is_empty() && !c.is_empty());
        }
    }

    #[test]
    fn title_reports_classified_counts_and_clean_state() {
        let mut dirty = base(WorkspaceGitState::Ok);
        dirty.counts = Some(WorkspaceGitCounts {
            staged: 1,
            unstaged: 2,
            untracked: 3,
            conflicted: 4,
            total: 10,
        });
        let title = git_badge_title(&dirty, Locale::En);
        for n in ["1", "2", "3", "4"] {
            assert!(
                title.contains(n),
                "counts title should mention {n}: {title}"
            );
        }

        let clean = base(WorkspaceGitState::Ok);
        assert_eq!(
            git_badge_title(&clean, Locale::En),
            crate::i18n::ws_git_clean_title(Locale::En)
        );
    }

    #[test]
    fn non_ok_state_without_counts_falls_back_to_clean_title() {
        // `state != ok` 时 `counts` 缺省：不要因 `None` 而显示「有 N 处改动」。
        let r = base(WorkspaceGitState::Failed);
        assert_eq!(
            git_badge_title(&r, Locale::En),
            crate::i18n::ws_git_clean_title(Locale::En)
        );
    }
}
