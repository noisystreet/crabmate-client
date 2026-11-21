//! 用水合页的会话级 `layout` 把历史行升级成 v2 投影键（#545）。
//!
//! `GET /conversation/messages` 的 `layout` 是**会话级**元数据（不随 `limit` /
//! `before_index` 对 `messages` 切片），服务端对 v2 会话稳定写入
//! `layout_schema_version = 2`，并逐段给出 `segment_kind` / `before_tool_call_id` / `turn_id`。
//! 本模块只做一件事：把同一页的历史行**按段对齐**后改写成稳定行键
//! （`turn-commentary-*` / `turn-tool-*` / `turn-timeline-*`），让刷新既不整段重建、也不再把
//! 整页 id 换成本次加载的 `h_{毫秒}_{序号}`。
//!
//! # 严格全有或全无
//!
//! 段数、kind 序、工具 id、旁白锚点任一对不上就整体返回
//! [`LayoutProjection::Fallback`]，调用方继续走 legacy（服务端快照权威）路径。**不**做部分
//! 映射：半信半疑地 stamp v2 会让后续水合误判「本地已有可复用投影」，比不映射更糟。
//!
//! # 不动的行
//!
//! - `assistant` 空正文行（只带 reasoning）：服务端 `layout` 不产段，无法对齐，保持原 id。
//! - 页首那个**不完整回合**（其 user 行不在本页，`window_start_index > 0`）：无法校验，
//!   整段保持原 id。
//! - 终答行：键的规范化独立于旁白（见 [`V2_FINAL_ANSWER_ROW_ID`] 的回合独占语义）。

use std::collections::HashMap;

use crabmate::cm_api_contract::chat::{
    CONVERSATION_LAYOUT_SCHEMA_VERSION_V2, ConversationLayoutMeta, ConversationLayoutSegment,
};

use crate::session_model::{StoredMessage, V2_COMMENTARY_ROW_ID_PREFIX};

/// 服务端 `layout` 里「首条计次用户之前」那一组的 `turn_id`。
const LEAD_TURN_ID: &str = "lead";
/// 布局段 kind：工具行。
const SEGMENT_KIND_TOOL: &str = "tool";
/// 布局段 kind：工具前旁白。
const SEGMENT_KIND_COMMENTARY: &str = "assistant_commentary";
/// 布局段 kind：终答。
const SEGMENT_KIND_ANSWER: &str = "assistant_answer";
/// 布局段 kind：时间线旁注。
const SEGMENT_KIND_TIMELINE: &str = "assistant_timeline";
/// 水合侧自建的工具行前缀（流式路径的工具行是随机 id，不与之共用键）。
const V2_TOOL_ROW_ID_PREFIX: &str = "turn-tool-";
/// 水合侧自建的时间线行前缀（按 `turn_id` 分段，故跨回合不撞键）。
const V2_TIMELINE_ROW_ID_PREFIX: &str = "turn-timeline-";
/// 同一投影键在更早回合出现时的归档后缀（与流式路径 `turn_row_queue` 同形）。
const ARCHIVED_ROW_SUFFIX: &str = "#prev";

/// 映射失败原因；调用方据此记一条观测并**不** stamp v2。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProjectionFallback {
    /// 响应里没有会话级 `layout`（老会话 / 元数据已清）。
    NoLayout,
    /// `layout_schema_version < 2`：段键不稳定，不能当投影键。
    LayoutSchemaTooOld,
    /// 本页没有任何可映射行（全被过滤或只有空正文）。
    NoProjectableRows,
    /// 本地回合数与 `layout` 回合数对不上。
    TurnCountMismatch,
    /// 某回合的段序列 / 工具 id / 旁白锚点对不上。
    TurnShapeMismatch,
}

/// 映射结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutProjection {
    /// 已按 `layout` 改写稳定行键（其余字段原样）。
    Mapped(Vec<StoredMessage>),
    /// 对不齐，调用方保持 legacy 行为。
    Fallback(LayoutProjectionFallback),
}

/// 本地可投影行的种类（与 `layout` 的段 kind 一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalKind {
    Timeline,
    Tool,
    Commentary,
    Answer,
}

/// 一条可投影的历史行。
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalRow {
    /// 在输入 `rows` 里的下标。
    index: usize,
    kind: LocalKind,
    /// 工具行 / 旁白的 `tool_call_id`（旁白取紧邻工具行的 id）。
    tool_call_id: Option<String>,
}

