//! Web「设置」中 `client_llm.api_base` 的预设反查。
//!
//! 预设表本体在 `crabmate_client_api::client_llm_presets`——与 TUI 设置面板共用
//! 的单一来源（此前两端各持一份逐字相同的 `id`+`url` 表，靠注释约定「两仓同步」）。
//! 本模块只保留 Web 侧的对外名字，调用点无需改动。
//!
//! 原本地拷贝里的 `suggested_model`（「选此项时填建议模型名」）在 `frontend` 内
//! 没有任何消费点，随这次下沉一并移除，未接线的功能不再以字段形式留存。

pub use crabmate_client_api::client_llm_presets::select_id_for_url as api_base_select_value_for_draft;

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
