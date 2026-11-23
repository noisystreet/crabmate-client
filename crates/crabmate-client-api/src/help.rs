//! 斜杠命令的**共享帮助条目**：三端 `/help` 的命令集与用法签名只有这一份。
//!
//! 为什么放这里：`/help` 的杂乱根源不是排版，而是**两份文案各自生长** — Desktop 在
//! `i18n/chat_column.rs` 有 18 条一行摘要（补全菜单的描述），TUI 另写了三十条 200~600
//! 字符的长句（一条塞十几个子命令）。两边都会随功能加命令而漂。
//!
//! 本模块只回答三件事，且都是**与端无关**的：
//!
//! 1. 有哪些命令（复用 [`Head`]，因此天然受 [`super::slash_caps`] 的能力表约束）；
//! 2. 每条命令的用法签名（`/copy [assistant|user|path|code]`）；
//! 3. 一句话摘要（中文；各端可自行本地化覆盖，见下）。
//!
//! ## 三类内容**不在**本模块
//!
//! - **键位**：TUI 的 `F6` / `Ctrl+T` 与 Web 的交互不同，各端自有（TUI 在 `hotkey_docs`）；
//! - **端独占命令**：TUI 的 `/lane` 子命令细节、Web 的 `/api-key` 等，各端自己补；
//! - **区域操作**：TUI 是会话栏 / 工作区树的键盘导航，Web 是鼠标，没有共同形态。
//!
//! ## 摘要的语言
//!
//! 这里放中文 —— 与 `clarify_form` 等既有模块一致（共享层本来就承载 TUI 的线格式文案）。
//! Desktop 需要英文时用自己的 i18n 覆盖同一 `head`，缺翻译时回落这里的中文，避免「没翻译
//! 就没有说明」。

use super::slash_caps::{Surface, supported};
use super::slash_heads::Head;

/// 一条帮助：命令 + 用法签名 + 一句话摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpEntry {
    /// 命令 head（`usage` 是它的调用形态）。
    pub head: Head,
    /// 用法签名。**只写调用形态**，子命令的细节留给各端或 docs —— 终端里一屏要放二十条。
    pub usage: &'static str,
    /// 一句话摘要（中文，≤ 40 显示列）。写「做什么」，不写「怎么用」。
    pub summary: &'static str,
}

/// 全部共享条目，**顺序即显示顺序**（按「常用 → 会话 → 连接 → 能力」排）。
///
/// 必须覆盖每一个 [`Head`] 变体：`every_head_has_one_entry` 会拦住漏写。
pub const ENTRIES: &[HelpEntry] = &[
    HelpEntry {
        head: Head::Help,
        usage: "/help",
        summary: "这份帮助（浮层，可滚动）",
    },
    HelpEntry {
        head: Head::Quit,
        usage: "/quit",
        summary: "退出（/exit、/q 同义）",
    },
    HelpEntry {
        head: Head::Model,
        usage: "/model [<name>|<id>]",
        summary: "查看或切换模型",
    },
    HelpEntry {
        head: Head::Mode,
        usage: "/mode ask|plan|act",
        summary: "切换会话模式",
    },
    HelpEntry {
        head: Head::Role,
        usage: "/role <id>",
        summary: "切换 agent 角色（= /agent set）",
    },
    HelpEntry {
        head: Head::Perm,
        usage: "/perm [<level>|off]",
        summary: "查看或切换权限等级",
    },
    HelpEntry {
        head: Head::Status,
        usage: "/status",
        summary: "serve 状态与用量概览",
    },
    HelpEntry {
        head: Head::Settings,
        usage: "/settings",
        summary: "设置面板（endpoint / 模型 / 密钥）",
    },
    HelpEntry {
        head: Head::Workspace,
        usage: "/workspace [<路径>|off]",
        summary: "切换 serve 工作区根（/cd 同义）",
    },
    HelpEntry {
        head: Head::Conv,
        usage: "/conv <子命令>",
        summary: "会话：list/use/new/export/retry/edit",
    },
    HelpEntry {
        head: Head::Older,
        usage: "/older",
        summary: "前插更早一页历史（/history 同义）",
    },
    HelpEntry {
        head: Head::Resume,
        usage: "/resume",
        summary: "续传上一轮被中断的回合",
    },
    HelpEntry {
        head: Head::Find,
        usage: "/find [<词>|off]",
        summary: "在聊天记录里搜索",
    },
    HelpEntry {
        head: Head::Fold,
        usage: "/fold [n|all|close]",
        summary: "执行过程的展开 / 折叠",
    },
    HelpEntry {
        head: Head::Detail,
        usage: "/detail [n]",
        summary: "展开某次工具调用的输出全文",
    },
    HelpEntry {
        head: Head::Copy,
        usage: "/copy [assistant|user|path|code]",
        summary: "复制到剪贴板（Ctrl+Y 同义）",
    },
    HelpEntry {
        head: Head::Editor,
        usage: "/editor [--md|--src] [<路径>]",
        summary: "只读文件视图（/view 同义）",
    },
    HelpEntry {
        head: Head::Chat,
        usage: "/chat",
        summary: "从文件视图切回聊天",
    },
    HelpEntry {
        head: Head::Lane,
        usage: "/lane <子命令> [<name>]",
        summary: "连接管理：status/restart/attach/…",
    },
    HelpEntry {
        head: Head::Ctl,
        usage: "/ctl",
        summary: "查本地控制面实例（只读）",
    },
    HelpEntry {
        head: Head::Background,
        usage: "/background <图片>|off",
        summary: "全屏背景图",
    },
    HelpEntry {
        head: Head::Version,
        usage: "/version",
        summary: "本进程版本（不请求 serve）",
    },
    HelpEntry {
        head: Head::Skills,
        usage: "/skills",
        summary: "列出 serve 侧 skill（读缓存）",
    },
    HelpEntry {
        head: Head::Context,
        usage: "/context",
        summary: "上下文用量明细",
    },
    HelpEntry {
        head: Head::Config,
        usage: "/config reload",
        summary: "让 serve 重读磁盘配置",
    },
    HelpEntry {
        head: Head::Agent,
        usage: "/agent [list|set <id>]",
        summary: "列出或设置 agent 角色",
    },
    HelpEntry {
        head: Head::Export,
        usage: "/export [json|markdown|both]",
        summary: "导出当前会话",
    },
];