/// 一条被判定过的投影键：要么改写，要么保持原 id（如终答）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyDecision {
    Key(String),
    Keep,
}

/// `layout` 的回合分组（相邻同 `turn_id` 的段归一组）。
struct LayoutTurn<'a> {
    turn_id: Option<&'a str>,
    segments: Vec<&'a ConversationLayoutSegment>,
}

/// 按 `layout` 把 `rows` 改写成稳定 v2 行键。
///
/// `page_covers_session_start` 为 `true` 表示本页从会话首条开始
/// （调用方传 `window_start_index == 0`）；只有此时页首的 `lead` 组才可被校验。
#[must_use]
pub fn project_hydrated_rows(
    rows: &[StoredMessage],
    layout: Option<&ConversationLayoutMeta>,
    page_covers_session_start: bool,
) -> LayoutProjection {
    let Some(meta) = layout else {
        return LayoutProjection::Fallback(LayoutProjectionFallback::NoLayout);
    };
    if meta.layout_schema_version < CONVERSATION_LAYOUT_SCHEMA_VERSION_V2 {
        return LayoutProjection::Fallback(LayoutProjectionFallback::LayoutSchemaTooOld);
    }
    let (lead, turns) = group_rows_by_turn(rows);
    let layout_turns = group_layout_turns(meta);
    let planned = plan_page(
        rows,
        &lead,
        &turns,
        &layout_turns,
        page_covers_session_start,
    );
    match planned {
        Ok((_plan, 0)) => LayoutProjection::Fallback(LayoutProjectionFallback::NoProjectableRows),
        Ok((plan, _)) => LayoutProjection::Mapped(apply_plan(rows, plan)),
        Err(reason) => LayoutProjection::Fallback(reason),
    }
}

/// 把行切成「首条 user 之前的前缀」+「每个 user 起一个回合」。
fn group_rows_by_turn(rows: &[StoredMessage]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut lead: Vec<usize> = Vec::new();
    let mut turns: Vec<Vec<usize>> = Vec::new();
    for (index, message) in rows.iter().enumerate() {
        if is_user_row(message) {
            turns.push(vec![index]);
        } else if let Some(current) = turns.last_mut() {
            current.push(index);
        } else {
            lead.push(index);
        }
    }
    (lead, turns)
}

fn is_user_row(message: &StoredMessage) -> bool {
    message.role == "user" && !message.is_tool
}

fn group_layout_turns(meta: &ConversationLayoutMeta) -> Vec<LayoutTurn<'_>> {
    let mut out: Vec<LayoutTurn<'_>> = Vec::new();
    for segment in &meta.segments {
        let same_turn = out
            .last()
            .is_some_and(|turn| turn.turn_id == segment.turn_id.as_deref());
        if let Some(last) = out.last_mut().filter(|_| same_turn) {
            last.segments.push(segment);
        } else {
            out.push(LayoutTurn {
                turn_id: segment.turn_id.as_deref(),
                segments: vec![segment],
            });
        }
    }
    out
}

