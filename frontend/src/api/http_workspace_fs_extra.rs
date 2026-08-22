//! `GET /workspace/dir/archive` 与 `POST /workspace/file/move`（工作区目录 zip / 文件重命名）。

use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use crate::i18n::Locale;

use super::browser::{
    api_url, auth_headers, format_fetch_transport_error, prepare_api_auth, window,
};
use super::http::http_error_detail_from_body;
use super::http_workspace_raw::fetch_workspace_get_bytes;

/// `GET /workspace/dir/archive`；空 `path` 表示工作区根（省略查询参数）。
#[must_use]
pub(crate) fn workspace_dir_archive_url(path: &str) -> String {
    let path = path.trim().trim_matches('/');
    if path.is_empty() {
        "/workspace/dir/archive".to_string()
    } else {
        format!("/workspace/dir/archive?path={}", urlencoding::encode(path))
    }
}

/// `GET /workspace/dir/archive`：目录 zip 字节（不跟随符号链接；上限由 serve 执行）。
pub async fn fetch_workspace_dir_archive(path: &str, loc: Locale) -> Result<Vec<u8>, String> {
    fetch_workspace_get_bytes(&workspace_dir_archive_url(path), loc).await
}

#[derive(Serialize)]
struct WorkspaceFileMovePayload<'a> {
    from: &'a str,
    to: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<&'a str>,
}

/// `POST /workspace/file/move` 失败（含 HTTP 状态，供覆盖确认识别 409）。
#[derive(Debug, Clone)]
pub struct PostWorkspaceFileMoveError {
    pub status: u16,
    pub message: String,
}

impl PostWorkspaceFileMoveError {
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        self.status == 409
    }
}

/// `POST /workspace/file/move`：工作区内重命名/移动常规文件。成功 204。
pub async fn post_workspace_file_move(
    from: &str,
    to: &str,
    overwrite: bool,
    conversation_id: Option<&str>,
    loc: Locale,
) -> Result<(), PostWorkspaceFileMoveError> {
    let w = window().ok_or_else(|| PostWorkspaceFileMoveError {
        status: 0,
        message: crate::i18n::api_err_no_window(loc).to_string(),
    })?;
    let cid = conversation_id.map(str::trim).filter(|s| !s.is_empty());
    let payload = WorkspaceFileMovePayload {
        from,
        to,
        overwrite,
        conversation_id: cid,
    };
    let body = serde_json::to_string(&payload).map_err(|e| PostWorkspaceFileMoveError {
        status: 0,
        message: e.to_string(),
    })?;
    let init = RequestInit::new();
    init.set_method("POST");
    prepare_api_auth(&init).await;
    let h = auth_headers();
    let _ = h.set("Content-Type", "application/json");
    init.set_headers(&h);
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let req =
        Request::new_with_str_and_init(&api_url("/workspace/file/move"), &init).map_err(|e| {
            PostWorkspaceFileMoveError {
                status: 0,
                message: format!("request: {:?}", e),
            }
        })?;
    let resp_val = JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|e| PostWorkspaceFileMoveError {
            status: 0,
            message: format_fetch_transport_error(&e),
        })?;
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| PostWorkspaceFileMoveError {
            status: 0,
            message: crate::i18n::api_err_response_type(loc).to_string(),
        })?;
    let status = resp.status();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let text = JsFuture::from(resp.text().map_err(|e| PostWorkspaceFileMoveError {
        status,
        message: format!("text: {:?}", e),
    })?)
    .await
    .ok()
    .and_then(|v| v.as_string())
    .unwrap_or_default();
    Err(PostWorkspaceFileMoveError {
        status,
        message: crate::i18n::api_err_http_status(
            loc,
            status,
            http_error_detail_from_body(&text).as_str(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::workspace_dir_archive_url;

    #[test]
    fn archive_url_omits_path_for_workspace_root() {
        assert_eq!(workspace_dir_archive_url(""), "/workspace/dir/archive");
        assert_eq!(workspace_dir_archive_url("/"), "/workspace/dir/archive");
        assert_eq!(workspace_dir_archive_url("  "), "/workspace/dir/archive");
    }

    #[test]
    fn archive_url_percent_encodes_cjk_path() {
        let url = workspace_dir_archive_url("笔记/src");
        assert!(url.starts_with("/workspace/dir/archive?path="), "{url}");
        assert!(!url.contains("笔记"), "{url}");
        assert!(url.contains("%"), "{url}");
    }
}
