#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_lifecycle;
mod desktop_main_window;
mod os_theme;

use tauri::{Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;
use url::Url;

pub(crate) fn desktop_window_state_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

/// 连接页默认建议地址（本机常见 `serve` 端口；用户可改）。
const DEFAULT_SUGGESTED_SERVE_URL: &str = "http://127.0.0.1:8080/";

/// `CM_E2E_FIXTURES=1` 时默认隐藏 main，避免 Wayland 上在 xvfb 外仍弹窗。
///
/// WebKitGTK 在 `visible(false)` 时往往不跑页面 JS，Victauri bridge/`eval_js` 会失败
///（`bridge not responding`）。在 xvfb 内或显式 `CM_E2E_SHOW_WINDOWS=1` 时改为显示
///（窗口只在虚拟屏上，不打扰本机桌面）。
pub(crate) fn e2e_hide_app_windows() -> bool {
    if std::env::var("CM_E2E_SHOW_WINDOWS").is_ok_and(|v| !v.is_empty() && v != "0") {
        return false;
    }
    if std::env::var("VICTAURI_INSIDE_XVFB").is_ok_and(|v| !v.is_empty() && v != "0") {
        return false;
    }
    std::env::var("CM_E2E_FIXTURES").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// E2E 或显式跳过时：不展示连接页，直接打开 **`CM_DESKTOP_SERVE_URL`**（须已有 `serve`）。
pub(crate) fn skip_connect_page() -> bool {
    e2e_hide_app_windows()
        || std::env::var("CM_DESKTOP_SKIP_CONNECT").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "0")
}

/// 连接页预填：`CM_DESKTOP_SUGGESTED_URL` → 默认本机 8080。
fn suggested_serve_url() -> String {
    env_nonempty("CM_DESKTOP_SUGGESTED_URL")
        .unwrap_or_else(|| DEFAULT_SUGGESTED_SERVE_URL.to_string())
}

/// 跳过连接页时必填的已运行 `serve` URL。
fn direct_serve_url() -> Result<String, String> {
    env_nonempty("CM_DESKTOP_SERVE_URL").ok_or_else(|| {
        "跳过连接页时须设置 CM_DESKTOP_SERVE_URL（指向已运行的 crabmate serve，例如 http://127.0.0.1:8080/）。桌面壳不再拉起后端。"
            .to_string()
    })
}

#[tauri::command]
async fn save_text_file_via_dialog(
    app: tauri::AppHandle,
    default_name: String,
    content: String,
) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<FilePath>>();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });

    let picked = rx
        .await
        .map_err(|e| format!("save dialog channel failed: {e}"))?;
    let Some(file_path) = picked else {
        return Ok(false);
    };

    let path = match file_path {
        FilePath::Path(p) => p,
        FilePath::Url(url) => url
            .to_file_path()
            .map_err(|_| "save dialog returned a non-file URL".to_string())?,
    };
    std::fs::write(&path, content).map_err(|e| format!("write file failed: {e}"))?;
    Ok(true)
}

#[tauri::command]
async fn pick_workspace_folder_via_dialog(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<FilePath>>();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });

    let picked = rx
        .await
        .map_err(|e| format!("pick folder dialog channel failed: {e}"))?;

    Ok(match picked {
        None => None,
        Some(FilePath::Path(p)) => Some(p.to_string_lossy().into_owned()),
        Some(FilePath::Url(url)) => Some(
            url.to_file_path()
                .map_err(|_| "pick folder returned a non-file URL".to_string())?
                .to_string_lossy()
                .into_owned(),
        ),
    })
}

