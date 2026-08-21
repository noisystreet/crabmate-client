//! 聊天附图与工作区图：仅相对 API 路径可走壳内 Bearer fetch。

use crate::markdown::workspace_image::is_workspace_raw_img_src;

/// 仅接受单段文件名的 **`/uploads/<file>`**（与 Server `normalize_chat_image_urls` 一致）。
#[must_use]
pub fn safe_chat_upload_path(url: &str) -> Option<&str> {
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

/// 可带鉴权拉取的相对路径：工作区 raw 或聊天 `/uploads/`。
#[must_use]
pub fn relative_auth_image_src(src: &str) -> Option<&str> {
    let t = src.trim();
    if is_workspace_raw_img_src(t) {
        return Some(t);
    }
    safe_chat_upload_path(t)
}

#[must_use]
pub fn chat_upload_filename(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
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
    fn relative_auth_allows_workspace_and_uploads() {
        assert_eq!(
            relative_auth_image_src("/uploads/a.png"),
            Some("/uploads/a.png")
        );
        assert_eq!(
            relative_auth_image_src("/workspace/file/raw?path=plots%2Fa.png"),
            Some("/workspace/file/raw?path=plots%2Fa.png")
        );
        assert!(relative_auth_image_src("https://example.com/a.png").is_none());
    }
}
