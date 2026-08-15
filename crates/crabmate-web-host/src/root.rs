//! UI 根目录探测与浏览器打开用的 URL。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// `.deb` 安装后的静态资源目录。
pub const INSTALLED_DIST: &str = "/usr/share/crabmate-web/dist";

#[must_use]
pub fn dist_has_index(dir: &Path) -> bool {
    dir.join("index.html").is_file()
}

/// `--root` / `CRABMATE_WEB_ROOT` / 安装路径 / 可执行文件旁 / 本仓 `frontend/dist`。
pub fn resolve_web_root(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return dist_has_index(p).then(|| p.to_path_buf());
    }
    if let Ok(env_root) = std::env::var("CRABMATE_WEB_ROOT") {
        let p = PathBuf::from(env_root);
        if dist_has_index(&p) {
            return Some(p);
        }
    }
    let installed = Path::new(INSTALLED_DIST);
    if dist_has_index(installed) {
        return Some(installed.to_path_buf());
    }
    if let Some(p) = dist_next_to_executable() {
        return Some(p);
    }
    compile_time_frontend_dist()
}

fn dist_next_to_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for rel in ["../share/crabmate-web/dist", "dist"] {
        let p = dir.join(rel);
        if dist_has_index(&p) {
            return p.canonicalize().ok();
        }
    }
    None
}

fn compile_time_frontend_dist() -> Option<PathBuf> {
    let manifest = option_env!("CARGO_MANIFEST_DIR")?;
    let p = Path::new(manifest).join("../../frontend/dist");
    dist_has_index(&p).then(|| p.canonicalize().ok()).flatten()
}

/// `xdg-open` 用的 host:port（`0.0.0.0` 改成 `127.0.0.1`）。
#[must_use]
pub fn browse_host(listen: SocketAddr) -> String {
    if listen.ip().is_unspecified() {
        format!("127.0.0.1:{}", listen.port())
    } else {
        listen.to_string()
    }
}

#[must_use]
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 打开 UI 的 URL；可选 hash 交接 `cm_api_base` / `cm_web_api_bearer`（与壳 connect 页相同）。
#[must_use]
pub fn page_url(listen: SocketAddr, api_base: Option<&str>, bearer: Option<&str>) -> String {
    let mut url = format!("http://{}/", browse_host(listen));
    let mut parts: Vec<String> = Vec::new();
    if let Some(base) = api_base.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("cm_api_base={}", percent_encode(base)));
    }
    if let Some(token) = bearer.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("cm_web_api_bearer={}", percent_encode(token)));
    }
    if !parts.is_empty() {
        url.push('#');
        url.push_str(&parts.join("&"));
    }
    url
}

/// Same as [`page_url`] but Bearer is `redacted` so logs/journald do not store the token.
#[must_use]
pub fn page_url_for_log(
    listen: SocketAddr,
    api_base: Option<&str>,
    bearer: Option<&str>,
) -> String {
    let redacted = bearer
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|_| "redacted");
    page_url(listen, api_base, redacted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn browse_unspecified_becomes_loopback() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4173);
        assert_eq!(browse_host(addr), "127.0.0.1:4173");
    }

    #[test]
    fn page_url_encodes_hash_handoff() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4173);
        let u = page_url(addr, Some("http://127.0.0.1:8080"), Some("a/b"));
        assert!(u.starts_with("http://127.0.0.1:4173/#"));
        assert!(u.contains("cm_api_base=http%3A%2F%2F127.0.0.1%3A8080"));
        assert!(u.contains("cm_web_api_bearer=a%2Fb"));
    }

    #[test]
    fn page_url_omits_empty_secrets() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4173);
        assert_eq!(page_url(addr, None, Some("  ")), "http://127.0.0.1:4173/");
    }

    #[test]
    fn page_url_for_log_redacts_bearer() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4173);
        let u = page_url_for_log(addr, Some("http://127.0.0.1:8080"), Some("s3cret/token"));
        assert!(u.contains("cm_api_base=http%3A%2F%2F127.0.0.1%3A8080"));
        assert!(u.contains("cm_web_api_bearer=redacted"));
        assert!(!u.contains("s3cret"));
    }
}
