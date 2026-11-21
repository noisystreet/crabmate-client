//! 用水合页的会话级 `layout` 把历史行升级成 v2 投影键（#545）。
//!
//! `GET /conversation/messages` 的 `layout` 是**会话级**元数据（不随 `limit` /
//! `before_index` 对 `messages` 切片），服务端对 v2 会话稳定写入
//! `layout_schema_version = 2`，并逐段给出 `segment_kind` / `before_tool_call_id` / `turn_id`。
//! 本模块只做一件事：把同一页的历史行**按段对齐**后改写成稳定行键
//! （`turn-commentary-*` / `turn-tool-*` / `turn-timeline-*` / `turn-final-answer#prev{n}`），
//! 让刷新既不整段重建、也不再把整页 id 换成本次加载的 `h_{毫秒}_{序号}`。
//!
//! # 严格全有或全无
//!
//! 段数、kind 序、工具 id、旁白锚点任一对不上就整体返回
//! [`LayoutProjection::Fallback`]，调用方继续走 legacy（服务端快照权威）路径。**不**做部分
//! 映射：半信半疑地 stamp v2 会让后续水合误判「本地已有可复用投影」，比不映射更糟。
//!
//! # 不动的行：只剩「无法对齐」的那一段
//!
//! 对齐成功时**本页所有行**都会拿到确定性 id：user 行 `turn-user-{turn_id}`、服务端不产段的
//! 行（`assistant` 空正文只带 reasoning 等）`turn-row-{turn_id}-{回合内下标}`、窗口页首半截
//! 回合 `turn-prev-{页内下标}`。留着 `h_{毫秒}_{序号}` 的话，纯问答会话每次刷新都会重新赋值并
//! 重写持久化——「刷新 N 次 id 序列不变」就不成立了。真正保持原 id 的只有「整页回落 legacy」
//! 这一种情况。
//!
//! # 终答行不占规范键：`turn-final-answer` 是流式路径**当前那一行**的独占键，水合历史行
//!   若也占用它，下一回合 `sync_turn_projection` 会 upsert 到历史行上，把已显示的旧正文挤掉。
//!   历史终答因此一律归档成 `turn-final-answer#prev{n}`（与回合收尾的
//!   `detach_final_answer_row_in_messages` 产物同形，故刷新前后 id 一致）；也正因为如此，
//!   `ChatSession::has_v2_finalized_rows` 只认规范键——纯问答会话 id 稳定了，但仍走 legacy
//!   的服务端快照替换路径（不 append-only，因为上下文压缩会真改历史）。

use std::collections::{HashMap, HashSet};

use crabmate::cm_api_contract::chat::{
    CONVERSATION_LAYOUT_SCHEMA_VERSION_V2, ConversationLayoutMeta, ConversationLayoutSegment,
};

use crate::session_model::{StoredMessage, V2_COMMENTARY_ROW_ID_PREFIX, V2_FINAL_ANSWER_ROW_ID};

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
/// 水合侧自建的 user 行前缀（`turn-user-{turn_id}`）：让 user 行也有确定性 id。
const V2_USER_ROW_ID_PREFIX: &str = "turn-user-";
/// 水合侧自建的兜底行前缀（`turn-row-{turn_id}-{回合内下标}`）：服务端 `layout` 不为这类行产段
/// （`assistant` 空正文只带 reasoning、未知 role 等）。
const V2_SCAFFOLD_ROW_ID_PREFIX: &str = "turn-row-";
/// 窗口页首那个**不完整回合**的前缀：其 user 行不在本页，`turn_id` 无从得知，故按页内下标编号。
/// 同一窗口重复加载产出同一批 id；窗口前移（新回合入页）时本就会换页。
const V2_PARTIAL_TURN_ROW_ID_PREFIX: &str = "turn-prev-";
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
    /// 本地比服务端少一条**工具行**（例如末回合只有 `assistant(text)+tool_calls`、
    /// 还没有 `role=tool` 结果，水合不会为它造短卡）：判定与回落行为同 `TurnShapeMismatch`，
    /// 单列只为观测——这条最容易在「回合被打断」时出现。
    MissingLocalToolRow,
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

/// 一条被判定过的投影键（`None` 表示形状不符，整页回落）。
type KeyDecision = String;

/// `layout` 的回合分组（相邻同 `turn_id` 的段归一组）。
struct LayoutTurn<'a> {
    turn_id: Option<&'a str>,
    segments: Vec<&'a ConversationLayoutSegment>,
}

