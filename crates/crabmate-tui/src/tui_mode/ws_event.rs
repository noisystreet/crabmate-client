//! 工作区拉取结果的分派：目录列表与文件内容，从 `mod.rs` 拆出。
//!
//! 两者必须过**同一道** `ws_generation` 门控（中途切过工作区 = 这条结果属于旧工作区，丢弃），
//! 写在一处比在两个 match 臂里各写一遍更不容易漏掉一半；`mod.rs` 也贴着 920 行门禁。

use super::LaneApp;
use super::state::LineKind;
use super::worker::UiEvent;

impl LaneApp<'_> {
    /// 目录 / 文件回包的分派（其它事件不该走到这里）。
    ///
    /// 代数不符时**静默丢弃**：用户已经切到别的工作区了，把 A 的内容写进 B 才是真正的错误，
    /// 而「你切走了所以旧结果丢了」这句话每切一次就刷一遍反而是噪音。
    pub(super) fn on_ws_event(&mut self, ev: UiEvent) {
        match ev {
            UiEvent::WorkspaceDir {
                rel,
                generation,
                result,
            } => {
                if generation != self.st.ws_generation {
                    return;
                }
                match result {
                    Ok(data) => self.st.ws_dir_ok(&rel, data),
                    Err(e) => {
                        if self.st.ws_dir_failed(&rel) {
                            self.st.push_line(
                                LineKind::System,
                                &format!("展开 {rel} 失败：{e}（← 可重试）"),
                            );
                        }
                    }
                }
            }
            // 文件内容多一道判据（视图内部还会按 `rel` 再认一次：读取在途时可能已经换了文件）。
            UiEvent::WorkspaceFile {
                rel,
                generation,
                result,
            } if generation == self.st.ws_generation => self.on_workspace_file(&rel, result),
            _ => {}
        }
    }
}
