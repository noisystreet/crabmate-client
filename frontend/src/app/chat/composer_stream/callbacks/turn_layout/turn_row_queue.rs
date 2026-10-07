//! v2 布局：流式 delta → loading overlay preview；已关闭 commentary 按工具键不可变落盘。

use crabmate::cm_turn_layout::{ASSISTANT_COMMENTARY, project_turn_projection};

use crate::message_loading::is_loading_plain_assistant;
use crate::storage::{V2_COMMENTARY_ROW_ID_PREFIX, V2_FINAL_ANSWER_ROW_ID, V2_THINK_ROW_ID_PREFIX};

use super::super::super::turn_canonical::TurnCanonicalState;
use super::projection_reconciler;

/// 工具批结束后终答块的稳定 id（与 `project_turn_web` · `assistant_answer` 对应）。
pub(crate) const FINAL_ANSWER_ROW_ID: &str = V2_FINAL_ANSWER_ROW_ID;

pub(crate) fn commentary_row_id(tool_call_id: &str) -> String {
    format!("{V2_COMMENTARY_ROW_ID_PREFIX}{tool_call_id}")
}

pub(crate) fn is_commentary_row_id(message_id: &str) -> bool {
    message_id.starts_with(V2_COMMENTARY_ROW_ID_PREFIX)
}

/// 工具前思维链（reasoning）的稳定行 id：锚定工具**之前**，避免随 loading 尾泡下移。
pub(crate) fn think_row_id(tool_call_id: &str) -> String {
    format!("{V2_THINK_ROW_ID_PREFIX}{tool_call_id}")
}

/// 上一回合遗留的同键投影行的归档后缀。
///
/// `turn-commentary-{tool_call_id}` / `turn-think-{tool_call_id}` 只在**本回合**内唯一：
/// 模型跨回合复用同一 `tool_call_id` 时，直接 upsert 会把上一回合的行改写成本回合的。仿
/// [`super::TurnLayout::detach_final_answer_projection`] / [`super::projection_reconciler::detach_final_answer_row_in_messages`]，让历史行让出规范键；
/// 仍保留各自前缀，故 `is_commentary_row_id` 与 v2 缓存识别不受影响。
const ARCHIVED_ROW_SUFFIX: &str = "#prev";

/// 本回合起点（最后一条 user 行之后）。
fn current_turn_start(messages: &[crate::storage::StoredMessage]) -> usize {
    messages
        .iter()
        .rposition(|m| m.role == "user")
        .map_or(0, |idx| idx + 1)
}

/// 本回合内 `row_id` 所在下标；上一回合的同键行不算命中。
pub(super) fn current_turn_position(
    messages: &[crate::storage::StoredMessage],
    row_id: &str,
) -> Option<usize> {
    let start = current_turn_start(messages);
    messages[start..]
        .iter()
        .position(|m| m.id == row_id)
        .map(|idx| idx + start)
}

fn current_turn_tool_position(
    messages: &[crate::storage::StoredMessage],
    tool_call_id: &str,
) -> Option<usize> {
    let start = current_turn_start(messages);
    messages[start..]
        .iter()
        .position(|m| m.is_tool && m.tool_call_id.as_deref() == Some(tool_call_id))
        .map(|idx| idx + start)
}

/// 新锚定的 think 行应落的下标：同键 commentary 行若已在工具前，则插入其前，以保持
/// think → commentary → 工具的流式顺序；否则直接落在工具行前。
fn anchor_before_tool_index(
    messages: &[crate::storage::StoredMessage],
    tool_call_id: &str,
) -> Option<usize> {
    let tool_idx = current_turn_tool_position(messages, tool_call_id)?;
    let commentary_idx = current_turn_position(messages, commentary_row_id(tool_call_id).as_str())
        .filter(|&idx| idx < tool_idx);
    Some(commentary_idx.unwrap_or(tool_idx))
}

/// 让上一回合遗留的同键投影行让出规范键，避免本回合 upsert 覆盖其正文。
fn archive_stale_projection_rows(messages: &mut [crate::storage::StoredMessage], row_id: &str) {
    let start = current_turn_start(messages);
    let stale: Vec<usize> = (0..start)
        .filter(|&idx| messages[idx].id == row_id)
        .collect();
    for idx in stale {
        let archived = next_archived_row_id(messages, row_id);
        messages[idx].id = archived;
    }
}

