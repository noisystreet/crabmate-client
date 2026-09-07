//! `GET /health` JSON 子集（无 HTTP；不含壳 CORS）。
//!
//! 镜像瘦身（0.5.2）：响应形状直接使用契约 `HealthReportView`（不进 client-api 公开面），
//! client 仅保留 degraded 摘要文案逻辑。

use crabmate::cm_api_contract::HealthReportView;

/// 解析 `/health` JSON：`status=degraded` 时返回失败检查名摘要（不含密钥等敏感值）。
///
/// 非 JSON、非 `degraded`、或形状不符契约（如单项缺 `ok`）时返回 `None`。
/// degraded 但缺 `checks` 键（契约 default 为空 map）视为无失败项，返回 `Some("status=degraded")`。
#[must_use]
pub fn health_degraded_note(body: &str) -> Option<String> {
    let v: HealthReportView = serde_json::from_str(body).ok()?;
    if v.status != "degraded" {
        return None;
    }
    let mut failed = Vec::new();
    for (name, check) in &v.checks {
        if check.ok {
            continue;
        }
        let detail = check
            .detail
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match detail {
            Some(d) => failed.push(format!("{name}: {d}")),
            None => failed.push(name.clone()),
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
