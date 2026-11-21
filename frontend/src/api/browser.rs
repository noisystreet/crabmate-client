//! 浏览器侧共享：`window` / 受保护 API 的鉴权头 / API 基址。
//!
//! Web API Bearer 须同时存在于：
//! - **服务端** `CM_WEB_API_BEARER_TOKEN` / `web_api_bearer_token`（校验用）
//! - **本页**内存：请求头 `Authorization` / `X-API-Key`
//!
//! 持久化：
//! - **官方壳**：仅本机钥匙串 / Android Keystore（见 [`super::web_api_bearer_local`]）；**禁止**明文 `localStorage`
//! - **纯浏览器**：可选 `localStorage` 引导键（弱存储）
//!
//! 与侧栏「模型 API 密钥」(`client_llm`，本机钥匙串/Keystore) 不是同一字段。
//!
//! **API 基址**（路径 A Phase 2）：空 = 同 Origin 相对路径；非空时拼到 `/chat/stream` 等路径前。
//! 可选构建期默认：`option_env!("CRABMATE_API_BASE")`。
//! - **键缺失**：用构建期默认。
//! - **键存在**（含显式空串）：以 localStorage 为准（清空后不再回落到构建期默认）。

use std::cell::RefCell;

use web_sys::{Headers, Window};

use super::connection_scope;
use super::storage_scope;

// 键名只有一处来源（[`super::storage_scope`]）：它们要和「本页属于哪个连接」组合成
// 分域键，散开定义就会出现「一个键分了域、另一个没分」的半迁移状态。
pub(crate) use super::storage_scope::{API_BASE_URL_KEY, WEB_API_BEARER_TOKEN_KEY};