fn next_archived_row_id(messages: &[crate::storage::StoredMessage], row_id: &str) -> String {
    let mut seq = 1_usize;
    loop {
        let candidate = format!("{row_id}{ARCHIVED_ROW_SUFFIX}{seq}");
        if messages.iter().all(|m| m.id != candidate) {
            return candidate;
        }
        seq += 1;
    }
}

/// 工具前**投影行**的内容种类。
///
/// 行 id、正文字段、锚定下标全部由本枚举收口：新增一种工具前投影行**只需加一个枚举值**
/// （补 `row_id` / `anchor_index` / `text_slot` / `new_row` 各一臂），不必再复制
/// 「归档 → upsert → 误落搬回」这套顺序敏感逻辑——那正是 thinking 气泡漂移 bug 的复发面。
#[derive(Clone, Copy, Debug)]
enum ProjectionRowKind {
    /// 工具批说明（正文写 `text`），锚定在工具行**正前**。
    Commentary,
    /// 工具前思维链（正文写 `reasoning_text`），锚定在 commentary **之前**（无则工具行前）。
    Reasoning,
}

impl ProjectionRowKind {
    fn row_id(self, tool_call_id: &str) -> String {
        match self {
            Self::Commentary => commentary_row_id(tool_call_id),
            Self::Reasoning => think_row_id(tool_call_id),
        }
    }

    /// 新行 / 误落后搬回时应落的下标（均在工具行之前）。
    fn anchor_index(
        self,
        messages: &[crate::storage::StoredMessage],
        tool_call_id: &str,
        tool_idx: usize,
    ) -> Option<usize> {
        match self {
            // 说明块紧跟工具。
            Self::Commentary => Some(tool_idx),
            // 思维链在说明块之前；说明块尚未落盘时退回工具行前。
            Self::Reasoning => anchor_before_tool_index(messages, tool_call_id),
        }
    }

    fn text_slot(self, row: &mut crate::storage::StoredMessage) -> &mut String {
        match self {
            Self::Commentary => &mut row.text,
            Self::Reasoning => &mut row.reasoning_text,
        }
    }

    fn new_row(self, tool_call_id: &str, content: String) -> crate::storage::StoredMessage {
        match self {
            Self::Commentary => {
                TurnRowQueue::new_commentary_row(self.row_id(tool_call_id), content)
            }
            Self::Reasoning => TurnRowQueue::new_assistant_projection_row(
                self.row_id(tool_call_id),
                String::new(),
                content,
            ),
        }
    }
}

/// 流式 preview / 边界 flush 队列。
#[derive(Default, Debug)]
pub(crate) struct TurnRowQueue;

impl TurnRowQueue {
    /// 将工具前投影行 upsert 到锚定工具行之前（可更新正文；若误落在工具后则搬回）。
    ///
    /// `kind` 决定行 id、正文字段与锚定位置（见 [`ProjectionRowKind`]）。返回是否已锚定。
    fn upsert_projection_before_tool(
        messages: &mut Vec<crate::storage::StoredMessage>,
        kind: ProjectionRowKind,
        tool_call_id: &str,
        content: String,
    ) -> bool {
        if content.trim().is_empty() {
            return false;
        }
        let Some(tool_idx) = current_turn_tool_position(messages, tool_call_id) else {
            return false;
        };
        let row_id = kind.row_id(tool_call_id);
        archive_stale_projection_rows(messages, row_id.as_str());
        if let Some(idx) = current_turn_position(messages, row_id.as_str()) {
            let slot = kind.text_slot(&mut messages[idx]);
            if *slot != content {
                *slot = content;
            }
            if idx > tool_idx {
                let row = messages.remove(idx);
                let new_idx = kind
                    .anchor_index(messages, tool_call_id, tool_idx)
                    .unwrap_or(tool_idx);
                messages.insert(new_idx, row);
            }
            return true;
        }
        let Some(anchor) = kind.anchor_index(messages, tool_call_id, tool_idx) else {
            return false;
        };
        messages.insert(anchor, kind.new_row(tool_call_id, content));
        true
    }

    /// 工具批说明（commentary）upsert 到工具行前；见 [`Self::upsert_projection_before_tool`]。
    ///
    /// 用于：已关闭旁注 flush，以及晚到 open 旁注在工具行已存在时的流式预览。
    pub(super) fn upsert_commentary_before_tool(
        messages: &mut Vec<crate::storage::StoredMessage>,
        tool_call_id: &str,
        text: String,
    ) -> bool {
        Self::upsert_projection_before_tool(
            messages,
            ProjectionRowKind::Commentary,
            tool_call_id,
            text,
        )
    }

