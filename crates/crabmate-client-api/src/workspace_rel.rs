//! 工作区相对路径片段运算（POSIX 风格，与 serve 的 `path` 查询参数一致）。
//!
//! 只收两端**逐字等价**的纯函数：父 + 子名拼装、父目录推导。端内的树状态
//! （展开集、目录缓存、可见行扁平化、祖先展开、子树裁剪）仍留 `frontend`
//! 与 `crabmate-tui` 各自实现——那些是各端 UI 结构，不是同一份规则。

/// 父相对路径 + 子名 → 子相对路径（根下子项返回 `name`）。
#[must_use]
pub fn child_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// 相对路径的父目录（根下文件/夹返回 `""`）。
#[must_use]
pub fn parent_rel(rel: &str) -> String {
    rel.rfind('/')
        .map(|i| rel[..i].to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_at_root_is_the_name() {
        assert_eq!(child_rel("", "src"), "src");
    }

    #[test]
    fn child_joins_with_a_single_slash() {
        assert_eq!(child_rel("src", "main.rs"), "src/main.rs");
        assert_eq!(child_rel("a/b", "c"), "a/b/c");
    }

    #[test]
    fn parent_of_nested_is_the_prefix() {
        assert_eq!(parent_rel("src/lib/mod.rs"), "src/lib");
    }

    #[test]
    fn parent_of_top_level_is_empty() {
        assert_eq!(parent_rel("main.rs"), "");
        assert_eq!(parent_rel(""), "");
    }

    #[test]
    fn parent_of_trailing_slash_strips_it() {
        assert_eq!(parent_rel("src/"), "src");
    }
}
