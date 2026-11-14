#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_connections;
mod desktop_ctl;
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

const MAX_SAVE_BYTES: usize = 16 * 1024 * 1024;

fn save_dialog_basename(name: &str) -> String {
    let Some(last) = name.rsplit(['/', '\\']).find(|s| !s.is_empty()) else {
        return String::new();
    };
    last.chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .take(80)
        .collect()
}

#[tauri::command]
async fn save_bytes_file_via_dialog(
    app: tauri::AppHandle,
    default_name: String,
    content_base64: String,
) -> Result<bool, String> {
    let name = save_dialog_basename(&default_name);
    let name = if name.is_empty() {
        "image.png".to_string()
    } else {
        name
    };
    let bytes = decode_save_payload(&content_base64)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<FilePath>>();
    app.dialog()
        .file()
        .set_file_name(&name)
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
    std::fs::write(&path, bytes).map_err(|e| format!("write file failed: {e}"))?;
    Ok(true)
}

fn decode_save_payload(content_base64: &str) -> Result<Vec<u8>, String> {
    if content_base64.len() > MAX_SAVE_BYTES.saturating_mul(2) {
        return Err("image too large".to_string());
    }
    let bytes = decode_save_base64(content_base64)?;
    if bytes.len() > MAX_SAVE_BYTES {
        return Err("image too large".to_string());
    }
    Ok(bytes)
}

fn decode_save_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("invalid base64: {e}"))
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
    app.get_webview_window(desktop_main_window::MAIN_WINDOW_LABEL)
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
                .skip_initial_state(desktop_main_window::MAIN_WINDOW_LABEL)
                .with_state_flags(desktop_window_state_flags())
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
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
            save_bytes_file_via_dialog,
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
            crabmate_connect::get_recent_connect_urls,
            crabmate_connect::clear_recent_connect_urls,
            crabmate_connect::remove_recent_connect_url,
            crabmate_connect::get_connect_bearer,
            crabmate_connect::set_connect_bearer,
            crabmate_connect::get_llm_secret,
            crabmate_connect::set_llm_secret,
            desktop_connections::list_connections,
            desktop_connections::connections_tally,
            desktop_connections::open_connection,
            desktop_connections::open_connect_window,
            desktop_connections::register_connection,
            desktop_connections::report_connection_status,
            desktop_connections::report_connection_unread,
            desktop_connections::report_connection_metrics,
            desktop_connections::rename_connection,
            desktop_connections::focus_connection,
            desktop_connections::close_connection,
            // 控制面查询与重启：左栏「可连实例」分组的数据源（`desktop_ctl` 头注释
            // 说明为什么这份 socket IO 不在共享层，以及为什么只做 list / restart）。
            desktop_ctl::ctl_list_instances,
            desktop_ctl::ctl_restart_instance,
        ])
        .on_window_event(|window, event| {
            // 窗口销毁 = 那条连接没了。只摘自己那条，并广播给剩下的窗口。
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let app = window.app_handle();
                if let Some(state) = app.try_state::<desktop_connections::ConnectionRegistry>() {
                    state.remove(window.label());
                    desktop_connections::broadcast(app);
                }
            }
        })
        .setup(move |app| {
            desktop_lifecycle::setup_tray(app);
            os_theme::spawn_linux_color_scheme_watcher(app.handle().clone());
            app.manage(crabmate_connect::SuggestedServerUrl::default());
            app.manage(crabmate_connect::AllowedServeOrigin::default());
            app.manage(desktop_connections::ConnectionRegistry::default());

            // 行尾读数的秒级推进（左栏「连接」区的在途耗时）。必须在 `manage` 之后：
            // 线程第一次醒来时注册表得已经就位，否则它会当成「应用正在收尾」直接退。
            desktop_connections::spawn_busy_ticker(app.handle().clone());
            // `connect_remote` 住在 crabmate-connect、注册表住在这里，依赖方向不允许反向引用，
            // 所以用闭包把「同一台 serve 是否已有窗口」这个查询注入过去。
            // 闭包持 `AppHandle` 再取 state：`ConnectionRegistry` 不是 `Clone`，
            // 而 `State<'_, T>` 借用 `App`，捕不进 `'static` 闭包。
            let probe_handle = app.handle().clone();
            app.manage(crabmate_connect::ExistingConnectionProbe::new(
                move |label: &str, authority: &str| {
                    probe_handle
                        .try_state::<desktop_connections::ConnectionRegistry>()
                        .and_then(|state| state.existing_window_for_authority(label, authority))
                },
            ));

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

    #[test]
    fn save_dialog_basename_strips_path() {
        assert_eq!(super::save_dialog_basename("a/b/c.png"), "c.png");
        assert_eq!(super::save_dialog_basename("../x.png"), "x.png");
        assert!(super::save_dialog_basename("///").is_empty());
    }

    #[test]
    fn save_dialog_basename_keeps_cjk() {
        assert_eq!(super::save_dialog_basename("笔记/说明.txt"), "说明.txt");
        assert_eq!(super::save_dialog_basename("你好"), "你好");
    }
}