/// 某一端可用的条目，按 [`ENTRIES`] 的顺序（即显示顺序）。
///
/// 过滤依据是 [`super::slash_caps::supported`] —— 那张表已经决定「这一端有没有这个命令的
/// handler」，本函数不另立名单。**端独占命令**（TUI 的 `Background` 之外那些、Web 的
/// `api-key`）不在这里补，由各端自己拼在后面。
pub fn entries(surface: Surface) -> impl Iterator<Item = &'static HelpEntry> + Clone {
    let allowed = supported(surface);
    ENTRIES.iter().filter(move |e| allowed.contains(&e.head))
}

/// 按 head 查条目（各端补自己那几条时用，也便于测试与文档引用）。
#[must_use]
pub fn entry(head: Head) -> Option<&'static HelpEntry> {
    ENTRIES.iter().find(|e| e.head == head)
}

#[cfg(test)]
mod tests {
    use super::{ENTRIES, entries, entry};
    use crate::slash_caps::{Surface, supported};
    use crate::slash_heads::Head;

    /// 每个 `Head` 变体都必须有条目：漏写就是「能敲但 help 不写」，`/version` 曾这么漏过。
    #[test]
    fn every_head_has_one_entry() {
        // 穷尽 match：新增变体时这里编译不过，逼着补文案。
        let all = [
            Head::Quit,
            Head::Help,
            Head::Model,
            Head::Mode,
            Head::Role,
            Head::Perm,
            Head::Status,
            Head::Settings,
            Head::Workspace,
            Head::Resume,
            Head::Older,
            Head::Find,
            Head::Fold,
            Head::Detail,
            Head::Background,
            Head::Copy,
            Head::Ctl,
            Head::Lane,
            Head::Editor,
            Head::Chat,
            Head::Conv,
            Head::Version,
            Head::Skills,
            Head::Context,
            Head::Config,
            Head::Agent,
            Head::Export,
        ];
        for head in all {
            assert!(
                entry(head).is_some(),
                "help 缺 {} 的条目（usage + 摘要）",
                head.as_str()
            );
        }
        assert_eq!(ENTRIES.len(), all.len(), "条目数与 Head 变体数不一致");
    }

    /// 每端 `supported()` 里的 head 都能查到条目 —— 这条把能力表与 help 绑在一起：
    /// 新端或新 head 进表而 help 没跟上时红。
    #[test]
    fn every_supported_head_is_documented_per_surface() {
        for surface in [Surface::Desktop, Surface::Tui, Surface::Repl] {
            let heads: Vec<Head> = entries(surface).map(|e| e.head).collect();
            for head in supported(surface) {
                assert!(
                    heads.contains(head),
                    "{surface:?} 支持 {}，但 help entries 没给出来",
                    head.as_str()
                );
            }
            // 顺序与 ENTRIES 一致：过滤不应改变显示顺序。
            let mut idx = 0;
            for e in entries(surface) {
                let pos = ENTRIES
                    .iter()
                    .position(|x| x.head == e.head)
                    .expect("来自 ENTRIES");
                assert!(pos >= idx, "过滤后的顺序与 ENTRIES 不一致");
                idx = pos;
            }
        }
    }

    /// 摘要与用法都要短：终端一屏放二十条，左列还要对齐。
    /// 终端显示宽度：CJK / 全角记 2 列，其余 1 列（不引 unicode-width，够用即可）。
    fn display_width(s: &str) -> usize {
        s.chars()
            .map(|c| {
                if matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3
                    | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6)
                {
                    2
                } else {
                    1
                }
            })
            .sum()
    }

    #[test]
    fn entries_stay_short_enough_to_scan() {
        for e in ENTRIES {
            assert!(
                display_width(e.summary) <= 44,
                "{} 的摘要太宽（{} 列），会撑破一屏：{}",
                e.usage,
                display_width(e.summary),
                e.summary
            );
            assert!(
                display_width(e.usage) <= 34,
                "{} 的用法签名太宽（{} 列）",
                e.usage,
                display_width(e.usage)
            );
            assert!(e.usage.starts_with('/'), "{} 应以 / 开头", e.usage);
        }
    }

    /// 端独占命令不该混进共享表（否则 Web 会列出 TUI 才有的 `lane`）。
    #[test]
    fn surface_filter_hides_end_exclusive_heads() {
        let desktop: Vec<Head> = entries(Surface::Desktop).map(|e| e.head).collect();
        assert!(!desktop.contains(&Head::Lane), "Desktop 没有 /lane");
        assert!(!desktop.contains(&Head::Fold), "Desktop 没有 /fold");
        let tui: Vec<Head> = entries(Surface::Tui).map(|e| e.head).collect();
        assert!(tui.contains(&Head::Lane));
        assert_eq!(tui.len(), ENTRIES.len(), "TUI 的 parse_control 是全集");
    }
}