/// 按 `layout` 把 `rows` 改写成稳定 v2 行键。
///
/// `page_start_index` 是本页首条行在服务端过滤数组中的下标（`window_start_index`）：
/// 为 `0` 时页面覆盖会话首条，页首 `lead` 组可被校验；否则只尾部对齐，页首半截回合的
/// 确定性键用 `turn-prev-{page_start_index}-{pos}`（带页命名空间，避免与尾页互撞）。
#[must_use]
pub fn project_hydrated_rows(
    rows: &[StoredMessage],
    layout: Option<&ConversationLayoutMeta>,
    page_start_index: u32,
) -> LayoutProjection {
    let Some(meta) = layout else {
        return LayoutProjection::Fallback(LayoutProjectionFallback::NoLayout);
    };
    if meta.layout_schema_version < CONVERSATION_LAYOUT_SCHEMA_VERSION_V2 {
        return LayoutProjection::Fallback(LayoutProjectionFallback::LayoutSchemaTooOld);
    }
    let (lead, turns) = group_rows_by_turn(rows);
    let layout_turns = group_layout_turns(meta);
    let planned = plan_page(rows, &lead, &turns, &layout_turns, page_start_index);
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
    page_start_index: u32,
) -> Result<(Vec<(usize, String)>, usize), LayoutProjectionFallback> {
    let lead_verified = page_start_index == 0 && !lead.is_empty();
    let offset = align_offset(turns.len(), layout_turns.len(), lead_verified)?;
    let mut plan: Vec<(usize, String)> = Vec::new();
    let mut projectable = 0usize;
    if lead_verified {
        let lead_turn = layout_turns
            .first()
            .filter(|turn| turn.turn_id == Some(LEAD_TURN_ID))
            .ok_or(LayoutProjectionFallback::TurnCountMismatch)?;
        projectable += plan_turn(rows, lead, lead_turn, &mut plan)?;
    } else if !lead.is_empty() {
        push_partial_turn_keys(lead, page_start_index, &mut plan);
    }
    for (index, group) in turns.iter().enumerate() {
        let layout_turn = layout_turns
            .get(offset + index)
            .ok_or(LayoutProjectionFallback::TurnCountMismatch)?;
        projectable += plan_turn(rows, group, layout_turn, &mut plan)?;
    }
    Ok((plan, projectable))
}

/// 窗口页首那个**不完整回合**（user 行不在本页）的确定性键：无法校验，但也不能留时间戳 id，
/// 否则这半截回合每次刷新都换 id，映射就白做了。
fn push_partial_turn_keys(lead: &[usize], page_start_index: u32, plan: &mut Vec<(usize, String)>) {
    for (position, &index) in lead.iter().enumerate() {
        plan.push((
            index,
            format!("{V2_PARTIAL_TURN_ROW_ID_PREFIX}{page_start_index}-{position}"),
        ));
    }
}

/// 更早一页（`before_index` 拉回来的 `window`）的行只做**页内**确定性编号。
///
/// 为什么不做语义键：`layout` 的 `turn_id` 是全会话序号，而分页请求只给
/// `window_start_index`（消息下标），客户端无法把「某页首条」映射到「第几个回合」；
/// 硬按尾页那套尾部对齐会给更早页配错 `turn_id`，比不做更糟。同一窗口重复加载
/// 产出同一批 id，所以 prepend 的 id 去重仍然幂等，刷新也不会把整块换掉。
pub fn assign_page_scoped_row_ids(rows: &mut [StoredMessage], page_start_index: u32) {
    for (position, row) in rows.iter_mut().enumerate() {
        row.id = format!("{V2_PARTIAL_TURN_ROW_ID_PREFIX}{page_start_index}-{position}");
    }
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

/// 每回合：校验（时间线只比数量、其余逐项比 kind + 锚点）后产出行键；
/// 末了补上 user 行与服务端不产段的行的确定性键（见 [`push_scaffold_keys`]）。
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
    let planned_from = plan.len();
    push_timeline_keys(&local, layout_turn, plan);
    plan_sequence(&local, layout_turn, plan)?;
    push_scaffold_keys(rows, group, layout_turn, plan, planned_from);
    Ok(local.len())
}

