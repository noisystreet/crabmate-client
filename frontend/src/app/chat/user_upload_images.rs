//! 用户气泡中的聊天附图（`/uploads/...`，与 composer 待发预览同源）。

use crate::app::chat::tui_line_markdown::TuiBodyChunks;
use crate::chat_upload_src::{chat_upload_filename, safe_chat_upload_path};
use crate::i18n::{self, Locale};
use crate::markdown::plaintext_to_safe_html;

fn user_upload_images_html(urls: &[String], locale: Locale) -> String {
    let mut imgs = String::new();
    for url in urls {
        let Some(path) = safe_chat_upload_path(url) else {
            continue;
        };
        let src = plaintext_to_safe_html(path);
        let alt = plaintext_to_safe_html(&i18n::chat_image_attachment_alt(
            locale,
            chat_upload_filename(path),
        ));
        imgs.push_str("<img class=\"chat-tui-user-img\" src=\"");
        imgs.push_str(&src);
        imgs.push_str("\" alt=\"");
        imgs.push_str(&alt);
        imgs.push_str("\" referrerpolicy=\"no-referrer\" />");
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

pub(crate) fn append_user_upload_images(
    chunks: &mut TuiBodyChunks,
    urls: &[String],
    locale: Locale,
) {
    let html = user_upload_images_html(urls, locale);
    if !html.is_empty() {
        chunks.closed.push(html);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_html_uses_uploads_path_and_alt() {
        let html = user_upload_images_html(
            &["/uploads/a.png".into(), "/uploads/../x".into()],
            Locale::En,
        );
        assert!(html.contains("chat-tui-user-images"), "{html}");
        assert!(html.contains("src=\"/uploads/a.png\""), "{html}");
        assert!(html.contains("alt=\"Attachment a.png\""), "{html}");
        assert!(html.contains("referrerpolicy=\"no-referrer\""));
        assert!(!html.contains(".."));
        assert!(!html.contains("https://"));
    }
}
