//! serve 默认偏好：`/status?view=shell` 的默认 model/role/mode 解析
//! （override 未设置时状态行回退显示；独立文件以控制 `state.rs` 行数门禁 ≤ 920）。

use serde_json::Value;

/// serve 默认偏好（`GET /status?view=shell`），override 未设置时状态行回退显示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeDefaults {
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
}

impl ServeDefaults {
    /// 从 `/status?view=shell` 的 JSON 提取默认 model/role/mode。
    #[must_use]
    pub fn from_status(v: &Value) -> Self {
        let field = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self {
            model: field("model"),
            role: field("default_agent_role_id"),
            mode: field("default_session_mode"),
        }
    }
}