/// user 行与「服务端 `layout` 不产段」的行的确定性键。
///
/// 没有这一步，映射后的页里仍会留 `h_{毫秒}_{序号}`：纯问答会话（无旁白行 → 不 stamp v2）
/// 每次刷新都重新赋值 + 重写持久化，`#545` 的「刷新 N 次 id 序列不变」就名存实亡。
/// 键按**回合**（不是页内位置）派生：`turn-user-u2` / `turn-row-u2-3`（`3` 是该行在回合内
/// 的下标，与哪些行可投影无关，因此将来行种类变化也不会错位）。
fn push_scaffold_keys(
    rows: &[StoredMessage],
    group: &[usize],
    layout_turn: &LayoutTurn<'_>,
    plan: &mut Vec<(usize, String)>,
    planned_from: usize,
) {
    let turn_id = layout_turn.turn_id.unwrap_or(LEAD_TURN_ID);
    let planned: HashSet<usize> = plan[planned_from..].iter().map(|(idx, _)| *idx).collect();
    for (position, &index) in group.iter().enumerate() {
        if planned.contains(&index) {
            continue;
        }
        let key = if position == 0 && is_user_row(&rows[index]) {
            format!("{V2_USER_ROW_ID_PREFIX}{turn_id}")
        } else {
            format!("{V2_SCAFFOLD_ROW_ID_PREFIX}{turn_id}-{position}")
        };
        plan.push((index, key));
    }
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
        return Err(classify_sequence_mismatch(&local_seq, &layout_seq));
    }
    for (row, segment) in local_seq.iter().zip(layout_seq) {
        let Some(key) = decide_key(row, segment) else {
            return Err(LayoutProjectionFallback::TurnShapeMismatch);
        };
        plan.push((row.index, key));
    }
    Ok(())
}

/// 段数不符时细分原因（两者都回落，只为观测区分）。
fn classify_sequence_mismatch(
    local: &[&LocalRow],
    layout: &[&ConversationLayoutSegment],
) -> LayoutProjectionFallback {
    let local_tools = local
        .iter()
        .filter(|row| row.kind == LocalKind::Tool)
        .count();
    let layout_tools = layout
        .iter()
        .filter(|seg| seg.segment_kind == SEGMENT_KIND_TOOL)
        .count();
    if local.len() < layout.len() && local_tools < layout_tools {
        LayoutProjectionFallback::MissingLocalToolRow
    } else {
        LayoutProjectionFallback::TurnShapeMismatch
    }
}

/// 单行判定：`None` = 形状不符（整页回落）。
fn decide_key(row: &LocalRow, segment: &ConversationLayoutSegment) -> Option<KeyDecision> {
    match (row.kind, segment.segment_kind.as_str()) {
        (LocalKind::Tool, SEGMENT_KIND_TOOL) => tool_key(row, segment),
        (LocalKind::Commentary, SEGMENT_KIND_COMMENTARY) => commentary_key(row, segment),
        (LocalKind::Answer, SEGMENT_KIND_ANSWER) => Some(V2_FINAL_ANSWER_ROW_ID.to_string()),
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
        _ => Some(format!("{V2_TOOL_ROW_ID_PREFIX}{server_id}")),
    }
}

/// 旁白键由锚点工具 id 决定（与流式路径 `turn-commentary-{tool_call_id}` 同形）。
fn commentary_key(row: &LocalRow, segment: &ConversationLayoutSegment) -> Option<KeyDecision> {
    let anchor = trimmed(segment.before_tool_call_id.as_deref())?;
    if trimmed(row.tool_call_id.as_deref()) != Some(anchor) {
        return None;
    }
    Some(format!("{V2_COMMENTARY_ROW_ID_PREFIX}{anchor}"))
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
    let numbered = number_duplicate_keys(number_answer_keys(plan));
    let mut out: Vec<StoredMessage> = rows.to_vec();
    for (index, key) in numbered {
        if let Some(row) = out.get_mut(index) {
            row.id = key;
        }
    }
    out
}

/// 终答行**永不**占用规范键 `turn-final-answer`：那是流式路径当前那一行的独占键，水合若也占用，
/// 下一回合 `sync_turn_projection` 会 upsert 到历史行、把旧正文挤掉。历史终答一律按时间序
/// 归档为 `turn-final-answer#prev{n}`——与回合收尾的
/// `detach_final_answer_row_in_messages` 产物同形，故刷新前后 id 一致。
fn number_answer_keys(plan: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut ordinal = 1usize;
    let mut out = plan;
    for (_, key) in out.iter_mut() {
        if key == V2_FINAL_ANSWER_ROW_ID {
            *key = format!("{V2_FINAL_ANSWER_ROW_ID}{ARCHIVED_ROW_SUFFIX}{ordinal}");
            ordinal += 1;
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
#[path = "layout_projection_tests.rs"]
mod tests;
