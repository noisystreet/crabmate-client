//! CrabMate Desktop（Tauri WebView）壳层能力：检测与窗口装饰等。

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = r#"
export function hasTauriInvoke() {
  const direct = globalThis.__TAURI__ && globalThis.__TAURI__.core && globalThis.__TAURI__.core.invoke;
  const internal = globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke;
  return typeof direct === "function" || typeof internal === "function";
}

function tauriInvoke(cmd, args) {
  const invoke =
    (globalThis.__TAURI__ && globalThis.__TAURI__.core && globalThis.__TAURI__.core.invoke) ||
    (globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke);
  if (typeof invoke !== "function") {
    throw new Error("Tauri invoke unavailable");
  }
  return invoke(cmd, args);
}

export function invokeTauriSetMainWindowDecorations(decorations) {
  return tauriInvoke("set_main_window_decorations", { decorations });
}

export function invokeTauriMainWindowMinimize() {
  return tauriInvoke("main_window_minimize", {});
}

export function invokeTauriMainWindowToggleMaximize() {
  return tauriInvoke("main_window_toggle_maximize", {});
}

export function invokeTauriMainWindowClose() {
  return tauriInvoke("main_window_close", {});
}

export function invokeTauriOpenExternalUrl(url) {
  const href = String(url || "");
  const fallbackOpen = () => {
    try {
      window.open(href, "_blank");
    } catch (_) {}
  };
  try {
    const b = globalThis.CrabMateMobile;
    if (b && typeof b.openExternalUrl === "function") {
      b.openExternalUrl(href);
      return Promise.resolve();
    }
  } catch (_) {}
  try {
    return Promise.resolve(tauriInvoke("open_external_url", { url: href })).catch(() => {
      fallbackOpen();
    });
  } catch (_) {
    fallbackOpen();
    return Promise.resolve();
  }
}

export function invokeTauriOsPrefersDarkTheme() {
  return tauriInvoke("os_prefers_dark_theme", {});
}

export function invokeTauriDisconnectRemote() {
  return tauriInvoke("disconnect_remote", {});
}

export function invokeTauriListConnections() {
  return tauriInvoke("list_connections", {});
}

export function invokeTauriRegisterConnection(apiBase, displayName) {
  return tauriInvoke("register_connection", {
    apiBase,
    displayName: displayName ?? null,
  });
}

export function invokeTauriFocusConnection(key) {
  return tauriInvoke("focus_connection", { key });
}

// 只开一个连接页窗口（连哪台 serve 由用户在新窗口里选）；不带基址。
export function invokeTauriOpenConnectWindow(displayName) {
  return tauriInvoke("open_connect_window", {
    displayName: displayName ?? null,
  });
}

// 返回 Promise<unlisten|null>。null 表示这个壳没有 event 能力（浏览器 / 老壳），
// 调用方据此退化为「只读一次名单」，而不是当成错误。
export function listenTauriConnectionsChanged(cb) {
  const ev = globalThis.__TAURI__ && globalThis.__TAURI__.event;
  if (!ev || typeof ev.listen !== "function") {
    return Promise.resolve(null);
  }
  return ev.listen("connections_changed", (payload) => {
    cb(JSON.stringify(payload.payload));
  });
}

