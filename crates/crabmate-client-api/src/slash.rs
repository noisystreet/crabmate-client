//! 跨端控制斜杠命令名字表（head；小写、不含 `/`；handler 留端）。
//!
//! 「公共交集 + 端特有」分层：本模块只收 ≥2 端共用的命令名与别名，
//! 保证各端拦截判定与菜单 `match_key` 对同一命令用同一名字；
//! 单端命令（web 的 `agent`/`export`/`config`/…、tui-mode 的
//! `settings`/`find`/`h`）与其 handler 一起留在端内，避免出现
//! 「拦截了但没有 handler」的超集误伤。

/// `/help`（repl / tui-mode / web）。
pub const HELP: &str = "help";
/// `/help` 别名（repl / web）。
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
/// `/quit`（repl / tui-mode）。
pub const QUIT: &str = "quit";
/// `/quit` 别名（repl / tui-mode）。
pub const QUIT_EXIT: &str = "exit";
/// `/quit` 别名（repl / tui-mode）。
pub const QUIT_Q: &str = "q";

#[cfg(test)]
mod tests {
    use super::{
        CD, CONV, HELP, HELP_Q, MODE, MODEL, QUIT, QUIT_EXIT, QUIT_Q, ROLE, STATUS, WORKSPACE,
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
        assert_eq!(QUIT, "quit");
        assert_eq!(QUIT_EXIT, "exit");
        assert_eq!(QUIT_Q, "q");
    }
}
