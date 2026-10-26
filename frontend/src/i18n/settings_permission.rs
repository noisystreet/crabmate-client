//! 设置页「权限等级」只读块的文案（`GET /permission` 的策略投影）。
//!
//! 单独成文件而不塞进 `settings.rs`：那一支已接近单文件 920 行棘轮，按域拆分
//! 也与 `settings_mcp` / `settings_github` 的既有做法一致。

use super::Locale;

pub fn settings_permission_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "权限等级",
        Locale::En => "Permission level",
    }
}

pub fn settings_permission_current(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "当前等级",
        Locale::En => "Current level",
    }
}

pub fn settings_permission_ceiling(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "服务端上限",
        Locale::En => "Server ceiling",
    }
}

pub fn settings_permission_available(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "可选档位",
        Locale::En => "Available levels",
    }
}

pub fn settings_permission_clamped(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "收窄原因",
        Locale::En => "Narrowed by",
    }
}

pub fn settings_permission_clamped_none(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未被收窄",
        Locale::En => "not narrowed",
    }
}

pub fn settings_permission_source_config(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "配置默认",
        Locale::En => "config default",
    }
}

pub fn settings_permission_source_persisted(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "会话已保存",
        Locale::En => "saved in this conversation",
    }
}

pub fn settings_permission_loading(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "正在读取…",
        Locale::En => "Loading…",
    }
}

/// 404：该 serve 没有 `/permission`（老版本）。**这不是错误**，不显示等级也不弹错。
pub fn settings_permission_unsupported(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "该 serve 不支持权限等级（没有 /permission 路由）",
        Locale::En => "This serve does not expose permission levels (no /permission route)",
    }
}

pub fn settings_permission_unavailable(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "读取失败：",
        Locale::En => "Failed: ",
    }
}

pub fn settings_permission_refresh(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "刷新",
        Locale::En => "Refresh",
    }
}

pub fn settings_permission_readonly_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "只读：等级、上限与可选档位都由 serve 判定，客户端不自行推导。",
        Locale::En => "Read-only: serve decides the level, the ceiling and the selectable set.",
    }
}

pub fn settings_permission_selector(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "切换档位",
        Locale::En => "Switch level",
    }
}

/// 没有表达过意图时的说明。刻意不说「默认」——那是配置默认，这里是「沿用会话已保存的」。
pub fn settings_permission_intent_none(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未选择（沿用会话已保存的档位）",
        Locale::En => "No intent (this conversation's saved level applies)",
    }
}

/// 尚未绑定已持久化会话：选择只写本地意图，随下一条消息生效。
/// 服务端对写入口不做 upsert（空行会污染会话列表与 TTL 淘汰），所以这里不能假装成功。
pub fn settings_permission_send_to_apply(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已记录：绑定会话后发送一条消息即按此档执行。",
        Locale::En => "Recorded: it applies to your next message once a conversation is bound.",
    }
}

pub fn settings_permission_conversation_gone(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "该会话不存在或已过期，无法写入；请新建会话后再试。",
        Locale::En => {
            "That conversation no longer exists or expired; start a new one and try again."
        }
    }
}

pub fn settings_permission_write_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "写入失败：",
        Locale::En => "Write failed: ",
    }
}

/// 500 `CONVERSATION_STORE_ERROR`：库写失败，写入是幂等的，重试即可——
/// 不能沿用「会话不存在」的文案（那会让用户去重新绑定一个其实存在的会话）。
pub fn settings_permission_write_retry(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "服务端写入失败，可重试：",
        Locale::En => "Server write failed (retryable): ",
    }
}

/// 本地归一失败：当前 serve 的词表与客户端不一致。
pub fn settings_permission_unknown_level(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "该 serve 不接受这个档位（权限等级词表可能已变化）。",
        Locale::En => {
            "This serve does not accept that level (the level vocabulary may have changed)."
        }
    }
}

/// 不带 `agent_role` / `conversation_id` 时显示的说明。
///
/// 设置页只取静态投影，而标签写的是「当前等级」——不说清这一点，用户会把它当成
/// 这个会话里实际生效的档位（角色上限或会话模式收窄后两者确实不同）。
pub fn settings_permission_static_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "未按会话或角色收窄：这里显示的是服务端默认投影，会话内实际生效的档位可能不同。"
        }
        Locale::En => {
            "Not narrowed per conversation or role: this is serve's default projection, which can differ from the level in effect inside a conversation."
        }
    }
}

/// 五档等级的本地化显示名；未知值原样返回（不猜、不吞）。
#[must_use]
pub fn settings_permission_level_label(l: Locale, level: &str) -> String {
    match (l, level) {
        (Locale::ZhHans, "chat") => "聊天".into(),
        (Locale::ZhHans, "read") => "只读".into(),
        (Locale::ZhHans, "workspace") => "工作区".into(),
        (Locale::ZhHans, "approved") => "已批准".into(),
        (Locale::ZhHans, "unrestricted") => "不受限".into(),
        (Locale::En, "chat") => "Chat".into(),
        (Locale::En, "read") => "Read".into(),
        (Locale::En, "workspace") => "Workspace".into(),
        (Locale::En, "approved") => "Approved".into(),
        (Locale::En, "unrestricted") => "Unrestricted".into(),
        _ => level.to_owned(),
    }
}