    /// 工具前思维链（reasoning）upsert 到工具行前；见 [`Self::upsert_projection_before_tool`]。
    ///
    /// 行 id 为 [`think_row_id`]，正文存于 `reasoning_text`（`text` 留空），故该行渲染为
    /// 独立思考段气泡，且不参与 commentary / 终答的正文所有权判定。
    pub(super) fn upsert_reasoning_before_tool(
        messages: &mut Vec<crate::storage::StoredMessage>,
        tool_call_id: &str,
        reasoning: String,
    ) -> bool {
        Self::upsert_projection_before_tool(
            messages,
            ProjectionRowKind::Reasoning,
            tool_call_id,
            reasoning,
        )
    }

    fn new_commentary_row(row_id: String, text: String) -> crate::storage::StoredMessage {
        Self::new_assistant_projection_row(row_id, text, String::new())
    }

    fn new_assistant_projection_row(
        row_id: String,
        text: String,
        reasoning_text: String,
    ) -> crate::storage::StoredMessage {
        crate::storage::StoredMessage {
            id: row_id,
            role: "assistant".to_string(),
            text,
            reasoning_text,
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: {
                #[cfg(target_arch = "wasm32")]
                {
                    crate::session_ops::message_created_ms()
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    0
                }
            },
        }
    }

