//! 桌面 / 移动 Tauri 共用的「连接远程 `crabmate serve`」逻辑。
//!
//! - 规范化 URL、探测 `GET /health` + 受保护的 `GET /user-data/prefs`
//! - 成功后加载**包内业务 UI**（`index.html`），经 URL hash 交接 **API 基址** + 可选 Bearer
//! - 成功连接时将非空 Bearer **覆盖写入**本机系统钥匙串（账户 `tauri_connect_web_api_bearer`）
//!
//! 连接页静态资源见本 crate 的 `assets/connect.html`（由各壳同步进 `dist/`）。
//!
//! 发版与外仓钉版本（git tag + `path`、Tauri 2）：见仓库 **`docs/design/client_contract_versioning.md`** 与本目录 **`README.md`**。

mod allowed_origin;
mod cleartext;
mod commands;
mod handoff;
mod keyring_bearer;
mod keyring_llm;
mod navigation;
mod probe;
mod recent_urls;

pub use allowed_origin::AllowedServeOrigin;
pub use cleartext::enforce_cleartext_connect_policy;
pub use commands::{
    SuggestedServerUrl, clear_recent_connect_urls, connect_remote, disconnect_remote,
    get_connect_bearer, get_llm_secret, get_recent_connect_urls, get_suggested_server_url,
    seed_connect_home, set_connect_bearer, set_llm_secret,
};
pub use handoff::{
    API_BASE_HASH_KEY, BEARER_HASH_KEY, build_handoff_url, build_local_ui_handoff_url,
    local_business_ui_url, normalize_base_url,
};
pub use keyring_bearer::{
    read_connect_bearer, write_connect_bearer_on_connect, write_connect_bearer_unchecked,
};
pub use keyring_llm::{LlmSecretSlot, read_llm_secret, write_llm_secret};
pub use navigation::{
    ShellNavigationDecision, allow_shell_navigation, clear_allowed_if_app_origin_loaded,
    decide_shell_navigation, is_app_origin, is_connect_page_url,
};
pub use probe::{
    SHELL_WEBVIEW_CORS_ORIGINS, SHELL_WEBVIEW_FETCH_ORIGIN, SHELL_WEBVIEW_ORIGIN,
    acao_allows_requested_origin, cors_allows_shell_origin, probe_redirect_host_allowed,
    probe_server, probe_shell_cors,
};
