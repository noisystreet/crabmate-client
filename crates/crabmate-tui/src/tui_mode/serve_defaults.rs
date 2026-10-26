//! serve 默认偏好：`/status?view=shell` 的默认 model/role/mode 解析
//! （override 未设置时状态行回退显示；独立文件以控制 `state.rs` 行数门禁 ≤ 920）。

use std::collections::BTreeMap;

use crabmate_client_api::context_axis::new_session_baseline_prompt_tokens;
use crabmate_client_api::permission::PermissionView;
use serde_json::Value;

/// serve 默认偏好（`GET /status?view=shell`），override 未设置时状态行回退显示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeDefaults {
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
    /// 新会话（尚无服务端 `conversation_id`）时状态栏回落的 system-only prompt token 粗估，
    /// 按 agent role 存（键 `""` 是默认档）。来源 `tiktoken_new_session_baseline_by_agent_role`。
    ///
    /// Desktop 就是这么做的：新会话没有实测，用 `GET /status` 给的基线撑住状态栏，
    /// 并带 `~` 前缀标明是估算（`status_bar_context_is_baseline_estimate`）。
    pub baseline_prompt_tokens_by_role: BTreeMap<String, u32>,
    /// 当前权限投影（`GET /permission` 的完整响应）。
    ///
    /// **不来自** `/status`：那边只有 `permission_level_default` / `permission_level_max`
    /// 两个配置值，回答不了「此刻实际生效哪一档」。worker 在拉默认偏好的同一个任务里
    /// 另拉 `GET /permission` 合并进来；404（老 serve）或拉取失败时保持 `None`，
    /// 状态行就不显示这一段——不猜默认档。
    ///
    /// 存整个投影而不是只存 `level`：切换菜单要的 `available` 也在这里。只留 level 的话，
    /// 每次开菜单都得再打一次 `/permission`（Desktop chip 吃的也是同一份 view）。
    pub permission_view: Option<PermissionView>,
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
            baseline_prompt_tokens_by_role: baseline_by_role(v),
            // `/status` 不提供生效等级；由 worker 另拉 `/permission` 后回填。
            permission_view: None,
        }
    }

    /// 此刻生效的档（min 链之后）。`None` = 不知道：老 serve 没有 `/permission`、
    /// 还没拉到，或服务端给了空白。
    #[must_use]
    pub fn effective_level(&self) -> Option<&str> {
        self.permission_view
            .as_ref()
            .map(|v| v.level.trim())
            .filter(|s| !s.is_empty())
    }

    /// 选择器选项集（服务端算好的 `available`，自低到高）。没有投影时为空——
    /// 客户端**不**用本地的五档词表兜底：那会把「服务端不让选的高档」也列出来。
    #[must_use]
    pub fn available_levels(&self) -> &[String] {
        self.permission_view
            .as_ref()
            .map_or(&[], |v| v.available.as_slice())
    }

    /// 新会话的基线用量：显式角色 → 默认角色 → `""` 档。
    ///
    /// 查表口径在 `crabmate_client_api::context_axis`，与 Desktop
    /// `status_bar_new_session_baseline_prompt_tokens` 调的是同一个函数。
    #[must_use]
    pub fn baseline_prompt_tokens(&self, role: Option<&str>) -> Option<u32> {
        new_session_baseline_prompt_tokens(
            &self.baseline_prompt_tokens_by_role,
            role,
            self.role.as_deref(),
        )
    }
}

/// 解析 `tiktoken_new_session_baseline_by_agent_role`（role → tokens）；非对象 / 非数字项跳过。
fn baseline_by_role(v: &Value) -> BTreeMap<String, u32> {
    v.get("tiktoken_new_session_baseline_by_agent_role")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, val)| {
                    val.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .map(|n| (k.clone(), n))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::ServeDefaults;
    use serde_json::json;

    fn defaults() -> ServeDefaults {
        ServeDefaults::from_status(&json!({
            "model": "gpt-4o",
            "default_agent_role_id": "coder",
            "tiktoken_new_session_baseline_by_agent_role": {
                "": 9000,
                "coder": 12000,
                "writer": 5000
            }
        }))
    }

    #[test]
    fn baseline_prefers_selected_role_then_default_then_blank_key() {
        let d = defaults();
        assert_eq!(d.baseline_prompt_tokens(Some("writer")), Some(5_000));
        // 未知角色 → 回落到默认角色
        assert_eq!(d.baseline_prompt_tokens(Some("nope")), Some(12_000));
        // 没给角色 → 默认角色
        assert_eq!(d.baseline_prompt_tokens(None), Some(12_000));
    }

    #[test]
    fn baseline_falls_back_to_blank_key_and_handles_missing() {
        let mut d = defaults();
        d.role = Some("gone".to_string());
        // 默认角色不在 map 里 → `""` 档
        assert_eq!(d.baseline_prompt_tokens(None), Some(9_000));
        // 服务端没给基线：宁可不显示
        let none = ServeDefaults::from_status(&json!({"model": "m"}));
        assert_eq!(none.baseline_prompt_tokens(None), None);
    }

    #[test]
    fn from_status_keeps_existing_fields() {
        let d = defaults();
        assert_eq!(d.model.as_deref(), Some("gpt-4o"));
        assert_eq!(d.role.as_deref(), Some("coder"));
        assert_eq!(d.mode, None);
    }
}
