//! 各端斜杠命令的**能力表**：某个 [`Head`] 在本端到底有没有 handler。
//!
//! 为什么需要它：`slash_heads::head_of` 只说「这是不是一条命令、它的规范形是什么」，
//! 不说「这一端做不做」。三端的 handler 集本来就不同（Web 没有 `fold` / `lane`，
//! TUI 没有 `agent` / `config` / `api-key`），没有这张表时每端都要手抄一份名单 ——
//! 抄漏一次就是「拦截了却没有 handler」或「有 handler 却敲不出来」，而编译器看不见。
//!
//! 端独占命令（Web 的 `agent` / `config` / `context` / `api-key`、TUI 的 `background` …）
//! **不进本表**：它们不在 [`Head`] 里，由各端自己的表管，本表只描述共享 head 的覆盖面。
//!
//! 改动约定：新增 [`Head`] 变体时，必须决定它进哪几行常量；`every_head_is_implemented_somewhere`
//! 会拦住「加了变体但哪端都没实现」。

use super::slash_heads::Head;

/// 端（与 `docs/design/client_capability_matrix.md` 的列对齐）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Desktop / Android / Web —— 共用同一份 WASM UI，能力集一致。
    Desktop,
    /// 全屏 TUI（`crabmate-tui tui`）。
    Tui,
    /// 行模式 repl（`crabmate-tui repl`）。
    Repl,
}

/// Desktop / Web 的共享 head。
///
/// `mode` / `role` / `perm` / `status` 也是命令（issue #444 PR-A），但**不新增真相源**：
/// `/role` 归一到既有的 `/agent set`、`/status` 归一到既有的 `/config`（无参），
/// `/mode` / `/perm` 写的是状态栏那组同一个信号。`conv` / `quit` / `resume` / `find` 等
/// 仍只有端内形态（会话侧栏、进程退出、自动重挂、搜索框），敲出来会被当成普通消息发给模型。
///
/// 曾由 Web 独占的 `version` / `skills` / `context` / `config` / `agent` / `export`
/// 在 issue #444 第一阶段升级为共享 head —— 留着「Web 独占」就是第二份名单，
/// `/version` 那种「能敲但 help 不写」的漂移会再来一次。
const DESKTOP: &[Head] = &[
    Head::Help,
    Head::Status,
    Head::Model,
    Head::Mode,
    Head::Role,
    Head::Perm,
    Head::Workspace,
    Head::Version,
    Head::Skills,
    Head::Context,
    Head::Config,
    Head::Agent,
    Head::Export,
];

/// 全屏 TUI：`parse_control` 对每个 head 都有臂，所以这里是全集。
const TUI: &[Head] = &[
    Head::Help,
    Head::Status,
    Head::Settings,
    Head::Model,
    Head::Mode,
    Head::Role,
    Head::Perm,
    Head::Find,
    Head::Background,
    Head::Workspace,
    Head::Conv,
    Head::Version,
    Head::Skills,
    Head::Context,
    Head::Config,
    Head::Agent,
    Head::Export,
    Head::Resume,
    Head::Older,
    Head::Fold,
    Head::Lane,
    Head::Copy,
    Head::Ctl,
    Head::Editor,
    Head::Chat,
    Head::Quit,
];

/// 行模式 repl：`background` / `editor` / `chat` 只回一句「只在全屏可用」，
/// 不算有 handler，故不收。
///
/// `version` 收：版本号是本进程的编译期常量，不需要 serve，也不需要全屏 UI。
/// 其余新增命令（`skills` / `context` / `config` / `agent` / `export`）在 repl 里
/// 只回一句「只在全屏 tui 里可用」，同样不算 handler。
const REPL: &[Head] = &[
    Head::Quit,
    Head::Help,
    Head::Model,
    Head::Mode,
    Head::Role,
    Head::Perm,
    Head::Status,
    Head::Workspace,
    Head::Resume,
    Head::Conv,
    Head::Version,
];

