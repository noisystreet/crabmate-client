//! Tauri 命令：连接 / 断开 / 读取建议 URL 与钥匙串 Bearer。

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use url::Url;

use crate::allowed_origin::AllowedServeOrigin;
use crate::cleartext::enforce_cleartext_connect_policy;
use crate::connect_home::{SuggestedServerUrl, connect_home_url, remember_connect_home};
use crate::handoff::{build_local_ui_handoff_url, local_business_ui_url, normalize_base_url};
use crate::keyring_bearer::{read_connect_bearer, write_connect_bearer_unchecked};
use crate::keyring_llm::{LlmSecretSlot, read_llm_secret, write_llm_secret};
use crate::probe::probe_server;
use crate::recent_urls::{self, RECENT_FILE_NAME};

fn main_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "主窗口未就绪".to_string())
}

/// 探测 `/health` + `/user-data/prefs` 后加载**包内业务 UI**，经 hash 交接 API 基址与 Bearer。
///
/// 成功连接后写入系统钥匙串（非空覆盖；空串删除条目）。
/// `manual` 标记本次是否为用户手动提交（连接页自动登录为 `false`）：
/// 仅**自动登录**到建议服务器地址时不落最近连接；手动填写（含恰好等于建议地址）也记录。
/// 最近连接**在导航成功后才落盘**：若 navigate 失败，页面会回滚 localStorage，
/// 壳侧文件也不再记录，避免下次启动把一条“没连上过”的记录合并回来。
#[tauri::command]
pub async fn connect_remote(
    app: AppHandle,
    url: String,
    bearer: Option<String>,
    manual: Option<bool>,
) -> Result<(), String> {
    let bearer = bearer.unwrap_or_default();
    let api_base = normalize_base_url(&url)?;
    enforce_cleartext_connect_policy(&api_base)?;
    probe_server(&api_base, &bearer).await?;

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

    persist_recent_on_success(&app, &api_base, manual.unwrap_or(false));
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

fn persist_recent_on_success(app: &AppHandle, api_base: &Url, manual: bool) {
    let Ok(path) = recent_connect_urls_path(app) else {
        return;
    };
    // 自动登录到建议地址不落历史（避免每次重启把本机默认端口顶到最前）；
    // 手动提交则视为用户主动连接，即使等于建议地址也记录。
    let suggested = if manual {
        None
    } else {
        suggested_url_value(app)
    };
    recent_urls::record_success(&path, api_base.as_str(), suggested.as_deref());
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

/// 从壳侧最近连接列表移除单条（连接页每条「删除」）。
#[tauri::command]
pub fn remove_recent_connect_url(app: AppHandle, url: String) -> Result<(), String> {
    let path = recent_connect_urls_path(&app)?;
    let next = recent_urls::remove_recent(recent_urls::load_from_path(&path), &url);
    recent_urls::save_to_path(&path, &next)
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
