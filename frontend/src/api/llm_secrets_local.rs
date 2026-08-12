//! 模型 API 密钥：进程内内存 + 本机安全存储（桌面系统钥匙串 / Android Keystore）。
//!
//! - 壳内：**禁止**明文 `localStorage` 持久化（legacy 键仅在迁入安全存储成功后删除）
//! - 纯浏览器（无 Tauri / 无 Keystore）：会话内存；并写 legacy LS 作弱持久化，UI 须提示不安全
//! - 对话时经 `client_llm.api_key` / `executor_llm.api_key` 随请求体发送（须 HTTPS）

use std::cell::RefCell;
use std::collections::HashMap;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_futures::JsFuture;

use crabmate_client_api::SecretSlot;

use super::browser::window;

const LEGACY_CLIENT_LS: &str = "crabmate-client-llm-api-key";
const LEGACY_EXECUTOR_LS: &str = "crabmate-executor-llm-api-key";
const LEGACY_PRESET_LS: &str = "crabmate-saved-model-api-keys";

fn slot_client() -> &'static str {
    SecretSlot::ClientLlm.as_str()
}
fn slot_executor() -> &'static str {
    SecretSlot::ExecutorLlm.as_str()
}
fn slot_saved() -> &'static str {
    SecretSlot::SavedModels.as_str()
}

thread_local! {
    static CLIENT: RefCell<String> = const { RefCell::new(String::new()) };
    static CLIENT_HYDRATED: RefCell<bool> = const { RefCell::new(false) };
    static EXECUTOR: RefCell<String> = const { RefCell::new(String::new()) };
    static EXECUTOR_HYDRATED: RefCell<bool> = const { RefCell::new(false) };
    static PRESET_MAP: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
    static PRESET_HYDRATED: RefCell<bool> = const { RefCell::new(false) };
}

/// 密钥落盘结果：壳内钥匙串/Keystore，或浏览器弱持久化。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistKind {
    Durable,
    /// 无安全后端时的浏览器降级（legacy localStorage）。
    BrowserInsecure,
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

export function hasTauriLlmSecretInvoke() {
  try {
    const direct = globalThis.__TAURI__ && globalThis.__TAURI__.core && globalThis.__TAURI__.core.invoke;
    const internal = globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke;
    return typeof direct === "function" || typeof internal === "function";
  } catch (_) {
    return false;
  }
}

export function hasMobileLlmSecretBridge() {
  try {
    const b = globalThis.CrabMateMobile;
    return !!(b && typeof b.getSecureLlmSecret === "function" && typeof b.setSecureLlmSecret === "function");
  } catch (_) {
    return false;
  }
}

export function invokeGetLlmSecret(slot) {
  return tauriInvoke("get_llm_secret", { slot: String(slot || "") });
}

export function invokeSetLlmSecret(slot, value) {
  return tauriInvoke("set_llm_secret", {
    slot: String(slot || ""),
    value: String(value || ""),
  });
}

export function mobileGetSecureLlmSecret(slot) {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.getSecureLlmSecret !== "function") return "";
    return String(b.getSecureLlmSecret(String(slot || "")) || "").trim();
  } catch (_) {
    return "";
  }
}

export function mobileSetSecureLlmSecret(slot, value) {
  try {
    const b = globalThis.CrabMateMobile;
    if (!b || typeof b.setSecureLlmSecret !== "function") return false;
    return !!b.setSecureLlmSecret(String(slot || ""), String(value || ""));
  } catch (_) {
    return false;
  }
}

export function readE2eClientLlmKey() {
  try {
    const v = globalThis.__CRABMATE_E2E_CLIENT_LLM_KEY;
    if (typeof v !== "string") return "";
    return v.trim();
  } catch (_) {
    return "";
  }
}

