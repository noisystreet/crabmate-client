//! `/upload(s)` 的写路径：composer 移除附图时删掉 Server 侧暂存文件。
//!
//! 从 `http.rs` 拆出来：那个文件已经贴着 920 行的单文件上限（`fn-nloc-ratchet`），
//! 而「上传 / 删除暂存文件」本来就是能独立成篇的一组端点（与 `http_workspace_*` 同款切法）。
//! 上传本身仍在 `http::upload_files_multipart`（它是 SSE 之外的共享入口，搬动只会徒增改动面）。

use serde::Serialize;

use super::http::fetch_json_with_body;
use crate::i18n::Locale;

#[derive(Debug, Serialize)]
struct DeleteUploadsBody<'a> {
    urls: &'a [String],
}

/// `POST /uploads/delete`：删掉 composer 里被移除的暂存附图。
///
/// **路径是 `/uploads/delete`（复数）**：Server 路由表 `src/web/routes/chat/mod.rs` 把它和
/// `GET /uploads/{filename}` 放在同一段；`chat_handlers/upload.rs` 的文件头注释写的是
/// `/upload/delete`，以路由表为准（照注释写只会 404，而调用方忽略失败，看不出来）。
///
/// 响应里的 `deleted` / `skipped` 刻意不解析：调用方一律忽略失败，Server 自己按龄 /
/// 按总量清理，知道「哪几个没删掉」也做不了什么。
pub async fn delete_upload_urls(urls: &[String], loc: Locale) -> Result<(), String> {
    let body = serde_json::to_string(&DeleteUploadsBody { urls }).map_err(|e| e.to_string())?;
    let _: serde_json::Value = fetch_json_with_body("POST", "/uploads/delete", &body, loc).await?;
    Ok(())
}
