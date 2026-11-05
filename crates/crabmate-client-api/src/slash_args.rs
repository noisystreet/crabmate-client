//! 斜杠命令的**参数解析**：把子命令串归一成类型，副作用留给各端。
//!
//! 为什么放共享层：`slash_heads` 只回答「这是哪条命令」，`slash_caps` 只回答「这一端有没有
//! handler」，都不回答「`/agent` 后面跟的 `ls` 算不算 list」。各端自己 `match` 的结果是
//! 已经漂过一次 —— TUI 的 `cmd_agent` 认 `ls`，Web 的 `handle_agent` 不认，同一条命令在
//! 一端列角色、在另一端打印「用法」。归一成类型后，这类漂移在编译期就不可能再出现。
//!
//! 只收**纯分类**：怎么列、怎么报错、要不要发请求，全是端内的事（`/config` 就是例子 ——
//! 空参在 TUI 是「用法」、在 Web 是「摘要」，那是设计差异，不该在这里抹平）。

/// `/agent [<sub>] [<id>]` 的意图。
///
/// `set` 后面跟 `default` 算清除 override，但那是**端内的解释**：共享层只保证
/// 「`set` 有 id」与「`set` 没 id」这两件事被区分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSub<'a> {
    /// 列出可用 role：`list` / `ls` / 空参都归这里。
    ///
    /// `ls` 曾被 TUI 单独认、Web 漏掉 —— 归一到同一个分支就是为了不再重演。
    List,
    /// 设置 role，携带 id。
    Set(&'a str),
    /// `set` 后面没跟 id。与 [`Self::List`] 分开：「没给参数」和「给错了」的提示不一样。
    SetMissingId,
    /// 不认识的子命令。带上原文，好让端内把用户敲的词原样回显。
    Unknown(&'a str),
}

impl<'a> AgentSub<'a> {
    /// 分类子命令。空参数、`list`、`ls` 一律 [`Self::List`]（大小写不敏感）。
    #[must_use]
    pub fn parse(args: &'a [&'a str]) -> Self {
        match args.first().map(|w| w.trim()) {
            None | Some("") => Self::List,
            Some(w) if w.eq_ignore_ascii_case("list") || w.eq_ignore_ascii_case("ls") => Self::List,
            Some(w) if w.eq_ignore_ascii_case("set") => {
                match args.get(1).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                    Some(id) => Self::Set(id),
                    None => Self::SetMissingId,
                }
            }
            Some(other) => Self::Unknown(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_and_ls_all_mean_list() {
        assert_eq!(AgentSub::parse(&[]), AgentSub::List);
        assert_eq!(AgentSub::parse(&[""]), AgentSub::List);
        assert_eq!(AgentSub::parse(&["list"]), AgentSub::List);
        // `ls` 是 TUI 一直在用、Web 漏掉的写法：现在两边同源。
        assert_eq!(AgentSub::parse(&["ls"]), AgentSub::List);
        assert_eq!(AgentSub::parse(&["LS"]), AgentSub::List);
        assert_eq!(AgentSub::parse(&["  list  "]), AgentSub::List);
    }

    #[test]
    fn set_carries_the_id_trimmed() {
        assert_eq!(
            AgentSub::parse(&["set", "reviewer"]),
            AgentSub::Set("reviewer")
        );
        assert_eq!(
            AgentSub::parse(&["set", "  reviewer  "]),
            AgentSub::Set("reviewer")
        );
        assert_eq!(
            AgentSub::parse(&["SET", "default"]),
            AgentSub::Set("default")
        );
    }

    #[test]
    fn set_without_id_is_distinct_from_list() {
        // 二者的提示不同：一个是「用法」，一个是列角色。合并会让 `/agent set` 静默列出角色。
        assert_eq!(AgentSub::parse(&["set"]), AgentSub::SetMissingId);
        assert_eq!(AgentSub::parse(&["set", "   "]), AgentSub::SetMissingId);
        assert_ne!(AgentSub::parse(&["set"]), AgentSub::List);
    }

    #[test]
    fn unknown_subcommand_keeps_the_raw_word() {
        assert_eq!(AgentSub::parse(&["add", "x"]), AgentSub::Unknown("add"));
        assert_eq!(AgentSub::parse(&["啥"]), AgentSub::Unknown("啥"));
    }
}