export function setClientLlmKeySetFlag(set) {
  try {
    globalThis.__CRABMATE_CLIENT_LLM_KEY_SET = !!set;
  } catch (_) {}
}
"#)]
    extern "C" {
        #[wasm_bindgen(js_name = hasTauriLlmSecretInvoke)]
        pub fn has_tauri_llm_secret_invoke() -> bool;
        #[wasm_bindgen(js_name = hasMobileLlmSecretBridge)]
        pub fn has_mobile_llm_secret_bridge() -> bool;
        #[wasm_bindgen(js_name = invokeGetLlmSecret)]
        pub fn invoke_get_llm_secret(slot: &str) -> js_sys::Promise;
        #[wasm_bindgen(js_name = invokeSetLlmSecret)]
        pub fn invoke_set_llm_secret(slot: &str, value: &str) -> js_sys::Promise;
        #[wasm_bindgen(js_name = mobileGetSecureLlmSecret)]
        pub fn mobile_get_secure_llm_secret(slot: &str) -> String;
        #[wasm_bindgen(js_name = mobileSetSecureLlmSecret)]
        pub fn mobile_set_secure_llm_secret(slot: &str, value: &str) -> bool;
        #[wasm_bindgen(js_name = readE2eClientLlmKey)]
        pub fn read_e2e_client_llm_key() -> String;
        #[wasm_bindgen(js_name = setClientLlmKeySetFlag)]
        pub fn set_client_llm_key_set_flag(set: bool);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod bridge {
    pub fn has_tauri_llm_secret_invoke() -> bool {
        false
    }
    pub fn has_mobile_llm_secret_bridge() -> bool {
        false
    }
    pub fn invoke_get_llm_secret(_: &str) -> js_sys::Promise {
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)
    }
    pub fn invoke_set_llm_secret(_: &str, _: &str) -> js_sys::Promise {
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
    }
    pub fn mobile_get_secure_llm_secret(_: &str) -> String {
        String::new()
    }
    pub fn mobile_set_secure_llm_secret(_: &str, _: &str) -> bool {
        false
    }
    pub fn read_e2e_client_llm_key() -> String {
        String::new()
    }
    pub fn set_client_llm_key_set_flag(_: bool) {}
}

fn sync_client_key_set_flag() {
    bridge::set_client_llm_key_set_flag(client_llm_api_key_is_set());
}

/// 是否存在钥匙串 / Android Keystore 后端（壳内应为 true）。
#[must_use]
pub fn secure_llm_secret_backend_available() -> bool {
    bridge::has_mobile_llm_secret_bridge() || bridge::has_tauri_llm_secret_invoke()
}

fn legacy_ls_key(slot: &str) -> Option<&'static str> {
    if slot == slot_client() {
        Some(LEGACY_CLIENT_LS)
    } else if slot == slot_executor() {
        Some(LEGACY_EXECUTOR_LS)
    } else if slot == slot_saved() {
        Some(LEGACY_PRESET_LS)
    } else {
        None
    }
}

fn read_legacy_ls(key: &str) -> Option<String> {
    let w = window()?;
    let storage = w.local_storage().ok().flatten()?;
    let v = storage.get_item(key).ok().flatten()?;
    let t = v.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
}

fn write_legacy_ls(key: &str, value: &str) {
    let Some(w) = window() else {
        return;
    };
    let Ok(Some(storage)) = w.local_storage() else {
        return;
    };
    let t = value.trim();
    if t.is_empty() {
        let _ = storage.remove_item(key);
    } else {
        let _ = storage.set_item(key, t);
    }
}

