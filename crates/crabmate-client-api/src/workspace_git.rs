//! 工作区 git 状态投影（`GET /workspace/git`）→ 展示语义。
//!
//! 纯逻辑，Web UI 与 TUI 共用（本 crate 禁 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! **文案分工**：这里只产出**技术标识**——分支名、`HEAD@<短 oid>`、脏计数标记 `●N`，
//! 这些在任何语言下写法相同。三种降级态（非 git 仓库 / git 不可用 / 读取失败）的那句话
//! 交给各自 UI 的 i18n（Desktop 有 zh / en，TUI 只有中文）。
//!
//! **不做截断**：Desktop 交给 CSS `text-overflow: ellipsis`，TUI 有自己的
//! `truncate_display`（基于 `unicode-width`，比手写两档表更准）。曾经在这里实现过一套
//! 自己的宽度表，结果是两端都不用、仓库里多出第二个口径 —— 已删除。
//!
//! **口径**（与 Server `docs/design/workspace_git_status_projection.md` 一致）：
//! - `dirty` 指「**当前工作区根下**有无未提交改动」，不等于整个仓库干净；
//! - 本模块**不**把 git 脏状态与会话变更集（`GET /workspace/changelog`）混用，
//!   两者是不同概念，UI 必须分措辞。

use crabmate::cm_api_contract::workspace::{
    WorkspaceGitHead, WorkspaceGitState, WorkspaceGitStatusResponse,
};

/// detached HEAD 的展示前缀：`HEAD@1a2b3c4`（不返回空串冒充分支名）。
pub const DETACHED_LABEL_PREFIX: &str = "HEAD@";

/// 脏标记字符。**纯文本、不靠颜色**表达，色弱 / 无色终端同样可读。
pub const DIRTY_MARKER: char = '●';

/// 短 oid 的长度（与 `git` 默认 `--short` 的 7 位一致）。
const SHORT_OID_LEN: usize = 7;

/// 单行徽标 + 脏标记，如 `main ●5`；不可用时返回 `None`（由 UI 给 i18n 文案）。
///
/// 只有 `state == Ok` 才有徽标：`not_a_repo` / `git_unavailable` / `failed` 都不该
/// 渲染成「有分支名但空着」的形态（那正是 issue 里点名要避免的误导性空白）。
/// `source == StaleCache` 仍返回徽标——它是同一真源的旧值，由 UI 决定是否弱化显示。
pub fn badge_text(resp: &WorkspaceGitStatusResponse) -> Option<String> {
    if resp.state != WorkspaceGitState::Ok {
        return None;
    }
    let head = resp.head.as_ref()?;
    let label = head_label(head);
    match dirty_marker(resp) {
        Some(marker) => Some(format!("{label} {marker}")),
        None => Some(label),
    }
}

/// `head` 的展示标签：`main` / `HEAD@1a2b3c4`；`oid` 缺失时为 `HEAD@?`。
pub fn head_label(head: &WorkspaceGitHead) -> String {
    match head {
        WorkspaceGitHead::Branch { name, .. } if !name.is_empty() => name.clone(),
        // 空分支名不可信（契约用「字段缺失」而非空串区分 detached），退回 oid 形态。
        WorkspaceGitHead::Branch { oid, .. } | WorkspaceGitHead::Detached { oid } => {
            format!(
                "{DETACHED_LABEL_PREFIX}{}",
                oid.as_deref().map(short_oid).unwrap_or("?")
            )
        }
    }
}

/// 取短 oid：前 [`SHORT_OID_LEN`] 个字符（按字符切，不按字节切，避免非 ASCII 边界 panic）。
pub fn short_oid(oid: &str) -> &str {
    match oid.char_indices().nth(SHORT_OID_LEN) {
        Some((idx, _)) => &oid[..idx],
        None => oid,
    }
}

/// 「有没有未提交改动」的**唯一**判定 —— 文本标记与着色都必须问它。
///
/// `dirty` 与 `counts` 是响应里两个**独立**的可选字段。各写一遍判定，在两者不一致时
/// 就会自相矛盾：徽标文本没有 `●`、颜色却是警示色，而「字符本身即信息」正是这里对
/// 色弱 / 无色终端的承诺。服务端目前保证 `dirty == (total > 0)`，但那是服务端实现细节，
/// 客户端不该各自复述。
///
/// 取「或」：有计数就算脏（`dirty` 缺省时不能把「字段没给」渲染成「干净」）。
pub fn is_dirty(resp: &WorkspaceGitStatusResponse) -> bool {
    resp.counts.map(|c| c.total > 0).unwrap_or(false) || resp.dirty.unwrap_or(false)
}

