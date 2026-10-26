//! `POST /conversation/state`（会话级运行态意图写入口）的客户端侧纯逻辑，无 IO。
//!
//! **契约类型直接取自契约 crate**，本仓不镜像：字段增减只能随 pin 升级，
//! 镜像一份就会在服务端改字段时静默漂移（`permission` 模块对 `PermissionView` 已按同一口径处理）。
//!
//! 本模块只收「两端必须逐字一致」的那部分判断：一次响应到底属于哪种结局。
//! 前端与 TUI 各写一份「404 两义」的判别，就是新的漂移点——同一个 404，
//! 老 serve 是「没有这条路由」，新 serve 是「会话不存在」，两者该给的界面完全不同
//! （只读回退 vs 提示重新绑定）。
//!
//! 这里**不做**等级比较、不算 min 链、不复制「至少给一个字段」以外的校验：
//! `available` / `level` / `clamped_by` 全是服务端算好的。

use serde_json::Value;

pub use crabmate::cm_api_contract::session_state::{ConversationStatePatch, ConversationStateView};

use crate::messages::api_error_code;

/// `CONVERSATION_NOT_FOUND` 的线值（与 Server `cm_api_contract/error_codes.rs` 的常量名同形，
/// 那边是一组 `&str` 常量而不是枚举，跨仓没有可 import 的类型）。
const CODE_CONVERSATION_NOT_FOUND: &str = "CONVERSATION_NOT_FOUND";

/// 一次写入口调用的结局。
///
/// 变体划分的依据是**调用方要做不同的事**，不是 HTTP 状态码本身：
/// 前三者各自对应一种界面，其余全部落 [`Self::Failed`] 由上层按 `status` / `code` 出文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationStateOutcome {
    /// 写入成功：`active_*` 是**落库的意图**（可能高于生效上限），
    /// `permission` 是写后重算的投影——选择器与只读块都吃这一份，不必再打 `GET /permission`。
    /// 装箱是因为它带整套权限投影（六轴 + 选项集），内联会让其余三个小变体都按它的
    /// 体积走，而失败路径（更常见的那一半）不该为成功载荷付内存。
    Ok(Box<ConversationStateView>),
    /// 404 + `CONVERSATION_NOT_FOUND`：会话不存在或已被 TTL 淘汰。
    /// 提示重新绑定 / 新建，**不要**静默重试（重试永远同一个结果）。
    NotFound,
    /// 404 且 body 不是 `ApiError` 形状：该 serve 没有这条路由（老版本）。
    /// 与 `GET /permission` 的裸 404 是同一件事，必须落进**同一个** capability 状态，
    /// 否则会出现「只读块说不支持、选择器说会话不存在」的自相矛盾。
    Unsupported,
    /// 其它失败（400 词表不符 / 500 库写失败 / 401 缺 Bearer / 非 2xx 但解析不出 code）。
    ///
    /// `code` 为 `None` 时**不要**反推成某个具体理由：那只是「非结构化失败」。
    /// 500 `CONVERSATION_STORE_ERROR` 是幂等可重试的（同值重复 POST 只是重复 UPDATE）。
    Failed { status: u16, code: Option<String> },
}

