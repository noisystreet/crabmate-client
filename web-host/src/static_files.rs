//! 回环静态文件：防 `..` 穿越，WASM 使用 `application/wasm`。
//!
//! 服务层直接用 `std::net`（不复用 tiny_http）：每个连接恰好处理一个请求后
//! 关闭（`Connection: close`），并设置读写超时 —— 浏览器 reload 时若复用已被
//! 服务端丢弃（read 错误 / 半关闭）的 keep-alive 连接，请求会永远等不到响应。

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// 读写超时：stuck 连接（浏览器中止 / 半关闭）在此后报错回收，避免挂死。
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// 请求行 + 全部头部的总上限，防无界读取。
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// 处理一个连接上的单个请求；结束后连接由调用方关闭。
pub fn serve_connection(stream: TcpStream, root: &Path) -> Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let (method, target) = match read_request_line(&mut reader)? {
        Some(pair) => pair,
        None => return Ok(()), // 连接被对端直接关闭（EOF），无事可做
    };

    read_and_discard_headers(&mut reader, method.len() + target.len() + 2)?;

    match (method.as_str(), target) {
        ("GET" | "HEAD", target) => serve_file_response(stream, root, &target, method == "HEAD"),
        _ => respond(
            stream,
            405,
            "method not allowed",
            b"method not allowed",
            "text/plain; charset=utf-8",
            false,
        ),
    }
}

/// 读剩余请求头直到空行；超限报错（连接关闭）。
fn read_and_discard_headers(reader: &mut impl BufRead, mut head_bytes: usize) -> Result<()> {
    loop {
        let line = read_line_limited(reader)?;
        head_bytes += line.len();
        if head_bytes > MAX_HEAD_BYTES {
            bail!("request head too large");
        }
        if line.trim().is_empty() {
            return Ok(());
        }
    }
}

/// 映射 URL 到文件并回响应；HEAD 只回头部。
fn serve_file_response(
    stream: TcpStream,
    root: &Path,
    target: &str,
    head_only: bool,
) -> Result<()> {
    let Some(path) = map_url_to_file(root, target) else {
        return respond(
            stream,
            404,
            "not found",
            b"not found",
            "text/plain; charset=utf-8",
            false,
        );
    };
    let body = match fs::read(&path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("warn: read {}: {e}", path.display());
            return respond(
                stream,
                404,
                "not found",
                b"not found",
                "text/plain; charset=utf-8",
                false,
            );
        }
    };
    respond(stream, 200, "OK", &body, mime_for_path(&path), head_only)
}

/// 写一个 HTTP/1.1 响应；`Connection: close`，HEAD 请求不发正文（Content-Length 仍如实给出）。
fn respond(
    mut stream: TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
    mime: &str,
    head_only: bool,
) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .with_context(|| format!("write head {status}"))?;
    if !head_only {
        stream
            .write_all(body)
            .with_context(|| format!("write body {status}"))?;
    }
    stream.flush().context("flush")?;
    Ok(())
}

/// 读请求行 `METHOD SP TARGET SP HTTP/x.y`；EOF 时返回 `None`。
fn read_request_line(reader: &mut impl BufRead) -> Result<Option<(String, String)>> {
    let line = read_line_limited(reader)?;
    if line.is_empty() {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/");
    if method.is_empty() {
        bail!("empty method");
    }
    Ok(Some((method, target.to_string())))
}

fn read_line_limited(reader: &mut impl BufRead) -> Result<String> {
    let mut buf = String::with_capacity(128);
    let n = reader.read_line(&mut buf).map_err(|e| {
        if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
            anyhow::anyhow!("read timeout")
        } else {
            anyhow::anyhow!("read: {e}")
        }
    })?;
    if n == 0 {
        return Ok(String::new());
    }
    if buf.len() > MAX_HEAD_BYTES {
        bail!("request head too large");
    }
    Ok(buf)
}

/// 把 URL 路径映射到 `root` 下的文件；目录落到 `index.html`。
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

/// 按目标文件的扩展名给出响应 `Content-Type`。
pub fn mime_for_path(path: &Path) -> &'static str {
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

    #[test]
    fn read_request_line_parses_method_target() {
        let mut cursor =
            std::io::Cursor::new("GET /index.html HTTP/1.1\r\nHost: x\r\n\r\n".as_bytes());
        let (method, target) = read_request_line(&mut cursor).expect("line").expect("some");
        assert_eq!(method, "GET");
        assert_eq!(target, "/index.html");
    }

    #[test]
    fn read_request_line_eof_is_none() {
        let mut cursor = std::io::Cursor::new("".as_bytes());
        assert!(read_request_line(&mut cursor).expect("ok").is_none());
    }
}
