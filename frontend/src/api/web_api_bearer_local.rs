//! Web API Bearer：进程内内存 + 壳安全存储（桌面钥匙串 / Android Keystore）。
//!
//! - 官方壳：**禁止**明文 `localStorage`（legacy 键仅在迁入安全存储成功后删除）
//! - 纯浏览器：内存 + legacy LS 弱持久化；设置页须提示不安全
//!
//! 连接页成功路径已写入安全存储；本模块供包内 UI 水合、设置保存/清除、断开清理。

use std::cell::RefCell;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::JsFuture;

use super::browser::{
    clear_local_storage_web_api_bearer, read_local_storage_web_api_bearer,
    set_web_api_bearer_token_memory_only, web_api_bearer_token_memory_peek,
    write_local_storage_web_api_bearer,
};
use super::llm_secrets_local::PersistKind;

thread_local! {
    static SECURE_HYDRATE_DONE: RefCell<bool> = const { RefCell::new(false) };
}

#[cfg(target_arch = "wasm32")]
mod bridge {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(inline_js = r#"
function tauriInvoke(cmd, args) {
  const invoke =
    (globalThis.__TAURI__ && globalThis.__TAURI__.core && globalThis.__TAURI__.core.invoke) ||
    (globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke);
  if (typeof invoke !== "function") {
    throw new Error("Tauri invoke unavailable");
  }
  return invoke(cmd, args);
}

export function hasTauriConnectBearerInvoke() {
  try {
    const direct = globalThis.__TAURI__ && globalThis.__TAURI__.core && globalThis.__TAURI__.core.invoke;
    const internal = globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke;
    return typeof direct === "function" || typeof internal === "function";
  } catch (_) {
    return false;
  }
}

export function hasMobileSecureBearerBridge() {
  try {
    const b = globalThis.CrabMateMobile;
    return !!(b && typeof b.getSecureBearer === "function" && typeof b.setSecureBearer === "function");
  } catch (_) {
    return false;
  }
}

export function invokeGetConnectBearer() {
  return tauriInvoke("get_connect_bearer", {});
}

export function invokeSetConnectBearer(bearer) {
  return tauriInvoke("set_connect_bearer", { bearer: String(bearer || "") });
}

export function mobileGetSecureBearer() {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.getSecureBearer !== "function") return "";
    return String(b.getSecureBearer() || "").trim();
  } catch (_) {
    return "";
  }
}

export function mobileSetSecureBearer(bearer) {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.setSecureBearer !== "function") return false;
    return !!b.setSecureBearer(String(bearer || ""));
  } catch (_) {
    return false;
  }
}
"#)]
    extern "C" {
        #[wasm_bindgen(js_name = hasTauriConnectBearerInvoke)]
        pub fn has_tauri_connect_bearer_invoke() -> bool;
        #[wasm_bindgen(js_name = hasMobileSecureBearerBridge)]
        pub fn has_mobile_secure_bearer_bridge() -> bool;
        #[wasm_bindgen(js_name = invokeGetConnectBearer)]
        pub fn invoke_get_connect_bearer() -> js_sys::Promise;
        #[wasm_bindgen(js_name = invokeSetConnectBearer)]
        pub fn invoke_set_connect_bearer(bearer: &str) -> js_sys::Promise;
        #[wasm_bindgen(js_name = mobileGetSecureBearer)]
        pub fn mobile_get_secure_bearer() -> String;
        #[wasm_bindgen(js_name = mobileSetSecureBearer)]
        pub fn mobile_set_secure_bearer(bearer: &str) -> bool;
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod bridge {
    pub fn has_tauri_connect_bearer_invoke() -> bool {
        false
    }
    pub fn has_mobile_secure_bearer_bridge() -> bool {
        false
    }
    pub fn invoke_get_connect_bearer() -> js_sys::Promise {
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)
    }
    pub fn invoke_set_connect_bearer(_: &str) -> js_sys::Promise {
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
    }
    pub fn mobile_get_secure_bearer() -> String {
        String::new()
    }
    pub fn mobile_set_secure_bearer(_: &str) -> bool {
        false
    }
}

