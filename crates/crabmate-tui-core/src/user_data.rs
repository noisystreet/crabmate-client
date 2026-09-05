//! `GET` / `PUT` **`/user-data/prefs`** 与 **`/user-data/llm-overrides`**（对齐 Desktop 设置）。
//!
//! DTO 与前端 `frontend/src/api/user_data.rs` 同构：serve user-data 目录承担跨端共享的
//! 非机密设置。**PUT 为全量 DTO**，调用方必须先 GET 再改自己管理的键后回写（合并保真），
//! 避免覆盖 Desktop 独有的字段。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::ServeClient;
use crate::error::TermError;

/// `GET/PUT /user-data/prefs` 的完整 DTO（镜像前端 `UserPrefsDto`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPrefsDto {
    #[serde(default)]
    pub last_workspace_root: Option<String>,
    /// 最近打开的工作区根（新在前；与 `last_workspace_root` 同步为首项）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_workspace_roots: Vec<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_panel_view: Option<String>,
    #[serde(default)]
    pub side_width: Option<f64>,
    #[serde(default)]
    pub editor_layout_mode: Option<bool>,
    #[serde(default)]
    pub sidebar_rail_collapsed: Option<bool>,
    #[serde(default)]
    pub session_ui_font: Option<String>,
    #[serde(default)]
    pub session_chat_font: Option<String>,
    #[serde(default)]
    pub session_chat_font_size: Option<u32>,
    #[serde(default)]
    pub ide_editor_font: Option<String>,
    #[serde(default)]
    pub ide_editor_font_size: Option<u32>,
    #[serde(default)]
    pub ide_editor_line_numbers: Option<bool>,
    #[serde(default)]
    pub ide_editor_word_wrap: Option<bool>,
    #[serde(default)]
    pub ide_editor_tab_size: Option<u32>,
    #[serde(default)]
    pub bg_decor: Option<bool>,
    /// 聊天主列是否展示本轮 `context_inject` / `context_trim` 旁注；缺省视为关。
    #[serde(default)]
    pub show_turn_context_inject: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_bar_visible: Option<bool>,
    #[serde(default)]
    pub cm_role: Option<String>,
    #[serde(default)]
    pub session_mode: Option<String>,
    #[serde(default)]
    pub disable_readonly_tool_ttl_cache: Option<bool>,
}

/// `llm-overrides` 中单个 llm 端点的覆盖字段（镜像前端 `LlmEndpointOverrideDto`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmEndpointOverrideDto {
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<String>,
    #[serde(default)]
    pub llm_context_tokens: Option<String>,
    #[serde(default)]
    pub llm_thinking_mode: Option<String>,
}

/// `GET/PUT /user-data/llm-overrides` 的完整 DTO（镜像前端 `LlmOverridesDto`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmOverridesDto {
    #[serde(default)]
    pub client_llm: LlmEndpointOverrideDto,
    #[serde(default)]
    pub executor_llm: LlmEndpointOverrideDto,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub saved_models: Vec<Value>,
}

/// `GET /user-data/prefs`。
pub async fn fetch_user_data_prefs(client: &ServeClient) -> Result<UserPrefsDto, TermError> {
    client.get_json("/user-data/prefs").await
}

/// `PUT /user-data/prefs`（全量 DTO；2xx 即成功，不解析响应体）。
pub async fn put_user_data_prefs(
    client: &ServeClient,
    prefs: &UserPrefsDto,
) -> Result<(), TermError> {
    client
        .put_json_no_content(
            "/user-data/prefs",
            &serde_json::to_value(prefs).map_err(|e| TermError::Message(e.to_string()))?,
        )
        .await
}

/// `GET /user-data/llm-overrides`。
pub async fn fetch_llm_overrides(client: &ServeClient) -> Result<LlmOverridesDto, TermError> {
    client.get_json("/user-data/llm-overrides").await
}

/// `PUT /user-data/llm-overrides`（全量 DTO；2xx 即成功，不解析响应体）。
pub async fn put_llm_overrides(
    client: &ServeClient,
    overrides: &LlmOverridesDto,
) -> Result<(), TermError> {
    client
        .put_json_no_content(
            "/user-data/llm-overrides",
            &serde_json::to_value(overrides).map_err(|e| TermError::Message(e.to_string()))?,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefs_dto_round_trips_and_skips_empty() {
        let d = UserPrefsDto {
            cm_role: Some("coder".into()),
            session_mode: Some("plan".into()),
            ..Default::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        // 空 Vec 与 skip_serializing_if 键按规则省略；管理的键保留。
        assert_eq!(v.get("cm_role").unwrap(), "coder");
        assert_eq!(v.get("session_mode").unwrap(), "plan");
        assert!(v.get("recent_workspace_roots").is_none());
        // 无 skip 的 Option 键序列化为 null（全量保真回写语义）。
        assert_eq!(v.get("locale").unwrap(), &serde_json::Value::Null);
        let back: UserPrefsDto = serde_json::from_value(v).unwrap();
        assert_eq!(back.cm_role.as_deref(), Some("coder"));
    }

    #[test]
    fn llm_overrides_round_trip_preserves_unmanaged_sections() {
        let d = LlmOverridesDto {
            client_llm: LlmEndpointOverrideDto {
                model: Some("deepseek-chat".into()),
                ..Default::default()
            },
            executor_llm: LlmEndpointOverrideDto {
                model: Some("exec-x".into()),
                api_base: Some("https://x.example/v1".into()),
                ..Default::default()
            },
            execution_mode: Some("autonomous".into()),
            saved_models: vec![json!({"label": "mine", "enabled": true})],
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["client_llm"]["model"], "deepseek-chat");
        assert_eq!(v["executor_llm"]["model"], "exec-x");
        assert_eq!(v["execution_mode"], "autonomous");
        assert_eq!(v["saved_models"].as_array().unwrap().len(), 1);
        let back: LlmOverridesDto = serde_json::from_value(v).unwrap();
        assert_eq!(back.client_llm.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(back.executor_llm.model.as_deref(), Some("exec-x"));
        assert_eq!(back.saved_models.len(), 1);
        assert!(back.client_llm.api_base.is_none());
    }

    #[test]
    fn empty_strings_kept_for_full_dto_fidelity() {
        // 全量回写保真：值为空串也要按原样保留（写前由上层决定是否置 None）。
        let d = UserPrefsDto {
            cm_role: Some(String::new()),
            session_mode: None,
            ..Default::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v.get("cm_role").unwrap(), "");
        let back: UserPrefsDto = serde_json::from_value(json!({"cm_role": ""})).unwrap();
        assert_eq!(back.cm_role.as_deref(), Some(""));
    }
}
