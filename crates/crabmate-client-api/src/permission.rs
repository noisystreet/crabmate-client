//! 权限等级（Permission Levels）的共享词表（纯逻辑，无 IO）。
//!
//! 权威定义在服务端 `cm_types/permission_level.rs`：判定、min 链钳制、六轴投影都在那边。
//! 这里只固化**线格式词表**与**固定展示顺序**，避免前端 / TUI 各写一份同义词。
//!
//! Client 侧铁律：不复制任何等级比较规则（`available` / `effective` 全由服务端算），
//! 本模块只做「字符串归一」与「顺序声明」。

/// 五档词表，**自低到高**（与服务端 `PermissionLevel` 的 serde 线格式一致）。
pub const LEVELS: [&str; 5] = ["chat", "read", "workspace", "approved", "unrestricted"];

/// 服务端在「无会话意图 + 无配置覆盖」时落地的默认档。
pub const DEFAULT_LEVEL: &str = "approved";

/// `clamped_by` 的**固定展示顺序**：`ceiling` → `role` → `session_mode`。
///
/// 服务端 `PermissionClamp` 只有这三个变体（文档里出现过的 `sandbox` 未实现）。
pub const CLAMP_ORDER: [&str; 3] = ["ceiling", "role", "session_mode"];

/// 把任意输入归一成词表里的规范值（`trim` + 大小写不敏感）。
///
/// 服务端 `parse_permission_level` 也是同样口径；不认识的值返回 `None`，
/// 由调用方决定「忽略」还是「走未知值兜底文案」，**不要**在这里猜一个默认档。
#[must_use]
pub fn normalize_level(raw: &str) -> Option<&'static str> {
    let t = raw.trim();
    LEVELS.iter().copied().find(|l| l.eq_ignore_ascii_case(t))
}

/// 该值是否落在五档词表内（大小写不敏感）。
#[must_use]
pub fn is_known_level(raw: &str) -> bool {
    normalize_level(raw).is_some()
}

#[cfg(test)]
mod tests {
    use super::{CLAMP_ORDER, DEFAULT_LEVEL, LEVELS, is_known_level, normalize_level};

    #[test]
    fn levels_are_the_five_wire_values_in_ascending_order() {
        assert_eq!(
            LEVELS,
            ["chat", "read", "workspace", "approved", "unrestricted"]
        );
        assert_eq!(DEFAULT_LEVEL, "approved");
        assert!(LEVELS.contains(&DEFAULT_LEVEL));
    }

    #[test]
    fn normalize_trims_and_ignores_case() {
        assert_eq!(normalize_level(" Workspace "), Some("workspace"));
        assert_eq!(normalize_level("UNRESTRICTED"), Some("unrestricted"));
        assert_eq!(normalize_level(""), None);
        // 不认识的档位不猜默认值，交给调用方兜底。
        assert_eq!(normalize_level("root"), None);
        assert!(!is_known_level("root"));
        assert!(is_known_level("read"));
    }

    #[test]
    fn clamp_order_covers_only_the_three_server_variants() {
        assert_eq!(CLAMP_ORDER, ["ceiling", "role", "session_mode"]);
        // `sandbox` 在服务端文档里写过但代码没有，防止照着文档又加回来。
        assert!(!CLAMP_ORDER.contains(&"sandbox"));
    }
}
