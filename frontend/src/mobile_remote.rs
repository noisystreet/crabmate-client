//! Android 远程薄客户端桥：`MainActivity` 注入的 `window.CrabMateMobile`。
//! 远程 `serve` 源上无 Tauri IPC，断开须走此桥（或系统返回键）。

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = r#"
/** 是否在 Android 远程薄客户端 WebView（含已导航到远程 serve 的页面）。 */
export function isCrabMateMobileRemoteClient() {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b) return false;
    if (typeof b.isRemoteClient === "function") {
      return !!b.isRemoteClient();
    }
    return typeof b.disconnect === "function";
  } catch (_) {
    return false;
  }
}

export function hasCrabMateMobileDisconnect() {
  try {
    return !!(
      globalThis.CrabMateMobile &&
      typeof globalThis.CrabMateMobile.disconnect === "function"
    );
  } catch (_) {
    return false;
  }
}

export function invokeCrabMateMobileDisconnect() {
  if (
    !globalThis.CrabMateMobile ||
    typeof globalThis.CrabMateMobile.disconnect !== "function"
  ) {
    throw new Error("CrabMateMobile.disconnect unavailable");
  }
  globalThis.CrabMateMobile.disconnect();
}

export function hasCrabMateMobileOpenExternalUrl() {
  try {
    return !!(
      globalThis.CrabMateMobile &&
      typeof globalThis.CrabMateMobile.openExternalUrl === "function"
    );
  } catch (_) {
    return false;
  }
}

export function invokeCrabMateMobileOpenExternalUrl(url) {
  if (
    !globalThis.CrabMateMobile ||
    typeof globalThis.CrabMateMobile.openExternalUrl !== "function"
  ) {
    throw new Error("CrabMateMobile.openExternalUrl unavailable");
  }
  globalThis.CrabMateMobile.openExternalUrl(String(url || ""));
}

/** 从原生读取顶栏/底栏安全区并写入 CSS 变量。可多次调用。 */
export function applyCrabMateMobileSafeTop() {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.getStatusBarInsetPx !== "function") {
      return false;
    }
    let top = Number(b.getStatusBarInsetPx());
    if (!Number.isFinite(top) || top < 0) {
      top = 24;
    }
    top = Math.max(top, 24);
    let bottom = 24;
    if (typeof b.getNavBarInsetPx === "function") {
      bottom = Number(b.getNavBarInsetPx());
      if (!Number.isFinite(bottom) || bottom < 0) {
        bottom = 24;
      }
      bottom = Math.max(bottom, 24);
    }
    const root = document.documentElement;
    root.style.setProperty("--cm-safe-top", top + "px");
    root.style.setProperty("--cm-safe-bottom", bottom + "px");
    root.setAttribute("data-cm-mobile-shell", "");
    return true;
  } catch (_) {
    return false;
  }
}

/** 启动时重试：桥或 insets 可能晚于 WASM bootstrap。 */
export function scheduleCrabMateMobileSafeTop() {
  const run = () => applyCrabMateMobileSafeTop();
  if (run()) return true;
  [50, 200, 600, 1500].forEach((ms) => setTimeout(run, ms));
  return false;
}

export function crabMateMobileStartStreamKeepAlive(locale) {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.startStreamKeepAlive !== "function") return "";
    return String(b.startStreamKeepAlive(String(locale || "")) || "");
  } catch (_) {
    return "";
  }
}

export function crabMateMobileStopStreamKeepAlive() {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.stopStreamKeepAlive !== "function") return;
    b.stopStreamKeepAlive();
  } catch (_) {}
}

export function crabMateMobileNotifyApproval(command, args, locale) {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.notifyApproval !== "function") return;
    b.notifyApproval(String(command || ""), String(args || ""), String(locale || ""));
  } catch (_) {}
}

export function crabMateMobileClearApprovalNotification() {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.clearApprovalNotification !== "function") return;
    b.clearApprovalNotification();
  } catch (_) {}
}

