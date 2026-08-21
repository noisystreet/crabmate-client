//! 灯箱 `blob:` 图：应用内复制 / 另存（WebView 原生菜单对 blob 无效）。

use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::i18n::{self, Locale};
use crate::tauri_shell::tauri_shell_available;

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

function androidShareAvailable() {
  try {
    const b = globalThis.CrabMateMobile;
    return !!(
      b &&
      typeof b.beginChatImageSave === "function" &&
      typeof b.appendChatImageSave === "function" &&
      typeof b.finishChatImageSave === "function"
    );
  } catch (_) {
    return false;
  }
}

function rasterBlobToPng(blob) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext("2d");
      if (!ctx) {
        URL.revokeObjectURL(url);
        reject(new Error("canvas"));
        return;
      }
      ctx.drawImage(img, 0, 0);
      URL.revokeObjectURL(url);
      c.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob"))), "image/png");
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("decode"));
    };
    img.src = url;
  });
}

function blobToPng(blobUrl) {
  return fetch(blobUrl)
    .then((r) => r.blob())
    .then((blob) => {
      const type = blob.type || "image/png";
      if (!type.startsWith("image/") || type.indexOf("svg") !== -1) {
        throw new Error("not a raster image");
      }
      if (type === "image/png") {
        return blob;
      }
      return rasterBlobToPng(blob);
    });
}

function copyPngViaExecCommand(blob) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => {
      const div = document.createElement("div");
      div.contentEditable = "true";
      div.style.cssText = "position:fixed;left:-9999px;top:0";
      div.appendChild(img);
      document.body.appendChild(div);
      const range = document.createRange();
      range.selectNode(img);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      let ok = false;
      try {
        ok = document.execCommand("copy");
      } catch (_) {
        ok = false;
      }
      sel.removeAllRanges();
      div.remove();
      URL.revokeObjectURL(url);
      ok ? resolve() : reject(new Error("execCommand"));
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("decode"));
    };
    img.src = url;
  });
}

export function copyBlobUrlToClipboard(blobUrl) {
  const pngP = blobToPng(blobUrl);
  if (navigator.clipboard && typeof ClipboardItem === "function") {
    return navigator.clipboard
      .write([new ClipboardItem({ "image/png": pngP })])
      .catch(() => pngP.then(copyPngViaExecCommand));
  }
  return pngP.then(copyPngViaExecCommand);
}

function bytesToBase64(bytes) {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunk)));
  }
  return btoa(binary);
}

export async function saveBlobUrlViaAndroid(blobUrl, filename) {
  const buf = await (await fetch(blobUrl)).arrayBuffer();
  const b64 = bytesToBase64(new Uint8Array(buf));
  if (!globalThis.CrabMateMobile.beginChatImageSave(filename)) {
    throw new Error("beginChatImageSave");
  }
  const step = 240000;
  for (let i = 0; i < b64.length; i += step) {
    if (!globalThis.CrabMateMobile.appendChatImageSave(b64.slice(i, i + step))) {
      throw new Error("appendChatImageSave");
    }
  }
  if (!globalThis.CrabMateMobile.finishChatImageSave()) {
    throw new Error("finishChatImageSave");
  }
}

export async function saveBlobUrlViaAnchor(blobUrl, filename) {
  const blob = await (await fetch(blobUrl)).blob();
  const u = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = u;
  a.download = filename;
  a.rel = "noopener";
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(u), 2000);
}

export async function saveBlobUrlViaTauri(blobUrl, filename) {
  const buf = await (await fetch(blobUrl)).arrayBuffer();
  const b64 = bytesToBase64(new Uint8Array(buf));
  return tauriInvoke("save_bytes_file_via_dialog", {
    default_name: filename,
    defaultName: filename,
    content_base64: b64,
    contentBase64: b64,
  });
}

export function hasAndroidChatImageShare() {
  return androidShareAvailable();
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = copyBlobUrlToClipboard)]
    fn js_copy_blob_url_to_clipboard(blob_url: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = saveBlobUrlViaAnchor)]
    fn js_save_blob_url_via_anchor(blob_url: &str, filename: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = saveBlobUrlViaTauri)]
    fn js_save_blob_url_via_tauri(blob_url: &str, filename: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = saveBlobUrlViaAndroid)]
    fn js_save_blob_url_via_android(blob_url: &str, filename: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = hasAndroidChatImageShare)]
    fn js_has_android_chat_image_share() -> bool;
}

fn alert_msg(msg: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.alert_with_message(msg);
    }
}

pub fn copy_lightbox_blob(blob_url: &str, loc: Locale) {
    if !blob_url.starts_with("blob:") {
        return;
    }
    let p = js_copy_blob_url_to_clipboard(blob_url);
    spawn_local(async move {
        if JsFuture::from(p).await.is_err() {
            alert_msg(i18n::clipboard_failed(loc));
        }
    });
}

pub fn save_lightbox_blob(blob_url: &str, filename: &str, loc: Locale) {
    if !blob_url.starts_with("blob:") {
        return;
    }
    let url = blob_url.to_string();
    let name = filename.to_string();
    spawn_local(async move {
        if let Err(msg) = save_lightbox_blob_async(&url, &name, loc).await {
            alert_msg(&msg);
        }
    });
}

async fn save_lightbox_blob_async(url: &str, name: &str, loc: Locale) -> Result<(), String> {
    if js_has_android_chat_image_share() {
        return js_await_save(js_save_blob_url_via_android(url, name), loc).await;
    }
    if tauri_shell_available() {
        return save_via_tauri(url, name, loc).await;
    }
    js_await_save(js_save_blob_url_via_anchor(url, name), loc).await
}

async fn js_await_save(p: js_sys::Promise, loc: Locale) -> Result<(), String> {
    JsFuture::from(p)
        .await
        .map(|_| ())
        .map_err(|_| i18n::chat_image_save_failed(loc).to_string())
}

async fn save_via_tauri(url: &str, name: &str, loc: Locale) -> Result<(), String> {
    match JsFuture::from(js_save_blob_url_via_tauri(url, name)).await {
        Ok(v) => {
            if v.as_bool().is_some_and(|saved| !saved) {
                Err(i18n::export_tauri_save_cancelled_alert(loc).to_string())
            } else {
                Ok(())
            }
        }
        Err(e) => Err(i18n::export_tauri_save_failed_alert(loc, &format!("{e:?}"))),
    }
}
