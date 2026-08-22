//! `PUT /workspace/file/raw` 与 `GET /workspace/file/download`：工作区原始字节。

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use crate::i18n::Locale;

use super::browser::{
    api_url, auth_headers, format_fetch_transport_error, prepare_api_auth, window,
};
use super::http::http_error_detail_from_body;

/// `GET`/`PUT /workspace/file/raw?path=`；`create_only` 仅写入用。
pub(crate) fn workspace_file_raw_url(path: &str, create_only: bool) -> String {
    let mut url = format!(
        "/workspace/file/raw?path={}",
        urlencoding::encode(path.trim())
    );
    if create_only {
        url.push_str("&create_only=true");
    }
    url
}

/// `GET /workspace/file/download?path=`（任意类型原始字节；非聊天图片 raw）。
pub(crate) fn workspace_file_download_url(path: &str) -> String {
    format!(
        "/workspace/file/download?path={}",
        urlencoding::encode(path.trim())
    )
}

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
    let url = workspace_file_raw_url(path, create_only);
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

/// `GET /workspace/file/download`：原样字节（PDF 等任意类型；不要走图片专用 `GET …/file/raw`）。
pub async fn fetch_workspace_file_download(path: &str, loc: Locale) -> Result<Vec<u8>, String> {
    let w = window().ok_or_else(|| crate::i18n::api_err_no_window(loc).to_string())?;
    let url = workspace_file_download_url(path);
    let init = RequestInit::new();
    init.set_method("GET");
    prepare_api_auth(&init).await;
    init.set_headers(&auth_headers());
    let req = Request::new_with_str_and_init(&api_url(&url), &init)
        .map_err(|e| format!("request: {:?}", e))?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| format_fetch_transport_error(&e))?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| crate::i18n::api_err_response_type(loc).to_string())?;
    let status = resp.status();
    if !(200..300).contains(&status) {
        return Err(raw_get_status_message(resp, status, loc).await);
    }
    response_body_bytes(resp).await
}

async fn raw_get_status_message(resp: Response, status: u16, loc: Locale) -> String {
    let Ok(text_p) = resp.text() else {
        return crate::i18n::api_err_http_status(loc, status, "");
    };
    let text = JsFuture::from(text_p)
        .await
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    crate::i18n::api_err_http_status(loc, status, http_error_detail_from_body(&text).as_str())
}

async fn response_body_bytes(resp: Response) -> Result<Vec<u8>, String> {
    let buf = JsFuture::from(
        resp.array_buffer()
            .map_err(|e| format!("arrayBuffer: {:?}", e))?,
    )
    .await
    .map_err(|e| format!("arrayBuffer: {:?}", e))?;
    let arr = js_sys::Uint8Array::new(&buf);
    let mut bytes = vec![0u8; arr.length() as usize];
    arr.copy_to(&mut bytes);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{workspace_file_download_url, workspace_file_raw_url};

    #[test]
    fn raw_url_percent_encodes_cjk_path() {
        let url = workspace_file_raw_url("笔记/说明.txt", false);
        assert!(url.starts_with("/workspace/file/raw?path="));
        assert!(!url.contains("说明"));
        assert!(url.contains("%"), "CJK must be percent-encoded, got {url}");
        assert!(!url.contains("create_only"));
    }

    #[test]
    fn download_url_is_not_image_raw() {
        let url = workspace_file_download_url("说明.pdf");
        assert!(url.starts_with("/workspace/file/download?path="), "{url}");
        assert!(!url.contains("/file/raw"));
        assert!(!url.contains("说明"));
    }

    #[test]
    fn raw_url_create_only_query() {
        let url = workspace_file_raw_url("a.bin", true);
        assert!(url.ends_with("&create_only=true"), "{url}");
    }
}
