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

/// E2E / `CM_DESKTOP_SKIP_CONNECT`：包内业务 UI + API 基址指向已运行的 `serve`。
pub(crate) fn create_main_window_from_url(
    app_handle: &tauri::AppHandle,
    ready_url: String,
) -> Result<(), String> {
    let api_base = crabmate_connect::normalize_base_url(&ready_url)
        .map_err(|e| format!("invalid serve url `{ready_url}`: {e}"))?;
    let backend_origin = api_base.origin();
    if let Some(allowed) = app_handle.try_state::<crabmate_connect::AllowedServeOrigin>() {
        allowed.set_from_url(&api_base);
    }
    let bearer = std::env::var("CM_WEB_API_BEARER_TOKEN").unwrap_or_default();
    let home = Url::parse("http://tauri.localhost/connect.html")
        .map_err(|e| format!("invalid local connect placeholder: {e}"))?;
    crabmate_connect::seed_connect_home(&home);
    let ui = crabmate_connect::local_business_ui_url(&home);
    let target = crabmate_connect::build_local_ui_handoff_url(ui, &api_base, &bearer);
    // 先挂 App 资产，再 navigate 带 hash 交接（避免 External 解析差异）。
    let window = finish_create_main_window(
        app_handle,
        WebviewUrl::App("index.html".into()),
        backend_origin,
        MainWindowMode::DirectUi,
    )?;
    window
        .navigate(target)
        .map_err(|e| format!("failed to open local UI with API handoff: {e}"))?;
    Ok(())
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
    crabmate_connect::is_connect_page_url(url)
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

fn open_url_external(app: &tauri::AppHandle, url: &Url) {
    let _ = app.opener().open_url(url.as_str(), None::<&str>);
}

fn is_tauri_localhost_host(url: &Url) -> bool {
    url.host_str()
        .unwrap_or("")
        .eq_ignore_ascii_case("tauri.localhost")
}

fn is_trusted_serve_navigation(
    app: &tauri::AppHandle,
    url: &Url,
    backend_origin: &url::Origin,
) -> bool {
    let allowed = app
        .try_state::<crabmate_connect::AllowedServeOrigin>()
        .is_some_and(|s| s.matches_url(url));
    url.origin() == *backend_origin || allowed
}

/// 当前页为 http(s) 时：同 Origin 放行，跨 Origin 外开；否则 `None`（交给后续策略）。
fn same_origin_http_decision(cur: &Url, url: &Url) -> Option<bool> {
    if !matches!(cur.scheme(), "http" | "https") {
        return None;
    }
    Some(cur.origin() == url.origin())
}

fn deny_and_open_external(app: &tauri::AppHandle, url: &Url) -> bool {
    open_url_external(app, url);
    false
}

fn allow_http_navigation(app: &tauri::AppHandle, url: &Url, backend_origin: &url::Origin) -> bool {
    if is_tauri_localhost_host(url) {
        return true;
    }
    let trusted = is_trusted_serve_navigation(app, url, backend_origin);
    if let Some(w) = app.get_webview_window("main")
        && let Ok(cur) = w.url()
    {
        if is_connect_page_url(&cur) {
            return trusted || deny_and_open_external(app, url);
        }
        if let Some(same_origin) = same_origin_http_decision(&cur, url) {
            return same_origin || deny_and_open_external(app, url);
        }
    }
    trusted || deny_and_open_external(app, url)
}

fn on_main_navigation(app: &tauri::AppHandle, url: &Url, backend_origin: &url::Origin) -> bool {
    match url.scheme() {
        "tauri" | "asset" => true,
        "mailto" => deny_and_open_external(app, url),
        "http" | "https" => allow_http_navigation(app, url, backend_origin),
        _ => false,
    }
}

fn handle_main_page_load(
    window: &WebviewWindow,
    event: PageLoadEvent,
    page_url: &Url,
    revealed: &AtomicBool,
) {
    let connect = is_connect_page_url(page_url);
    match event {
        // 离开连接页后尽早 maximize，不必等 serve WASM Finished。
        PageLoadEvent::Started if !connect => {
            let _ = window.set_resizable(true);
            if revealed.load(Ordering::SeqCst) {
                apply_main_ui_geometry(window);
            } else {
                reveal_main_window_once(window, revealed);
            }
        }
        PageLoadEvent::Finished if connect => {
            apply_connect_page_geometry(window);
            reveal_main_window_once(window, revealed);
        }
        PageLoadEvent::Finished if !connect => {
            if revealed.load(Ordering::SeqCst) {
                apply_main_ui_geometry(window);
            } else {
                let _ = window.set_resizable(true);
                reveal_main_window_once(window, revealed);
            }
        }
        _ => {}
    }
}

fn main_window_builder_size(
    mode: MainWindowMode,
    connect_geo: Option<WorkAreaGeometry>,
) -> (f64, f64) {
    match mode {
        MainWindowMode::ConnectPage => connect_geo
            .map(|g| (g.logical_width, g.logical_height))
            .unwrap_or((CONNECT_FALLBACK_WIDTH, CONNECT_FALLBACK_HEIGHT)),
        MainWindowMode::DirectUi => (MAIN_UI_WIDTH, MAIN_UI_HEIGHT),
    }
}

fn apply_mode_after_build(window: &WebviewWindow, mode: MainWindowMode) {
    match mode {
        MainWindowMode::ConnectPage => {
            apply_connect_page_geometry(window);
        }
        MainWindowMode::DirectUi => {
            let _ = window.set_resizable(true);
        }
    }
}

fn spawn_reveal_fallback(app_handle: &tauri::AppHandle, revealed: Arc<AtomicBool>) {
    let app_fallback = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        let revealed = Arc::clone(&revealed);
        let app = app_fallback.clone();
        let _ = app_fallback.run_on_main_thread(move || {
            if let Some(main) = app.get_webview_window("main") {
                reveal_main_window_once(&main, &revealed);
            }
        });
    });
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
    let (width, height) = main_window_builder_size(mode, connect_geo);

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
        .on_navigation(move |url| on_main_navigation(&app_handle_clone, url, &backend_origin))
        .on_page_load(move |window, payload| {
            handle_main_page_load(&window, payload.event(), payload.url(), &revealed_on_load);
        })
        .build()
        .map_err(|e| format!("failed to create main window: {e}"))?;

    apply_mode_after_build(&window, mode);
    spawn_reveal_fallback(app_handle, revealed);
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