export function installChatExternalLinkHandler() {
  if (globalThis.__crabmateChatExternalLinkHandlerInstalled) {
    return;
  }
  globalThis.__crabmateChatExternalLinkHandlerInstalled = true;
  document.addEventListener(
    "click",
    (ev) => {
      const target = ev.target;
      if (!target || typeof target.closest !== "function") {
        return;
      }
      const anchor = target.closest("a[href]");
      if (!anchor) {
        return;
      }
      const raw = anchor.getAttribute("href");
      if (!raw || raw.startsWith('#')) {
        return;
      }
      let parsed;
      try {
        parsed = new URL(raw, window.location.href);
      } catch {
        return;
      }
      const scheme = parsed.protocol.replace(":", "");
      if (scheme !== "http" && scheme !== "https" && scheme !== "mailto") {
        return;
      }
      if (parsed.origin === window.location.origin) {
        return;
      }
      ev.preventDefault();
      ev.stopPropagation();
      void invokeTauriOpenExternalUrl(parsed.href);
    },
    true
  );
}
"#)]
#[cfg(target_arch = "wasm32")]
extern "C" {
    #[wasm_bindgen(js_name = hasTauriInvoke)]
    fn has_tauri_invoke() -> bool;
    #[wasm_bindgen(js_name = invokeTauriSetMainWindowDecorations)]
    fn invoke_tauri_set_main_window_decorations(decorations: bool) -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriMainWindowMinimize)]
    fn invoke_tauri_main_window_minimize() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriMainWindowToggleMaximize)]
    fn invoke_tauri_main_window_toggle_maximize() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriMainWindowClose)]
    fn invoke_tauri_main_window_close() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriOpenExternalUrl)]
    fn invoke_tauri_open_external_url(url: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriOsPrefersDarkTheme)]
    fn invoke_tauri_os_prefers_dark_theme() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriDisconnectRemote)]
    fn invoke_tauri_disconnect_remote() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriListConnections)]
    fn invoke_tauri_list_connections() -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriRegisterConnection)]
    fn invoke_tauri_register_connection(
        api_base: &str,
        display_name: Option<String>,
    ) -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriFocusConnection)]
    fn invoke_tauri_focus_connection(key: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = invokeTauriOpenConnectWindow)]
    fn invoke_tauri_open_connect_window(display_name: Option<String>) -> js_sys::Promise;
    #[wasm_bindgen(js_name = listenTauriConnectionsChanged)]
    fn listen_tauri_connections_changed(cb: &js_sys::Function) -> js_sys::Promise;
    #[wasm_bindgen(js_name = installChatExternalLinkHandler)]
    fn install_chat_external_link_handler();
}

// Native stubs for non-wasm targets (tests / SSR)
#[cfg(not(target_arch = "wasm32"))]
fn has_tauri_invoke() -> bool {
    false
}
#[cfg(not(target_arch = "wasm32"))]
fn install_chat_external_link_handler() {}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_set_main_window_decorations(_: bool) -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_main_window_minimize() -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_main_window_toggle_maximize() -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_main_window_close() -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_open_external_url(_: &str) -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_os_prefers_dark_theme() -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)
}
#[cfg(not(target_arch = "wasm32"))]
fn invoke_tauri_disconnect_remote() -> js_sys::Promise {
    js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
}

/// 非 wasm（测试 / SSR）下切窗口是 no-op。
///
/// 只有这一个需要 stub：它是唯一在 `cfg` 之外被调用（`view!` 的点击处理）的多窗口 API。
/// `list` / `register` / `listen` 全部由 `#[cfg(target_arch = "wasm32")]` 的
/// `install_connections_feed` 独占调用，host 目标上给它们留 stub 只会变成 dead code
/// （`-D warnings` 直接拒），所以这里干脆不定义。
#[cfg(not(target_arch = "wasm32"))]
pub fn tauri_focus_connection(_: &str) {}

/// 是否在 **桌面** Tauri WebView 内运行。
/// Android 远程薄客户端虽可能注入 `__TAURI__`，但无本机选目录等桌面命令，须视为非桌面壳。
#[must_use]
pub fn tauri_shell_available() -> bool {
    if crate::mobile_remote::mobile_remote_client() {
        return false;
    }
    has_tauri_invoke()
}

