//! Tauri 命令：连接 / 断开 / 读取建议 URL 与钥匙串 Bearer。

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use url::Url;

use crabmate_client_api::url::api_base_authority;

use crate::allowed_origin::AllowedServeOrigin;
use crate::cleartext::enforce_cleartext_connect_policy;
use crate::connect_home::{SuggestedServerUrl, connect_home_url, remember_connect_home};
use crate::existing_connection::ExistingConnectionProbe;
use crate::handoff::{build_local_ui_handoff_url, local_business_ui_url, normalize_base_url};
use crate::keyring_bearer::{
    clear_connect_bearer_for, read_connect_bearer_for, write_connect_bearer_for,
};
use crate::keyring_llm::{LlmSecretSlot, read_llm_secret, write_llm_secret};
use crate::probe::probe_server;
use crate::recent_urls::{self, RECENT_FILE_NAME};

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
    window: tauri::WebviewWindow,
    url: String,
    bearer: Option<String>,
    manual: Option<bool>,
) -> Result<(), String> {
    let bearer = bearer.unwrap_or_default();
    let api_base = normalize_base_url(&url)?;
    enforce_cleartext_connect_policy(&api_base)?;
    probe_server(&api_base, &bearer).await?;

    // 同一台 serve 已经有窗口在连 → 不建第二条。
    //
    // 对齐 TUI 的 `/lane attach`：同一个 `base_url` 上换一个名字只切过去、不新增 lane
    // （`tui_multi_server.md`）。放在 `probe_server` **之后**：地址都不可达时不该先谈占用。
    // 本窗口留在连接页（不导航），所以错误信息回到连接页的错误位，用户看得见为什么没进去。
    if let Some(existing) = same_serve_window(&app, window.label(), &api_base) {
        focus_existing_window(&app, &existing);
        return Err(String::from(
            "这台 serve 已经在另一个窗口里连着，已切过去；同一个 serve 不需要开两个窗口",
        ));
    }

    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        // 按**发起连接的窗口**记：多窗口各连一台 serve 时，这条白名单不能互相覆盖
        // （进程级单值会让先连的那个窗口丢掉放行依据）。
        allowed.set_from_url(window.label(), &api_base);
    }

    // 空串删除钥匙串条目；非空覆盖写入（Android 无后端时由连接页写 Keystore）。
    // 按发起连接的窗口写各自的槽：别的窗口还连着它们各自的 serve，不能覆盖（见
    // `keyring_bearer` 模块文档）。
    if let Err(e) = write_connect_bearer_for(window.label(), bearer.trim()) {
        eprintln!("[crabmate-connect] keyring write skipped: {e}");
    }

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
pub async fn disconnect_remote(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if let Some(allowed) = app.try_state::<AllowedServeOrigin>() {
        // 只摘本窗口：别的窗口还连着它们各自的 serve。
        allowed.clear_label(window.label());
    }
    if let Err(e) = clear_connect_bearer_for(window.label()) {
        eprintln!("[crabmate-connect] keyring clear on disconnect skipped: {e}");
    }
    let mut home = connect_home_url();
    home.set_query(Some("manual=1"));
    window
        .navigate(home)
        .map_err(|e| format!("无法返回连接页: {e}"))?;
    Ok(())
}

/// 除 `label` 之外，是否已有窗口连着同一台 `serve`。
///
/// 没有 [`ExistingConnectionProbe`] state（浏览器 / Android / 老壳）→ `None`：那些壳
/// 只有一条连接，没有第二条可以撞。
fn same_serve_window(app: &AppHandle, label: &str, api_base: &Url) -> Option<String> {
    let probe = app.try_state::<ExistingConnectionProbe>()?;
    probe.same_serve_window(label, &api_base_authority(api_base.as_str()))
}

/// 把已有那个窗口提到前台（它可能被本窗口盖着，也可能最小化了）。
fn focus_existing_window(app: &AppHandle, label: &str) {
    if let Some(target) = app.get_webview_window(label) {
        let _ = target.unminimize();
        let _ = target.set_focus();
    }
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
///
/// 按调用它的窗口读各自的槽：这个值会被拿去水合该窗口的会话，读错了就会拿着别的
/// `serve` 的凭据连过去。
#[tauri::command]
pub fn get_connect_bearer(window: tauri::WebviewWindow) -> Option<String> {
    match read_connect_bearer_for(window.label()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[crabmate-connect] keyring read skipped: {e}");
            None
        }
    }
}

/// 写入或清除该窗口的连接用 Web API Bearer（空串删除钥匙串条目）。
///
/// 供包内业务 UI 设置页保存/清除；连接成功路径仍走 [`connect_remote`]。
#[tauri::command]
pub fn set_connect_bearer(window: tauri::WebviewWindow, bearer: String) -> Result<(), String> {
    write_connect_bearer_for(window.label(), &bearer)
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
