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

/// WebView 底色（与前端深色壳一致，等于 `frontend/styles/tokens.css` 的 `--bg`）。
///
/// 无法引用 CSS 变量，只能把色值抄一份，故与启动页底色同理由
/// `scripts/check-token-mirrors.sh` 冻结：改 `--bg` 未同步改这里即失败。
const BOOT_SHELL_BG: Color = Color(0x07, 0x09, 0x0E, 0xFF);

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

/// 主窗口的 label。
///
/// 现在只有这一个窗口，但**所有**按 label 找窗口的地方都走这个常量（导航钩子、
/// `AllowedServeOrigin` 的白名单键、`get_webview_window`）。多窗口化时把它换成每个
/// 窗口自己的 label 即可，不必再去各处的字符串字面量里捞。
///
/// 值来自共享层：Bearer 的分槽账户名带同一个后缀，TUI 的钥匙串回退也读它。这里再写
/// 一遍 `"main"` 只是抄写副本 —— 改名时 Desktop 照样编译通过，TUI 静默 miss。
pub(crate) use crabmate_client_api::secrets::DESKTOP_MAIN_WINDOW_LABEL as MAIN_WINDOW_LABEL;

/// E2E / `CM_DESKTOP_SKIP_CONNECT`：包内业务 UI + API 基址指向已运行的 `serve`。
pub(crate) fn create_main_window_from_url(
    app_handle: &tauri::AppHandle,
    ready_url: String,
) -> Result<(), String> {
    let api_base = crabmate_connect::normalize_base_url(&ready_url)
        .map_err(|e| format!("invalid serve url `{ready_url}`: {e}"))?;
    let backend_origin = api_base.origin();
    if let Some(allowed) = app_handle.try_state::<crabmate_connect::AllowedServeOrigin>() {
        allowed.set_from_url(MAIN_WINDOW_LABEL, &api_base);
    }
    let bearer = std::env::var("CM_WEB_API_BEARER_TOKEN").unwrap_or_default();
    // 与 connect_remote 一致：非空 Bearer 写入钥匙串，供包内 UI 刷新后水合（不落明文 LS）。
    if let Err(e) = crabmate_connect::write_connect_bearer_on_connect(MAIN_WINDOW_LABEL, &bearer) {
        eprintln!("[crabmate-desktop] e2e keyring write skipped: {e}");
    }
    let home = Url::parse("http://tauri.localhost/connect.html")
        .map_err(|e| format!("invalid local connect placeholder: {e}"))?;
    crabmate_connect::seed_connect_home(&home);
    let ui = crabmate_connect::local_business_ui_url(&home);
    let target = crabmate_connect::build_local_ui_handoff_url(ui, &api_base, &bearer);
    // 先挂 App 资产，再 navigate 带 hash 交接（避免 External 解析差异）。
    let window = finish_create_main_window(
        app_handle,
        MAIN_WINDOW_LABEL,
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
        MAIN_WINDOW_LABEL,
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
    label: &str,
    url: &Url,
    backend_origin: &url::Origin,
) -> bool {
    let allowed = app
        .try_state::<crabmate_connect::AllowedServeOrigin>()
        .is_some_and(|s| s.matches_url(label, url));
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

fn allow_http_navigation(
    app: &tauri::AppHandle,
    label: &str,
    url: &Url,
    backend_origin: &url::Origin,
) -> bool {
    if is_tauri_localhost_host(url) {
        return true;
    }
    let trusted = is_trusted_serve_navigation(app, label, url, backend_origin);
    // 用**发起导航的那个窗口**的当前页判同源。原先写死 `"main"`：单窗口时没问题，
    // 多窗口时会拿 main 的 URL 去判别的窗口要去哪儿 —— 误判成跨源后把 URL 甩给外部
    // 浏览器，而那个地址本来就在本窗口的白名单里。
    if let Some(w) = app.get_webview_window(label)
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

fn on_main_navigation(
    app: &tauri::AppHandle,
    label: &str,
    url: &Url,
    backend_origin: &url::Origin,
) -> bool {
    match url.scheme() {
        "tauri" | "asset" => true,
        "mailto" => deny_and_open_external(app, url),
        "http" | "https" => allow_http_navigation(app, label, url, backend_origin),
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
            if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                reveal_main_window_once(&main, &revealed);
            }
        });
    });
}

fn finish_create_main_window(
    app_handle: &tauri::AppHandle,
    label: &str,
    webview_url: WebviewUrl,
    backend_origin: url::Origin,
    mode: MainWindowMode,
) -> Result<WebviewWindow, String> {
    let app_handle_clone = app_handle.clone();
    let nav_label = label.to_string();
    let revealed = Arc::new(AtomicBool::new(false));
    let revealed_on_load = Arc::clone(&revealed);
    let connect_geo = matches!(mode, MainWindowMode::ConnectPage)
        .then(|| primary_work_area(app_handle))
        .flatten();
    let (width, height) = main_window_builder_size(mode, connect_geo);

    // 连接页：builder 即铺满工作区（勿用居中小窗；未映射时 center/position 常被忽略）。
    let mut builder = WebviewWindowBuilder::new(app_handle, label, webview_url)
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
        .on_navigation(move |url| {
            on_main_navigation(&app_handle_clone, &nav_label, url, &backend_origin)
        })
        .on_page_load(move |window, payload| {
            handle_main_page_load(&window, payload.event(), payload.url(), &revealed_on_load);
        })
        .build()
        .map_err(|e| format!("failed to create main window: {e}"))?;

    apply_mode_after_build(&window, mode);
    spawn_reveal_fallback(app_handle, revealed);
    Ok(window)
}