/// 产出「行下标 → 新键」计划 + 可投影行总数；任一回合对不齐即整体失败。
fn plan_page(
    rows: &[StoredMessage],
    lead: &[usize],
    turns: &[Vec<usize>],
    layout_turns: &[LayoutTurn<'_>],
    page_covers_session_start: bool,
) -> Result<(Vec<(usize, String)>, usize), LayoutProjectionFallback> {
    let lead_verified = page_covers_session_start && !lead.is_empty();
    let offset = align_offset(turns.len(), layout_turns.len(), lead_verified)?;
    let mut plan: Vec<(usize, String)> = Vec::new();
    let mut projectable = 0usize;
    if lead_verified {
        let lead_turn = layout_turns
            .first()
            .filter(|turn| turn.turn_id == Some(LEAD_TURN_ID))
            .ok_or(LayoutProjectionFallback::TurnCountMismatch)?;
        projectable += plan_turn(rows, lead, lead_turn, &mut plan)?;
    }
    for (index, group) in turns.iter().enumerate() {
        let layout_turn = layout_turns
            .get(offset + index)
            .ok_or(LayoutProjectionFallback::TurnCountMismatch)?;
        projectable += plan_turn(rows, group, layout_turn, &mut plan)?;
    }
    Ok((plan, projectable))
}

/// 本地回合窗口在 `layout` 回合列表里的起点。
///
/// 覆盖会话首条时两侧必须**完全等长**（含 `lead` 组）；只覆盖尾部时按尾部对齐，
/// 更早的 `layout` 回合（本页未加载）不参与校验。
fn align_offset(
    local_turns: usize,
    layout_turns: usize,
    lead_verified: bool,
) -> Result<usize, LayoutProjectionFallback> {
    let expected = local_turns + usize::from(lead_verified);
    if layout_turns == expected {
        return Ok(usize::from(lead_verified));
    }
    if !lead_verified && layout_turns > local_turns {
        return Ok(layout_turns - local_turns);
    }
    Err(LayoutProjectionFallback::TurnCountMismatch)
}

fn plan_turn(
    rows: &[StoredMessage],
    group: &[usize],
    layout_turn: &LayoutTurn<'_>,
    plan: &mut Vec<(usize, String)>,
) -> Result<usize, LayoutProjectionFallback> {
    let local = projectable_rows(rows, group);
    let local_timelines = local
        .iter()
        .filter(|row| row.kind == LocalKind::Timeline)
        .count();
    let layout_timelines = layout_turn
        .segments
        .iter()
        .filter(|seg| seg.segment_kind == SEGMENT_KIND_TIMELINE)
        .count();
    if local_timelines != layout_timelines {
        return Err(LayoutProjectionFallback::TurnShapeMismatch);
    }
    push_timeline_keys(&local, layout_turn, plan);
    plan_sequence(&local, layout_turn, plan)?;
    Ok(local.len())
}

/// 时间线行按 `turn_id` + 回合内序号取键：跨回合不会撞键，故无需归档编号。
fn push_timeline_keys(
    local: &[LocalRow],
    layout_turn: &LayoutTurn<'_>,
    plan: &mut Vec<(usize, String)>,
) {
    let turn_id = layout_turn.turn_id.unwrap_or(LEAD_TURN_ID);
    for (ordinal, row) in local
        .iter()
        .filter(|row| row.kind == LocalKind::Timeline)
        .enumerate()
    {
        plan.push((
            row.index,
            format!("{V2_TIMELINE_ROW_ID_PREFIX}{turn_id}-{ordinal}"),
        ));
    }
}

fn plan_sequence(
    local: &[LocalRow],
    layout_turn: &LayoutTurn<'_>,
    plan: &mut Vec<(usize, String)>,
) -> Result<(), LayoutProjectionFallback> {
    let local_seq: Vec<&LocalRow> = local
        .iter()
        .filter(|row| row.kind != LocalKind::Timeline)
        .collect();
    let layout_seq: Vec<&ConversationLayoutSegment> = layout_turn
        .segments
        .iter()
        .filter(|seg| seg.segment_kind != SEGMENT_KIND_TIMELINE)
        .copied()
        .collect();
    if local_seq.len() != layout_seq.len() {
        return Err(LayoutProjectionFallback::TurnShapeMismatch);
    }
    for (row, segment) in local_seq.iter().zip(layout_seq) {
        match decide_key(row, segment) {
            Some(KeyDecision::Key(key)) => plan.push((row.index, key)),
            Some(KeyDecision::Keep) => {}
            None => return Err(LayoutProjectionFallback::TurnShapeMismatch),
        }
    }
    Ok(())
}

/// 单行判定：`None` = 形状不符（整页回落）。
fn decide_key(row: &LocalRow, segment: &ConversationLayoutSegment) -> Option<KeyDecision> {
    match (row.kind, segment.segment_kind.as_str()) {
        (LocalKind::Tool, SEGMENT_KIND_TOOL) => tool_key(row, segment),
        (LocalKind::Commentary, SEGMENT_KIND_COMMENTARY) => commentary_key(row, segment),
        (LocalKind::Answer, SEGMENT_KIND_ANSWER) => Some(KeyDecision::Keep),
        _ => None,
    }
}

fn tool_key(row: &LocalRow, segment: &ConversationLayoutSegment) -> Option<KeyDecision> {
    let server_id = segment.segment_id.trim();
    if server_id.is_empty() {
        return None;
    }
    match trimmed(row.tool_call_id.as_deref()) {
        Some(local_id) if local_id != server_id => None,
        _ => Some(KeyDecision::Key(format!(
            "{V2_TOOL_ROW_ID_PREFIX}{server_id}"
        ))),
    }
}

/// 旁白键由锚点工具 id 决定（与流式路径 `turn-commentary-{tool_call_id}` 同形）。
fn commentary_key(row: &LocalRow, segment: &ConversationLayoutSegment) -> Option<KeyDecision> {
    let anchor = trimmed(segment.before_tool_call_id.as_deref())?;
    if trimmed(row.tool_call_id.as_deref()) != Some(anchor) {
        return None;
    }
    Some(KeyDecision::Key(format!(
        "{V2_COMMENTARY_ROW_ID_PREFIX}{anchor}"
    )))
}

fn projectable_rows(rows: &[StoredMessage], group: &[usize]) -> Vec<LocalRow> {
    let classified: Vec<(usize, LocalKind, Option<String>)> = group
        .iter()
        .filter_map(|&index| classify_row(&rows[index]).map(|(kind, id)| (index, kind, id)))
        .collect();
    classified
        .iter()
        .enumerate()
        .map(|(position, (index, kind, tool_call_id))| {
            let next = classified.get(position + 1);
            let next_is_tool = next.is_some_and(|item| item.1 == LocalKind::Tool);
            if *kind == LocalKind::Answer && next_is_tool {
                LocalRow {
                    index: *index,
                    kind: LocalKind::Commentary,
                    tool_call_id: next.and_then(|item| item.2.clone()),
                }
            } else {
                LocalRow {
                    index: *index,
                    kind: *kind,
                    tool_call_id: tool_call_id.clone(),
                }
            }
        })
        .collect()
}

/// 服务端 `layout` 只为「时间线 / 有正文的助手 / 工具」产段，其余行不参与对齐。
fn classify_row(message: &StoredMessage) -> Option<(LocalKind, Option<String>)> {
    if message.is_tool {
        return Some((
            LocalKind::Tool,
            trimmed(message.tool_call_id.as_deref()).map(str::to_string),
        ));
    }
    match message.role.as_str() {
        "system" => Some((LocalKind::Timeline, None)),
        "assistant" if !message.text.trim().is_empty() => Some((LocalKind::Answer, None)),
        _ => None,
    }
}

fn apply_plan(rows: &[StoredMessage], plan: Vec<(usize, String)>) -> Vec<StoredMessage> {
    let numbered = number_duplicate_keys(plan);
    let mut out: Vec<StoredMessage> = rows.to_vec();
    for (index, key) in numbered {
        if let Some(row) = out.get_mut(index) {
            row.id = key;
        }
    }
    out
}

/// 同一键在页内出现多次（模型跨回合复用 `tool_call_id` 等）：更早的让出规范键，
/// 归档序号随时间递增——与流式路径 `archive_stale_projection_rows` 的产物同形。
fn number_duplicate_keys(plan: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut positions: HashMap<String, Vec<usize>> = HashMap::new();
    for (position, (_, key)) in plan.iter().enumerate() {
        positions.entry(key.clone()).or_default().push(position);
    }
    let mut out = plan;
    for (key, slots) in positions {
        let total = slots.len();
        if total < 2 {
            continue;
        }
        for (ordinal, position) in slots.into_iter().enumerate().take(total - 1) {
            out[position].1 = format!("{key}{ARCHIVED_ROW_SUFFIX}{}", ordinal + 1);
        }
    }
    out
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{LayoutProjection, LayoutProjectionFallback, project_hydrated_rows};
    use crate::session_model::StoredMessage;
    use crabmate::cm_api_contract::chat::{ConversationLayoutMeta, ConversationLayoutSegment};

    fn message(id: &str, role: &str, text: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: role.into(),
            text: text.into(),
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    fn tool_row(id: &str, tool_call_id: &str) -> StoredMessage {
        StoredMessage {
            is_tool: true,
            tool_call_id: Some(tool_call_id.into()),
            ..message(id, "tool", "read")
        }
    }

    fn timeline_row(id: &str, text: &str) -> StoredMessage {
        StoredMessage {
            ..message(id, "system", text)
        }
    }

    fn segment(
        turn: &str,
        kind: &str,
        segment_id: &str,
        anchor: Option<&str>,
        seq: u32,
    ) -> ConversationLayoutSegment {
        ConversationLayoutSegment {
            turn_id: Some(turn.into()),
            segment_id: segment_id.into(),
            segment_kind: kind.into(),
            before_tool_call_id: anchor.map(str::to_string),
            sequence: seq,
        }
    }

    fn meta(segments: Vec<ConversationLayoutSegment>) -> ConversationLayoutMeta {
        ConversationLayoutMeta {
            layout_schema_version: 2,
            projection_hash: Some("deadbeef".into()),
            segments,
        }
    }

    /// 服务端对同一条 assistant 消息（正文 + tool_calls）产「旁白 + 工具」两段。
    fn one_tool_turn(turn: &str, tcid: &str, seq: u32) -> Vec<ConversationLayoutSegment> {
        vec![
            segment(
                turn,
                "assistant_commentary",
                &format!("seg-before-{tcid}"),
                Some(tcid),
                seq,
            ),
            segment(turn, "tool", tcid, None, seq + 1),
        ]
    }

    fn ids(rows: &[StoredMessage]) -> Vec<&str> {
        rows.iter().map(|row| row.id.as_str()).collect()
    }

    fn mapped(
        rows: &[StoredMessage],
        layout: &ConversationLayoutMeta,
        start: bool,
    ) -> Vec<StoredMessage> {
        match project_hydrated_rows(rows, Some(layout), start) {
            LayoutProjection::Mapped(rows) => rows,
            other => panic!("expected mapped, got {other:?}"),
        }
    }

    #[test]
    fn no_layout_or_old_schema_falls_back() {
        let rows = vec![message("h_0", "assistant", "hi")];
        assert_eq!(
            project_hydrated_rows(&rows, None, true),
            LayoutProjection::Fallback(LayoutProjectionFallback::NoLayout)
        );
        let mut old = meta(vec![]);
        old.layout_schema_version = 1;
        assert_eq!(
            project_hydrated_rows(&rows, Some(&old), true),
            LayoutProjection::Fallback(LayoutProjectionFallback::LayoutSchemaTooOld)
        );
    }

    #[test]
    fn commentary_and_tool_rows_get_stable_keys() {
        let rows = vec![
            message("u", "user", "go"),
            message("h_1", "assistant", "先读。"),
            tool_row("h_2", "tc1"),
        ];
        let layout = meta(one_tool_turn("u0", "tc1", 0));
        let out = mapped(&rows, &layout, true);
        assert_eq!(ids(&out), vec!["u", "turn-commentary-tc1", "turn-tool-tc1"]);
    }

    #[test]
    fn answer_row_keeps_hydration_id() {
        let rows = vec![
            message("u", "user", "go"),
            message("h_1", "assistant", "答"),
        ];
        let layout = meta(vec![segment(
            "u0",
            "assistant_answer",
            "assistant_answer-0",
            None,
            0,
        )]);
        let out = mapped(&rows, &layout, true);
        assert_eq!(ids(&out), vec!["u", "h_1"]);
    }

    #[test]
    fn timeline_rows_are_turn_scoped_and_counted() {
        let rows = vec![
            timeline_row("h_lead", "启动"),
            message("u", "user", "go"),
            timeline_row("h_tl", "中间旁注"),
            message("h_a", "assistant", "答"),
        ];
        let layout = meta(vec![
            segment(
                "lead",
                "assistant_timeline",
                "assistant_timeline-0",
                None,
                0,
            ),
            segment("u0", "assistant_timeline", "assistant_timeline-1", None, 1),
            segment("u0", "assistant_answer", "assistant_answer-2", None, 2),
        ]);
        let out = mapped(&rows, &layout, true);
        assert_eq!(
            ids(&out),
            vec!["turn-timeline-lead-0", "u", "turn-timeline-u0-0", "h_a"]
        );
    }

    #[test]
    fn reasoning_only_assistant_row_is_not_projectable() {
        let rows = vec![
            message("u", "user", "go"),
            StoredMessage {
                reasoning_text: "想一想".into(),
                ..message("h_think", "assistant", "")
            },
            message("h_a", "assistant", "答"),
        ];
        let layout = meta(vec![segment(
            "u0",
            "assistant_answer",
            "assistant_answer-0",
            None,
            0,
        )]);
        let out = mapped(&rows, &layout, true);
        assert_eq!(ids(&out), vec!["u", "h_think", "h_a"]);
    }

    #[test]
    fn kind_order_mismatch_falls_back() {
        let rows = vec![
            message("u", "user", "go"),
            tool_row("h_t", "tc1"),
            message("h_a", "assistant", "答"),
        ];
        let layout = meta(vec![
            segment(
                "u0",
                "assistant_commentary",
                "seg-before-tc1",
                Some("tc1"),
                0,
            ),
            segment("u0", "tool", "tc1", None, 1),
        ]);
        assert_eq!(
            project_hydrated_rows(&rows, Some(&layout), true),
            LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
        );
    }

    #[test]
    fn tool_id_mismatch_falls_back() {
        let rows = vec![
            message("u", "user", "go"),
            message("h_1", "assistant", "先读。"),
            tool_row("h_2", "tc9"),
        ];
        let layout = meta(one_tool_turn("u0", "tc1", 0));
        assert_eq!(
            project_hydrated_rows(&rows, Some(&layout), true),
            LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
        );
    }

    #[test]
    fn turn_count_mismatch_falls_back() {
        let rows = vec![
            message("u1", "user", "q1"),
            message("h_1", "assistant", "a1"),
            message("u2", "user", "q2"),
            message("h_2", "assistant", "a2"),
        ];
        let layout = meta(vec![segment(
            "u0",
            "assistant_answer",
            "assistant_answer-0",
            None,
            0,
        )]);
        assert_eq!(
            project_hydrated_rows(&rows, Some(&layout), true),
            LayoutProjection::Fallback(LayoutProjectionFallback::TurnCountMismatch)
        );
    }

    #[test]
    fn repeated_tool_call_id_across_turns_archives_earlier_turn() {
        let rows = vec![
            message("u1", "user", "q1"),
            message("h_1", "assistant", "先读。"),
            tool_row("h_2", "tc1"),
            message("u2", "user", "q2"),
            message("h_3", "assistant", "再读。"),
            tool_row("h_4", "tc1"),
        ];
        let mut segments = one_tool_turn("u0", "tc1", 0);
        segments.extend(one_tool_turn("u1", "tc1", 2));
        let layout = meta(segments);
        let out = mapped(&rows, &layout, true);
        assert_eq!(
            ids(&out),
            vec![
                "u1",
                "turn-commentary-tc1#prev1",
                "turn-tool-tc1#prev1",
                "u2",
                "turn-commentary-tc1",
                "turn-tool-tc1",
            ]
        );
    }

    #[test]
    fn windowed_page_maps_only_aligned_tail_turns() {
        // 页首是上一回合的尾巴（user 行不在本页）：整段保持原 id，只映射完整回合。
        let rows = vec![
            message("h_old_1", "assistant", "旧旁白"),
            tool_row("h_old_2", "tc0"),
            message("u2", "user", "q2"),
            message("h_3", "assistant", "再读。"),
            tool_row("h_4", "tc1"),
        ];
        let mut segments = one_tool_turn("u0", "tc0", 0);
        segments.extend(one_tool_turn("u1", "tc1", 2));
        let layout = meta(segments);
        let out = mapped(&rows, &layout, false);
        assert_eq!(
            ids(&out),
            vec![
                "h_old_1",
                "h_old_2",
                "u2",
                "turn-commentary-tc1",
                "turn-tool-tc1",
            ]
        );
    }

    #[test]
    fn mapping_is_idempotent() {
        let rows = vec![
            message("u", "user", "go"),
            message("h_1", "assistant", "先读。"),
            tool_row("h_2", "tc1"),
        ];
        let layout = meta(one_tool_turn("u0", "tc1", 0));
        let first = mapped(&rows, &layout, true);
        let second = mapped(&first, &layout, true);
        assert_eq!(ids(&first), ids(&second));
    }

    #[test]
    fn batch_narration_segment_is_not_silently_accepted() {
        let rows = vec![
            message("u", "user", "go"),
            message("h_1", "assistant", "批说明"),
        ];
        let layout = meta(vec![segment(
            "u0",
            "assistant_batch_narration",
            "assistant_batch_narration-0",
            None,
            0,
        )]);
        assert_eq!(
            project_hydrated_rows(&rows, Some(&layout), true),
            LayoutProjection::Fallback(LayoutProjectionFallback::TurnShapeMismatch)
        );
    }
}
