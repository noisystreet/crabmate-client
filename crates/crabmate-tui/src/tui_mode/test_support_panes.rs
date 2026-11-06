//! 列宽落盘的测试辅助与装配接线测试（issue #407 的 P2）。
//!
//! 从 `test_support.rs` 拆出来：那边贴着 920 行/文件的 `fn-nloc-ratchet` 门禁，而这里
//! 的两件事都只与落盘有关 —— 给每条 lane 一个临时的落盘目的地，以及验证
//! `LaneApp::assemble` 真的把读到的宽度装进 `st.pane`。
//!
//! 挂载方式是 `test_support.rs` 里的 `#[path]`（与 `md.rs` → `md_tests.rs` 同一手法）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use crate::tui_mode::drafts::store::DraftStore;
use crate::tui_mode::inbound::Inbound;
use crate::tui_mode::pane_store::PaneStore;
use crate::tui_mode::pane_width::PaneWidths;
use crate::tui_mode::shell::LaneTemplate;
use crate::tui_mode::test_support::test_prefs;

/// 单测的列宽落盘目的地：进程内一个**唯一**的临时文件（每条 lane 一个）。
///
/// 不能用 `PaneStore::Unavailable` —— 那会让「拖完 → 写盘」这条路每次都以提示收尾，
/// 而用例根本没想验证提示；也不能用 `from_env()`：拖拽用例会真的走到写盘，那就把宽度
/// 写进跑测试那个人的 `~/.local/state` 了。
///
/// 每条 lane 一个名字而不是共用按 pid 命名的那个：并行用例会互相覆盖对方的文件 ——
/// 现在没有用例读回它，但下一个人加一条「写完读回」就会变成 flaky。残留的小文件落在
/// 系统临时目录，由系统清理。
pub(super) fn test_pane_store() -> PaneStore {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nth = NEXT.fetch_add(1, Ordering::Relaxed);
    PaneStore::at(std::env::temp_dir().join(format!(
        "crabmate-tui-panes-test-{}-{nth}.json",
        std::process::id()
    )))
}

/// 单测的草稿落盘目的地：进程内一个**唯一**的临时文件（每条 lane 一个）。
///
/// 理由与 [`test_pane_store`] 同：不能用 `Unavailable`（那会让每次切会话都以「草稿没能
/// 保存」的提示收尾），也不能用 `from_env()`（切会话用例会真的写进跑测试那个人的
/// `~/.local/state`）。
pub(super) fn test_draft_store() -> DraftStore {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nth = NEXT.fetch_add(1, Ordering::Relaxed);
    DraftStore::at(std::env::temp_dir().join(format!(
        "crabmate-tui-drafts-test-{}-{nth}.json",
        std::process::id()
    )))
}

/// 装配模板（启动与 `/lane attach|new` 共用同一份字段清单）。
///
/// `pane` 可覆盖：测试要能直接问「落盘的宽度有没有进到新 lane 里」，而默认值会让
/// 「读到了」和「没读到」看不出区别。
pub(super) fn test_template(pane: PaneWidths, tx: mpsc::Sender<Inbound>) -> LaneTemplate {
    LaneTemplate {
        overrides: test_prefs(),
        yes: true,
        no_keyring: true,
        pane,
        pane_store: test_pane_store(),
        draft_store: test_draft_store(),
        bearer: String::new(),
        // 测试连接没有显式 Bearer：面板「连接」栏显示「未提供」，与真实启动
        // （钥匙串没命中）同一状态。
        bearer_source: Default::default(),
        ctl: None,
        tx,
    }
}

mod tests {
    use std::borrow::Cow;
    use std::sync::mpsc;

    use super::test_template;
    use crate::tui_mode::LaneApp;
    use crate::tui_mode::pane_width::PaneWidths;
    use crate::tui_mode::test_support::test_client;
    use crate::tui_mode::worker::{JobCancel, LaneId};

    /// 装配接线（P2 的核心承诺）：读到的宽度真的进了 `st.pane`，且**同时**算作已保存 ——
    /// 否则启动第一帧就会白白写一次文件。
    ///
    /// 放在这里而不是 `shell.rs`：那边贴着 920 行门禁，而 `assemble` 已按测试需要放开到
    /// `pub(super)`；`test_support` 直接构造 `LaneApp` 会绕过它，这个缺口只有这里补得上。
    #[test]
    fn assemble_starts_from_the_stored_widths_and_treats_them_as_saved() {
        let client = test_client();
        let (tx, _rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let stored = PaneWidths {
            left: 44,
            right: 52,
        };
        let lane = LaneApp::assemble(
            LaneId(1),
            "local".to_string(),
            Cow::Borrowed(&client),
            &test_template(stored, tx),
            job_tx,
            JobCancel::default(),
        );
        assert_eq!(lane.st.pane, stored, "启动就用落盘的宽度");
        assert_eq!(
            lane.pane_saved, stored,
            "装配即视为已保存：没拖过不该写一次文件"
        );
        assert_eq!(
            lane.st.pane_layout().widths,
            stored,
            "渲染读的是同一份，不是又一份默认值"
        );
    }
}
