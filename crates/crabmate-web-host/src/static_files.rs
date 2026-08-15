//! 回环静态文件：防 `..` 穿越，WASM 使用 `application/wasm`。

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use tiny_http::{Header, Method, Request, Response, StatusCode};

pub fn handle_request(root: &Path, request: Request) -> Result<()> {
    let method = request.method().clone();
    if method != Method::Get && method != Method::Head {
        return respond_text(request, 405, "method not allowed");
    }
    let url = request.url().to_string();
    let Some(path) = map_url_to_file(root, &url) else {
        return respond_text(request, 404, "not found");
    };
    serve_file(request, method, &path)
}

fn respond_text(request: Request, status: u16, body: &str) -> Result<()> {
    let resp = Response::from_string(body).with_status_code(StatusCode(status));
    request
        .respond(resp)
        .with_context(|| format!("respond {status}"))
}

fn content_type_header(mime: &str) -> Option<Header> {
    Header::from_bytes(&b"Content-Type"[..], mime.as_bytes()).ok()
}

fn serve_file(request: Request, method: Method, path: &Path) -> Result<()> {
    let header = content_type_header(mime_for_path(path));
    if method == Method::Head {
        let mut resp = Response::empty(StatusCode(200));
        if let Some(h) = header {
            resp = resp.with_header(h);
        }
        request.respond(resp).context("respond HEAD")?;
        return Ok(());
    }
    let body = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut resp = Response::from_data(body);
    if let Some(h) = header {
        resp = resp.with_header(h);
    }
    request.respond(resp).context("respond 200")?;
    Ok(())
}

fn map_url_to_file(root: &Path, url: &str) -> Option<PathBuf> {
    let path_only = url.split('?').next().unwrap_or(url);
    let decoded = percent_decode_path(path_only)?;
    let joined = join_under_root(root, &decoded)?;
    let target = if joined.is_dir() {
        joined.join("index.html")
    } else {
        joined
    };
    if !target.is_file() {
        return None;
    }
    confine_under_root(root, &target)
}

/// Follows symlinks, then requires the canonical path to stay under `root`.
fn confine_under_root(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let canon = path.canonicalize().ok()?;
    canon.starts_with(&root).then_some(canon)
}

fn join_under_root(root: &Path, url_path: &str) -> Option<PathBuf> {
    if url_path.contains('\0') {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let rel = url_path.trim_start_matches('/');
    if rel.is_empty() {
        return Some(root);
    }
    let mut candidate = root.clone();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => candidate.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(candidate)
}

fn percent_decode_path(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = from_hex(bytes[i + 1])?;
            let lo = from_hex(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

pub fn require_index(root: &Path) -> Result<()> {
    if !root.join("index.html").is_file() {
        bail!(
            "no index.html under {} (run make frontend-release, or set --root / CRABMATE_WEB_ROOT)",
            root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dist(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crabmate-web-host-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join("index.html"), "<html></html>").expect("index");
        fs::write(dir.join("app.wasm"), b"\0asm").expect("wasm");
        dir
    }

    #[test]
    fn serves_index_and_rejects_parent() {
        let dir = temp_dist("parent");
        assert!(map_url_to_file(&dir, "/").is_some());
        assert!(map_url_to_file(&dir, "/index.html").is_some());
        assert!(map_url_to_file(&dir, "/app.wasm").is_some());
        assert!(map_url_to_file(&dir, "/../Cargo.toml").is_none());
        assert!(map_url_to_file(&dir, "/%2e%2e/Cargo.toml").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = temp_dist("symlink");
        let outside = dir
            .parent()
            .expect("parent")
            .join(format!("crabmate-web-host-outside-{}", std::process::id()));
        fs::write(&outside, "secret").expect("outside");
        let link = dir.join("escape.txt");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        assert!(map_url_to_file(&dir, "/escape.txt").is_none());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn wasm_mime() {
        assert_eq!(mime_for_path(Path::new("x.wasm")), "application/wasm");
    }
}
