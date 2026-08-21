//! 最近连接的 `serve` 地址：写入应用数据目录，不依赖 `connect.html` 在 navigate 后仍能跑 JS。

use std::path::Path;

use crate::handoff::normalize_base_url;

pub const MAX_RECENT: usize = 8;
pub const RECENT_FILE_NAME: &str = "recent_connect_urls.json";

/// 与桌面建议地址（通常为 `http://127.0.0.1:8080/`）相同的目标不记入最近列表，
/// 避免每次自动登录把本机默认端口顶到最前。其它地址（含其它回环端口）都记。
/// 连接页「上次地址」更严：有建议 URL 时任意回环都不落盘（见 `connect.html` persistLastServerUrl）。
#[must_use]
pub fn should_record_recent(href: &str, suggested: Option<&str>) -> bool {
    let Ok(url) = normalize_base_url(href) else {
        return false;
    };
    let Some(raw) = suggested.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    match normalize_base_url(raw) {
        Ok(sug) => url.as_str() != sug.as_str(),
        Err(_) => true,
    }
}

#[must_use]
pub fn canonicalize_list(existing: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for item in existing {
        let Ok(u) = normalize_base_url(&item) else {
            continue;
        };
        let s = u.as_str().to_string();
        if !out.contains(&s) && out.len() < MAX_RECENT {
            out.push(s);
        }
    }
    out
}

#[must_use]
pub fn push_recent(existing: Vec<String>, href: &str) -> Vec<String> {
    let Ok(fresh) = normalize_base_url(href) else {
        return canonicalize_list(existing);
    };
    let key = fresh.as_str().to_string();
    let mut out = vec![key.clone()];
    for item in canonicalize_list(existing) {
        if item != key && out.len() < MAX_RECENT {
            out.push(item);
        }
    }
    out
}

#[must_use]
pub fn parse_recent_json(raw: &str) -> Vec<String> {
    let Ok(arr) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    canonicalize_list(arr)
}

#[must_use]
pub fn load_from_path(path: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_recent_json(&raw)
}

pub fn save_to_path(path: &Path, urls: &[String]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("recent urls dir: {e}"))?;
    }
    let body = serde_json::to_string(urls).map_err(|e| format!("recent urls json: {e}"))?;
    std::fs::write(path, body).map_err(|e| format!("recent urls write: {e}"))
}

pub fn record_success(path: &Path, href: &str, suggested: Option<&str>) {
    if !should_record_recent(href, suggested) {
        return;
    }
    let next = push_recent(load_from_path(path), href);
    if let Err(e) = save_to_path(path, &next) {
        eprintln!("[crabmate-connect] recent urls write skipped: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn push_recent_moves_existing_to_front() {
        let list = vec![
            "http://192.168.1.10:8080/".into(),
            "http://10.0.0.2:8080/".into(),
        ];
        let out = push_recent(list, "http://10.0.0.2:8080");
        assert_eq!(
            out,
            vec![
                "http://10.0.0.2:8080/".to_string(),
                "http://192.168.1.10:8080/".to_string()
            ]
        );
    }

    #[test]
    fn push_recent_appends_new_without_dropping_old() {
        let list = vec!["http://192.168.1.10:8080/".into()];
        let out = push_recent(list, "https://serve.example:8443");
        assert_eq!(
            out,
            vec![
                "https://serve.example:8443/".to_string(),
                "http://192.168.1.10:8080/".to_string()
            ]
        );
    }

    #[test]
    fn suggested_loopback_is_not_recorded() {
        assert!(!should_record_recent(
            "http://127.0.0.1:8080/",
            Some("http://127.0.0.1:8080/")
        ));
        assert!(should_record_recent(
            "http://127.0.0.1:9090/",
            Some("http://127.0.0.1:8080/")
        ));
        assert!(should_record_recent(
            "http://192.168.1.10:8080/",
            Some("http://127.0.0.1:8080/")
        ));
        assert!(should_record_recent("http://127.0.0.1:8080/", None));
    }

    #[test]
    fn record_success_persists_second_url() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cm-recent-urls-{nanos}"));
        let path = dir.join(RECENT_FILE_NAME);
        record_success(&path, "http://192.168.1.10:8080/", None);
        record_success(&path, "http://10.0.0.2:8080/", None);
        let got = load_from_path(&path);
        assert_eq!(
            got,
            vec![
                "http://10.0.0.2:8080/".to_string(),
                "http://192.168.1.10:8080/".to_string()
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
