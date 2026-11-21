//! Web API Bearer：进程内内存 + 壳安全存储（桌面钥匙串 / Android Keystore）。
//!
//! - 官方壳：**禁止**明文 `localStorage`（legacy 键仅在迁入安全存储成功后删除）
//! - 纯浏览器：内存 + legacy LS 弱持久化；设置页须提示不安全
//!
//! 连接页成功路径已写入安全存储；本模块供包内 UI 水合、设置保存/清除、断开清理。

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::JsFuture;

use super::browser::{
    clear_local_storage_web_api_bearer_on, read_local_storage_web_api_bearer_on,
    write_local_storage_web_api_bearer_on,
};
use super::connection_scope::ScopeHolder;
use super::llm_secrets_local::PersistKind;
use crate::i18n::{self, load_locale_from_storage};

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

export function invokeGetConnectBearer(fp) {
  return tauriInvoke("get_connect_bearer", { fp: String(fp || "") });
}

export function invokeSetConnectBearer(bearer, fp) {
  return tauriInvoke("set_connect_bearer", { bearer: String(bearer || ""), fp: String(fp || "") });
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
        pub fn invoke_get_connect_bearer(fp: &str) -> js_sys::Promise;
        #[wasm_bindgen(js_name = invokeSetConnectBearer)]
        pub fn invoke_set_connect_bearer(bearer: &str, fp: &str) -> js_sys::Promise;
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
    pub fn invoke_get_connect_bearer(_: &str) -> js_sys::Promise {
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)
    }
    pub fn invoke_set_connect_bearer(_: &str, _: &str) -> js_sys::Promise {
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

/// 读**这一份 holder** 对应的钥匙串槽位（#558 项 7）：槽名按它自己的基址指纹分域。
///
/// **移动端例外**：Android Keystore 桥是单槽（`mobile_get_secure_bearer` 不带指纹），
/// 多 lane 在那边会互相覆盖 —— 设计上多 lane 只给宽屏 Desktop
/// （`docs/design/desktop_single_window_lanes.md` §3）。
async fn load_secure_bearer_for(holder: &ScopeHolder) -> Option<String> {
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
        // 带上**这条 lane 的**指纹：同一个窗口连过几台 serve 就有几个槽，切回上一台
        // 仍能水合到那一台的凭据（#457）；多 lane 时各取各的槽，不会互相覆盖。
        let fp = super::browser::current_conn_fp_on(holder);
        if let Ok(val) = JsFuture::from(bridge::invoke_get_connect_bearer(&fp)).await {
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

/// 写**这一份 holder** 对应的钥匙串槽位（#558 项 7）。移动端同 [`load_secure_bearer_for`]：
/// Keystore 是单槽，多 lane 不适用。
async fn persist_secure_bearer_for(
    holder: &ScopeHolder,
    value: &str,
) -> Result<PersistKind, String> {
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
        return Err(i18n::api_err_keystore_write_bearer_failed(load_locale_from_storage()).into());
    }
    if bridge::has_tauri_connect_bearer_invoke() {
        let fp = super::browser::current_conn_fp_on(holder);
        JsFuture::from(bridge::invoke_set_connect_bearer(v, &fp))
            .await
            .map_err(|e| {
                i18n::api_err_keychain_write_failed(
                    load_locale_from_storage(),
                    &js_err_to_string(&e),
                )
            })?;
        return Ok(PersistKind::Durable);
    }
    // 浏览器弱存储也按这条 lane 的域写（#558 项 3b）：否则纯浏览器路径下两条 lane
    // 会共用一个明文键。
    write_local_storage_web_api_bearer_on(holder, v);
    Ok(PersistKind::BrowserInsecure)
}

/// 从钥匙串 / Keystore 水合**指定的**那一份 scope（#558）；内存已有值时不覆盖。
///
/// 多 lane 时必须走这个：写进**哪个** holder 决定「哪条 lane 拿到凭据」。#558 之前唯一
/// 的坑就是「水合写进全局、header 取自别处」→ 拿空 Bearer 发请求 → 401。
pub async fn hydrate_web_api_bearer_into(holder: &ScopeHolder) {
    if !secure_web_api_bearer_backend_available() {
        holder.mark_bearer_settled();
        return;
    }
    if holder.bearer().trim().is_empty() {
        if let Some(t) = load_secure_bearer_for(holder).await {
            holder.set_bearer(&t);
        } else if let Some(legacy) = read_local_storage_web_api_bearer_on(holder) {
            match persist_secure_bearer_for(holder, &legacy).await {
                Ok(PersistKind::Durable) => {
                    holder.set_bearer(&legacy);
                    clear_local_storage_web_api_bearer_on(holder);
                }
                Ok(PersistKind::BrowserInsecure) | Err(_) => {
                    holder.set_bearer(&legacy);
                }
            }
        }
    } else {
        // hash 交接已写入内存：确保明文 LS 不残留
        clear_local_storage_web_api_bearer_on(holder);
    }
    holder.mark_bearer_settled();
}

/// [`hydrate_web_api_bearer_from_secure_store`] 的**显式 holder** 版本（#558）。
///
/// 幂等标记在 holder 上（per-holder）：A 那条 lane 读过盘，不代表 B 读过。
pub async fn ensure_web_api_bearer_hydrated_into(holder: &ScopeHolder) {
    if holder.is_bearer_settled() {
        return;
    }
    hydrate_web_api_bearer_into(holder).await;
}

/// 设置页保存/清除：先落盘再改内存；壳内清除明文 LS。
///
/// 零参版本操作**前台**那一份 —— 设置页是「前台天然唯一」的调用点（#558 收窄口径）。
pub async fn set_web_api_bearer_token_async(token: &str) -> Result<PersistKind, String> {
    set_web_api_bearer_token_async_for(&super::connection_scope::active_holder(), token).await
}

/// 显式 holder 版本（#558 项 7）：凭据与它的钥匙串槽位都落在**这一份**上。
pub async fn set_web_api_bearer_token_async_for(
    holder: &ScopeHolder,
    token: &str,
) -> Result<PersistKind, String> {
    let t = token.trim().to_string();
    let kind = persist_secure_bearer_for(holder, &t).await?;
    holder.set_bearer(&t);
    if kind == PersistKind::Durable {
        clear_local_storage_web_api_bearer_on(holder);
    }
    Ok(kind)
}

/// 断开连接：清内存，并尽力清安全存储 / 浏览器 LS。零参版本作用于**前台**那一份。
pub async fn clear_web_api_bearer_on_disconnect() {
    clear_web_api_bearer_on_disconnect_for(&super::connection_scope::active_holder()).await;
}

/// 显式 holder 版本（#558 项 7）：只清**这一条 lane** 的内存、分域 LS 与钥匙串槽位。
pub async fn clear_web_api_bearer_on_disconnect_for(holder: &ScopeHolder) {
    holder.set_bearer("");
    clear_local_storage_web_api_bearer_on(holder);
    if secure_web_api_bearer_backend_available() {
        let _ = persist_secure_bearer_for(holder, "").await;
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