#[tauri::command]
fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|e| format!("invalid url: {e}"))?;
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// 托盘「退出」与前端显式退出共用。
pub(crate) fn request_desktop_quit(app: &tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn quit_desktop_app(app: tauri::AppHandle) {
    request_desktop_quit(&app);
}

/// 与建窗逻辑一致：Linux 读 portal / gsettings / GTK / KDE；其它平台 `None`（前端用 matchMedia）。
#[tauri::command]
fn os_prefers_dark_theme() -> Option<bool> {
    os_theme::os_prefers_dark_theme()
}

fn main_webview_window(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".into())
}

#[tauri::command]
fn set_main_window_decorations(app: tauri::AppHandle, decorations: bool) -> Result<(), String> {
    main_webview_window(&app)?
        .set_decorations(decorations)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn main_window_minimize(app: tauri::AppHandle) -> Result<(), String> {
    let window = main_webview_window(&app)?;
    if desktop_lifecycle::tray_available(&app) {
        window.hide().map_err(|e| e.to_string())
    } else {
        window.minimize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn main_window_toggle_maximize(app: tauri::AppHandle) -> Result<(), String> {
    let win = main_webview_window(&app)?;
    if win.is_maximized().map_err(|e| e.to_string())? {
        win.unmaximize().map_err(|e| e.to_string())
    } else {
        win.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn main_window_close(app: tauri::AppHandle) -> Result<(), String> {
    main_webview_window(&app)?
        .close()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn confirm_delete_session_via_dialog(
    app: tauri::AppHandle,
    message: String,
) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    app.dialog()
        .message(message)
        .title("确认删除会话")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "删除".to_string(),
            "取消".to_string(),
        ))
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    rx.await
        .map_err(|e| format!("confirm dialog channel failed: {e}"))
}

/// 单窗启动：直接建 `main`（连接页或 DirectUi），先藏后定位再显示，无独立闪屏。
fn open_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    if skip_connect_page() {
        let url = direct_serve_url()?;
        desktop_main_window::create_main_window_from_url(app, url)
    } else {
        desktop_main_window::create_main_window_connect_page(app, Some(suggested_serve_url()))
    }
}

fn main() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop_lifecycle::focus_existing_instance(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // 几何恢复由 `create_main_window_*` 在 `show()` 前后同步处理，
                // 避免插件异步 restore 造成默认尺寸闪一下。
                .skip_initial_state("main")
                .with_state_flags(desktop_window_state_flags())
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(feature = "victauri")]
    {
        builder = builder.plugin(
            victauri_plugin::VictauriBuilder::new()
                .auth_disabled()
                .build()
                .unwrap(),
        );
    }
    builder
        .invoke_handler(tauri::generate_handler![
            save_text_file_via_dialog,
            pick_workspace_folder_via_dialog,
            confirm_delete_session_via_dialog,
            open_external_url,
            set_main_window_decorations,
            main_window_minimize,
            main_window_toggle_maximize,
            main_window_close,
            quit_desktop_app,
            os_prefers_dark_theme,
            crabmate_connect::connect_remote,
            crabmate_connect::disconnect_remote,
            crabmate_connect::get_suggested_server_url,
            crabmate_connect::get_connect_bearer,
            crabmate_connect::set_connect_bearer,
            crabmate_connect::get_llm_secret,
            crabmate_connect::set_llm_secret,
        ])
        .setup(move |app| {
            desktop_lifecycle::setup_tray(app);
            os_theme::spawn_linux_color_scheme_watcher(app.handle().clone());
            app.manage(crabmate_connect::SuggestedServerUrl::default());
            app.manage(crabmate_connect::AllowedServeOrigin::default());

            if let Err(e) = open_main_window(app.handle()) {
                eprintln!("[crabmate-desktop] startup failed: {e}");
                if !e2e_hide_app_windows() {
                    app.dialog()
                        .message(format!(
                            "{e}\n\n请先在本机或远程启动 crabmate serve，再在连接页填写地址。"
                        ))
                        .title("CrabMate Desktop")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                }
                return Err(e.into());
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(|_app_handle, _event| {
            // 壳不再持有后端子进程；Exit 无需额外回收。
        });
}

#[cfg(test)]
mod tests {
    use super::desktop_window_state_flags;
    use tauri_plugin_window_state::StateFlags;

    #[test]
    fn window_state_flags_include_geometry() {
        let flags = desktop_window_state_flags();
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::POSITION));
        assert!(flags.contains(StateFlags::MAXIMIZED));
    }
}
