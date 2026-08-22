//! `GET /health` JSON 子集（无 HTTP；不含壳 CORS）。

/// 解析 `/health` JSON：`status=degraded` 时返回失败检查名摘要（不含密钥等敏感值）。
///
/// 非 JSON、非 `degraded`、或缺 `checks` 对象时返回 `None`。
#[must_use]
pub fn health_degraded_note(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("status").and_then(|s| s.as_str()) != Some("degraded") {
        return None;
    }
    let checks = v.get("checks")?.as_object()?;
    let mut failed = Vec::new();
    for (name, check) in checks {
        let ok = check.get("ok").and_then(|x| x.as_bool()).unwrap_or(true);
        if !ok {
            let detail = check
                .get("detail")
                .and_then(|d| d.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match detail {
                Some(d) => failed.push(format!("{name}: {d}")),
                None => failed.push(name.clone()),
            }
        }
    }
    if failed.is_empty() {
        Some("status=degraded".into())
    } else {
        Some(failed.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::health_degraded_note;

    #[test]
    fn degraded_note_lists_failed_checks() {
        let body = r#"{"status":"degraded","checks":{"dep_bc":{"ok":false,"detail":"未安装"},"api_key":{"ok":true}}}"#;
        let note = health_degraded_note(body).expect("note");
        assert!(note.contains("dep_bc"));
        assert!(note.contains("未安装"));
        assert!(!note.contains("api_key"));
    }

    #[test]
    fn ok_status_yields_no_note() {
        let body = r#"{"status":"ok","checks":{}}"#;
        assert!(health_degraded_note(body).is_none());
    }

    #[test]
    fn degraded_without_failed_checks_uses_fallback() {
        let body = r#"{"status":"degraded","checks":{}}"#;
        assert_eq!(
            health_degraded_note(body).as_deref(),
            Some("status=degraded")
        );
    }

    #[test]
    fn invalid_json_yields_none() {
        assert!(health_degraded_note("not-json").is_none());
    }
}