/// 上限来源（`ceiling_source`）的本地化显示名。
///
/// 契约里它是**裸字符串**（`config` / `derived_loopback` / `derived_non_loopback`），
/// 与 `level_source` 的枚举不同——不本地化就会在界面上直接露出 `derived_non_loopback`。
/// 未知来源原样返回：宁可露出一个标识符，也不要把它吞成「配置」而说谎。
#[must_use]
pub fn settings_permission_ceiling_source_label(l: Locale, source: &str) -> String {
    match (l, source) {
        (Locale::ZhHans, "config") => "配置上限".into(),
        (Locale::ZhHans, "derived_loopback") => "由本机回环地址推导".into(),
        (Locale::ZhHans, "derived_non_loopback") => "由非回环地址推导".into(),
        (Locale::En, "config") => "config ceiling".into(),
        (Locale::En, "derived_loopback") => "derived from a loopback bind".into(),
        (Locale::En, "derived_non_loopback") => "derived from a non-loopback bind".into(),
        _ => source.to_owned(),
    }
}

/// 钳制轴的本地化显示名。取值只用共享层 [`crabmate_client_api::CLAMP_ORDER`] 里的三项，
/// 未知轴在共享层就被丢弃了；这里的兜底只为不 panic。
#[must_use]
pub fn settings_permission_axis_label(l: Locale, axis: &str) -> String {
    match (l, axis) {
        (Locale::ZhHans, "ceiling") => "受服务端上限限制".into(),
        (Locale::ZhHans, "role") => "受角色上限限制".into(),
        (Locale::ZhHans, "session_mode") => "受会话模式限制".into(),
        (Locale::En, "ceiling") => "ceiling cap".into(),
        (Locale::En, "role") => "role cap".into(),
        (Locale::En, "session_mode") => "session mode".into(),
        _ => axis.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wire_level_has_a_label_in_both_locales() {
        // 五档全覆盖：漏一档，界面就会露出一条裸线格式（`workspace` 之类）。
        for level in ["chat", "read", "workspace", "approved", "unrestricted"] {
            assert_ne!(
                settings_permission_level_label(Locale::ZhHans, level),
                level
            );
            assert_ne!(settings_permission_level_label(Locale::En, level), level);
        }
        assert_eq!(
            settings_permission_level_label(Locale::ZhHans, "approved"),
            "已批准"
        );
        assert_eq!(
            settings_permission_level_label(Locale::En, "approved"),
            "Approved"
        );
        // 未知档位不猜默认值，原样返回。
        assert_eq!(
            settings_permission_level_label(Locale::ZhHans, "root"),
            "root"
        );
    }

    #[test]
    fn ceiling_source_is_localised_and_unknown_falls_back() {
        // 三个已知来源都不许露出下划线标识符。
        for source in ["config", "derived_loopback", "derived_non_loopback"] {
            let zh = settings_permission_ceiling_source_label(Locale::ZhHans, source);
            let en = settings_permission_ceiling_source_label(Locale::En, source);
            assert!(!zh.contains('_'), "zh 未本地化：{source} -> {zh}");
            assert!(!en.contains('_'), "en 未本地化：{source} -> {en}");
        }
        assert_eq!(
            settings_permission_ceiling_source_label(Locale::ZhHans, "derived_loopback"),
            "由本机回环地址推导"
        );
        // 服务端将来新增来源时原样透出，而不是错误地归到「配置上限」。
        assert_eq!(
            settings_permission_ceiling_source_label(Locale::ZhHans, "request"),
            "request"
        );
    }

    #[test]
    fn clamp_axis_labels_say_what_narrowed_not_how_high() {
        // 「收窄原因」下只有「服务端上限」会读成「上限是多少」，所以带上「受…限制」。
        assert_eq!(
            settings_permission_axis_label(Locale::ZhHans, "ceiling"),
            "受服务端上限限制"
        );
        assert_eq!(
            settings_permission_axis_label(Locale::En, "session_mode"),
            "session mode"
        );
        assert_ne!(
            settings_permission_axis_label(Locale::ZhHans, "ceiling"),
            settings_permission_ceiling(Locale::ZhHans)
        );
    }

    #[test]
    fn static_projection_hint_is_bilingual() {
        assert!(
            settings_permission_static_hint(Locale::ZhHans).contains("静态")
                || settings_permission_static_hint(Locale::ZhHans).contains("默认投影")
        );
        assert!(settings_permission_static_hint(Locale::En).contains("default projection"));
    }
}
