//! B3 hydration 双读：消费会话 `layout` 做差分观测；持久化水合行保持 legacy id。
//!
//! 流式活键（`turn-commentary-*` / `turn-final-answer`）只属于本回合投影，
//! 不得 stamp 到 `GET /conversation/messages` 还原的历史行，否则会误触发
//! 「保留本地 v2、禁止整页替换」。差分只记行数 / 角色序 / 文本 hash。

use serde::Deserialize;

use crate::storage::{CURRENT_LAYOUT_SCHEMA_VERSION, StoredMessage};

/// 与服务端 `ConversationLayoutSegment` JSON 对齐。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConversationLayoutSegment {
    #[serde(default)]
    pub turn_id: Option<String>,
    pub segment_id: String,
    pub segment_kind: String,
    #[serde(default)]
    pub before_tool_call_id: Option<String>,
    pub sequence: u32,
}

/// 与服务端 `ConversationLayoutMeta` JSON 对齐。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConversationLayoutMeta {
    pub layout_schema_version: u32,
    #[serde(default)]
    pub projection_hash: Option<String>,
    #[serde(default)]
    pub segments: Vec<ConversationLayoutSegment>,
}

/// 脱敏差分快照（不含全文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationDiffSnapshot {
    pub row_count: usize,
    pub role_order: String,
    pub text_hash: String,
}

/// 水合行来源：有 v2 `layout` 仍走 legacy 行，只观测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrationRowSource {
    Legacy,
    LayoutObserved,
}

/// 双读对照（layout 段 vs 非 user 水合行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationDualRead {
    pub source: HydrationRowSource,
    pub projected: HydrationDiffSnapshot,
    pub layout_segment_count: Option<usize>,
    pub layout_kind_order: Option<String>,
    pub projection_hash: Option<String>,
    pub segment_count_matches: bool,
    pub mapped_role_order_matches: bool,
}

/// schema ≥ 2 的 `layout` 可供双读观测（即使 `segments` 为空）。
#[must_use]
pub fn layout_prefers_projection(meta: Option<&ConversationLayoutMeta>) -> bool {
    meta.is_some_and(|m| m.layout_schema_version >= CURRENT_LAYOUT_SCHEMA_VERSION)
}

/// 非 user 行的差分指纹（与 layout 段对照；FNV-1a 64 十六进制）。
#[must_use]
pub fn fingerprint_projected_rows(messages: &[StoredMessage]) -> HydrationDiffSnapshot {
    fingerprint_rows(messages.iter().filter(|m| m.role != "user"))
}

