//! GitHub OAuth Device Flow 前端 API。

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use super::browser::{api_url, apply_api_auth, auth_headers, window};
use super::github_secrets_local::github_token_secure_backend_available;
use crate::i18n::Locale;

#[derive(Debug, Clone, Deserialize)]
pub struct GithubDeviceStartDto {
    pub user_code: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubDeviceStatusDto {
    pub state: String,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub scopes: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// 仅壳在带 `X-CrabMate-GitHub-Token-Delivery: body` 时收到；浏览器路径无此字段。
    #[serde(default)]
    pub access_token: Option<String>,
}

pub async fn post_github_oauth_device_start(
    client_id: &str,
    loc: Locale,
) -> Result<GithubDeviceStartDto, String> {
    let init = RequestInit::new();
    init.set_method("POST");
    apply_api_auth(&init);
    let h = auth_headers();
    let _ = h.set("Content-Type", "application/json");
    init.set_headers(&h);
    let body = serde_json::json!({ "client_id": client_id.trim() }).to_string();
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let req = Request::new_with_str_and_init(&api_url("/github/oauth/device/start"), &init)
        .map_err(|e| format!("request: {e:?}"))?;
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc))?;
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("read body: {e:?}"))?;
    let s = text
        .as_string()
        .ok_or_else(|| crate::i18n::api_err_body_type(loc).to_string())?;
    if !resp.ok() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
            && let Some(err) = v.get("error").and_then(|x| x.as_str())
        {
            return Err(err.to_string());
        }
        return Err(crate::i18n::api_err_request_failed(loc).to_string());
    }
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub async fn fetch_github_oauth_device_status(
    loc: Locale,
) -> Result<GithubDeviceStatusDto, String> {
    let init = RequestInit::new();
    init.set_method("GET");
    apply_api_auth(&init);
    if github_token_secure_backend_available() {
        let h = auth_headers();
        let _ = h.set("X-CrabMate-GitHub-Token-Delivery", "body");
        init.set_headers(&h);
    }
    let req = Request::new_with_str_and_init(&api_url("/github/oauth/device/status"), &init)
        .map_err(|e| format!("request: {e:?}"))?;
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc))?;
    if !resp.ok() {
        return Err(crate::i18n::api_err_request_failed(loc).to_string());
    }
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("read body: {e:?}"))?;
    let s = text
        .as_string()
        .ok_or_else(|| crate::i18n::api_err_body_type(loc).to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub async fn post_github_oauth_device_cancel(loc: Locale) -> Result<(), String> {
    let init = RequestInit::new();
    init.set_method("POST");
    apply_api_auth(&init);
    let req = Request::new_with_str_and_init(&api_url("/github/oauth/device/cancel"), &init)
        .map_err(|e| format!("request: {e:?}"))?;
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc))?;
    if !resp.ok() {
        return Err(crate::i18n::api_err_request_failed(loc).to_string());
    }
    Ok(())
}

/// 清浏览器 HttpOnly Cookie；壳断开时亦应调用（幂等）。
pub async fn post_github_oauth_device_logout(loc: Locale) -> Result<(), String> {
    let init = RequestInit::new();
    init.set_method("POST");
    apply_api_auth(&init);
    let req = Request::new_with_str_and_init(&api_url("/github/oauth/device/logout"), &init)
        .map_err(|e| format!("request: {e:?}"))?;
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc))?;
    if !resp.ok() {
        return Err(crate::i18n::api_err_request_failed(loc).to_string());
    }
    Ok(())
}
