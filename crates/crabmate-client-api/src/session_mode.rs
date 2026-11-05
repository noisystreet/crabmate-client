//! 会话模式（`ask` / `plan` / `act`）的**协议取值词表**。
//!
//! 这是 serve 契约里 `session_mode` 的合法取值，不是 UI 文案 —— 与
//! [`crate::permission::LEVELS`] 同类。两端各自手写 `matches!(m, "ask" | "plan" | "act")`
//! 的代价是：服务端新增一档时要改 9 处，且漏改的两个方向都不是显式失败 ——
//! 漏在「解析」侧会把合法值当非法丢掉，漏在「校验」侧则让用户选得进去、发不出去。
//!
//! 判定与改造前的既有写法**逐字等价**（严格比较，不 trim、不忽略大小写）：下沉是为了
//! 让词表有唯一出处，不是为了顺手放宽输入。

/// 合法会话模式，顺序即 UI 的展示顺序。
pub const SESSION_MODES: [&str; 3] = ["ask", "plan", "act"];

/// 是否是合法会话模式。
#[must_use]
pub fn is_known_mode(raw: &str) -> bool {
    SESSION_MODES.contains(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exactly_the_three_modes() {
        for m in SESSION_MODES {
            assert!(is_known_mode(m), "{m} should be known");
        }
        assert!(!is_known_mode("off"));
        assert!(!is_known_mode(""));
        // 严格：与改造前的 `matches!` 一致，不放宽大小写 / 空白。
        assert!(!is_known_mode("ASK"));
        assert!(!is_known_mode(" ask"));
    }

    #[test]
    fn table_order_is_the_ui_order() {
        assert_eq!(SESSION_MODES, ["ask", "plan", "act"]);
    }
}
