//! 端内斜杠 head 的**规范形**与别名归一（单一真相源）。
//!
//! 各端原本在自己的解析器里手写「主命令 + 别名」两组 arm，于是同一张名单存在两三份 ——
//! 解析用一份、补全菜单用一份。改名或新增时漏掉一处就**静默**失效：`/perm` 在 repl 里
//! 曾经既被拦截表认作控制命令（`is_control_slash` 含 `slash::PERM`）、又没有 handler arm，
//! 于是它就掉进「unknown control slash」，而 `/help` 明明写着支持。
//!
//! 这里只做一件事：`/<head>` → [`Head`]。别名（`q` `exit` `h` `cd` `yank` …）归一到主命令，
//! 于是
//!
//! - 执行端只对主命令写 arm（`match` 穷尽，新增变体时编译器会拦）；
//! - 补全菜单从 [`Head::ALL`] 派生，不会再出现「有 handler 却没有候选」。
//!
//! **这不是共享「命令集」**：主命令是各端 handler 的子集（Web 没有 `fold` / `lane`，
//! TUI 没有 `agent` / `config` / `api-key`），端内 `match` 的 `_ =>` 分支才是
//! 「本端不支持」的落点；端独占命令与其别名不进这张表。

/// 斜杠命令的规范形（`/<head>` 去掉别名后的身份）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    Quit,
    Help,
    Model,
    Mode,
    Role,
    Perm,
    Status,
    Settings,
    Workspace,
    Resume,
    Older,
    Find,
    Fold,
    Background,
    Copy,
    Ctl,
    Lane,
    Editor,
    Chat,
    Conv,
}

impl Head {
    /// 主命令名（不含 `/`）—— 即补全菜单的 `match_key` / `label` 来源。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Help => "help",
            Self::Model => "model",
            Self::Mode => "mode",
            Self::Role => "role",
            Self::Perm => "perm",
            Self::Status => "status",
            Self::Settings => "settings",
            Self::Workspace => "workspace",
            Self::Resume => "resume",
            Self::Older => "older",
            Self::Find => "find",
            Self::Fold => "fold",
            Self::Background => "background",
            Self::Copy => "copy",
            Self::Ctl => "ctl",
            Self::Lane => "lane",
            Self::Editor => "editor",
            Self::Chat => "chat",
            Self::Conv => "conv",
        }
    }

    /// 全部主命令。**顺序即 TUI 补全菜单的展示顺序**（执行端的 `match` 顺序无关）。
    pub const ALL: &'static [Head] = &[
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
}

/// `/<head>` 的 head（大小写不敏感、别名归一）→ 规范命令；未识别返回 `None`。
///
/// `None` 的语义是「不是控制斜杠」：端内据此把整行当普通消息发给模型
/// （`/<skill-id>` 就走这条路，skill 名单是运行期才知道的）。
#[must_use]
pub fn head_of(raw: &str) -> Option<Head> {
    let h = raw.trim().to_ascii_lowercase();
    Some(match h.as_str() {
        "quit" | "exit" | "q" => Head::Quit,
        "help" | "h" | "?" => Head::Help,
        "model" => Head::Model,
        "mode" => Head::Mode,
        "role" => Head::Role,
        "perm" => Head::Perm,
        "status" => Head::Status,
        "settings" => Head::Settings,
        "workspace" | "cd" => Head::Workspace,
        "resume" => Head::Resume,
        "older" | "history" => Head::Older,
        "find" | "search" | "grep" => Head::Find,
        "fold" | "unfold" => Head::Fold,
        "background" | "bg" => Head::Background,
        "copy" | "yank" => Head::Copy,
        "ctl" => Head::Ctl,
        "lane" => Head::Lane,
        "editor" | "view" => Head::Editor,
        "chat" => Head::Chat,
        "conv" => Head::Conv,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_fold_onto_their_primary() {
        for (alias, want) in [
            ("exit", Head::Quit),
            ("q", Head::Quit),
            ("h", Head::Help),
            ("?", Head::Help),
            ("cd", Head::Workspace),
            ("history", Head::Older),
            ("search", Head::Find),
            ("grep", Head::Find),
            ("unfold", Head::Fold),
            ("bg", Head::Background),
            ("yank", Head::Copy),
            ("view", Head::Editor),
        ] {
            assert_eq!(head_of(alias), Some(want), "别名 {alias} 没有归一到主命令");
        }
    }

    #[test]
    fn head_matching_is_case_insensitive_and_trimmed() {
        assert_eq!(
            head_of("  /MODE  "),
            None,
            "调用方传的是 head，不是带斜杠的行"
        );
        assert_eq!(head_of("  MODE  "), Some(Head::Mode));
        assert_eq!(head_of("WorkSpace"), Some(Head::Workspace));
    }

    #[test]
    fn unknown_heads_are_not_controls() {
        // `<skill-id>` 与普通文本都必须落到 None，否则会被拦下来却没人处理。
        assert_eq!(head_of("my-skill"), None);
        assert_eq!(head_of(""), None);
        assert_eq!(head_of("agent"), None);
        assert_eq!(head_of("export"), None);
    }

    #[test]
    fn every_primary_resolves_to_itself() {
        for head in Head::ALL {
            assert_eq!(
                head_of(head.as_str()),
                Some(*head),
                "{} 是主命令却解析不回自己",
                head.as_str()
            );
        }
    }
}
