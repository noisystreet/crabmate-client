//! `/help` 的渲染：命令来自共享条目表，TUI 专属的键位与区域操作在这里补。
//!
//! 三条分工（与 `crabmate_client_api::help` 的模块文档同一口径）：
//!
//! - **命令 + usage + 摘要**：`crabmate_client_api::help`，与 Desktop 同一份，按
//!   `Surface::Tui` 过滤 —— 各端能敲哪些命令由 `slash_caps` 决定，这里不另立名单；
//! - **键位**：`hotkey_docs`（TUI 的 `F6` / `Ctrl+T` 在 Web 上不存在，不共享）；
//! - **区域操作**：会话栏 / 工作区树的键盘导航，Web 是鼠标，没有共同形态。
//!
//! 排布是「左列 usage + 右列摘要」，一条命令一行。旧版把十几个子命令串成一条
//! 200~600 字符的长句，终端折成没有结构的文字墙 —— 那是 `/help` 看起来乱的主因。

use super::hotkeys::{self, HotkeyGroup};
use super::state::{LineKind, UiState};

/// 左列宽度：最长 usage 是 `/copy [assistant|user|path|code]`（32 列）。
const USAGE_COL: usize = 34;

/// 键位分组的小标题前缀（`hotkey_docs` 的分组名接在后面）。
const KEY_GROUP_PREFIX: &str = "快捷键 · ";

/// 判定散在别的模块、没有热键条目的键（快捷键分组之外的那些）。
const MISC_KEYS_LINE: &str =
    "其它键：Ctrl+G 折叠导航 · PgUp/PgDn 翻页 · Ctrl+End 回底部 · 回合中 Enter 排队发送";

/// 分区域操作（会话栏 / 连接 / 工作区 / 审批 / 澄清）。只写「在哪按什么」，长尾规则进 docs。
const AREA_LINES: &[(&str, &str)] = &[
    (
        "会话栏",
        "↑↓ 选 · Enter 用 · n 新建 · r 刷新 · Del 删除（再确认）",
    ),
    (
        "连接",
        "左栏下半列出全部 serve 连接（单条也显示，> 为当前）",
    ),
    (
        "工作区树",
        "↑↓ 选 · Enter/→ 展开 · ← 收起/回父 · r 刷新 · Tab 回输入",
    ),
    (
        "项目切换",
        "p 从项目池选择/切换 · ↑↓ 选 · Enter 切换 · r 刷新",
    ),
    ("审批浮层", "Enter 一次 · a 始终 · Esc/n 拒绝"),
    (
        "澄清问卷",
        "Tab/↑↓ 换题 · Enter 提交 · Esc 收起（等于没答）· F7 重开",
    ),
];

impl UiState {
    /// `/help`：命令 → 快捷键（按键位表生成）→ 分区域操作，逐行推进 transcript。
    ///
    /// 左列 `usage` 对齐后右列扫读，一条命令一行。细节（子命令语义、注意事项）放在
    /// `docs/design/tui_usability.md`，不在这里展开 —— 终端里一屏要放下二十几条。
    pub(super) fn show_help(&mut self) {
        let entries: Vec<_> =
            crabmate_client_api::help::entries(crabmate_client_api::slash_caps::Surface::Tui)
                .collect();
        self.push_line(
            LineKind::System,
            &format!("命令（{} 条，键入 / 可补全）", entries.len()),
        );
        for e in entries {
            self.push_line(
                LineKind::System,
                &format!("  {:<USAGE_COL$}{}", e.usage, e.summary),
            );
        }
        // 键位由 `hotkey_docs` 的键位表生成：新增顶层热键时这里与设置面板一起自动跟上，
        // 不会再出现「面板写了、/help 漏了」这种手写字符串的漂移（#433）。
        for group in HotkeyGroup::ALL {
            self.push_line(
                LineKind::System,
                &format!(
                    "{KEY_GROUP_PREFIX}{}：{}",
                    group.label(),
                    hotkeys::group_line(group)
                ),
            );
        }
        self.push_line(LineKind::System, MISC_KEYS_LINE);
        self.push_line(LineKind::System, "区域操作");
        for (area, keys) in AREA_LINES {
            self.push_line(LineKind::System, &format!("  {area:<USAGE_COL$}{keys}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::slash_caps::{Surface, supported};

    use super::{AREA_LINES, USAGE_COL};

    /// `/help` 的命令清单必须覆盖本端**每一个**能敲的共享 head。
    ///
    /// 清单现在来自共享条目表（不再手写字面量），本用例钉的是「按 Surface::Tui 过滤后仍
    /// 覆盖全部 head」—— 加 head 忘了补条目时，共享层与这里都会红。
    #[test]
    fn help_lists_every_tui_head() {
        let usages: Vec<&str> = crabmate_client_api::help::entries(Surface::Tui)
            .map(|e| e.usage)
            .collect();
        for head in supported(Surface::Tui) {
            let needle = format!("/{}", head.as_str());
            assert!(
                usages.iter().any(|u| u.starts_with(&needle)),
                "/help 命令清单漏了 {needle}"
            );
        }
    }

    /// 左列宽度要容得下最长的那条 usage / 区域名，否则右列被顶歪 —— 对齐一失效就退回「文字墙」。
    #[test]
    fn usage_column_fits_the_longest_entry() {
        for e in crabmate_client_api::help::entries(Surface::Tui) {
            assert!(
                e.usage.chars().count() < USAGE_COL,
                "{} 超出左列宽度 {USAGE_COL}，右列会错位",
                e.usage
            );
        }
        for (area, _) in AREA_LINES {
            assert!(
                area.chars().count() < USAGE_COL,
                "区域名 {area} 超出左列宽度"
            );
        }
    }
}
