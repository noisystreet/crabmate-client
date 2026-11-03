//! `GET` / `PUT` **`/user-data/prefs`** 与 **`/user-data/llm-overrides`**（对齐 Desktop 设置）。
//!
//! DTO 与前端 `frontend/src/api/user_data.rs` 同构：serve user-data 目录承担跨端共享的
//! 非机密设置。**PUT 为全量 DTO**，调用方必须先 GET 再改自己管理的键后回写（合并保真），
//! 避免覆盖 Desktop 独有的字段。

use crabmate_client_api::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::serve::client::ServeClient;
use crate::serve::error::TermError;

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
    client.get_json(paths::USER_DATA_PREFS).await
}

/// `PUT /user-data/prefs`（全量 DTO；2xx 即成功，不解析响应体）。
pub async fn put_user_data_prefs(
    client: &ServeClient,
    prefs: &UserPrefsDto,
) -> Result<(), TermError> {
    client
        .put_json_no_content(
            paths::USER_DATA_PREFS,
            &serde_json::to_value(prefs).map_err(|e| TermError::Message(e.to_string()))?,
        )
        .await
}

/// `GET /user-data/llm-overrides`。
pub async fn fetch_llm_overrides(client: &ServeClient) -> Result<LlmOverridesDto, TermError> {
    client.get_json(paths::USER_DATA_LLM_OVERRIDES).await
}

/// `PUT /user-data/llm-overrides`（全量 DTO；2xx 即成功，不解析响应体）。
pub async fn put_llm_overrides(
    client: &ServeClient,
    overrides: &LlmOverridesDto,
) -> Result<(), TermError> {
    client
        .put_json_no_content(
            paths::USER_DATA_LLM_OVERRIDES,
            &serde_json::to_value(overrides).map_err(|e| TermError::Message(e.to_string()))?,
        )
        .await
}

/// `GET/PUT /user-data/mcp-servers` 的完整 DTO（镜像前端 `McpServersFileDto`）。
///
/// TUI 只读写两个**标量**（`global_enabled` / `tool_timeout_secs`）：服务器条目里带
/// `env` / `headers` / `bearer` 这类机密，面板既不展示也不编辑。因此 `servers` 用
/// `Vec<Value>` 原样透传 —— 全量 PUT 时把服务端给的结构照搬回去，既保真又不会因为
/// 这里少一个字段（服务端新增的键）而把它们抹掉。
///
/// **前提（端点契约，不是本文件的自由选择）**：GET 必须**全量**返回这些条目、不做脱敏。
/// 若哪天把 `bearer` 之类打成 `***`，这里的原样回写就会把掩码写回磁盘，真正的密钥被
/// 静默覆盖。前端 `McpServersFileDto` 同样是整份 PUT，所以这条约束两边一致。
/// 另一个已知窗口：保存是读-改-写（GET → 改两个标量 → PUT），两端之间别处（Desktop/Web）
/// 的增删会被这次回写覆盖 —— 只改标量的端点若能收窄成 PATCH，这个窗口才会消失。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersFileDto {
    /// 服务端文件格式版本：TUI 不改它，`None` 时序列化时省略（不写入 `null`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default = "mcp_global_enabled_default")]
    pub global_enabled: bool,
    #[serde(default = "mcp_tool_timeout_default")]
    pub tool_timeout_secs: u64,
    /// 服务器条目：原样回写，不解析（见类型注释）。
    #[serde(default)]
    pub servers: Vec<Value>,
}

/// 缺省全局开关为**开**（与前端 `default_true` 同语义：老文件没有该键时 MCP 仍是启用的）。
fn mcp_global_enabled_default() -> bool {
    true
}

/// 缺省工具超时 60 秒（与前端 `default_mcp_timeout` 对齐）。
fn mcp_tool_timeout_default() -> u64 {
    60
}

/// `GET /user-data/mcp-servers`。
///
/// 404 → `None`（老 serve 没有这个端点）：MCP 分区据此显示「未提供」，而不是把
/// 整个设置快照的加载拖垮。
pub async fn fetch_mcp_servers(
    client: &ServeClient,
) -> Result<Option<McpServersFileDto>, TermError> {
    client.get_json_optional(paths::USER_DATA_MCP_SERVERS).await
}

/// `PUT /user-data/mcp-servers`（全量 DTO；2xx 即成功，不解析响应体）。
pub async fn put_mcp_servers(
    client: &ServeClient,
    file: &McpServersFileDto,
) -> Result<(), TermError> {
    client
        .put_json_no_content(
            paths::USER_DATA_MCP_SERVERS,
            &serde_json::to_value(file).map_err(|e| TermError::Message(e.to_string()))?,
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
