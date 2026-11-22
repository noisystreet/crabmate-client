//! 权限等级（Permission Levels）的共享词表（纯逻辑，无 IO）。
//!
//! 权威定义在服务端 `cm_types/permission_level.rs`：判定、min 链钳制、六轴投影都在那边。
//! 这里只固化**线格式词表**与**固定展示顺序**，避免前端 / TUI 各写一份同义词。
//!
//! Client 侧铁律：不复制任何等级比较规则（`available` / `effective` 全由服务端算），
//! 本模块只做「字符串归一」与「顺序声明」。

/// `GET /permission` 的响应契约**直接取自契约 crate**，不在本仓镜像字段。
///
/// 线形状权威在 Server `cm_api_contract/permission.rs`（`level` / `level_source` /
/// `ceiling` / `ceiling_source` / `available` / `clamped_by` / `caps`），
/// 由契约 semver / git rev 钉住；客户端改字段只能随契约升级。
pub use crabmate::cm_api_contract::permission::{PermissionLevelSource, PermissionView};

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

/// 把服务端 `clamped_by` 排成 [`CLAMP_ORDER`] 的固定展示顺序，并**丢弃**词表外的轴。
///
/// 服务端本来就是这个顺序，这里再排一次是让 UI 不依赖数组顺序（服务端调换顺序不该让界面跳）；
/// 未知轴丢弃而不是原样透出——它是服务端实现细节，客户端没有对应文案，硬渲染只会露出一个
/// 英文标识符。全未命中（未被钳制）返回空 `Vec`，调用方据此显示「未收窄」。
#[must_use]
pub fn ordered_clamped_axes(clamped_by: &[String]) -> Vec<&'static str> {
    CLAMP_ORDER
        .iter()
        .copied()
        .filter(|axis| {
            clamped_by
                .iter()
                .any(|raw| raw.trim().eq_ignore_ascii_case(axis))
        })
        .collect()
}

/// 权限等级**显示形态**（纯逻辑，不含文案）：TUI 状态行的 `perm` 段与桌面底栏那颗 chip
/// 显示的是同一件事，规则各写一遍就会分叉。
///
/// 取值照 TUI 的实测结论（`tui_mode/permission_intent.rs` 的 `status_values` + `perm_value`）：
///
/// - **有已持久化会话**（`bound`）：`GET /permission` 带 `conversation_id`，回答的是「这个会话
///   此刻是哪一档」，所以意图与生效可以并列成 `意图 → 生效`（服务端允许意图高于生效，两个都
///   要显示，否则用户以为切换没生效）；
/// - **没有会话**：同一个请求是裸的，回答的是「配置默认档 + 无会话意图」。把摆到箭头右侧就是
///   把「意图随下一条消息生效」读成「服务端把我钳制到了某档」—— 完全不同的结论。所以此时
///   **只给意图**；连意图都没有才把读数标成 [`PermDisplay::Estimated`]（各端渲染成 `~x`）。
/// - 两者都不知道 → `None`：**不拿** `permission_level_default` 顶替，那是「没意图时会落到
///   哪」，不是「此刻是哪」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermDisplay {
    /// 只有一个值（意图或生效其一）。
    Single(String),
    /// 意图与生效不同，按「意图 → 生效」并列。
    IntentThenEffective { intent: String, effective: String },
    /// 估算：没有已持久化会话，这个值是「没意图时会落到哪」。渲染时加 `~` 前缀。
    Estimated(String),
}

impl PermDisplay {
    /// 线格式（无本地化）：TUI 状态行用。`→` 两侧不留空格（终端宽度紧张）。
    #[must_use]
    pub fn plain(&self) -> String {
        match self {
            Self::Single(v) => v.clone(),
            Self::IntentThenEffective { intent, effective } => format!("{intent}→{effective}"),
            Self::Estimated(v) => format!("~{v}"),
        }
    }
}

/// 见 [`PermDisplay`] 的文档：`bound` = 有没有已持久化的 `conversation_id`。
#[must_use]
pub fn perm_display(bound: bool, intent: Option<&str>, level: Option<&str>) -> Option<PermDisplay> {
    let intent = intent.map(str::trim).filter(|s| !s.is_empty());
    let level = level.map(str::trim).filter(|s| !s.is_empty());
    if !bound {
        return match intent {
            Some(i) => Some(PermDisplay::Single(i.to_string())),
            None => level.map(|l| PermDisplay::Estimated(l.to_string())),
        };
    }
    match (intent, level) {
        (Some(i), Some(l)) if i == l => Some(PermDisplay::Single(i.to_string())),
        (Some(i), Some(l)) => Some(PermDisplay::IntentThenEffective {
            intent: i.to_string(),
            effective: l.to_string(),
        }),
        (Some(i), None) => Some(PermDisplay::Single(i.to_string())),
        (None, Some(l)) => Some(PermDisplay::Single(l.to_string())),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLAMP_ORDER, DEFAULT_LEVEL, LEVELS, PermDisplay, is_known_level, normalize_level,
        ordered_clamped_axes, perm_display,
    };

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

    #[test]
    fn clamp_axes_follow_the_fixed_order_and_drop_unknown() {
        // 服务端顺序被打乱、夹了一个客户端不认识的轴：输出仍是固定顺序，且只留认识的。
        let raw = vec![
            "session_mode".to_string(),
            " Sandbox ".to_string(),
            "ROLE".to_string(),
        ];
        assert_eq!(ordered_clamped_axes(&raw), vec!["role", "session_mode"]);
        // 未被钳制 = 空输入，调用方据此显示「未收窄」。
        assert!(ordered_clamped_axes(&[]).is_empty());
        assert_eq!(
            ordered_clamped_axes(&["ceiling".to_string()]),
            vec!["ceiling"]
        );
    }

    /// 场景矩阵：`bound` × (意图, 生效)。与 TUI 状态行、桌面 chip 共用这一份判定。
    #[test]
    fn perm_display_follows_bound_and_never_invents_a_level() {
        // 有会话：两者不同 → 并列（意图在左，箭头指向生效）。
        assert_eq!(
            perm_display(true, Some("unrestricted"), Some("read")),
            Some(PermDisplay::IntentThenEffective {
                intent: "unrestricted".to_string(),
                effective: "read".to_string(),
            })
        );
        assert_eq!(
            perm_display(true, Some("read"), Some("read")).map(|d| d.plain()),
            Some("read".to_string())
        );
        // 只有一侧：给那一侧。
        assert_eq!(
            perm_display(true, Some("workspace"), None).map(|d| d.plain()),
            Some("workspace".to_string())
        );
        assert_eq!(
            perm_display(true, None, Some("approved")).map(|d| d.plain()),
            Some("approved".to_string())
        );
        assert_eq!(perm_display(true, None, None), None);
        // 空白不算「有意图」。
        assert_eq!(
            perm_display(true, Some("  "), Some("read")).map(|d| d.plain()),
            Some("read".to_string())
        );

        // 没有会话：有意图只给意图（**不**并列，否则会被读成「服务端钳制」）。
        assert_eq!(
            perm_display(false, Some("unrestricted"), Some("approved")).map(|d| d.plain()),
            Some("unrestricted".to_string())
        );
        // 没有会话也没有意图：标成估算 `~`。
        assert_eq!(
            perm_display(false, None, Some("approved")).map(|d| d.plain()),
            Some("~approved".to_string())
        );
        assert_eq!(perm_display(false, None, None), None);
    }
}