/// 建一个「连接窗口」：直接进业务 UI，几何与 main 的会话窗一致（铺满工作区）。
///
/// 这是多窗口里的「第二个及以后」。与 [`finish_create_main_window`] 的差别只有两点：
/// 没有连接页 / 直接 UI 之分（总是直接 UI），也不要 reveal 防白屏那套 —— 那是为了
/// 「先藏、定位好再显示」不让人看到默认尺寸闪一下，这里建完立刻 show。
///
/// 白名单按 `label` 记（见 `AllowedServeOrigin`），导航守卫按 `label` 取窗口
/// （见 `allow_http_navigation`）—— 两处都靠这个参数，所以 label 必须是最终的窗口名。
pub(crate) fn create_connection_window(
    app: &tauri::AppHandle,
    label: &str,
    api_base: &Url,
    display_name: &str,
    bearer: &str,
) -> Result<(), String> {
    let backend_origin = api_base.origin();
    if let Some(allowed) = app.try_state::<crabmate_connect::AllowedServeOrigin>() {
        allowed.set_from_url(label, api_base);
    }

    let home = Url::parse("http://tauri.localhost/connect.html")
        .map_err(|e| format!("invalid local connect placeholder: {e}"))?;
    let ui = crabmate_connect::local_business_ui_url(&home);
    let target = crabmate_connect::build_local_ui_handoff_url(ui, api_base, bearer);

    let nav_label = label.to_string();
    let app_handle_clone = app.clone();
    // 标题带连接名：两个窗口都铺满工作区时，标题栏是**唯一**能区分它们的静态标记。
    let title = if display_name.is_empty() {
        String::from("CrabMate")
    } else {
        format!("CrabMate · {display_name}")
    };

    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(MAIN_UI_WIDTH, MAIN_UI_HEIGHT)
        .resizable(true)
        .background_color(BOOT_SHELL_BG)
        .visible(false)
        .theme(os_theme::initial_window_theme())
        .on_navigation(move |url| {
            on_main_navigation(&app_handle_clone, &nav_label, url, &backend_origin)
        })
        .build()
        .map_err(|e| format!("failed to create connection window: {e}"))?;

    window
        .navigate(target)
        .map_err(|e| format!("failed to open local UI with API handoff: {e}"))?;

    // 几何必须在 show 之后应用：X11 / Wayland 上未映射时 maximize 与 current_monitor
    // 都不可靠（见 `apply_main_ui_geometry` 的 doc）。
    window
        .show()
        .map_err(|e| format!("failed to show connection window: {e}"))?;
    apply_main_ui_geometry(&window);
    Ok(())
}

/// 建一个**待连接**窗口：只载连接页，连哪台 `serve` 由用户在那个窗口里选。
///
/// 与 [`create_connection_window`] 的分工：那条已经知道 `api_base`，建完直接交接进业务 UI；
/// 这条只知道「要再开一条」，把选址交给连接页。设置页的按钮因此不必先读一个 draft 基址 ——
/// 否则它会和旁边「保存基址」（改**当前窗口**连谁）的语义打架。
///
/// 白名单不在这里写：连接页探测成功后由 `connect_remote` 按 `window.label()` 记
/// （`crates/crabmate-connect/src/commands.rs`），所以多窗口各选各的地址不会互相覆盖。
/// 这也意味着本函数无需 `AllowedServeOrigin`，与 [`create_main_window_connect_page`] 同口径。
pub(crate) fn create_pending_connection_window(
    app: &tauri::AppHandle,
    label: &str,
    display_name: &str,
) -> Result<(), String> {
    let placeholder = Url::parse("http://tauri.localhost/connect.html")
        .map_err(|e| format!("invalid connect placeholder url: {e}"))?;
    let backend_origin = placeholder.origin();
    let window = finish_create_main_window(
        app,
        label,
        WebviewUrl::App("connect.html".into()),
        backend_origin,
        MainWindowMode::ConnectPage,
    )?;
    // 标题带连接名：窗口都铺满工作区时，标题栏是唯一能区分它们的静态标记
    // （同 `create_connection_window`）。`finish_create_main_window` 的标题是固定的。
    if !display_name.is_empty() {
        let _ = window.set_title(&format!("CrabMate · {display_name}"));
    }
    if let Ok(current) = window.url() {
        crabmate_connect::seed_connect_home(&current);
    } else {
        crabmate_connect::seed_connect_home(&placeholder);
    }
    Ok(())
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