export function setCrabMateKeepAlivePermissionHandler(cb) {
  globalThis.__cmKeepAlivePermission = function (granted) {
    try {
      cb(!!granted);
    } catch (_) {}
  };
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = isCrabMateMobileRemoteClient)]
    fn is_crabmate_mobile_remote_client() -> bool;
    #[wasm_bindgen(js_name = hasCrabMateMobileDisconnect)]
    fn has_crabmate_mobile_disconnect() -> bool;
    #[wasm_bindgen(js_name = invokeCrabMateMobileDisconnect)]
    fn invoke_crabmate_mobile_disconnect();
    #[wasm_bindgen(js_name = hasCrabMateMobileOpenExternalUrl)]
    fn has_crabmate_mobile_open_external_url() -> bool;
    #[wasm_bindgen(js_name = invokeCrabMateMobileOpenExternalUrl)]
    fn invoke_crabmate_mobile_open_external_url(url: &str);
    #[wasm_bindgen(js_name = scheduleCrabMateMobileSafeTop)]
    fn schedule_crabmate_mobile_safe_top() -> bool;
    #[wasm_bindgen(js_name = crabMateMobileStartStreamKeepAlive)]
    fn crabmate_mobile_start_stream_keep_alive(locale: &str) -> String;
    #[wasm_bindgen(js_name = crabMateMobileStopStreamKeepAlive)]
    fn crabmate_mobile_stop_stream_keep_alive();
    #[wasm_bindgen(js_name = crabMateMobileNotifyApproval)]
    fn crabmate_mobile_notify_approval(command: &str, args: &str, locale: &str);
    #[wasm_bindgen(js_name = crabMateMobileClearApprovalNotification)]
    fn crabmate_mobile_clear_approval_notification();
    #[wasm_bindgen(js_name = setCrabMateKeepAlivePermissionHandler)]
    fn set_crabmate_keepalive_permission_handler(cb: &js_sys::Function);
}

#[cfg(not(target_arch = "wasm32"))]
fn is_crabmate_mobile_remote_client() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn has_crabmate_mobile_disconnect() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn invoke_crabmate_mobile_disconnect() {}

#[cfg(not(target_arch = "wasm32"))]
fn has_crabmate_mobile_open_external_url() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn invoke_crabmate_mobile_open_external_url(_: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn schedule_crabmate_mobile_safe_top() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn crabmate_mobile_start_stream_keep_alive(_: &str) -> String {
    String::new()
}

#[cfg(not(target_arch = "wasm32"))]
fn crabmate_mobile_stop_stream_keep_alive() {}

#[cfg(not(target_arch = "wasm32"))]
fn crabmate_mobile_notify_approval(_: &str, _: &str, _: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn crabmate_mobile_clear_approval_notification() {}

/// 是否在 Android 远程薄客户端 WebView 内（`CrabMateMobile`）。
#[must_use]
pub fn mobile_remote_client() -> bool {
    is_crabmate_mobile_remote_client()
}

/// 是否可断开回连接页（同 [`mobile_remote_client`] 的常见能力探测）。
#[must_use]
pub fn mobile_remote_disconnect_available() -> bool {
    has_crabmate_mobile_disconnect()
}

/// 请求壳导航回本地连接页。
pub fn mobile_remote_disconnect() {
    if !has_crabmate_mobile_disconnect() {
        return;
    }
    mobile_stop_stream_keep_alive();
    invoke_crabmate_mobile_disconnect();
}

/// 经原生桥在系统浏览器打开 URL；不可用时返回 `false`。
#[must_use]
pub fn mobile_remote_open_external_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || !has_crabmate_mobile_open_external_url() {
        return false;
    }
    invoke_crabmate_mobile_open_external_url(u);
    true
}

/// 应用 Android 顶栏安全区 CSS 变量（尽早调用，并安排短延迟重试）。
pub fn apply_mobile_remote_safe_top() {
    let _ = schedule_crabmate_mobile_safe_top();
}

/// 启动 Android 流式前台保活。无桥返回空串；`ok` / `need_permission` 见原生实现。
#[must_use]
pub fn mobile_start_stream_keep_alive(locale_slug: &str) -> String {
    crabmate_mobile_start_stream_keep_alive(locale_slug)
}

pub fn mobile_stop_stream_keep_alive() {
    crabmate_mobile_stop_stream_keep_alive();
}

pub fn mobile_notify_stream_approval(command: &str, args: &str, locale_slug: &str) {
    crabmate_mobile_notify_approval(command, args, locale_slug);
}

pub fn mobile_clear_stream_approval_notification() {
    crabmate_mobile_clear_approval_notification();
}

/// 安装一次性权限结果回调（后续 attach 忽略）。桌面/非 wasm 为 no-op。
pub fn install_keepalive_permission_handler(f: impl FnMut(bool) + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::Cell;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        thread_local! {
            static INSTALLED: Cell<bool> = const { Cell::new(false) };
        }
        if INSTALLED.with(Cell::get) {
            return;
        }
        INSTALLED.with(|c| c.set(true));
        let mut f = f;
        let cb = Closure::<dyn FnMut(bool)>::new(move |granted: bool| f(granted));
        set_crabmate_keepalive_permission_handler(cb.as_ref().unchecked_ref());
        cb.forget();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = f;
    }
}