async fn tauri_invoke_void(promise: js_sys::Promise) -> Result<(), String> {
    JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// 隐藏系统标题栏（保留应用内最小化/最大化/关闭按钮）。
pub async fn tauri_set_main_window_decorations(decorations: bool) -> Result<(), String> {
    tauri_invoke_void(invoke_tauri_set_main_window_decorations(decorations)).await
}

/// Tauri 启动后始终使用无边框主窗口（会话与 IDE 模式一致）。
pub fn tauri_apply_frameless_window_chrome() {
    if !tauri_shell_available() {
        return;
    }
    install_chat_external_link_handler();
    spawn_local(async move {
        let _ = tauri_set_main_window_decorations(false).await;
    });
}

/// 安装聊天区外链点击处理（桌面 Tauri / 移动端原生桥均可；幂等）。
pub fn ensure_external_link_handler() {
    install_chat_external_link_handler();
}

fn tauri_spawn_window_action(f: fn() -> js_sys::Promise) {
    if !tauri_shell_available() {
        return;
    }
    spawn_local(async move {
        let _ = tauri_invoke_void(f()).await;
    });
}

/// 最小化主窗口（Tauri）。
pub fn tauri_main_window_minimize() {
    tauri_spawn_window_action(invoke_tauri_main_window_minimize);
}

/// 切换主窗口最大化（Tauri）。
pub fn tauri_main_window_toggle_maximize() {
    tauri_spawn_window_action(invoke_tauri_main_window_toggle_maximize);
}

/// 关闭主窗口（Tauri）。
pub fn tauri_main_window_close() {
    tauri_spawn_window_action(invoke_tauri_main_window_close);
}

/// 导航回连接页（桌面 `disconnect_remote`）。
pub fn tauri_disconnect_remote() {
    if !tauri_shell_available() {
        return;
    }
    spawn_local(async move {
        let _ = tauri_invoke_void(invoke_tauri_disconnect_remote()).await;
    });
}

/// 读一次全部连接名单（桌面多窗口协调面 `list_connections`）。
///
/// 壳返回的是**快照数组**（`Vec<ConnectionSnapshot>` 经 serde → JS 数组），不是 JSON
/// 文本，所以这里先 `JSON.stringify` 成文本再交出去：结构仍由
/// `crabmate_client_api::connections::ConnectionSnapshot` 定义，解析属于调用方。
///
/// 为什么不把 `JsValue` 直接交给调用方：`parse_connection_list` 收 `&str`，非 wasm 目标
/// 上的 `cargo test` 也跑同一条解析；改用 `serde-wasm-bindgen` 解析 `JsValue` 会为一条
/// 首帧多引一个依赖。**别把 `as_string()` 单独当成功路径** —— 壳返回数组时它恒为
/// `None`，历史上正是这一条让左栏连接区永远空白（见 `install_connections_feed`）。
#[cfg(target_arch = "wasm32")]
pub async fn tauri_list_connections_json() -> Result<String, String> {
    if !tauri_shell_available() {
        return Err("not in tauri shell".to_string());
    }
    let value = JsFuture::from(invoke_tauri_list_connections())
        .await
        .map_err(|e| format!("{e:?}"))?;
    // 老壳若本来就回文本，原样透传（这是唯一跳过 stringify 的快路径）。
    if let Some(text) = value.as_string() {
        return Ok(text);
    }
    // `null` / `undefined` = 这个壳不认识这个命令：按空名单处理，与「没有连接」同义
    // （空表会退回单条 chip，而「不知道」不该被当成错误）。
    if value.is_null() || value.is_undefined() {
        return Ok("[]".to_string());
    }
    let text: String = js_sys::JSON::stringify(&value)
        .map_err(|e| format!("list_connections result not serializable: {e:?}"))?
        .into();
    Ok(text)
}

/// 订阅 `connections_changed`；回调收到 JSON 文本。
///
/// 返回 `Ok(None)` = 这个壳没有 event 能力（浏览器 / 老壳），调用方应退化为「只读一次
/// 名单」，**不要**当成错误。注册成功返回 `Ok(Some(unlisten))` —— 调用方**必须**在
/// 卸载时调用它：否则回调会继续 `set` 一个已经 drop 的 signal。
#[cfg(target_arch = "wasm32")]
pub async fn tauri_listen_connections_changed(
    cb: js_sys::Function,
) -> Result<Option<wasm_bindgen::JsValue>, String> {
    if !tauri_shell_available() {
        return Ok(None);
    }
    let value = JsFuture::from(listen_tauri_connections_changed(&cb))
        .await
        .map_err(|e| format!("{e:?}"))?;
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// 把自己的连接登记进壳的协调面（`register_connection`）。
///
/// 没有这一步，注册表里就只有「用 `open_connection` 开出来的窗口」—— main 窗口自己在
/// 名单里缺席，于是列表永远只有一条（或干脆不切列表）。幂等：重复调用是覆盖，无害。
///
/// 返回 `Result` 而不是 fire-and-forget：调用方要**先登记再取名单**，顺序反了会读到
/// 一份不含自己的列表，`len` 永远差一条。
#[cfg(target_arch = "wasm32")]
pub async fn tauri_register_connection(
    api_base: &str,
    display_name: Option<&str>,
) -> Result<(), String> {
    if !tauri_shell_available() || api_base.trim().is_empty() {
        return Ok(());
    }
    tauri_invoke_void(invoke_tauri_register_connection(
        api_base,
        display_name.map(str::to_string),
    ))
    .await
}

/// 切到某条连接对应的窗口（`focus_connection`）。
#[cfg(target_arch = "wasm32")]
pub fn tauri_focus_connection(key: &str) {
    if !tauri_shell_available() {
        return;
    }
    let key = key.to_string();
    spawn_local(async move {
        let _ = tauri_invoke_void(invoke_tauri_focus_connection(&key)).await;
    });
}

/// 开一个**只载连接页**的新窗口（桌面多窗口）。
///
/// 与 [`tauri_focus_connection`] 同为 fire-and-forget：建窗失败（注册表未就绪、同名 label）
/// 只落在壳日志里。所以调用方要把按钮的渲染条件绑在 [`tauri_shell_available`] 上 ——
/// 非桌面壳里这条命令根本不存在，点了只会静默无事。
#[cfg(target_arch = "wasm32")]
pub fn tauri_open_connect_window(display_name: Option<&str>) {
    if !tauri_shell_available() {
        return;
    }
    let name = display_name.map(str::to_string);
    spawn_local(async move {
        let _ = tauri_invoke_void(invoke_tauri_open_connect_window(name)).await;
    });
}

/// 非 wasm（测试 / SSR）下开窗是 no-op。
///
/// 与 [`tauri_focus_connection`] 同理：它被设置页 `view!` 的点击处理引用，而那段代码在
/// 两个目标下都编译。
#[cfg(not(target_arch = "wasm32"))]
pub fn tauri_open_connect_window(_: Option<&str>) {}

/// 在系统默认浏览器中打开 URL。
///
/// 优先级：Android `CrabMateMobile.openExternalUrl` → 桌面 Tauri `open_external_url` → `window.open`。
pub fn tauri_open_external_url(url: &str) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    if crate::mobile_remote::mobile_remote_open_external_url(url) {
        return;
    }
    if !tauri_shell_available() {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
        return;
    }
    let url = url.to_string();
    spawn_local(async move {
        // JS 桥已在 invoke 失败时 `window.open`；此处再兜底一次，避免 Promise 被吞掉。
        if tauri_invoke_void(invoke_tauri_open_external_url(&url))
            .await
            .is_err()
        {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target(&url, "_blank");
            }
        }
    });
}

/// 拉取桌面侧 OS 明暗提示并写入 [`crate::app_prefs::set_tauri_os_prefers_dark_hint`]。
/// 返回 `Some(dark)` 表示 Linux 侧有明确结果；非 Tauri / 非 Linux 为 `None`。
pub async fn tauri_fetch_os_prefers_dark_hint() -> Option<bool> {
    if !tauri_shell_available() {
        return None;
    }
    let value = JsFuture::from(invoke_tauri_os_prefers_dark_theme())
        .await
        .ok()?;
    if value.is_null() || value.is_undefined() {
        crate::app_prefs::set_tauri_os_prefers_dark_hint(None);
        return None;
    }
    let dark = value.as_bool()?;
    crate::app_prefs::set_tauri_os_prefers_dark_hint(Some(dark));
    Some(dark)
}