fn js_err_to_string(e: &wasm_bindgen::JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(e, &wasm_bindgen::JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// 是否存在桌面钥匙串 / Android Keystore 后端（官方壳内应为 true）。
#[must_use]
pub fn secure_web_api_bearer_backend_available() -> bool {
    bridge::has_mobile_secure_bearer_bridge() || bridge::has_tauri_connect_bearer_invoke()
}

/// 纯逻辑：有安全后端时不得把 Bearer 写入明文 localStorage。
#[must_use]
pub fn should_persist_web_api_bearer_to_local_storage(secure_backend: bool) -> bool {
    !secure_backend
}

async fn load_secure_bearer() -> Option<String> {
    if bridge::has_mobile_secure_bearer_bridge() {
        for attempt in 0..4u32 {
            let mobile = bridge::mobile_get_secure_bearer();
            if !mobile.is_empty() {
                return Some(mobile);
            }
            if attempt + 1 < 4 {
                TimeoutFuture::new(60 * (attempt + 1)).await;
            }
        }
    }
    if bridge::has_tauri_connect_bearer_invoke() {
        if let Ok(val) = JsFuture::from(bridge::invoke_get_connect_bearer()).await {
            if let Some(s) = val.as_string() {
                let t = s.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
    }
    None
}

async fn persist_secure_bearer(value: &str) -> Result<PersistKind, String> {
    let v = value.trim();
    if bridge::has_mobile_secure_bearer_bridge() {
        for attempt in 0..4u32 {
            if bridge::mobile_set_secure_bearer(v) {
                return Ok(PersistKind::Durable);
            }
            if attempt + 1 < 4 {
                TimeoutFuture::new(60 * (attempt + 1)).await;
            }
        }
        return Err("Android Keystore 写入 Web API Bearer 失败".into());
    }
    if bridge::has_tauri_connect_bearer_invoke() {
        JsFuture::from(bridge::invoke_set_connect_bearer(v))
            .await
            .map_err(|e| format!("系统钥匙串写入失败: {}", js_err_to_string(&e)))?;
        return Ok(PersistKind::Durable);
    }
    write_local_storage_web_api_bearer(v);
    Ok(PersistKind::BrowserInsecure)
}

/// 从钥匙串 / Keystore 水合内存；内存已有值时不覆盖。
///
/// 壳内若发现 legacy localStorage，成功迁入安全存储后删除明文键。
pub async fn hydrate_web_api_bearer_from_secure_store() {
    if !secure_web_api_bearer_backend_available() {
        SECURE_HYDRATE_DONE.with(|h| *h.borrow_mut() = true);
        return;
    }
    let mem = web_api_bearer_token_memory_peek();
    if mem.trim().is_empty() {
        if let Some(t) = load_secure_bearer().await {
            set_web_api_bearer_token_memory_only(&t);
        } else if let Some(legacy) = read_local_storage_web_api_bearer() {
            match persist_secure_bearer(&legacy).await {
                Ok(PersistKind::Durable) => {
                    set_web_api_bearer_token_memory_only(&legacy);
                    clear_local_storage_web_api_bearer();
                }
                Ok(PersistKind::BrowserInsecure) | Err(_) => {
                    set_web_api_bearer_token_memory_only(&legacy);
                }
            }
        }
    } else {
        // hash 交接已写入内存：确保明文 LS 不残留
        clear_local_storage_web_api_bearer();
    }
    SECURE_HYDRATE_DONE.with(|h| *h.borrow_mut() = true);
}

/// 在发起需鉴权的请求前调用：官方壳异步水合完成前阻塞到完成（幂等）。
pub async fn ensure_web_api_bearer_hydrated() {
    if SECURE_HYDRATE_DONE.with(|h| *h.borrow()) {
        return;
    }
    hydrate_web_api_bearer_from_secure_store().await;
}

/// 设置页保存/清除：先落盘再改内存；壳内清除明文 LS。
pub async fn set_web_api_bearer_token_async(token: &str) -> Result<PersistKind, String> {
    let t = token.trim().to_string();
    let kind = persist_secure_bearer(&t).await?;
    set_web_api_bearer_token_memory_only(&t);
    if kind == PersistKind::Durable {
        clear_local_storage_web_api_bearer();
    }
    Ok(kind)
}

/// 断开连接：清内存，并尽力清安全存储 / 浏览器 LS。
pub async fn clear_web_api_bearer_on_disconnect() {
    set_web_api_bearer_token_memory_only("");
    clear_local_storage_web_api_bearer();
    if secure_web_api_bearer_backend_available() {
        let _ = persist_secure_bearer("").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_must_not_write_plaintext_local_storage() {
        assert!(!should_persist_web_api_bearer_to_local_storage(true));
        assert!(should_persist_web_api_bearer_to_local_storage(false));
    }
}