/// 布局段 kind 序（不含 user）。
#[must_use]
pub fn layout_kind_order(meta: &ConversationLayoutMeta) -> String {
    meta.segments
        .iter()
        .map(|s| s.segment_kind.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// 对照 GET `layout` 与当前水合行；不改 id / schema。
#[must_use]
pub fn dual_read_hydration(
    messages: &[StoredMessage],
    layout: Option<&ConversationLayoutMeta>,
) -> HydrationDualRead {
    let projected = fingerprint_projected_rows(messages);
    let Some(meta) = layout else {
        return legacy_dual_read(projected);
    };
    if !layout_prefers_projection(Some(meta)) {
        return legacy_dual_read(projected);
    }
    let kind_order = layout_kind_order(meta);
    let mapped = mapped_role_order(meta);
    HydrationDualRead {
        source: HydrationRowSource::LayoutObserved,
        segment_count_matches: meta.segments.len() == projected.row_count,
        mapped_role_order_matches: mapped == projected.role_order,
        projected,
        layout_segment_count: Some(meta.segments.len()),
        layout_kind_order: Some(kind_order),
        projection_hash: meta.projection_hash.clone(),
    }
}

/// 双读并打脱敏 debug 日志。
pub fn observe_hydration_dual_read(
    messages: &[StoredMessage],
    layout: Option<&ConversationLayoutMeta>,
) -> HydrationDualRead {
    let report = dual_read_hydration(messages, layout);
    crate::layout_debug_counters::note_hydration_dual_read(&format!(
        "source={:?} rows={} segs={} count_match={} role_match={} text_hash={} projection_hash={}",
        report.source,
        report.projected.row_count,
        report.layout_segment_count.unwrap_or(0),
        report.segment_count_matches,
        report.mapped_role_order_matches,
        report.projected.text_hash,
        report.projection_hash.as_deref().unwrap_or("-")
    ));
    report
}

fn legacy_dual_read(projected: HydrationDiffSnapshot) -> HydrationDualRead {
    HydrationDualRead {
        source: HydrationRowSource::Legacy,
        projected,
        layout_segment_count: None,
        layout_kind_order: None,
        projection_hash: None,
        segment_count_matches: true,
        mapped_role_order_matches: true,
    }
}

fn fingerprint_rows<'a, I>(rows: I) -> HydrationDiffSnapshot
where
    I: Iterator<Item = &'a StoredMessage>,
{
    let rows: Vec<&StoredMessage> = rows.collect();
    let role_order = rows
        .iter()
        .map(|m| m.role.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut payload = String::new();
    for m in &rows {
        payload.push_str(&m.role);
        payload.push('\n');
        payload.push_str(&m.text);
        payload.push('\n');
    }
    HydrationDiffSnapshot {
        row_count: rows.len(),
        role_order,
        text_hash: format!("{:016x}", fnv1a64(payload.as_bytes())),
    }
}

fn mapped_role_order(meta: &ConversationLayoutMeta) -> String {
    meta.segments
        .iter()
        .map(|s| mapped_role_for_segment_kind(&s.segment_kind))
        .collect::<Vec<_>>()
        .join(",")
}

fn mapped_role_for_segment_kind(kind: &str) -> &str {
    if kind == "tool" {
        "tool"
    } else if kind.starts_with("assistant") {
        "assistant"
    } else {
        kind
    }
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationLayoutMeta, ConversationLayoutSegment, HydrationRowSource, dual_read_hydration,
        fingerprint_projected_rows, layout_kind_order, layout_prefers_projection,
    };
    use crate::storage::StoredMessage;

    fn seg(kind: &str, tool: Option<&str>, seq: u32) -> ConversationLayoutSegment {
        ConversationLayoutSegment {
            turn_id: Some("u0".into()),
            segment_id: format!("seg-{seq}"),
            segment_kind: kind.into(),
            before_tool_call_id: tool.map(str::to_string),
            sequence: seq,
        }
    }

    fn msg(
        id: &str,
        role: &str,
        text: &str,
        is_tool: bool,
        tool_call_id: Option<&str>,
    ) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: role.into(),
            text: text.into(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool,
            tool_call_id: tool_call_id.map(str::to_string),
            tool_name: None,
            created_at: 0,
        }
    }

    fn v2_meta(segments: Vec<ConversationLayoutSegment>) -> ConversationLayoutMeta {
        ConversationLayoutMeta {
            layout_schema_version: 2,
            projection_hash: Some("aa".into()),
            segments,
        }
    }

    #[test]
    fn parses_server_layout_json() {
        let raw = r#"{
            "layout_schema_version":2,
            "projection_hash":"aa",
            "segments":[{
                "turn_id":"u0",
                "segment_id":"seg-before-tc1",
                "segment_kind":"assistant_commentary",
                "before_tool_call_id":"tc1",
                "sequence":0
            }]
        }"#;
        let meta: ConversationLayoutMeta = serde_json::from_str(raw).unwrap();
        assert!(layout_prefers_projection(Some(&meta)));
        assert_eq!(layout_kind_order(&meta), "assistant_commentary");
    }

    #[test]
    fn missing_layout_is_legacy() {
        assert!(!layout_prefers_projection(None));
        let report = dual_read_hydration(&[], None);
        assert_eq!(report.source, HydrationRowSource::Legacy);
        assert!(report.segment_count_matches);
    }

    #[test]
    fn projected_fingerprint_skips_user_and_is_stable() {
        let rows = vec![
            msg("u", "user", "hi", false, None),
            msg("a", "assistant", "ok", false, None),
        ];
        let a = fingerprint_projected_rows(&rows);
        let b = fingerprint_projected_rows(&rows);
        assert_eq!(a, b);
        assert_eq!(a.row_count, 1);
        assert_eq!(a.role_order, "assistant");
        assert_eq!(a.text_hash.len(), 16);
    }

    #[test]
    fn dual_read_with_layout_does_not_require_v2_row_ids() {
        let meta = v2_meta(vec![
            seg("assistant_commentary", Some("tc1"), 0),
            seg("tool", None, 1),
        ]);
        let rows = vec![
            msg("u", "user", "go", false, None),
            msg("h_a", "assistant", "先读。", false, None),
            msg("h_t", "tool", "read", true, Some("tc1")),
        ];
        let report = dual_read_hydration(&rows, Some(&meta));
        assert_eq!(report.source, HydrationRowSource::LayoutObserved);
        assert!(report.segment_count_matches);
        assert!(report.mapped_role_order_matches);
        assert_eq!(rows[1].id, "h_a");
        assert_eq!(rows[2].id, "h_t");
    }
}
