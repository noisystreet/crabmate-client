//! GitHub OAuth Device Flow 前端 API。

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use super::browser::{api_url, auth_headers, prepare_api_auth, window};
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
    /// 壳 body 投递：一次性 refresh_token（GitHub App 才有；浏览器在 HttpOnly Cookie）。
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// access token 有效期（秒）；仅壳 body 投递时收到。
    #[serde(default)]
    #[allow(dead_code)]
    pub expires_in: Option<u64>,
    /// refresh token 有效期（秒）；仅壳 body 投递时收到。
    #[serde(default)]
    #[allow(dead_code)]
    pub refresh_token_expires_in: Option<u64>,
}

async fn fetch_github_response(
    init: &RequestInit,
    path: &str,
    loc: Locale,
) -> Result<Response, String> {
    let req = Request::new_with_str_and_init(&api_url(path), init)
        .map_err(|e| format!("request: {e:?}"))?;
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc).to_string())
}

async fn response_body_text(resp: &Response, loc: Locale) -> Result<String, String> {
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("read body: {e:?}"))?;
    text.as_string()
        .ok_or_else(|| crate::i18n::api_err_body_type(loc).to_string())
}

fn err_from_json_body_or_request_failed(body: &str, loc: Locale) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(err) = v.get("error").and_then(|x| x.as_str())
    {
        return err.to_string();
    }
    crate::i18n::api_err_request_failed(loc).to_string()
}

async fn parse_ok_json<T: for<'de> Deserialize<'de>>(
    resp: Response,
    loc: Locale,
) -> Result<T, String> {
    if !resp.ok() {
        return Err(crate::i18n::api_err_http_status(loc, resp.status(), "").to_string());
    }
    let s = response_body_text(&resp, loc).await?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

async fn post_empty_ok(path: &str, loc: Locale) -> Result<(), String> {
    let init = RequestInit::new();
    init.set_method("POST");
    prepare_api_auth(&init).await;
    let resp = fetch_github_response(&init, path, loc).await?;
    if resp.ok() {
        Ok(())
    } else {
        Err(crate::i18n::api_err_request_failed(loc).to_string())
    }
}

pub async fn post_github_oauth_device_start(
    client_id: &str,
    loc: Locale,
) -> Result<GithubDeviceStartDto, String> {
    let init = RequestInit::new();
    init.set_method("POST");
    prepare_api_auth(&init).await;
    let h = auth_headers();
    let _ = h.set("Content-Type", "application/json");
    init.set_headers(&h);
    let body = serde_json::json!({ "client_id": client_id.trim() }).to_string();
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let resp = fetch_github_response(&init, "/github/oauth/device/start", loc).await?;
    let s = response_body_text(&resp, loc).await?;
    if !resp.ok() {
        return Err(err_from_json_body_or_request_failed(&s, loc));
    }
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub async fn fetch_github_oauth_device_status(
    loc: Locale,
) -> Result<GithubDeviceStatusDto, String> {
    let init = RequestInit::new();
    init.set_method("GET");
    prepare_api_auth(&init).await;
    if github_token_secure_backend_available() {
        let h = auth_headers();
        let _ = h.set("X-CrabMate-GitHub-Token-Delivery", "body");
        init.set_headers(&h);
    }
    let resp = fetch_github_response(&init, "/github/oauth/device/status", loc).await?;
    parse_ok_json(resp, loc).await
}

pub async fn post_github_oauth_device_cancel(loc: Locale) -> Result<(), String> {
    post_empty_ok("/github/oauth/device/cancel", loc).await
}

/// 清浏览器 HttpOnly Cookie；壳断开时亦应调用（幂等）。
pub async fn post_github_oauth_device_logout(loc: Locale) -> Result<(), String> {
    post_empty_ok("/github/oauth/device/logout", loc).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubTokenRefreshDto {
    pub access_token: String,
    /// 轮换后的新 refresh_token（服务端保证有值；缺失时壳沿用旧值）。
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub refresh_token_expires_in: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub scope: Option<String>,
}

/// 用 refresh_token 换新 access token：壳传 JSON body（`refresh_token` + `client_id`）；
/// 浏览器传 `{}`，凭服务端 HttpOnly Cookie `crabmate_github_refresh` 刷新并接收新 Set-Cookie。
pub async fn post_github_oauth_token_refresh(
    body: &str,
    loc: Locale,
) -> Result<GithubTokenRefreshDto, String> {
    let init = RequestInit::new();
    init.set_method("POST");
    prepare_api_auth(&init).await;
    let h = auth_headers();
    let _ = h.set("Content-Type", "application/json");
    init.set_headers(&h);
    init.set_body(&wasm_bindgen::JsValue::from_str(body));
    let resp = fetch_github_response(&init, "/github/oauth/token/refresh", loc).await?;
    let s = response_body_text(&resp, loc).await?;
    if !resp.ok() {
        return Err(err_from_json_body_or_request_failed(&s, loc));
    }
    serde_json::from_str(&s).map_err(|e| e.to_string())
}
