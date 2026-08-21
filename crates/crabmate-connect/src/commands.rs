//! Tauri 命令：连接 / 断开 / 读取建议 URL 与钥匙串 Bearer。

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State, Url};

use crate::allowed_origin::AllowedServeOrigin;
use crate::cleartext::enforce_cleartext_connect_policy;
use crate::handoff::{build_local_ui_handoff_url, local_business_ui_url, normalize_base_url};
use crate::keyring_bearer::{read_connect_bearer, write_connect_bearer_unchecked};
use crate::keyring_llm::{LlmSecretSlot, read_llm_secret, write_llm_secret};
use crate::navigation::is_app_origin;
use crate::probe::probe_server;
use crate::recent_urls::{self, RECENT_FILE_NAME};

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
/// 成功连接后写入系统钥匙串（非空覆盖；空串删除条目）。
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
    persist_recent_after_probe(&app, &api_base);

    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.set_from_url(&api_base);
    }

    // 空串删除钥匙串条目；非空覆盖写入（Android 无后端时由连接页写 Keystore）。
    if let Err(e) = write_connect_bearer_unchecked(bearer.trim()) {
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
///
/// 同时清除本机连接 Bearer 槽（桌面钥匙串）；Android Keystore 由业务 UI 在断开前经桥清除。
#[tauri::command]
pub async fn disconnect_remote(app: AppHandle) -> Result<(), String> {
    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        allowed.clear();
    }
    if let Err(e) = write_connect_bearer_unchecked("") {
        eprintln!("[crabmate-connect] keyring clear on disconnect skipped: {e}");
    }
    let window = main_window(&app)?;
    let mut home = connect_home_url();
    home.set_query(Some("manual=1"));
    window
        .navigate(home)
        .map_err(|e| format!("无法返回连接页: {e}"))?;
    Ok(())
}

fn recent_connect_urls_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    Ok(dir.join(RECENT_FILE_NAME))
}

fn suggested_url_value(app: &AppHandle) -> Option<String> {
    app.try_state::<SuggestedServerUrl>()
        .and_then(|s| s.0.lock().ok().and_then(|g| g.clone()))
}

fn persist_recent_after_probe(app: &AppHandle, api_base: &Url) {
    let Ok(path) = recent_connect_urls_path(app) else {
        return;
    };
    recent_urls::record_success(
        &path,
        api_base.as_str(),
        suggested_url_value(app).as_deref(),
    );
}

/// 连接页预填建议地址（桌面默认本机 `8080`）；移动端通常为 `null`。
#[tauri::command]
pub fn get_suggested_server_url(state: State<'_, SuggestedServerUrl>) -> Option<String> {
    state.0.lock().ok().and_then(|g| g.clone())
}

/// 探测成功后记下的最近服务器地址（应用数据目录；与 `connect.html` 的 localStorage 合并展示）。
#[tauri::command]
pub fn get_recent_connect_urls(app: AppHandle) -> Vec<String> {
    recent_connect_urls_path(&app)
        .map(|p| recent_urls::load_from_path(&p))
        .unwrap_or_default()
}

/// 清空壳侧最近连接列表（连接页「清空」）。
#[tauri::command]
pub fn clear_recent_connect_urls(app: AppHandle) -> Result<(), String> {
    let path = recent_connect_urls_path(&app)?;
    recent_urls::save_to_path(&path, &[])
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

/// 写入或清除连接用 Web API Bearer（空串删除钥匙串条目）。
///
/// 供包内业务 UI 设置页保存/清除；连接成功路径仍走 [`connect_remote`]。
#[tauri::command]
pub fn set_connect_bearer(bearer: String) -> Result<(), String> {
    write_connect_bearer_unchecked(&bearer)
}

/// 系统钥匙串中的密钥槽（`client_llm` / `executor_llm` / `saved_models` / `github`）。
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
