//! Web「设置」中 `client_llm.api_base` 网关预设。
//!
//! 与 Server [`cm_types::llm_gateway_presets`](https://github.com/noisystreet/CrabMate/blob/main/src/cm_types/llm_gateway_presets.rs)
//! 同源拷贝（展示 crate 下沉 W1）；改 URL / 建议模型时请两仓同步。

/// 单条预设：`id` 用于 UI；`url` 写入 `api_base`（空串表示「沿用服务端 / 自定义」占位）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientLlmApiBasePreset {
    pub id: &'static str,
    pub url: &'static str,
    /// 选此项时，若模型名为空则填入建议值（不覆盖用户已填的 model）。
    pub suggested_model: Option<&'static str>,
}

/// 常用 OpenAI 兼容网关（与 Server 预设表、README 供应商示例一致）。
pub const CLIENT_LLM_API_BASE_PRESETS: &[ClientLlmApiBasePreset] = &[
    ClientLlmApiBasePreset {
        id: "server",
        url: "",
        suggested_model: None,
    },
    ClientLlmApiBasePreset {
        id: "ollama",
        url: "http://127.0.0.1:11434/v1",
        suggested_model: None,
    },
    ClientLlmApiBasePreset {
        id: "deepseek",
        url: "https://api.deepseek.com/v1",
        suggested_model: Some("deepseek-v4-flash"),
    },
    ClientLlmApiBasePreset {
        id: "minimax",
        url: "https://api.minimaxi.com/v1",
        suggested_model: None,
    },
    ClientLlmApiBasePreset {
        id: "zhipu",
        url: "https://open.bigmodel.cn/api/paas/v4",
        suggested_model: Some("glm-5"),
    },
    ClientLlmApiBasePreset {
        id: "moonshot",
        url: "https://api.moonshot.cn/v1",
        suggested_model: Some("kimi-k2.5"),
    },
    ClientLlmApiBasePreset {
        id: "custom",
        url: "",
        suggested_model: None,
    },
];

/// 当前草稿对应的预设 id：空 → `server`；与任一非 server/custom 预设 `url` 完全一致 → 该 id；否则 `custom`。
#[must_use]
pub fn api_base_select_value_for_draft(draft: &str) -> &'static str {
    let t = draft.trim();
    if t.is_empty() {
        return "server";
    }
    for p in CLIENT_LLM_API_BASE_PRESETS {
        if p.id == "server" || p.id == "custom" {
            continue;
        }
        if p.url == t {
            return p.id;
        }
    }
    "custom"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_select_value_empty_is_server() {
        assert_eq!(api_base_select_value_for_draft(""), "server");
    }

    #[test]
    fn api_base_select_value_matches_preset_url() {
        assert_eq!(
            api_base_select_value_for_draft("https://api.deepseek.com/v1"),
            "deepseek"
        );
        assert_eq!(
            api_base_select_value_for_draft("http://127.0.0.1:11434/v1"),
            "ollama"
        );
    }

    #[test]
    fn api_base_select_value_unknown_is_custom() {
        assert_eq!(
            api_base_select_value_for_draft("https://example.com/v1"),
            "custom"
        );
    }
}