impl ConversationStateOutcome {
    /// 机器可读错误码（成功 / 会话不存在 / 不支持时为 `None`）。
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Failed { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

/// 把一次 HTTP 响应收敛成 [`ConversationStateOutcome`]。
///
/// 判别顺序是有讲究的：
///
/// 1. **2xx 先解析 body**。状态码说成功但 body 解析不出 `ConversationStateView` 时记为失败，
///    而不是造一个「半截成功」——调用方若信任它就会把界面刷成空意图。
/// 2. **404 只在解析不出 `error.code` 时才算「不支持」**。老 serve 的 404 是 axum 默认响应
///    （空 body 或纯文本），新 serve 的 404 带 `{"error":{"code":"CONVERSATION_NOT_FOUND"}}`。
///    按状态码分流会把「会话过期」误报成「该 serve 太旧」，或反过来。
/// 3. 其余状态码一律 [`ConversationStateOutcome::Failed`]，`code` 能取就带上（401 → 既有 Bearer 引导，
///    400 → 词表漂移，500 → 可重试），取不到就只留 `status`。
#[must_use]
pub fn classify_conversation_state_response(status: u16, body: &str) -> ConversationStateOutcome {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    // 先转成 owned：后面 `parsed` 要 move 进 `from_value`。
    let code = parsed.as_ref().and_then(api_error_code).map(str::to_string);
    if (200..300).contains(&status) {
        return match parsed.and_then(|v| serde_json::from_value::<ConversationStateView>(v).ok()) {
            Some(view) => ConversationStateOutcome::Ok(Box::new(view)),
            None => ConversationStateOutcome::Failed { status, code },
        };
    }
    if status == 404 {
        if code.is_none() {
            return ConversationStateOutcome::Unsupported;
        }
        return if code.as_deref() == Some(CODE_CONVERSATION_NOT_FOUND) {
            ConversationStateOutcome::NotFound
        } else {
            // 404 却给了别的 code（当前服务端不会这样）：保留 code 让上层如实显示，
            // 不要折叠成上面两义的任何一个。
            ConversationStateOutcome::Failed { status, code }
        };
    }
    ConversationStateOutcome::Failed { status, code }
}

#[cfg(test)]
mod tests {
    use super::{ConversationStateOutcome, classify_conversation_state_response};

    /// 一个形状完整的成功响应（`caps` 的取值不参与反序列化，字段齐全即可）。
    fn view_body() -> String {
        serde_json::json!({
            "conversation_id": "c1",
            "active_permission_level": "workspace",
            "active_session_mode": "act",
            "permission": {
                "level": "read",
                "level_source": "persisted",
                "ceiling": "unrestricted",
                "ceiling_source": "derived_loopback",
                "available": ["chat", "read", "workspace"],
                "clamped_by": ["role"],
                "caps": {
                    "tool_visibility": "readonly",
                    "external_tools": "denied",
                    "path_scope": "session_root",
                    "command_scope": "none",
                    "net_scope": "prefixes",
                    "approval": "gate"
                }
            }
        })
        .to_string()
    }

    #[test]
    fn success_carries_both_intent_and_effective_projection() {
        let out = classify_conversation_state_response(200, &view_body());
        let ConversationStateOutcome::Ok(view) = out else {
            panic!("200 + 完整 body 必须解析成 Ok：{out:?}");
        };
        // 意图与生效是两个值：意图高于生效是**允许**的服务端状态（越界由 min 链钳制），
        // 客户端两个都要能读到，不能把意图改写成生效值。
        assert_eq!(view.active_permission_level.as_deref(), Some("workspace"));
        assert_eq!(view.permission.level, "read");
        assert_eq!(view.permission.clamped_by, vec!["role".to_string()]);
    }

    #[test]
    fn two_kinds_of_404_are_told_apart_by_the_code() {
        // 新 serve：会话不存在 / 已过期。
        let not_found = r#"{"error":{"code":"CONVERSATION_NOT_FOUND","message":"会话不存在"}}"#;
        assert_eq!(
            classify_conversation_state_response(404, not_found),
            ConversationStateOutcome::NotFound
        );
        // 老 serve：axum 默认 404（无 ApiError 形状）→ 能力缺失，不是错误。
        assert_eq!(
            classify_conversation_state_response(404, ""),
            ConversationStateOutcome::Unsupported
        );
        assert_eq!(
            classify_conversation_state_response(404, "404 Not Found"),
            ConversationStateOutcome::Unsupported
        );
        // 裸 JSON 但没有 error.code：仍视为「没有这条路由」，不然会把老 serve 报成失败。
        assert_eq!(
            classify_conversation_state_response(404, r#"{"message":"nope"}"#),
            ConversationStateOutcome::Unsupported
        );
    }

    #[test]
    fn store_failure_is_retryable_and_never_reported_as_missing_session() {
        let body = r#"{"error":{"code":"CONVERSATION_STORE_ERROR","message":"写入失败"}}"#;
        let out = classify_conversation_state_response(500, body);
        assert_eq!(
            out,
            ConversationStateOutcome::Failed {
                status: 500,
                code: Some("CONVERSATION_STORE_ERROR".to_string())
            }
        );
        assert_eq!(out.code(), Some("CONVERSATION_STORE_ERROR"));
        // 关键：500 不能落进 NotFound（那会让 UI 提示「重新绑定」，而其实重试就好）。
        assert_ne!(out, ConversationStateOutcome::NotFound);
    }

    #[test]
    fn unauthorized_keeps_status_for_the_existing_bearer_banner() {
        let out = classify_conversation_state_response(401, "");
        assert_eq!(
            out,
            ConversationStateOutcome::Failed {
                status: 401,
                code: None
            }
        );
        assert_eq!(out.code(), None);
    }

    #[test]
    fn invalid_level_is_surfaced_by_code_not_by_guessing() {
        let body = r#"{"error":{"code":"INVALID_PERMISSION_LEVEL","message":"x"}}"#;
        assert_eq!(
            classify_conversation_state_response(400, body).code(),
            Some("INVALID_PERMISSION_LEVEL")
        );
    }

    #[test]
    fn two_xx_without_a_parsable_view_is_a_failure_not_a_half_success() {
        // 200 但 body 缺 `permission`：构造不出 View，就不能给上层一个「成功」。
        let out = classify_conversation_state_response(200, r#"{"conversation_id":"c1"}"#);
        assert!(matches!(
            out,
            ConversationStateOutcome::Failed { status: 200, .. }
        ));
        // 200 但 body 根本不是 JSON。
        let out = classify_conversation_state_response(200, "<html>");
        assert!(matches!(
            out,
            ConversationStateOutcome::Failed {
                status: 200,
                code: None
            }
        ));
    }
}
