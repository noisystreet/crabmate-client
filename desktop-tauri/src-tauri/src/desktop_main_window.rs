//! 主窗口创建：打开共用连接页，或（E2E）直连已运行的 `serve` URL。
//!
//! 桌面壳**不**拉起 `crabmate serve`；用户须自行启动后端或连接远程。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::webview::{Color, PageLoadEvent};
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::e2e_hide_app_windows;
use crate::os_theme;

/// 主 UI（连上 `serve` 后）默认逻辑尺寸。
const MAIN_UI_WIDTH: f64 = 1280.0;
const MAIN_UI_HEIGHT: f64 = 840.0;

/// 无显示器信息时连接页回退尺寸。
const CONNECT_FALLBACK_WIDTH: f64 = 480.0;
const CONNECT_FALLBACK_HEIGHT: f64 = 420.0;

/// WebView 底色（与前端深色壳一致）。
const BOOT_SHELL_BG: Color = Color(0x0A, 0x0D, 0x12, 0xFF);

/// 主屏（或第一块屏）工作区：逻辑宽高 + 物理原点（供连接页铺满，页内 CSS 居中卡片）。
#[derive(Clone, Copy, Debug)]
struct WorkAreaGeometry {
    logical_width: f64,
    logical_height: f64,
    logical_x: f64,
    logical_y: f64,
    physical_position: tauri::PhysicalPosition<i32>,
    physical_size: tauri::PhysicalSize<u32>,
}

fn primary_work_area(app: &tauri::AppHandle) -> Option<WorkAreaGeometry> {
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| app.available_monitors().ok()?.into_iter().next())?;
    let scale = monitor.scale_factor();
    let area = *monitor.work_area();
    Some(WorkAreaGeometry {
        logical_width: f64::from(area.size.width) / scale,
        logical_height: f64::from(area.size.height) / scale,
        logical_x: f64::from(area.position.x) / scale,
        logical_y: f64::from(area.position.y) / scale,
        physical_position: area.position,
        physical_size: area.size,
    })
}

fn apply_work_area_geometry(window: &WebviewWindow, geo: WorkAreaGeometry) {
    let _ = window.set_position(tauri::Position::Physical(geo.physical_position));
    let _ = window.set_size(tauri::Size::Physical(geo.physical_size));
}

#[derive(Clone, Copy)]
enum MainWindowMode {
    /// 连接页：铺满工作区（页内居中）；导航到 `serve` 后再 maximize。
    ConnectPage,
    /// E2E / 跳过连接页：直接全尺寸 UI（须已有可连的 `serve`）。
    DirectUi,
}

/// E2E / `CM_DESKTOP_SKIP_CONNECT`：直接打开已运行的 `serve` URL。
pub(crate) fn create_main_window_from_url(
    app_handle: &tauri::AppHandle,
    ready_url: String,
) -> Result<(), String> {
    let parsed_url: Url = ready_url
        .parse()
        .map_err(|e| format!("invalid serve url `{ready_url}`: {e}"))?;
    let backend_origin = parsed_url.origin();
    if let Some(allowed) = app_handle.try_state::<crabmate_connect::AllowedServeOrigin>() {
        allowed.set_from_url(&parsed_url);
    }
    finish_create_main_window(
        app_handle,
        WebviewUrl::External(parsed_url),
        backend_origin,
        MainWindowMode::DirectUi,
    )
    .map(|_| ())
}

/// 展示共用连接页；可选预填建议服务器 URL（经 `get_suggested_server_url`）。
///
/// 初始导航白名单不含任何 `serve` Origin；探测成功后由 `connect_remote` 写入
/// [`AllowedServeOrigin`](crabmate_connect::AllowedServeOrigin)。
pub(crate) fn create_main_window_connect_page(
    app_handle: &tauri::AppHandle,
    suggested_url: Option<String>,
) -> Result<(), String> {
    if let Some(url) = suggested_url.filter(|s| !s.trim().is_empty()) {
        app_handle
            .state::<crabmate_connect::SuggestedServerUrl>()
            .set(Some(url.trim().to_string()));
    }
    let placeholder = Url::parse("http://tauri.localhost/connect.html")
        .map_err(|e| format!("invalid connect placeholder url: {e}"))?;
    let backend_origin = placeholder.origin();
    let window = finish_create_main_window(
        app_handle,
        WebviewUrl::App("connect.html".into()),
        backend_origin,
        MainWindowMode::ConnectPage,
    )?;
    if let Ok(current) = window.url() {
        crabmate_connect::seed_connect_home(&current);
    } else {
        crabmate_connect::seed_connect_home(&placeholder);
    }
    Ok(())
}

