//! 用户气泡中的聊天附图（`/uploads/...`，与 composer 待发预览同源）。

use crate::app::chat::tui_line_markdown::TuiBodyChunks;
use crate::markdown::plaintext_to_safe_html;

#[cfg(target_arch = "wasm32")]
use crate::api::api_url;

/// 仅接受单段文件名的 **`/uploads/<file>`**（与 Server `normalize_chat_image_urls` 一致）。
#[must_use]
pub(crate) fn safe_chat_upload_path(url: &str) -> Option<&str> {
    let t = url.trim();
    if !t.starts_with("/uploads/") || t.contains("..") || t.contains('\\') || t.contains("//") {
        return None;
    }
    let name = t.strip_prefix("/uploads/")?;
    if name.is_empty() || name.contains('/') || name.contains('?') || name.contains('#') {
        return None;
    }
    Some(t)
}

fn upload_img_src(path: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        api_url(path)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        path.to_string()
    }
}

fn user_upload_images_html(urls: &[String]) -> String {
    let mut imgs = String::new();
    for url in urls {
        let Some(path) = safe_chat_upload_path(url) else {
            continue;
        };
        let src = plaintext_to_safe_html(&upload_img_src(path));
        imgs.push_str("<img class=\"chat-tui-user-img\" src=\"");
        imgs.push_str(&src);
        imgs.push_str("\" alt=\"\" referrerpolicy=\"no-referrer\" />");
    }
    if imgs.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<div class=\"chat-tui-user-images\" data-testid=\"chat-user-upload-images\">",
    );
    out.push_str(&imgs);
    out.push_str("</div>");
    out
}

pub(crate) fn append_user_upload_images(chunks: &mut TuiBodyChunks, urls: &[String]) {
    let html = user_upload_images_html(urls);
    if !html.is_empty() {
        chunks.closed.push(html);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_segment_uploads() {
        assert_eq!(
            safe_chat_upload_path("/uploads/u1_2_3.png"),
            Some("/uploads/u1_2_3.png")
        );
        assert!(safe_chat_upload_path("/uploads/../x").is_none());
        assert!(safe_chat_upload_path("/uploads/a/b.png").is_none());
        assert!(safe_chat_upload_path("https://evil/uploads/a.png").is_none());
    }

    #[test]
    fn gallery_html_uses_uploads_path() {
        let html = user_upload_images_html(&["/uploads/a.png".into(), "/uploads/../x".into()]);
        assert!(html.contains("chat-tui-user-images"));
        assert!(html.contains("/uploads/a.png"));
        assert!(html.contains("referrerpolicy=\"no-referrer\""));
        assert!(!html.contains(".."));
    }
}
