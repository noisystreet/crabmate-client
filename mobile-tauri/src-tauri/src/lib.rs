//! CrabMate Android 远程薄客户端库入口。
//! 不 spawn 本机 Agent sidecar；连接页探测远程 `serve` 后加载其 Web UI。

use tauri::Manager;
use tauri::plugin::Builder as PluginBuilder;

/// 接线 [`crabmate_connect::AllowedServeOrigin`]：拦截跨 Origin 乱跳；回连接页时清空白名单。
fn navigation_guard_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    PluginBuilder::new("crabmate-shell-navigation")
        .on_navigation(|webview, url| crabmate_connect::allow_shell_navigation(webview, url))
        .on_page_load(|webview, payload| {
            crabmate_connect::clear_allowed_if_app_origin_loaded(
                webview.app_handle(),
                payload.url(),
            );
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(navigation_guard_plugin())
        .manage(crabmate_connect::SuggestedServerUrl::default())
        .manage(crabmate_connect::AllowedServeOrigin::default())
        .invoke_handler(tauri::generate_handler![
            crabmate_connect::connect_remote,
            crabmate_connect::disconnect_remote,
            crabmate_connect::get_suggested_server_url,
            crabmate_connect::get_connect_bearer,
        ])
        .setup(|app| {
            if let Some(window) = tauri::Manager::get_webview_window(app, "main")
                && let Ok(url) = window.url()
            {
                crabmate_connect::seed_connect_home(&url);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running CrabMate mobile");
}
