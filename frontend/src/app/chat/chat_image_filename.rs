//! 灯箱另存为文件名：只保留安全的单段光栅名。

use crate::chat_upload_src::{chat_upload_filename, safe_chat_upload_path};
use crate::markdown::workspace_image::is_workspace_raw_img_src;

const DEFAULT_NAME: &str = "image.png";
const MAX_NAME_CHARS: usize = 80;

/// 从水合 `data-cm-ws-raw` 与 alt 推出可下载文件名。
#[must_use]
pub fn lightbox_download_filename(raw_src: Option<&str>, alt: &str) -> String {
    if let Some(raw) = raw_src.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(path) = safe_chat_upload_path(raw) {
            return sanitize_image_filename(chat_upload_filename(path));
        }
        if let Some(name) = workspace_raw_basename(raw) {
            return sanitize_image_filename(&name);
        }
    }
    sanitize_image_filename(alt)
}

fn workspace_raw_basename(src: &str) -> Option<String> {
    if !is_workspace_raw_img_src(src) {
        return None;
    }
    let qs = src.split_once('?')?.1;
    for part in qs.split('&') {
        let Some(v) = part.strip_prefix("path=") else {
            continue;
        };
        let decoded = urlencoding::decode(v).ok()?;
        let name = decoded.rsplit('/').next()?.trim();
        if name.is_empty() {
            return None;
        }
        return Some(name.to_string());
    }
    None
}

#[must_use]
pub fn sanitize_image_filename(raw: &str) -> String {
    let last = raw
        .rsplit(['/', '\\', ' ', ':', '（', '）'])
        .find(|s| !s.trim().is_empty())
        .unwrap_or(raw);
    let mut out = String::new();
    for c in last.chars() {
        if out.chars().count() >= MAX_NAME_CHARS {
            break;
        }
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
            out.push(c);
        }
    }
    let out = out.trim_matches('.').to_string();
    if out.is_empty() {
        return DEFAULT_NAME.to_string();
    }
    if has_raster_ext(&out) {
        out
    } else {
        format!("{out}.png")
    }
}

fn has_raster_ext(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif"
        )
}

#[cfg(test)]
mod tests {
    use super::{lightbox_download_filename, sanitize_image_filename};

    #[test]
    fn sanitize_strips_path_and_keeps_ext() {
        assert_eq!(sanitize_image_filename("/tmp/../a.png"), "a.png");
        assert_eq!(sanitize_image_filename("foo BAR"), "BAR.png");
        assert_eq!(sanitize_image_filename(""), "image.png");
        assert_eq!(sanitize_image_filename("...."), "image.png");
    }

    #[test]
    fn filename_from_upload_raw_and_alt() {
        assert_eq!(
            lightbox_download_filename(Some("/uploads/shot.webp"), "x"),
            "shot.webp"
        );
        assert_eq!(lightbox_download_filename(None, "附图 cat.jpg"), "cat.jpg");
        assert_eq!(
            lightbox_download_filename(Some("/workspace/file/raw?path=plots%2Fa.png"), "alt"),
            "a.png"
        );
        assert_eq!(
            lightbox_download_filename(Some("/uploads/../x"), "附图"),
            "image.png"
        );
    }
}