/// 某一端支持的共享 head（顺序即文档展示顺序，不参与判定）。
#[must_use]
pub fn supported(surface: Surface) -> &'static [Head] {
    match surface {
        Surface::Desktop => DESKTOP,
        Surface::Tui => TUI,
        Surface::Repl => REPL,
    }
}

/// 本端是否有这个 head 的 handler。`false` 表示该行应原样发给模型（或落到端内独占表）。
#[must_use]
pub fn supports(surface: Surface, head: Head) -> bool {
    supported(surface).contains(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_covers_every_head() {
        // 全屏 TUI 的 `parse_control` 是穷尽 match，任何 head 漏在表外都说明两处不同步。
        for head in Head::ALL {
            assert!(
                supports(Surface::Tui, *head),
                "{} 没有进 TUI 能力表，但 parse_control 有它的臂",
                head.as_str()
            );
        }
    }

    #[test]
    fn desktop_and_repl_are_strict_sub_sets_of_tui() {
        for surface in [Surface::Desktop, Surface::Repl] {
            for head in supported(surface) {
                assert!(
                    supports(Surface::Tui, *head),
                    "{surface:?} 支持 {} 但 TUI 不支持 —— 共享 head 不该有此方向",
                    head.as_str()
                );
            }
            assert!(supported(surface).len() < Head::ALL.len());
        }
    }

    #[test]
    fn desktop_only_has_command_forms_it_actually_implements() {
        // 这十三条是 Web 真正写过分派的共享命令。`mode` / `role` / `perm` / `status`
        // 在 issue #444 PR-A 之后也算命令 —— 但都是既有实现的别名 / 同一组信号，
        // 不是第二套真相源。
        assert_eq!(
            supported(Surface::Desktop),
            &[
                Head::Help,
                Head::Status,
                Head::Model,
                Head::Mode,
                Head::Role,
                Head::Perm,
                Head::Workspace,
                Head::Version,
                Head::Skills,
                Head::Context,
                Head::Config,
                Head::Agent,
                Head::Export,
            ]
        );
        // 这些仍只有 UI 入口或端内形态（会话侧栏 / 进程退出 / 自动重挂 / 搜索框 /
        // 多窗口…），在 Web 敲出来会被当成普通消息发给模型。
        for ui_only in [
            Head::Conv,
            Head::Quit,
            Head::Settings,
            Head::Resume,
            Head::Find,
            Head::Older,
            Head::Fold,
            Head::Copy,
            Head::Ctl,
            Head::Lane,
            Head::Editor,
            Head::Chat,
            Head::Background,
        ] {
            assert!(
                !supports(Surface::Desktop, ui_only),
                "{} 在 Desktop 只有 UI 入口，不该算命令",
                ui_only.as_str()
            );
        }
    }

    #[test]
    fn repl_excludes_full_screen_only_heads() {
        // 这些在 repl 里只会打印「只在全屏 tui 里可用」，没有可执行语义。
        for head in [
            Head::Background,
            Head::Editor,
            Head::Chat,
            Head::Fold,
            Head::Find,
            Head::Older,
            Head::Copy,
            Head::Ctl,
            Head::Lane,
            Head::Settings,
        ] {
            assert!(
                !supports(Surface::Repl, head),
                "{} 在 repl 里没有 handler，不该算支持",
                head.as_str()
            );
        }
    }

    #[test]
    fn every_head_is_implemented_somewhere() {
        // 新增 Head 变体却忘了收进任何一端 = 这条命令谁都用不了。
        for head in Head::ALL {
            assert!(
                supports(Surface::Tui, *head)
                    || supports(Surface::Desktop, *head)
                    || supports(Surface::Repl, *head),
                "{} 三个端都没有 handler",
                head.as_str()
            );
        }
    }

    #[test]
    fn tables_have_no_duplicates() {
        for surface in [Surface::Desktop, Surface::Tui, Surface::Repl] {
            let mut seen: Vec<&str> = supported(surface).iter().map(|h| h.as_str()).collect();
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();
            assert_eq!(before, seen.len(), "{surface:?} 的能力表里有重复项");
        }
    }
}