fn is_connect_page_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    matches!(url.scheme(), "tauri" | "asset")
        || host.eq_ignore_ascii_case("tauri.localhost")
        || url.path().ends_with("connect.html")
}

fn apply_connect_page_geometry(window: &WebviewWindow) {
    // 会话窗可能处于最大化；不先取消则无法改回连接页几何。
    let _ = window.unmaximize();
    let _ = window.set_resizable(false);
    // 铺满工作区：合成器常把首个窗口丢到原点；全屏时 connect.html 的 flex 居中卡片
    // 仍在屏幕中央，不会出现「小窗从左上角跳到中间」。
    if let Some(geo) = primary_work_area(window.app_handle()) {
        apply_work_area_geometry(window, geo);
        return;
    }
    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
        width: CONNECT_FALLBACK_WIDTH,
        height: CONNECT_FALLBACK_HEIGHT,
    }));
    let _ = window.center();
}

fn maximize_main_ui(window: &WebviewWindow) -> bool {
    if let Err(e) = window.maximize() {
        eprintln!("[crabmate-desktop] maximize after connect failed: {e}");
        return false;
    }
    // 未映射时 is_maximized 常为 false；调用方应在 show() 之后再检查。
    matches!(window.is_maximized(), Ok(true))
}

fn fill_monitor_work_area_or_default(window: &WebviewWindow) {
    let _ = window.unmaximize();
    if let Ok(Some(monitor)) = window.current_monitor() {
        let area = *monitor.work_area();
        // 先定位到工作区原点，再设尺寸，避免从小窗中心放大后「沉」到右下。
        let _ = window.set_position(tauri::Position::Physical(area.position));
        let _ = window.set_size(tauri::Size::Physical(area.size));
        return;
    }
    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
        width: MAIN_UI_WIDTH,
        height: MAIN_UI_HEIGHT,
    }));
    let _ = window.center();
}

/// 会话窗几何：须在窗口 **show 之后**调用（X11/Wayland 上未映射时 maximize/monitor 不可靠）。
fn apply_main_ui_geometry(window: &WebviewWindow) {
    let _ = window.set_resizable(true);
    // 不 restore POSITION：连接页中心坐标被当成会话窗原点时，放大后会偏到右下。
    if maximize_main_ui(window) {
        return;
    }
    fill_monitor_work_area_or_default(window);
}

fn allow_http_navigation(app: &tauri::AppHandle, url: &Url, backend_origin: &url::Origin) -> bool {
    let host = url.host_str().unwrap_or("");
    if host.eq_ignore_ascii_case("tauri.localhost") {
        return true;
    }
    let allowed = app
        .try_state::<crabmate_connect::AllowedServeOrigin>()
        .is_some_and(|s| s.matches_url(url));
    let is_backend = url.origin() == *backend_origin;
    if let Some(w) = app.get_webview_window("main")
        && let Ok(cur) = w.url()
    {
        if is_connect_page_url(&cur) {
            if is_backend || allowed {
                return true;
            }
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
            return false;
        }
        if matches!(cur.scheme(), "http" | "https") && cur.origin() == url.origin() {
            return true;
        }
        if matches!(cur.scheme(), "http" | "https") && cur.origin() != url.origin() {
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
            return false;
        }
    }
    if is_backend || allowed {
        return true;
    }
    let _ = app.opener().open_url(url.as_str(), None::<&str>);
    false
}