fn clear_legacy_ls(key: &str) {
    write_legacy_ls(key, "");
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

/// 仅经壳安全后端读写（无后端返回 Err / None）；供 GitHub token 等禁止 LS 降级的槽位。
pub(crate) async fn bridge_persist_secure_slot(
    slot: &str,
    value: &str,
) -> Result<PersistKind, String> {
    if !secure_llm_secret_backend_available() {
        return Err("无本机安全存储后端".into());
    }
    persist_slot_async(slot, value).await
}

pub(crate) async fn bridge_load_secure_slot(slot: &str) -> Option<String> {
    if !secure_llm_secret_backend_available() {
        return None;
    }
    load_slot_async(slot).await
}

/// 写入槽位并等待确认。壳失败返回 Err；无安全后端时降级浏览器 LS。
pub async fn persist_slot_async(slot: &str, value: &str) -> Result<PersistKind, String> {
    let v = value.trim();
    if bridge::has_mobile_llm_secret_bridge() {
        // Android：URL 缓存未就绪或 Keystore 竞态时短延迟重试（桥内单次调用，避免嵌套放大）。
        for attempt in 0..4u32 {
            if bridge::mobile_set_secure_llm_secret(slot, v) {
                return Ok(PersistKind::Durable);
            }
            if attempt + 1 < 4 {
                TimeoutFuture::new(60 * (attempt + 1)).await;
            }
        }
        return Err("Android Keystore 写入模型密钥失败".into());
    }
    if bridge::has_tauri_llm_secret_invoke() {
        JsFuture::from(bridge::invoke_set_llm_secret(slot, v))
            .await
            .map_err(|e| format!("系统钥匙串写入失败: {}", js_err_to_string(&e)))?;
        return Ok(PersistKind::Durable);
    }
    // 纯浏览器：弱持久化，调用方应提示用户。
    if let Some(ls) = legacy_ls_key(slot) {
        write_legacy_ls(ls, v);
    }
    Ok(PersistKind::BrowserInsecure)
}

async fn load_slot_async(slot: &str) -> Option<String> {
    if bridge::has_mobile_llm_secret_bridge() {
        for attempt in 0..4u32 {
            let mobile = bridge::mobile_get_secure_llm_secret(slot);
            if !mobile.is_empty() {
                return Some(mobile);
            }
            if attempt + 1 < 4 {
                TimeoutFuture::new(60 * (attempt + 1)).await;
            }
        }
    }
    if bridge::has_tauri_llm_secret_invoke() {
        if let Ok(val) = JsFuture::from(bridge::invoke_get_llm_secret(slot)).await {
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

async fn migrate_legacy_if_needed(slot: &str, legacy_key: &str, value: &str) {
    if read_legacy_ls(legacy_key).is_none() {
        return;
    }
    if !secure_llm_secret_backend_available() {
        return;
    }
    match persist_slot_async(slot, value).await {
        Ok(PersistKind::Durable) => clear_legacy_ls(legacy_key),
        Ok(PersistKind::BrowserInsecure) | Err(_) => {
            // 保留 legacy，避免先删后写丢密钥。
        }
    }
}

fn parse_preset_secret_map(raw: Option<&str>) -> HashMap<String, String> {
    raw.and_then(|s| serde_json::from_str::<HashMap<String, String>>(s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
        .collect()
}

async fn hydrate_client_slot() {
    let client = load_slot_async(slot_client())
        .await
        .or_else(|| read_legacy_ls(LEGACY_CLIENT_LS))
        .or_else(|| {
            let e2e = bridge::read_e2e_client_llm_key();
            if e2e.is_empty() { None } else { Some(e2e) }
        });
    if let Some(ref k) = client {
        CLIENT.with(|c| *c.borrow_mut() = k.clone());
        migrate_legacy_if_needed(slot_client(), LEGACY_CLIENT_LS, k).await;
    }
    CLIENT_HYDRATED.with(|h| *h.borrow_mut() = true);
    sync_client_key_set_flag();
}

async fn hydrate_executor_slot() {
    let executor = load_slot_async(slot_executor())
        .await
        .or_else(|| read_legacy_ls(LEGACY_EXECUTOR_LS));
    if let Some(ref k) = executor {
        EXECUTOR.with(|c| *c.borrow_mut() = k.clone());
        migrate_legacy_if_needed(slot_executor(), LEGACY_EXECUTOR_LS, k).await;
    }
    EXECUTOR_HYDRATED.with(|h| *h.borrow_mut() = true);
}

async fn hydrate_saved_preset_map() {
    let saved_raw = load_slot_async(slot_saved())
        .await
        .or_else(|| read_legacy_ls(LEGACY_PRESET_LS));
    let map = parse_preset_secret_map(saved_raw.as_deref());
    if read_legacy_ls(LEGACY_PRESET_LS).is_some() && secure_llm_secret_backend_available() {
        let raw = if map.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&map).unwrap_or_default()
        };
        if let Ok(PersistKind::Durable) = persist_slot_async(slot_saved(), &raw).await {
            clear_legacy_ls(LEGACY_PRESET_LS);
        }
    }
    PRESET_MAP.with(|c| *c.borrow_mut() = Some(map));
    PRESET_HYDRATED.with(|h| *h.borrow_mut() = true);
}

/// 从钥匙串 / Keystore（及成功后的 legacy 迁移）水合进程内缓存。
pub async fn hydrate_llm_secrets_from_secure_store() {
    hydrate_client_slot().await;
    hydrate_executor_slot().await;
    hydrate_saved_preset_map().await;
}

/// 写入或清除主模型 API 密钥（先落盘成功再改内存）。
pub async fn set_client_llm_api_key_async(api_key: &str) -> Result<PersistKind, String> {
    let t = api_key.trim().to_string();
    let kind = persist_slot_async(slot_client(), &t).await?;
    CLIENT.with(|c| *c.borrow_mut() = t);
    CLIENT_HYDRATED.with(|h| *h.borrow_mut() = true);
    if kind == PersistKind::Durable {
        clear_legacy_ls(LEGACY_CLIENT_LS);
    }
    sync_client_key_set_flag();
    Ok(kind)
}

#[must_use]
pub fn client_llm_api_key() -> String {
    CLIENT.with(|c| c.borrow().clone())
}

#[must_use]
pub fn client_llm_api_key_is_set() -> bool {
    !client_llm_api_key().trim().is_empty()
}

pub async fn set_executor_llm_api_key_async(api_key: &str) -> Result<PersistKind, String> {
    let t = api_key.trim().to_string();
    let kind = persist_slot_async(slot_executor(), &t).await?;
    EXECUTOR.with(|c| *c.borrow_mut() = t);
    EXECUTOR_HYDRATED.with(|h| *h.borrow_mut() = true);
    if kind == PersistKind::Durable {
        clear_legacy_ls(LEGACY_EXECUTOR_LS);
    }
    Ok(kind)
}

#[must_use]
pub fn executor_llm_api_key() -> String {
    EXECUTOR.with(|c| c.borrow().clone())
}

#[must_use]
pub fn executor_llm_api_key_is_set() -> bool {
    !executor_llm_api_key().trim().is_empty()
}

#[must_use]
pub fn saved_preset_secret_id(label: &str, api_base: &str, model: &str) -> String {
    format!("{}\n{}\n{}", label.trim(), api_base.trim(), model.trim())
}

fn ensure_preset_map() -> HashMap<String, String> {
    PRESET_MAP.with(|cell| {
        if let Some(m) = cell.borrow().as_ref() {
            return m.clone();
        }
        let empty = HashMap::new();
        *cell.borrow_mut() = Some(empty.clone());
        empty
    })
}

async fn persist_preset_map_async(map: &HashMap<String, String>) -> Result<PersistKind, String> {
    PRESET_MAP.with(|cell| *cell.borrow_mut() = Some(map.clone()));
    let raw = if map.is_empty() {
        String::new()
    } else {
        serde_json::to_string(map).map_err(|e| e.to_string())?
    };
    let kind = persist_slot_async(slot_saved(), &raw).await?;
    if kind == PersistKind::Durable {
        clear_legacy_ls(LEGACY_PRESET_LS);
    }
    Ok(kind)
}

#[must_use]
pub fn get_saved_preset_api_key(label: &str, api_base: &str, model: &str) -> String {
    let id = saved_preset_secret_id(label, api_base, model);
    ensure_preset_map().get(&id).cloned().unwrap_or_default()
}

pub async fn set_saved_preset_api_key_async(
    label: &str,
    api_base: &str,
    model: &str,
    api_key: &str,
) -> Result<PersistKind, String> {
    let id = saved_preset_secret_id(label, api_base, model);
    if id.trim().is_empty() {
        return Ok(if secure_llm_secret_backend_available() {
            PersistKind::Durable
        } else {
            PersistKind::BrowserInsecure
        });
    }
    let mut map = ensure_preset_map();
    let t = api_key.trim();
    if t.is_empty() {
        map.remove(&id);
    } else {
        map.insert(id, t.to_string());
    }
    persist_preset_map_async(&map).await
}

/// 同步写入内存并尽力异步落盘（用于应用预设等非关键路径）；落盘失败时内存仍更新以便本会话可用。
pub fn set_client_llm_api_key(api_key: &str) {
    let t = api_key.trim().to_string();
    CLIENT.with(|c| *c.borrow_mut() = t.clone());
    CLIENT_HYDRATED.with(|h| *h.borrow_mut() = true);
    sync_client_key_set_flag();
    leptos::task::spawn_local(async move {
        let _ = set_client_llm_api_key_async(&t).await;
    });
}

pub fn set_executor_llm_api_key(api_key: &str) {
    let t = api_key.trim().to_string();
    EXECUTOR.with(|c| *c.borrow_mut() = t.clone());
    EXECUTOR_HYDRATED.with(|h| *h.borrow_mut() = true);
    leptos::task::spawn_local(async move {
        let _ = set_executor_llm_api_key_async(&t).await;
    });
}

#[allow(dead_code)] // 应用预设等 fire-and-forget；关键保存走 async
pub fn set_saved_preset_api_key(label: &str, api_base: &str, model: &str, api_key: &str) {
    let label = label.to_string();
    let api_base = api_base.to_string();
    let model = model.to_string();
    let api_key = api_key.to_string();
    // 先更新内存 map，再异步落盘。
    let id = saved_preset_secret_id(&label, &api_base, &model);
    if !id.trim().is_empty() {
        let mut map = ensure_preset_map();
        let t = api_key.trim();
        if t.is_empty() {
            map.remove(&id);
        } else {
            map.insert(id, t.to_string());
        }
        PRESET_MAP.with(|cell| *cell.borrow_mut() = Some(map));
    }
    leptos::task::spawn_local(async move {
        let _ = set_saved_preset_api_key_async(&label, &api_base, &model, &api_key).await;
    });
}

#[must_use]
pub fn saved_preset_api_key_is_set(label: &str, api_base: &str, model: &str) -> bool {
    !get_saved_preset_api_key(label, api_base, model)
        .trim()
        .is_empty()
}