thread_local! {
    /// 壳内 GitHub user token（由 `github_secrets_local` 水合）；浏览器路径保持空，靠 Cookie。
    ///
    /// 留在本模块而不进 [`connection_scope`]：它属于**账号**，不属于某台 `serve` ——
    /// 换一条连接不该把 Device Flow 的结果丢掉。
    static REQUEST_GITHUB_TOKEN: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn window() -> Option<Window> {
    web_sys::window()
}

fn write_local_storage_item(key: &str, value: &str) {
    let Some(w) = window() else {
        return;
    };
    let Ok(Some(storage)) = w.local_storage() else {
        return;
    };
    let t = value.trim();
    if t.is_empty() {
        let _ = storage.remove_item(key);
    } else {
        let _ = storage.set_item(key, t);
    }
}

/// 连接分域读：分域键优先；缺失时回落到旧键**并回填**（一次性迁移）。
///
/// 回填不是可选项：`localStorage` 里已有的基址键与思考模式键是分域之前写下的，
/// 不搬过来等于把用户的基址与偏好重置成默认值。幂等性由
/// [`storage_scope::resolve_scoped_read`] 保证（分域键存在就不看旧键）。
///
/// 不分域（纯浏览器 / Android / Desktop 主窗口）时两个键名相同，一次读取就是全部答案 ——
/// 这条分支让单连接语义与迁移前的字节完全一致。
#[must_use]
pub(crate) fn read_scoped_local_storage_raw(base_key: &str) -> Option<String> {
    let w = window()?;
    let storage = w.local_storage().ok().flatten()?;
    let key = storage_scope::scoped_key(base_key);
    let scoped = storage.get_item(&key).ok().flatten();
    if key == base_key {
        return scoped;
    }
    let legacy = storage.get_item(base_key).ok().flatten();
    let (value, backfill) = storage_scope::resolve_scoped_read(scoped, legacy);
    if backfill && let Some(v) = value.as_deref() {
        // 不要换成 `write_local_storage_item`：它把空串折成删除，而这里的空串是
        // 「显式同 Origin」的**值**，折掉就等于把用户「清空过基址」这件事丢了 ——
        // 下次启动会回落成构建期默认。`resolve_scoped_read` 的单测只覆盖纯函数，
        // 守不住这一行。
        let _ = storage.set_item(&key, v);
    }
    value
}

/// 连接分域写：空值删除该键（与 [`write_local_storage_item`] 同口径）。
pub(crate) fn write_scoped_local_storage_raw(base_key: &str, value: &str) {
    let key = storage_scope::scoped_key(base_key);
    write_local_storage_item(&key, value);
}

/// `None` = 无 storage / 键缺失；`Some` = 键存在（值可为空白，表示显式同 Origin）。
fn read_api_base_local_storage_raw() -> Option<String> {
    read_scoped_local_storage_raw(API_BASE_URL_KEY)
}

fn write_api_base_local_storage(normalized: &str) {
    let Some(w) = window() else {
        return;
    };
    let Ok(Some(storage)) = w.local_storage() else {
        return;
    };
    // 显式空串保留键，避免刷新后回落到 `CRABMATE_API_BASE`。
    let scoped = storage_scope::scoped_key(API_BASE_URL_KEY);
    let _ = storage.set_item(&scoped, normalized);
}

pub(crate) fn read_local_storage_web_api_bearer() -> Option<String> {
    read_scoped_local_storage_raw(WEB_API_BEARER_TOKEN_KEY).and_then(|v| {
        let t = v.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    })
}

pub(crate) fn write_local_storage_web_api_bearer(token: &str) {
    write_scoped_local_storage_raw(WEB_API_BEARER_TOKEN_KEY, token);
}

pub(crate) fn clear_local_storage_web_api_bearer() {
    // 两处都清：分域键（本页真正在用的那个）+ 旧键（分域之前写下的同名键）。
    // 不分域时两者是同一个键，第二次 remove 只是幂等冗余 —— 不值得为省它加一个
    // `connection_key().is_some()` 判断：那个判断读起来像安全门，实际只关乎
    // 「此刻存不存在两个键」。
    write_scoped_local_storage_raw(WEB_API_BEARER_TOKEN_KEY, "");
    write_local_storage_item(WEB_API_BEARER_TOKEN_KEY, "");
}

/// 仅写进程内内存（及已水合标记）；不触碰 localStorage。
pub(crate) fn set_web_api_bearer_token_memory_only(token: &str) {
    // trim 与「置已水合」都在 `connection_scope::set_bearer` 里。
    connection_scope::set_bearer(token);
}

/// 仅读进程内内存（不触发 localStorage hydrate）。
pub(crate) fn web_api_bearer_token_memory_peek() -> String {
    connection_scope::bearer()
}

fn hydrate_web_api_bearer_once() {
    if connection_scope::bearer_settled() {
        return;
    }
    connection_scope::mark_bearer_settled();
    // 官方壳：不从明文 LS 水合（由 `hydrate_web_api_bearer_from_secure_store` 异步补齐）。
    if super::web_api_bearer_local::secure_web_api_bearer_backend_available() {
        return;
    }
    if let Some(t) = read_local_storage_web_api_bearer() {
        if connection_scope::bearer().is_empty() {
            connection_scope::set_bearer(&t);
        }
    }
}

/// 规范化 API 基址：去空白与尾 `/`；非法时返回空（回退同 Origin）。
#[must_use]
pub fn normalize_api_base_url(raw: &str) -> String {
    crabmate_client_api::normalize_api_base(raw)
}

fn build_time_api_base_default() -> String {
    option_env!("CRABMATE_API_BASE")
        .map(normalize_api_base_url)
        .unwrap_or_default()
}

/// 由 localStorage 原始值与构建期默认决定基址（纯函数，便于单测）。
///
/// - `ls_raw == None`：键缺失或不可用 → 构建期默认
/// - `ls_raw == Some(_)`：键存在 → 规范化后的值（空 = 同 Origin）
#[must_use]
pub fn resolve_api_base_url(ls_raw: Option<&str>, build_default: &str) -> String {
    match ls_raw {
        Some(raw) => normalize_api_base_url(raw),
        None => normalize_api_base_url(build_default),
    }
}

fn hydrate_api_base_once() {
    if connection_scope::base_settled() {
        return;
    }
    // 先置「已决定」再读盘：后面不管读到什么都算数（包括「读到空、用构建期默认」），
    // 不该因为一次空的读取就每次访问都重打一次 `localStorage`。
    connection_scope::mark_base_settled();
    let chosen = resolve_api_base_url(
        read_api_base_local_storage_raw().as_deref(),
        &build_time_api_base_default(),
    );
    // 已经把值写进来的路径（URL hash 交接 / 设置页）不能被这里覆盖。
    if connection_scope::api_base().is_empty() {
        connection_scope::set_api_base(&chosen);
    }
}

/// 设置本页访问 CrabMate HTTP API 的 Bearer。
///
/// - **官方壳**：仅内存，并清除明文 `localStorage`（持久化已在连接页 / 设置异步写入钥匙串）
/// - **纯浏览器**：内存 + `localStorage` 弱引导键
///
/// 值须与服务端 `CM_WEB_API_BEARER_TOKEN` **完全一致**。
pub fn set_web_api_bearer_token(token: &str) {
    let t = token.trim().to_string();
    set_web_api_bearer_token_memory_only(&t);
    let secure = super::web_api_bearer_local::secure_web_api_bearer_backend_available();
    if super::web_api_bearer_local::should_persist_web_api_bearer_to_local_storage(secure) {
        write_local_storage_web_api_bearer(&t);
    } else {
        clear_local_storage_web_api_bearer();
    }
}

/// 当前内存中的 Web API Bearer（浏览器路径必要时从 `localStorage` 冷启动注入）。
#[must_use]
pub fn web_api_bearer_token() -> String {
    hydrate_web_api_bearer_once();
    connection_scope::bearer()
}

/// 是否已配置非空 Web API Bearer（本页）。
#[must_use]
pub fn web_api_bearer_token_is_set() -> bool {
    !web_api_bearer_token().trim().is_empty()
}

/// 设置远程 `serve` API 基址（空 = 同 Origin）。写入内存 + `localStorage`（空串仍保留键）。
pub fn set_api_base_url(base: &str) {
    let t = normalize_api_base_url(base);
    // `set_api_base` 同时置「已决定」：调用方不用记得两步都做。
    connection_scope::set_api_base(&t);
    write_api_base_local_storage(&t);
}

/// 当前 API 基址（无尾 `/`）；空表示同 Origin 相对路径。
#[must_use]
pub fn api_base_url() -> String {
    hydrate_api_base_once();
    connection_scope::api_base()
}

/// 从已规范化的 `http(s)://` API 基址取出 host（不含端口）。
#[must_use]
pub fn api_base_host(base: &str) -> Option<&str> {
    let rest = base
        .trim()
        .strip_prefix("https://")
        .or_else(|| base.trim().strip_prefix("http://"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return None;
    }
    if let Some(inner) = authority.strip_prefix('[') {
        return inner.split(']').next().filter(|s| !s.is_empty());
    }
    match authority.rsplit_once(':') {
        Some((h, port)) if !h.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => Some(h),
        _ => Some(authority),
    }
}

/// API 基址是否指向本机 loopback（与桌面壳共享同一文件系统）。
///
/// 空基址（同 Origin）**不算** loopback：桌面壳 Origin 是 `tauri.localhost`，不是 serve 工作区盘。
#[must_use]
pub fn api_base_host_is_loopback(base: &str) -> bool {
    let Some(host) = api_base_host(base) else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// WebKit 常把 CORS / 不可达写成不透明的 `TypeError: Load failed`。
pub fn format_fetch_transport_error(e: &wasm_bindgen::JsValue) -> String {
    let detail = e
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(e, &wasm_bindgen::JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"));
    let base = api_base_url();
    let loc = crate::i18n::load_locale_from_storage();
    let mut msg = crate::i18n::api_err_transport_fetch_failed(loc, &detail);
    if base.is_empty() {
        msg.push_str(crate::i18n::api_err_transport_no_base_hint(loc));
    } else {
        let lower = detail.to_ascii_lowercase();
        if lower.contains("load failed")
            || lower.contains("failed to fetch")
            || lower.contains("networkerror")
        {
            msg.push_str(&crate::i18n::api_err_transport_unreachable_hint(loc, &base));
        }
    }
    msg
}

/// 把 `/…` 路径接到 `base` 上（纯函数：无 IO、不读全局 current scope）。
///
/// 已是 `http(s)://` 绝对 URL 时原样返回；`path` 为空按 `/`；缺前导 `/` 时补上。
#[must_use]
pub fn join_api_path(base: &str, path: &str) -> String {
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.to_string();
    }
    let p = if p.is_empty() {
        "/"
    } else if p.starts_with('/') {
        p
    } else {
        // 容错：缺前导 `/` 时补上
        return join_api_path(base, &format!("/{p}"));
    };
    if base.is_empty() {
        p.to_string()
    } else {
        format!("{base}{p}")
    }
}

/// 将以 `/` 开头的 API 路径接到基址上；基址为空则原样返回 `path`。
///
/// 已是 `http(s)://` 绝对 URL 时原样返回。首次调用会先决定基址（保持原有水合语义）。
#[must_use]
pub fn api_url(path: &str) -> String {
    join_api_path(&api_base_url(), path)
}

/// 壳 Device Flow 成功后写入，供 `auth_headers` 附加 `X-CrabMate-GitHub-Token`。
pub(crate) fn set_request_github_token(token: &str) {
    REQUEST_GITHUB_TOKEN.with(|c| *c.borrow_mut() = token.trim().to_string());
}

pub(crate) fn clear_request_github_token() {
    REQUEST_GITHUB_TOKEN.with(|c| c.borrow_mut().clear());
}

/// 设置 CORS + `credentials: include` + 鉴权头（含壳 GitHub token）。
pub fn apply_api_auth(init: &web_sys::RequestInit) {
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_credentials(web_sys::RequestCredentials::Include);
    init.set_headers(&auth_headers());
}

/// 发起受保护 API 请求前调用：壳内先完成 Bearer + GitHub token 水合，再 [`apply_api_auth`]。
pub async fn prepare_api_auth(init: &web_sys::RequestInit) {
    ensure_web_api_bearer_hydrated_for_request().await;
    ensure_github_token_hydrated_for_request().await;
    apply_api_auth(init);
}

/// 供不经 [`prepare_api_auth`] 组装 headers 的路径（如 SSE）在发请求前调用。
pub async fn ensure_web_api_bearer_hydrated_for_request() {
    super::web_api_bearer_local::ensure_web_api_bearer_hydrated().await;
}

pub async fn ensure_github_token_hydrated_for_request() {
    super::github_secrets_local::ensure_github_token_hydrated().await;
}

pub fn auth_headers() -> Headers {
    // 保持旧语义：零参路径会触发 Bearer 水合（纯浏览器冷启动仍靠它取引导键）。
    auth_headers_with_bearer(&web_api_bearer_token())
}

fn auth_headers_with_bearer(bearer: &str) -> Headers {
    let h = Headers::new().expect("Headers::new");
    if let Some(creds) = crabmate_client_api::web_api_credential_pair(bearer) {
        let _ = h.set(
            crabmate_client_api::HEADER_AUTHORIZATION,
            &creds.authorization,
        );
        let _ = h.set(crabmate_client_api::HEADER_X_API_KEY, &creds.api_key);
    }
    REQUEST_GITHUB_TOKEN.with(|c| {
        if let Some(gh) = crabmate_client_api::github_token_header_value(c.borrow().as_str()) {
            let _ = h.set(crabmate_client_api::HEADER_GITHUB_TOKEN, gh);
        }
    });
    h
}

/// 错误串是否像 **Web API 共享密钥** 校验失败（非模型 `API_KEY`）。
#[must_use]
pub fn is_web_api_credential_error(err: &str) -> bool {
    let low = err.to_ascii_lowercase();
    if low.contains("llm_api_key_required") {
        return false;
    }
    low.contains("web api")
        || low.contains("x-api-key")
        || low.contains("web bearer")
        || low.contains("web_api")
        || (low.contains("缺少或无效") && low.contains("凭证"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_api_path_handles_the_four_shapes() {
        // 空基址 = 同 Origin：原样给出路径。
        assert_eq!(join_api_path("", "/status"), "/status");
        // 有基址就拼上去（基址已规范化，无尾斜杠）。
        assert_eq!(
            join_api_path("http://127.0.0.1:8080", "/status"),
            "http://127.0.0.1:8080/status"
        );
        // 绝对 URL 不拼。
        assert_eq!(
            join_api_path("http://a:1", "https://b:2/x"),
            "https://b:2/x"
        );
        // 空路径按 `/`；缺前导斜杠补上（#558 之前的行为，不能漂）。
        assert_eq!(join_api_path("http://a:1", "  "), "http://a:1/");
        assert_eq!(join_api_path("http://a:1", "status"), "http://a:1/status");
    }

    #[test]
    fn detects_server_web_api_credential_message() {
        assert!(is_web_api_credential_error(
            "请求失败 (401): 缺少或无效的 Web API 凭证（Authorization: Bearer 或 X-API-Key）"
        ));
        assert!(is_web_api_credential_error(
            "Request failed (401): missing or invalid Web API credentials"
        ));
    }

    #[test]
    fn detects_http_401_guide_from_api_err() {
        let zh = crate::i18n::api_err_http_status(
            crate::i18n::Locale::ZhHans,
            401,
            "缺少或无效的 Web API 凭证（Authorization: Bearer 或 X-API-Key）",
        );
        assert!(is_web_api_credential_error(&zh));
        let en = crate::i18n::api_err_http_status(crate::i18n::Locale::En, 401, "");
        assert!(is_web_api_credential_error(&en));
    }

    #[test]
    fn normalize_api_base_strips_slash_and_rejects_relative() {
        assert_eq!(
            normalize_api_base_url("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert_eq!(normalize_api_base_url("  "), "");
        assert_eq!(normalize_api_base_url("/only-path"), "");
        assert_eq!(normalize_api_base_url("ftp://x"), "");
    }

    #[test]
    fn api_base_host_is_loopback_only_for_local_serve() {
        assert!(api_base_host_is_loopback("http://127.0.0.1:8080"));
        assert!(api_base_host_is_loopback("http://localhost:8080"));
        assert!(api_base_host_is_loopback("http://[::1]:8080"));
        assert!(!api_base_host_is_loopback(""));
        assert!(!api_base_host_is_loopback("https://api.example.com"));
        assert!(!api_base_host_is_loopback("http://192.168.1.10:8080"));
        assert_eq!(
            api_base_host("https://api.example.com/v1"),
            Some("api.example.com")
        );
        assert_eq!(api_base_host("http://[::1]:8080"), Some("::1"));
    }

    #[test]
    fn resolve_api_base_prefers_explicit_empty_over_build_default() {
        assert_eq!(
            resolve_api_base_url(None, "http://build.example:8080"),
            "http://build.example:8080"
        );
        assert_eq!(
            resolve_api_base_url(Some(""), "http://build.example:8080"),
            ""
        );
        assert_eq!(
            resolve_api_base_url(
                Some(" http://127.0.0.1:8080/ "),
                "http://build.example:8080"
            ),
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn api_url_empty_base_keeps_relative() {
        // 测试线程未 hydrate 浏览器 storage；直接测拼接逻辑
        assert_eq!(
            {
                let path = "/chat/stream";
                let base = "";
                if base.is_empty() {
                    path.to_string()
                } else {
                    format!("{base}{path}")
                }
            },
            "/chat/stream"
        );
        assert_eq!(
            {
                let path = "/chat/stream";
                let base = "http://127.0.0.1:8080";
                format!("{base}{path}")
            },
            "http://127.0.0.1:8080/chat/stream"
        );
    }
}
