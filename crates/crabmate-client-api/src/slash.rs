//! 跨端控制斜杠命令名字表（head；小写、不含 `/`；handler 留端）。
//!
//! **本模块只是字符串常量。** 命令的规范身份与别名归一在 [`super::slash_heads`]，
//! 「哪一端真的有 handler」在 [`super::slash_caps`]，菜单候选与 `/help` 由各端从这两张表派生。
//!
//! 历史上这里按「≥2 端共用」分层，把 `agent` / `export` / `config` 称作单端命令 ——
//! issue #444 之后（第一阶段的六条 + PR-A 的 `mode`/`role`/`perm`/`status`）它们都是
//! 共享 head，那个说法不再成立。保留常量只为各端拼文案时不必再写一遍字面量；
//! **别名归一与主命令身份不在本模块**，见 [`super::slash_heads`]（`/<head>` → `Head`）。

/// `/help`（repl / tui-mode / web）。
pub const HELP: &str = "help";
/// `/help` 别名（repl / web / tui-mode —— 三端都经 `slash_heads::head_of` 归一）。
pub const HELP_Q: &str = "?";
/// `/workspace`（repl / web）。
pub const WORKSPACE: &str = "workspace";
/// `/cd`（repl / web；工作区目录）。
pub const CD: &str = "cd";
/// `/model`（tui-mode / web）。
pub const MODEL: &str = "model";
/// `/conv`（repl / tui-mode）。
pub const CONV: &str = "conv";
/// `/status`（repl / tui-mode）。
pub const STATUS: &str = "status";
/// `/mode`（repl / tui-mode）。
pub const MODE: &str = "mode";
/// `/role`（repl / tui-mode）。
pub const ROLE: &str = "role";
/// `/perm`（repl / tui-mode）：本轮随 `/chat` 发送的权限等级意图（`off` 清除）。
///
/// 与 `/mode` `/role` 同类：客户端侧 override，随下一条消息生效；服务端仍按
/// min 链（`ceiling → role → session_mode → 本字段`）决定实际档位。
pub const PERM: &str = "perm";
/// `/resume`（repl / tui-mode）：续传上一轮被中断、serve 侧 job 可能仍在跑的回合。
pub const RESUME: &str = "resume";
/// `/quit`（repl / tui-mode）。
pub const QUIT: &str = "quit";
/// `/quit` 别名（repl / tui-mode）。
pub const QUIT_EXIT: &str = "exit";
/// `/quit` 别名（repl / tui-mode）。
pub const QUIT_Q: &str = "q";

#[cfg(test)]
mod tests {
    use super::{
        CD, CONV, HELP, HELP_Q, MODE, MODEL, PERM, QUIT, QUIT_EXIT, QUIT_Q, RESUME, ROLE, STATUS,
        WORKSPACE,
    };

    #[test]
    fn head_names_are_stable() {
        assert_eq!(HELP, "help");
        assert_eq!(HELP_Q, "?");
        assert_eq!(WORKSPACE, "workspace");
        assert_eq!(CD, "cd");
        assert_eq!(MODEL, "model");
        assert_eq!(CONV, "conv");
        assert_eq!(STATUS, "status");
        assert_eq!(MODE, "mode");
        assert_eq!(ROLE, "role");
        assert_eq!(PERM, "perm");
        assert_eq!(RESUME, "resume");
        assert_eq!(QUIT, "quit");
        assert_eq!(QUIT_EXIT, "exit");
        assert_eq!(QUIT_Q, "q");
    }
}
