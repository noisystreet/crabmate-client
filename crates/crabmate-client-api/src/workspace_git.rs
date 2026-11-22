//! 工作区 git 状态投影（`GET /workspace/git`）→ 展示语义。
//!
//! 纯逻辑，Web UI 与 TUI 共用（本 crate 禁 Tauri / `web-sys` / `reqwest` / `tokio`）。
//!
//! **文案分工**：这里只产出**技术标识**——分支名、`HEAD@<短 oid>`、脏计数标记 `●N`，
//! 这些在任何语言下写法相同。三种降级态（非 git 仓库 / git 不可用 / 读取失败）的那句话
//! 交给各自 UI 的 i18n（Desktop 有 zh / en，TUI 只有中文），本模块只暴露
//! [`WorkspaceGitState`]，避免把中文文案钉死在一个 TUI 与桌面共用的 crate 里。
//!
//! **口径**（与 Server `docs/design/workspace_git_status_projection.md` 一致）：
//! - `dirty` 指「**当前工作区根下**有无未提交改动」，不等于整个仓库干净；
//! - 本模块**不**把 git 脏状态与会话变更集（`GET /workspace/changelog`）混用，
//!   两者是不同概念，UI 必须分措辞。

use crabmate::cm_api_contract::workspace::{
    WorkspaceGitCounts, WorkspaceGitHead, WorkspaceGitSource, WorkspaceGitState,
    WorkspaceGitStatusResponse,
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
/// `source == StaleCache` 仍返回徽标——它是同一真源的旧值，配 [`is_stale`] 提示即可。
pub fn badge_text(resp: &WorkspaceGitStatusResponse) -> Option<String> {
    if resp.state != WorkspaceGitState::Ok {
        return None;
    }
    let head = resp.head.as_ref()?;
    let label = head_label(head);
    match dirty_marker(resp.counts.as_ref(), resp.dirty) {
        Some(marker) => Some(format!("{label} {marker}")),
        None => Some(label),
    }
}

/// 同 [`badge_text`]，但按显示列数截断（窄终端 / 窄侧栏）。
///
/// 截断在**整条**徽标上做（而不是只截分支名），否则 `main ●5` 里的脏标记可能被切掉，
/// 变成「看着干净」的误导画面。
pub fn badge_text_fit(resp: &WorkspaceGitStatusResponse, cols: usize) -> Option<String> {
    badge_text(resp).map(|text| truncate_ellipsis(&text, cols))
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

/// 脏标记：`●`（有改动但计数为 0，理论上不出现）/ `●N`（N = `counts.total`）；干净返回 `None`。
///
/// `dirty` 缺省时以 `total > 0` 兜底，避免把「字段没给」渲染成「干净」。
pub fn dirty_marker(counts: Option<&WorkspaceGitCounts>, dirty: Option<bool>) -> Option<String> {
    let total = counts.map(|c| c.total).unwrap_or(0);
    if !dirty.unwrap_or(total > 0) {
        return None;
    }
    if total > 0 {
        Some(format!("{DIRTY_MARKER}{total}"))
    } else {
        Some(DIRTY_MARKER.to_string())
    }
}

/// 是否来自「重探失败、旧值兜底」的缓存（UI 可据此弱化显示或加「上次读取」提示）。
pub fn is_stale(resp: &WorkspaceGitStatusResponse) -> bool {
    resp.source == WorkspaceGitSource::StaleCache
}

/// 分类计数（`staged` / `unstaged` / `untracked` / `conflicted` / `total`），无可展示值时 `None`。
///
/// 明细行的措辞（「暂存 / 未暂存 / 未跟踪 / 冲突」）归 UI i18n，本模块只给数字。
pub fn counts(resp: &WorkspaceGitStatusResponse) -> Option<WorkspaceGitCounts> {
    resp.counts
}

/// 按显示列数截断，超出部分替换为 `…`（单字符，占 1 列）。
///
/// 列宽只区分「东亚宽字符（2 列） / 其余（1 列）」两档，不引 `unicode-width`
/// ——本 crate 的依赖里只有 `serde` / `serde_json`，为一个徽标增加依赖不值得。
/// 代价是零宽字符与组合字符会按 1 列算（分支名里不现实），属可接受偏差。
pub fn truncate_ellipsis(text: &str, cols: usize) -> String {
    if cols == 0 {
        return String::new();
    }
    if display_width(text) <= cols {
        return text.to_string();
    }
    // 留 1 列给 `…`。
    let budget = cols - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = char_cols(ch);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// 文本的显示列数（同上两档口径）。
pub fn display_width(text: &str) -> usize {
    text.chars().map(char_cols).sum()
}

fn char_cols(ch: char) -> usize {
    let cp = ch as u32;
    let wide = matches!(cp,
        0x1100..=0x115F        // Hangul Jamo 初声
        | 0x2E80..=0xA4CF      // CJK 部首 / 假名 / 汉字 / 注音
        | 0xAC00..=0xD7A3      // Hangul 音节
        | 0xF900..=0xFAFF      // CJK 兼容汉字
        | 0xFE30..=0xFE6F      // CJK 兼容形式
        | 0xFF00..=0xFF60      // 全角 ASCII
        | 0xFFE0..=0xFFE6      // 全角符号
        | 0x1F300..=0x1F64F    // emoji
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD); // CJK 扩展 B 及以后
    if wide { 2 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            source: WorkspaceGitSource::Probe,
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
        r.dirty = Some(true);
        r.counts = counts_of(5);
        assert_eq!(badge_text(&r).as_deref(), Some("main ●5"));
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
    fn stale_cache_keeps_the_badge() {
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        r.source = WorkspaceGitSource::StaleCache;
        assert!(is_stale(&r));
        assert_eq!(badge_text(&r).as_deref(), Some("main"));
    }

    #[test]
    fn truncate_fits_exactly_and_ellipsizes_on_overflow() {
        assert_eq!(truncate_ellipsis("main", 4), "main");
        assert_eq!(truncate_ellipsis("main", 10), "main");
        assert_eq!(truncate_ellipsis("feature/long-name", 8), "feature…");
        // 徽标整体截断：脏标记不能被切掉后假装干净。
        assert_eq!(truncate_ellipsis("main ●5", 7), "main ●5");
        assert_eq!(truncate_ellipsis("main ●5", 4), "mai…");
    }

    #[test]
    fn truncate_handles_degenerate_widths() {
        assert_eq!(truncate_ellipsis("main", 0), "");
        assert_eq!(truncate_ellipsis("main", 1), "…");
        assert_eq!(truncate_ellipsis("", 3), "");
    }

    #[test]
    fn truncate_counts_cjk_as_two_columns() {
        assert_eq!(display_width("分支"), 4);
        assert_eq!(truncate_ellipsis("分支名", 6), "分支名");
        assert_eq!(truncate_ellipsis("分支名", 5), "分支…");
    }

    #[test]
    fn short_oid_is_char_based() {
        assert_eq!(short_oid("1a2b3c4d5e6f"), "1a2b3c4");
        assert_eq!(short_oid("abc"), "abc");
        // 非 ASCII 也按字符切，不 panic。
        assert_eq!(short_oid("分支分支分支分支"), "分支分支分支分");
    }

    #[test]
    fn badge_fit_applies_truncation() {
        let mut r = resp(WorkspaceGitState::Ok, branch("feature/a-very-long-branch"));
        r.dirty = Some(true);
        r.counts = counts_of(12);
        assert_eq!(
            badge_text_fit(&r, 12).as_deref(),
            Some(truncate_ellipsis("feature/a-very-long-branch ●12", 12).as_str())
        );
        assert_eq!(
            badge_text_fit(&resp(WorkspaceGitState::NotARepo, None), 12),
            None
        );
    }

    #[test]
    fn counts_are_structural_only() {
        let mut r = resp(WorkspaceGitState::Ok, branch("main"));
        r.counts = Some(WorkspaceGitCounts {
            staged: 1,
            unstaged: 2,
            untracked: 3,
            conflicted: 4,
            total: 10,
        });
        let c = counts(&r).expect("counts present");
        assert_eq!(
            (c.staged, c.unstaged, c.untracked, c.conflicted, c.total),
            (1, 2, 3, 4, 10)
        );
        r.counts = None;
        assert!(counts(&r).is_none());
    }
}