/// 脏标记：`●N`（N = `counts.total`）/ `●`（脏但计数为 0，理论上不出现）；干净返回 `None`。
pub fn dirty_marker(resp: &WorkspaceGitStatusResponse) -> Option<String> {
    if !is_dirty(resp) {
        return None;
    }
    let total = resp.counts.map(|c| c.total).unwrap_or(0);
    if total > 0 {
        Some(format!("{DIRTY_MARKER}{total}"))
    } else {
        Some(DIRTY_MARKER.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crabmate::cm_api_contract::workspace::WorkspaceGitCounts;

    fn resp(
        state: WorkspaceGitState,
        head: Option<WorkspaceGitHead>,
    ) -> WorkspaceGitStatusResponse {
        WorkspaceGitStatusResponse {
            state,
            reason_code: None,
            message: None,
            head,
            upstream: None,
            ahead: None,
            behind: None,
            dirty: Some(false),
            counts: Some(WorkspaceGitCounts::default()),
            computed_at_unix: 1_735_689_600,
            source: crabmate::cm_api_contract::workspace::WorkspaceGitSource::Probe,
        }
    }

    fn branch(name: &str) -> Option<WorkspaceGitHead> {
        Some(WorkspaceGitHead::Branch {
            name: name.into(),
            oid: Some("1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9012".into()),
        })
    }

    fn counts_of(total: u32) -> Option<WorkspaceGitCounts> {
        Some(WorkspaceGitCounts {
            staged: 1,
            unstaged: 2,
            untracked: total.saturating_sub(3),
            conflicted: 0,
            total,
        })
    }

    #[test]
    fn clean_branch_has_no_marker() {
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        assert_eq!(badge_text(&r).as_deref(), Some("main"));
        assert!(!is_dirty(&r));
        r.dirty = Some(true);
        r.counts = counts_of(5);
        assert_eq!(badge_text(&r).as_deref(), Some("main ●5"));
        assert!(is_dirty(&r));
    }

    #[test]
    fn dirty_without_counts_still_shows_marker() {
        // dirty=true 但 total=0（理论上不出现）：宁可显示裸标记，也不要显示成干净。
        let mut r = resp(WorkspaceGitState::Ok, branch("dev"));
        r.dirty = Some(true);
        assert_eq!(badge_text(&r).as_deref(), Some("dev ●"));
    }

    #[test]
    fn dirty_field_missing_falls_back_to_total() {
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        r.dirty = None;
        r.counts = counts_of(2);
        assert_eq!(badge_text(&r).as_deref(), Some("main ●2"));
    }

    #[test]
    fn counts_win_over_dirty_flag_so_text_and_colour_agree() {
        // `dirty=false` 但 `total>0`（服务端实现细节变动时可能出现）：判定取「或」，
        // 于是徽标文本带 `●` 与着色一致，不会出现「有警示色却没有标记」。
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        r.dirty = Some(false);
        r.counts = counts_of(3);
        assert!(is_dirty(&r));
        assert_eq!(badge_text(&r).as_deref(), Some("main ●3"));
    }

    #[test]
    fn detached_shows_short_oid_not_empty_name() {
        let r = resp(
            WorkspaceGitState::Ok,
            Some(WorkspaceGitHead::Detached {
                oid: Some("1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9012".into()),
            }),
        );
        assert_eq!(badge_text(&r).as_deref(), Some("HEAD@1a2b3c4"));
    }

    #[test]
    fn detached_without_oid_shows_placeholder() {
        // 首提交仓库：`# branch.oid (initial)` ⇒ oid 为 null，不能渲染成空串。
        let r = resp(
            WorkspaceGitState::Ok,
            Some(WorkspaceGitHead::Detached { oid: None }),
        );
        assert_eq!(badge_text(&r).as_deref(), Some("HEAD@?"));
    }

    #[test]
    fn empty_branch_name_falls_back_to_oid_form() {
        let head = WorkspaceGitHead::Branch {
            name: String::new(),
            oid: Some("abcdef0123456789".into()),
        };
        assert_eq!(head_label(&head), "HEAD@abcdef0");
    }

    #[test]
    fn non_ok_states_have_no_badge() {
        for state in [
            WorkspaceGitState::NotARepo,
            WorkspaceGitState::GitUnavailable,
            WorkspaceGitState::Failed,
        ] {
            // 这三种都必须返回 None，让 UI 各给一句文案（互不相同的降级态）。
            let mut r = resp(state, branch("main"));
            r.reason_code = Some("GIT_PROBE_TIMEOUT".into());
            assert_eq!(badge_text(&r), None, "{state:?} 不该有徽标");
        }
    }

    #[test]
    fn ok_without_head_has_no_badge() {
        let r = resp(WorkspaceGitState::Ok, None);
        assert_eq!(badge_text(&r), None);
    }

    #[test]
    fn stale_cache_source_still_renders_the_badge() {
        // 旧值仍是同一真源的值：照常显示（UI 可据 `source` 弱化，但不该消失）。
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        r.source = crabmate::cm_api_contract::workspace::WorkspaceGitSource::StaleCache;
        assert_eq!(badge_text(&r).as_deref(), Some("main"));
    }

    #[test]
    fn short_oid_is_char_based() {
        assert_eq!(short_oid("1a2b3c4d5e6f"), "1a2b3c4");
        assert_eq!(short_oid("abc"), "abc");
        // 非 ASCII 也按字符切，不 panic。
        assert_eq!(short_oid("分支分支分支分支"), "分支分支分支分");
    }
}
