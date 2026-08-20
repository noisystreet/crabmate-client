//! 助手 Markdown 图片：工作区相对路径 → `GET /workspace/file/raw`。

/// 与 Server `GET /workspace/file/raw` 对齐。
pub const WORKSPACE_FILE_RAW_PREFIX: &str = "/workspace/file/raw";

/// 将 Markdown 图片 dest 改写成可走鉴权拉取的相对 API 路径；非法相对路径清空以便 ammonia 剥掉。
pub fn rewrite_chat_image_dest(dest: &str) -> String {
    let t = dest.trim();
    if t.is_empty() {
        return String::new();
    }
    if is_http_url(t) {
        return t.to_string();
    }
    if let Some(rel) = rel_from_raw_url(t) {
        return match normalize_rel_image_path(&rel) {
            Some(p) => raw_url_for_rel(&p),
            None => String::new(),
        };
    }
    match normalize_rel_image_path(t) {
        Some(p) => raw_url_for_rel(&p),
        None => String::new(),
    }
}

fn is_http_url(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

fn rel_from_raw_url(t: &str) -> Option<String> {
    let marker = "/workspace/file/raw?";
    let idx = t.find(marker)?;
    let qs = &t[idx + marker.len()..];
    for part in qs.split('&') {
        let Some(v) = part.strip_prefix("path=") else {
            continue;
        };
        return urlencoding::decode(v).ok().map(|c| c.into_owned());
    }
    None
}

fn normalize_rel_image_path(t: &str) -> Option<String> {
    let mut s = t.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    if s.is_empty() || s.starts_with('/') || s.contains("..") || s.contains(':') {
        return None;
    }
    if !has_allowed_image_ext(&s) {
        return None;
    }
    Some(s)
}

fn has_allowed_image_ext(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty() {
        return false;
    }
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif"
    )
}

fn raw_url_for_rel(rel: &str) -> String {
    format!(
        "{WORKSPACE_FILE_RAW_PREFIX}?path={}",
        urlencoding::encode(rel)
    )
}

/// 仅相对路径 `/workspace/file/raw?…`（不要用 `contains`，以免把鉴权 fetch 打到外站）。
pub fn is_workspace_raw_img_src(src: &str) -> bool {
    let s = src.trim();
    s.starts_with("/workspace/file/raw?") && !s.contains("..")
}

#[cfg(test)]
mod tests {
    use super::{is_workspace_raw_img_src, rewrite_chat_image_dest};

    #[test]
    fn relative_png_becomes_raw_query() {
        assert_eq!(
            rewrite_chat_image_dest("plots/a.png"),
            "/workspace/file/raw?path=plots%2Fa.png"
        );
    }

    #[test]
    fn rejects_svg_and_dotdot() {
        assert!(rewrite_chat_image_dest("x.svg").is_empty());
        assert!(rewrite_chat_image_dest("../a.png").is_empty());
        assert!(rewrite_chat_image_dest("/etc/a.png").is_empty());
    }

    #[test]
    fn keeps_https() {
        assert_eq!(
            rewrite_chat_image_dest("https://example.com/p.png"),
            "https://example.com/p.png"
        );
    }

    #[test]
    fn detects_raw_src() {
        assert!(is_workspace_raw_img_src(
            "/workspace/file/raw?path=plots%2Fa.png"
        ));
        assert!(!is_workspace_raw_img_src("https://example.com/p.png"));
        assert!(
            !is_workspace_raw_img_src("https://evil.example/workspace/file/raw?path=plots%2Fa.png"),
            "must not treat third-party URLs as workspace raw"
        );
    }
}
