//! Tauri 命令：连接 / 断开 / 读取建议 URL 与钥匙串 Bearer。

use std::sync::Mutex;

use tauri::{AppHandle, Manager, State, Url};

use crate::allowed_origin::AllowedServeOrigin;
use crate::cleartext::enforce_cleartext_connect_policy;
use crate::handoff::{build_local_ui_handoff_url, local_business_ui_url, normalize_base_url};
use crate::keyring_bearer::{read_connect_bearer, write_connect_bearer_on_connect};
use crate::keyring_llm::{LlmSecretSlot, read_llm_secret, write_llm_secret};
use crate::navigation::is_app_origin;
use crate::probe::probe_server;

/// Android 默认资产源（`useHttpsScheme=false`）；桌面 Tauri 2 亦常用此 origin。
const DEFAULT_CONNECT_HOME: &str = "http://tauri.localhost/connect.html";

static CONNECT_HOME: Mutex<Option<Url>> = Mutex::new(None);

/// 连接页预填的建议服务器 URL；桌面默认本机 `8080`，移动端保持 `None` 直至用户填写。
#[derive(Debug, Default)]
pub struct SuggestedServerUrl(pub Mutex<Option<String>>);

impl SuggestedServerUrl {
    pub fn new(url: Option<String>) -> Self {
        Self(Mutex::new(url))
    }

    pub fn set(&self, url: Option<String>) {
        if let Ok(mut g) = self.0.lock() {
            *g = url;
        }
    }
}

fn remember_connect_home(url: &Url) {
    if !is_app_origin(url) {
        return;
    }
    let mut home = url.clone();
    home.set_fragment(None);
    // 保留路径（桌面 / 移动均为 /connect.html）；disconnect 时再设 manual query。
    if let Ok(mut g) = CONNECT_HOME.lock() {
        *g = Some(home);
    }
}

fn connect_home_url() -> Url {
    CONNECT_HOME
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| Url::parse(DEFAULT_CONNECT_HOME).expect("DEFAULT_CONNECT_HOME"))
}

/// 在打开连接页后调用，确保断开时能回到正确的 App 资产 URL（`/connect.html`）。
pub fn seed_connect_home(url: &Url) {
    remember_connect_home(url);
}

fn main_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "主窗口未就绪".to_string())
}

/// 探测 `/health` + `/user-data/prefs` 后加载**包内业务 UI**，经 hash 交接 API 基址与 Bearer。
///
/// 成功连接且 Bearer 非空时**覆盖写入**系统钥匙串。
#[tauri::command]
pub async fn connect_remote(
    app: AppHandle,
    url: String,
    bearer: Option<String>,
) -> Result<(), String> {
    let bearer = bearer.unwrap_or_default();
    let api_base = normalize_base_url(&url)?;
    enforce_cleartext_connect_policy(&api_base)?;
    probe_server(&api_base, &bearer).await?;

    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.set_from_url(&api_base);
    }

    // 钥匙串失败不阻断连接（Android 无后端时由连接页写 Keystore 加密 prefs）。
    if let Err(e) = write_connect_bearer_on_connect(&bearer) {
        eprintln!("[crabmate-connect] keyring write skipped: {e}");
    }

    let window = main_window(&app)?;
    if let Ok(current) = window.url() {
        remember_connect_home(&current);
    }

    let ui = local_business_ui_url(&connect_home_url());
    let target = build_local_ui_handoff_url(ui, &api_base, &bearer);
    window
        .navigate(target)
        .map_err(|e| format!("无法打开本地界面: {e}"))?;
    Ok(())
}

/// 导航回 App 内连接页；带 `manual=1` 避免立刻自动重连。
#[tauri::command]
pub async fn disconnect_remote(app: AppHandle) -> Result<(), String> {
    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.clear();
    }
    let window = main_window(&app)?;
    let mut home = connect_home_url();
    home.set_query(Some("manual=1"));
    window
        .navigate(home)
        .map_err(|e| format!("无法返回连接页: {e}"))?;
    Ok(())
}

/// 连接页预填建议地址（桌面默认本机 `8080`）；移动端通常为 `null`。
#[tauri::command]
pub fn get_suggested_server_url(state: State<'_, SuggestedServerUrl>) -> Option<String> {
    state.0.lock().ok().and_then(|g| g.clone())
}

/// 系统钥匙串中的连接 Bearer（若有）。
#[tauri::command]
pub fn get_connect_bearer() -> Option<String> {
    match read_connect_bearer() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[crabmate-connect] keyring read skipped: {e}");
            None
        }
    }
}

/// 系统钥匙串中的模型 API 密钥槽（`client_llm` / `executor_llm` / `saved_models`）。
#[tauri::command]
pub fn get_llm_secret(slot: String) -> Option<String> {
    let Some(s) = LlmSecretSlot::parse(&slot) else {
        eprintln!("[crabmate-connect] unknown llm secret slot: {slot}");
        return None;
    };
    match read_llm_secret(s) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[crabmate-connect] llm keyring read skipped: {e}");
            None
        }
    }
}

/// 写入或清除模型 API 密钥槽（空串清除）。Android 无钥匙串后端时由前端走 Keystore 桥。
#[tauri::command]
pub fn set_llm_secret(slot: String, value: String) -> Result<(), String> {
    let s =
        LlmSecretSlot::parse(&slot).ok_or_else(|| format!("unknown llm secret slot: {slot}"))?;
    write_llm_secret(s, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::is_app_origin;

    #[test]
    fn app_origin_detects_tauri_localhost() {
        let u = Url::parse("http://tauri.localhost/connect.html").unwrap();
        assert!(is_app_origin(&u));
        let remote = Url::parse("http://192.168.1.10:8080/").unwrap();
        assert!(!is_app_origin(&remote));
    }
}
