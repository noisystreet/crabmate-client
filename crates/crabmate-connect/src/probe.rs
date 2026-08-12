//! 探测远程 `GET /health`，再探受保护的 `GET /user-data/prefs`（校验 Bearer），
//! 最后用壳 WebView Origin 探测 CORS（包内 UI 跨 Origin 必需）。

use std::time::Duration;

use crabmate_client_api::auth::{HEADER_X_API_KEY, web_api_credential_pair};
use url::Url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_MAX_REDIRECTS: usize = 3;

/// Desktop / Android 包内 UI 的资产 URL Origin（部分路径/文档仍写此值）。
pub const SHELL_WEBVIEW_ORIGIN: &str = "http://tauri.localhost";

/// Linux WebKitGTK 对包内页发起跨 Origin `fetch` 时实际带的 `Origin`（与资产 URL 的 http(s) 形式不同）。
pub const SHELL_WEBVIEW_FETCH_ORIGIN: &str = "tauri://localhost";

/// 壳 UI 可能出现的 Origin（CORS 白名单须覆盖 **fetch** 用的 [`SHELL_WEBVIEW_FETCH_ORIGIN`]）。
pub const SHELL_WEBVIEW_CORS_ORIGINS: &[&str] = &[
    SHELL_WEBVIEW_FETCH_ORIGIN,
    SHELL_WEBVIEW_ORIGIN,
    "https://tauri.localhost",
];

fn attach_bearer(mut req: reqwest::RequestBuilder, bearer: &str) -> reqwest::RequestBuilder {
    if let Some(creds) = web_api_credential_pair(bearer) {
        req = req
            .header(reqwest::header::AUTHORIZATION, creds.authorization.as_str())
            .header(HEADER_X_API_KEY, creds.api_key.as_str());
    }
    req
}

/// 是否允许探测跟随该重定向：必须与起始 URL **同 host**（防开放重定向骗过探测后仍导航到原 host）。
#[must_use]
pub fn probe_redirect_host_allowed(start: &Url, next: &Url) -> bool {
    match (start.host_str(), next.host_str()) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

fn same_host_redirect_policy(start: Url) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= PROBE_MAX_REDIRECTS {
            return attempt.error("探测重定向次数过多");
        }
        let next = attempt.url().clone();
        if probe_redirect_host_allowed(&start, &next) {
            attempt.follow()
        } else {
            let from = start.host_str().unwrap_or("?").to_string();
            let to = next.host_str().unwrap_or("?").to_string();
            attempt.error(format!("探测拒绝跨 host 重定向（{from} → {to}）"))
        }
    })
}

fn build_client(base: &Url) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(same_host_redirect_policy(base.clone()))
        // 避免 http_proxy 把 127.0.0.1 / 局域网探测拐走。
        .no_proxy()
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))
}

async fn probe_get(
    client: &reqwest::Client,
    url: Url,
    bearer: &str,
    label: &str,
) -> Result<reqwest::Response, String> {
    let req = attach_bearer(client.get(url), bearer);
    req.send().await.map_err(|e| {
        format!("无法连接服务器（网络/防火墙/地址错误，或明文 HTTP 被系统拦截）[{label}]: {e}")
    })
}

fn map_auth_or_status(status: reqwest::StatusCode, label: &str) -> Result<(), String> {
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(
            "服务器拒绝访问（401/403）。请填写与 CM_WEB_API_BEARER_TOKEN 一致的 Web API 共享密钥（不是模型 API_KEY）"
                .into(),
        );
    }
    Err(format!(
        "服务器 {label} 返回 HTTP {}，请确认 crabmate serve 已启动且地址正确",
        status.as_u16()
    ))
}

/// 对「请求带 `Origin: requested`」的响应，`Access-Control-Allow-Origin` 是否放行该 Origin。
#[must_use]
pub fn acao_allows_requested_origin(acao: &str, requested: &str) -> bool {
    let acao = acao.trim();
    let requested = requested.trim();
    acao == "*" || (!requested.is_empty() && acao.eq_ignore_ascii_case(requested))
}