    /// Phase B：open 锚定旁白直接写 `turn-commentary-*`（工具尚不存在则暂挂在 loading 前）。
    ///
    /// 工具到达后由 [`Self::upsert_commentary_before_tool`] / flush 搬到工具前。
    pub(super) fn upsert_streaming_anchored_commentary(
        messages: &mut Vec<crate::storage::StoredMessage>,
        tool_call_id: &str,
        text: String,
        loading_tail_id: Option<&str>,
    ) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        if current_turn_tool_position(messages, tool_call_id).is_some() {
            return Self::upsert_commentary_before_tool(messages, tool_call_id, text);
        }
        let row_id = commentary_row_id(tool_call_id);
        archive_stale_projection_rows(messages, row_id.as_str());
        let insert_idx = Self::insert_index_before_loading_tail(messages, loading_tail_id);
        if let Some(idx) = current_turn_position(messages, row_id.as_str()) {
            if messages[idx].text != text {
                messages[idx].text = text;
            }
            if let Some(load_id) = loading_tail_id.filter(|t| !t.is_empty())
                && let Some(load_idx) = messages.iter().position(|m| m.id == load_id)
                && idx > load_idx
            {
                let row = messages.remove(idx);
                let new_load = messages
                    .iter()
                    .position(|m| m.id == load_id)
                    .unwrap_or(messages.len());
                messages.insert(new_load, row);
            }
            return true;
        }
        messages.insert(
            insert_idx.min(messages.len()),
            Self::new_commentary_row(row_id, text),
        );
        true
    }

    /// loading 尾泡 overlay：**仅**未落盘终答（或无锚点的短暂 open 段）。
    ///
    /// 带 `before_tool_call_id` 的 open commentary 一律由
    /// [`Self::upsert_streaming_anchored_commentary`] 承载，此处返回空。
    pub(super) fn loading_preview_text(
        turn: &TurnCanonicalState,
        overlay_answer: Option<&str>,
        _messages: Option<&[crate::storage::StoredMessage]>,
    ) -> String {
        if turn.tool_phase_open() {
            let projection = project_turn_projection(turn.turn_ref());
            match projection.active_row.as_ref() {
                Some(active) if active.before_tool_call_id.is_some() => String::new(),
                Some(active) if active.kind == ASSISTANT_COMMENTARY => active.text.clone(),
                _ => String::new(),
            }
        } else {
            overlay_answer
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .unwrap_or_default()
        }
    }

    pub(super) fn insert_index_before_loading_tail(
        messages: &[crate::storage::StoredMessage],
        loading_tail_id: Option<&str>,
    ) -> usize {
        if let Some(id) = loading_tail_id.filter(|t| !t.is_empty()) {
            if let Some(idx) = messages.iter().position(|m| m.id == id) {
                return idx;
            }
        }
        messages.len()
    }

    pub(super) fn upsert_assistant_row(
        messages: &mut Vec<crate::storage::StoredMessage>,
        row_id: &str,
        text: String,
        insert_idx: usize,
    ) {
        if text.trim().is_empty() {
            return;
        }
        if let Some(idx) = messages.iter().position(|m| m.id == row_id) {
            if messages[idx].text != text {
                messages[idx].text = text.clone();
            }
            if messages[idx].tool_call_id.is_some() {
                messages[idx].tool_call_id = None;
            }
            if idx != insert_idx {
                let row = messages.remove(idx);
                let mut at = insert_idx;
                if idx < at {
                    at -= 1;
                }
                messages.insert(at.min(messages.len()), row);
            }
            return;
        }
        let row = crate::storage::StoredMessage {
            id: row_id.to_string(),
            role: "assistant".to_string(),
            text,
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: {
                #[cfg(target_arch = "wasm32")]
                {
                    crate::session_ops::message_created_ms()
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    0
                }
            },
        };
        messages.insert(insert_idx.min(messages.len()), row);
    }

    // dead: kept as insert-once helper for potential no-upsert paths; prefer upsert_*.
    #[allow(dead_code)]
    fn insert_finalized_assistant_row(
        messages: &mut Vec<crate::storage::StoredMessage>,
        row_id: &str,
        text: String,
        insert_idx: usize,
    ) {
        if text.trim().is_empty() || messages.iter().any(|message| message.id == row_id) {
            return;
        }
        let row = crate::storage::StoredMessage {
            id: row_id.to_string(),
            role: "assistant".to_string(),
            text,
            reasoning_text: String::new(),
            image_urls: vec![],
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: {
                #[cfg(target_arch = "wasm32")]
                {
                    crate::session_ops::message_created_ms()
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    0
                }
            },
        };
        messages.insert(insert_idx.min(messages.len()), row);
    }

    /// Web assistant 正文落盘入口（定稿 commentary + final → reconciler）。
    ///
    /// `overlay_answer`：当前 loading 尾泡的 overlay 正文（终答唯一来源）。
    /// `allow_final_answer`：为 false 时不写 `turn-final-answer`（工具前旁白仍在 overlay/loading，
    /// 避免 `turn_segment_end` 把旁白误刷进终答行，随后 demote/detach 造成助手气泡闪消失）。
    pub(super) fn sync_web_projection(
        &self,
        messages: &mut Vec<crate::storage::StoredMessage>,
        turn: &TurnCanonicalState,
        loading_tail_id: Option<&str>,
        overlay_answer: Option<&str>,
        allow_final_answer: bool,
    ) {
        projection_reconciler::reconcile_web_projection(
            messages,
            turn,
            loading_tail_id,
            overlay_answer,
            allow_final_answer,
        );
    }

    /// 工具相 open 锚定旁白：直接 upsert `turn-commentary-*`（工具可尚未存在）。
    ///
    /// 不要求 `tool_phase_open`：`turn_segment_start` 声明锚点后、`TOOL_CALL_START` 到达前
    /// 也须落出可见行，否则该窗口内 overlay 已清而 canonical 投影不出行（气泡闪没）。
    /// 段带 `before_tool_call_id` 即表明是某工具前的旁白，不会与终答混淆。
    pub(super) fn try_upsert_open_anchored_commentary(
        messages: &mut Vec<crate::storage::StoredMessage>,
        turn: &TurnCanonicalState,
        loading_tail_id: Option<&str>,
    ) -> bool {
        let projection = project_turn_projection(turn.turn_ref());
        projection_reconciler::try_reconcile_active_anchored_commentary(
            messages,
            &projection,
            loading_tail_id,
        )
    }

    /// preview 是否应写入 loading 尾泡（与 stored 一致则不再 duplicate）。
    pub(super) fn loading_preview_for_messages(
        turn: &TurnCanonicalState,
        messages: &[crate::storage::StoredMessage],
        overlay_answer: Option<&str>,
    ) -> String {
        let preview = Self::loading_preview_text(turn, overlay_answer, Some(messages));
        if preview.trim().is_empty() {
            return String::new();
        }
        if !turn.tool_phase_open() {
            if let Some(final_row) = messages.iter().find(|m| m.id == FINAL_ANSWER_ROW_ID) {
                if final_row.text.trim() == preview.trim() {
                    return String::new();
                }
            }
        }
        if !turn.tool_phase_open()
            && let Some(load) = messages.iter().find(|m| is_loading_plain_assistant(m))
            && load.text.trim() == preview.trim()
        {
            return String::new();
        }
        preview
    }
}

#[cfg(test)]
#[path = "turn_row_queue_tests.rs"]
mod tests;
