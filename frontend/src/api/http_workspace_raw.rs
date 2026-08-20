//! `PUT /workspace/file/raw`：工作区原始字节写入。

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use crate::i18n::Locale;

use super::browser::{
    api_url, auth_headers, format_fetch_transport_error, prepare_api_auth, window,
};
use super::http::http_error_detail_from_body;

/// `PUT /workspace/file/raw` 失败（含 HTTP 状态，供覆盖确认识别 409）。
#[derive(Debug, Clone)]
pub struct PutWorkspaceRawError {
    pub status: u16,
    pub message: String,
}

impl PutWorkspaceRawError {
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        self.status == 409
    }
}

/// `PUT /workspace/file/raw`：原始字节写入（文本与二进制）。成功 204；`create_only` 冲突为 409。
pub async fn put_workspace_file_raw(
    path: &str,
    bytes: &[u8],
    create_only: bool,
    loc: Locale,
) -> Result<(), PutWorkspaceRawError> {
    let w = window().ok_or_else(|| PutWorkspaceRawError {
        status: 0,
        message: crate::i18n::api_err_no_window(loc).to_string(),
    })?;
    let mut url = format!(
        "/workspace/file/raw?path={}",
        urlencoding::encode(path.trim())
    );
    if create_only {
        url.push_str("&create_only=true");
    }
    let init = RequestInit::new();
    init.set_method("PUT");
    prepare_api_auth(&init).await;
    let h = auth_headers();
    let _ = h.set("Content-Type", "application/octet-stream");
    init.set_headers(&h);
    let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    arr.copy_from(bytes);
    init.set_body(&arr);
    let req = Request::new_with_str_and_init(&api_url(&url), &init).map_err(|e| {
        PutWorkspaceRawError {
            status: 0,
            message: format!("request: {:?}", e),
        }
    })?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| PutWorkspaceRawError {
            status: 0,
            message: format_fetch_transport_error(&e),
        })?;
    let resp: Response = resp_val.dyn_into().map_err(|_| PutWorkspaceRawError {
        status: 0,
        message: crate::i18n::api_err_response_type(loc).to_string(),
    })?;
    let status = resp.status();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let text = JsFuture::from(resp.text().map_err(|e| PutWorkspaceRawError {
        status,
        message: format!("text: {:?}", e),
    })?)
    .await
    .ok()
    .and_then(|v| v.as_string())
    .unwrap_or_default();
    Err(PutWorkspaceRawError {
        status,
        message: crate::i18n::api_err_http_status(
            loc,
            status,
            http_error_detail_from_body(&text).as_str(),
        ),
    })
}