/// ACAO 是否落在壳已知 Origin 列表（或 `*`）。用于文档/校验白名单回显形态。
#[must_use]
pub fn cors_allows_shell_origin(acao: &str) -> bool {
    let acao = acao.trim();
    acao == "*"
        || SHELL_WEBVIEW_CORS_ORIGINS
            .iter()
            .any(|o| acao.eq_ignore_ascii_case(o))
}

fn shell_cors_env_hint() -> String {
    format!(
        "须放行 {SHELL_WEBVIEW_FETCH_ORIGIN} 与 {SHELL_WEBVIEW_ORIGIN}（新版 Server 默认已含；若显式清空了 CORS，请 unset CM_WEB_CORS_ALLOWED_ORIGINS，或设为 '{SHELL_WEBVIEW_FETCH_ORIGIN},{SHELL_WEBVIEW_ORIGIN}'）"
    )
}

/// 桌面 Linux（`tauri://localhost`）与 Android http 资产（`http://tauri.localhost`）均须在 CORS 白名单中。
const REQUIRED_SHELL_FETCH_ORIGINS: &[&str] = &[SHELL_WEBVIEW_FETCH_ORIGIN, SHELL_WEBVIEW_ORIGIN];

/// 包内 UI 跨 Origin 调 API：逐一探测必需 Origin（两端壳都覆盖）。
pub async fn probe_shell_cors(base: &Url) -> Result<(), String> {
    let client = build_client(base)?;
    let health = base
        .join("health")
        .map_err(|e| format!("无法构造 /health: {e}"))?;

    let mut failed: Vec<String> = Vec::new();
    for origin in REQUIRED_SHELL_FETCH_ORIGINS {
        let resp = client
            .get(health.clone())
            .header(reqwest::header::ORIGIN, *origin)
            .send()
            .await
            .map_err(|e| format!("CORS 探测失败（/health, Origin {origin}）: {e}"))?;
        let acao = resp
            .headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim();
        if !acao_allows_requested_origin(acao, origin) {
            failed.push(format!("{origin} → ACAO={acao:?}"));
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    Err(format!(
        "壳 WebView 跨 Origin 调用 API 需要 CORS。\nLinux 实际 Origin 为 {SHELL_WEBVIEW_FETCH_ORIGIN}；Android http 资产为 {SHELL_WEBVIEW_ORIGIN}。\n请重启 serve 并设置：\n  {}\n未放行：{}",
        shell_cors_env_hint(),
        failed.join("; ")
    ))
}

async fn maybe_log_health_degraded(health_resp: reqwest::Response) {
    if let Ok(body) = health_resp.text().await
        && let Some(note) = health_degraded_note(&body)
    {
        // 不阻断连接：缺可选 CLI 等仍可进入 UI。
        eprintln!("[crabmate-connect] /health degraded (可选依赖缺失，可忽略): {note}");
    }
}

async fn probe_health(client: &reqwest::Client, base: &Url, bearer: &str) -> Result<(), String> {
    let health = base
        .join("health")
        .map_err(|e| format!("无法构造 /health: {e}"))?;
    let health_resp = probe_get(client, health, bearer, "/health").await?;
    map_auth_or_status(health_resp.status(), "/health")?;
    maybe_log_health_degraded(health_resp).await;
    Ok(())
}

async fn probe_prefs(client: &reqwest::Client, base: &Url, bearer: &str) -> Result<(), String> {
    let prefs = base
        .join("user-data/prefs")
        .map_err(|e| format!("无法构造 /user-data/prefs: {e}"))?;
    let prefs_resp = probe_get(client, prefs, bearer, "/user-data/prefs").await?;
    map_auth_or_status(prefs_resp.status(), "/user-data/prefs")
}

/// 先探 `/health`（可达性；`degraded` 时给出可读失败检查摘要但仍允许继续），
/// 再探 `/user-data/prefs`（鉴权；无密钥中间件时亦应 2xx），
/// 最后探壳 Origin 的 CORS（包内 UI 必需）。
pub async fn probe_server(base: &Url, bearer: &str) -> Result<(), String> {
    let client = build_client(base)?;
    probe_health(&client, base, bearer).await?;
    probe_prefs(&client, base, bearer).await?;
    probe_shell_cors(base).await?;
    Ok(())
}

/// 解析 `/health` JSON：`status=degraded` 时返回失败检查名摘要（不含密钥等敏感值）。
fn health_degraded_note(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("status").and_then(|s| s.as_str()) != Some("degraded") {
        return None;
    }
    let checks = v.get("checks")?.as_object()?;
    let mut failed = Vec::new();
    for (name, check) in checks {
        let ok = check.get("ok").and_then(|x| x.as_bool()).unwrap_or(true);
        if !ok {
            let detail = check
                .get("detail")
                .and_then(|d| d.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match detail {
                Some(d) => failed.push(format!("{name}: {d}")),
                None => failed.push(name.clone()),
            }
        }
    }
    if failed.is_empty() {
        Some("status=degraded".into())
    } else {
        Some(failed.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SHELL_WEBVIEW_FETCH_ORIGIN, SHELL_WEBVIEW_ORIGIN, acao_allows_requested_origin,
        cors_allows_shell_origin, health_degraded_note, probe_redirect_host_allowed,
    };
    use url::Url;

    #[test]
    fn acao_must_echo_requested_origin_or_star() {
        assert!(acao_allows_requested_origin(
            "*",
            SHELL_WEBVIEW_FETCH_ORIGIN
        ));
        assert!(acao_allows_requested_origin(
            SHELL_WEBVIEW_FETCH_ORIGIN,
            SHELL_WEBVIEW_FETCH_ORIGIN
        ));
        assert!(acao_allows_requested_origin(
            SHELL_WEBVIEW_ORIGIN,
            SHELL_WEBVIEW_ORIGIN
        ));
        // 只回显另一壳 Origin ≠ 放行本次请求
        assert!(!acao_allows_requested_origin(
            SHELL_WEBVIEW_ORIGIN,
            SHELL_WEBVIEW_FETCH_ORIGIN
        ));
        assert!(!acao_allows_requested_origin(
            "",
            SHELL_WEBVIEW_FETCH_ORIGIN
        ));
    }

    #[test]
    fn shell_cors_accepts_tauri_scheme_and_http_forms() {
        assert!(cors_allows_shell_origin(SHELL_WEBVIEW_FETCH_ORIGIN));
        assert!(cors_allows_shell_origin(SHELL_WEBVIEW_ORIGIN));
        assert!(cors_allows_shell_origin("https://tauri.localhost"));
        assert!(cors_allows_shell_origin(" * "));
        assert!(!cors_allows_shell_origin("http://127.0.0.1:8080"));
        assert!(!cors_allows_shell_origin(""));
    }

    #[test]
    fn degraded_note_lists_failed_checks() {
        let body = r#"{"status":"degraded","checks":{"dep_bc":{"ok":false,"detail":"未安装"},"api_key":{"ok":true}}}"#;
        let note = health_degraded_note(body).expect("note");
        assert!(note.contains("dep_bc"));
        assert!(note.contains("未安装"));
        assert!(!note.contains("api_key"));
    }

    #[test]
    fn ok_status_yields_no_note() {
        let body = r#"{"status":"ok","checks":{}}"#;
        assert!(health_degraded_note(body).is_none());
    }

    #[test]
    fn redirect_same_host_ok_cross_host_denied() {
        let start = Url::parse("http://192.168.1.10:8080/health").unwrap();
        let same = Url::parse("http://192.168.1.10:8080/user-data/prefs").unwrap();
        let https_upgrade = Url::parse("https://192.168.1.10/health").unwrap();
        let evil = Url::parse("http://evil.example/health").unwrap();
        assert!(probe_redirect_host_allowed(&start, &same));
        assert!(probe_redirect_host_allowed(&start, &https_upgrade));
        assert!(!probe_redirect_host_allowed(&start, &evil));
    }
}