fn finish_create_main_window(
    app_handle: &tauri::AppHandle,
    webview_url: WebviewUrl,
    backend_origin: url::Origin,
    mode: MainWindowMode,
) -> Result<WebviewWindow, String> {
    let app_handle_clone = app_handle.clone();
    let revealed = Arc::new(AtomicBool::new(false));
    let revealed_on_load = Arc::clone(&revealed);
    let connect_geo = matches!(mode, MainWindowMode::ConnectPage)
        .then(|| primary_work_area(app_handle))
        .flatten();
    let (width, height) = match mode {
        MainWindowMode::ConnectPage => connect_geo
            .map(|g| (g.logical_width, g.logical_height))
            .unwrap_or((CONNECT_FALLBACK_WIDTH, CONNECT_FALLBACK_HEIGHT)),
        MainWindowMode::DirectUi => (MAIN_UI_WIDTH, MAIN_UI_HEIGHT),
    };

    // 连接页：builder 即铺满工作区（勿用居中小窗；未映射时 center/position 常被忽略）。
    let mut builder = WebviewWindowBuilder::new(app_handle, "main", webview_url)
        .title("CrabMate Desktop")
        .inner_size(width, height)
        .resizable(matches!(mode, MainWindowMode::DirectUi))
        .maximized(matches!(mode, MainWindowMode::DirectUi))
        .decorations(false)
        .background_color(BOOT_SHELL_BG)
        .visible(false)
        .theme(os_theme::initial_window_theme());
    if let Some(geo) = connect_geo {
        builder = builder.position(geo.logical_x, geo.logical_y);
    }
    let window = builder
        .on_navigation(move |url| match url.scheme() {
            "tauri" | "asset" => true,
            "mailto" => {
                let _ = app_handle_clone
                    .opener()
                    .open_url(url.as_str(), None::<&str>);
                false
            }
            "http" | "https" => allow_http_navigation(&app_handle_clone, url, &backend_origin),
            _ => false,
        })
        .on_page_load(move |window, payload| {
            let connect = is_connect_page_url(payload.url());
            match payload.event() {
                // 离开连接页后尽早 maximize，不必等 serve WASM Finished。
                PageLoadEvent::Started if !connect => {
                    let _ = window.set_resizable(true);
                    if revealed_on_load.load(Ordering::SeqCst) {
                        apply_main_ui_geometry(&window);
                    } else {
                        reveal_main_window_once(&window, &revealed_on_load);
                    }
                }
                PageLoadEvent::Finished if connect => {
                    apply_connect_page_geometry(&window);
                    reveal_main_window_once(&window, &revealed_on_load);
                }
                PageLoadEvent::Finished if !connect => {
                    if revealed_on_load.load(Ordering::SeqCst) {
                        apply_main_ui_geometry(&window);
                    } else {
                        let _ = window.set_resizable(true);
                        reveal_main_window_once(&window, &revealed_on_load);
                    }
                }
                _ => {}
            }
        })
        .build()
        .map_err(|e| format!("failed to create main window: {e}"))?;

    match mode {
        MainWindowMode::ConnectPage => {
            apply_connect_page_geometry(&window);
        }
        MainWindowMode::DirectUi => {
            let _ = window.set_resizable(true);
        }
    }

    let app_fallback = app_handle.clone();
    let revealed_fallback = Arc::clone(&revealed);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        let revealed = Arc::clone(&revealed_fallback);
        let app = app_fallback.clone();
        let _ = app_fallback.run_on_main_thread(move || {
            if let Some(main) = app.get_webview_window("main") {
                reveal_main_window_once(&main, &revealed);
            }
        });
    });

    Ok(window)
}

fn reveal_main_window_once(window: &WebviewWindow, revealed: &AtomicBool) {
    if revealed.swap(true, Ordering::SeqCst) {
        return;
    }
    if !e2e_hide_app_windows() {
        let on_connect = window.url().ok().as_ref().is_some_and(is_connect_page_url);
        if on_connect {
            apply_connect_page_geometry(window);
        }
        if let Err(e) = window.show() {
            eprintln!("[crabmate-desktop] failed to show main window: {e}");
        }
        // 映射后再钉几何 / 最大化：部分合成器忽略未映射时的定位与尺寸。
        if on_connect {
            apply_connect_page_geometry(window);
        } else {
            apply_main_ui_geometry(window);
        }
        if let Err(e) = window.set_focus() {
            eprintln!("[crabmate-desktop] failed to focus main window: {e}");
        }
    }
}
