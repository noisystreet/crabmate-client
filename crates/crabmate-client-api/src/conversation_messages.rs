//! `GET /conversation/messages` 响应解析与「回放行」归一（纯逻辑，无 IO）。
//!
//! Web / Desktop 的水合层（`frontend/src/conversation_hydrate*.rs`）把同一份响应转成
//! 它自己的 `StoredMessage` 与时间线卡片；TUI 只需要**与 UI 无关**的中性形态：一段文本
//! 属于「用户 / 思考 / 助手 / 工具」中的哪一类。两边共享的就是这份「哪条消息出哪一行」
//! 的判定，避免第三次出现「两份规则漂移」。
//!
//! 隐藏规则里有一半在服务端：`filter_messages_for_web_client_snapshot` 已经滤掉长期记忆、
//! 变更集、规划纠偏、`role=tool` 的 `name` 注入条（见 ADR-0003）。剩下 **首轮工作区上下文**
//! 那条 `user` 消息仍在响应里（靠 `name` 区分），客户端必须自己跳过——此处与 frontend 同名
//! 常量保持一致，否则回放会凭空多一句用户发言。

use std::collections::HashMap;

use serde::Deserialize;

use crate::prompt_tokens::TiktokenPromptTokensSnapshot;
use serde_json::Value;

/// 首轮工作区上下文注入条的 `user.name`（Server `types.rs` 与 frontend 同名常量）。
pub const FIRST_TURN_WORKSPACE_CONTEXT_NAME: &str = "crabmate_first_turn_workspace_context";

/// 回放默认页大小（与后端默认 80 对齐；上限由服务端
/// `MAX_CONVERSATION_MESSAGES_PAGE_LIMIT` 约束）。
pub const CONVERSATION_MESSAGES_PAGE_LIMIT: u32 = 80;

/// `GET /conversation/messages` 响应。
///
/// 只声明回放要用的字段；`layout` / `context_artifacts` / `tiktoken_prompt_tokens` 等
/// Web 专用字段刻意不解析（未知字段被 `serde` 忽略，前向兼容）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConversationMessagesResponse {
    #[serde(default)]
    pub conversation_id: String,
    /// 服务端权威修订号（回放不合并本地副本，仅用于显示/诊断）。
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub messages: Vec<ConversationMessage>,
    /// 过滤后可见消息总数。
    #[serde(default)]
    pub total_count: u32,
    /// 本页第一条在过滤后数组中的下标（往旧翻页时回传 `before_index`）。
    #[serde(default)]
    pub window_start_index: u32,
    /// 本页之前是否还有更早的消息。
    #[serde(default)]
    pub has_older: bool,
    /// 会话当前的上下文用量快照（服务端随本响应附带，snake_case 键）。
    ///
    /// 新会话建立后这里就有值（system prompt + 工具 schema 的初始占用），所以水合即可显示
    /// 初始用量，不必等回合末的 `RUN_FINISHED` —— Web 端就是这么显示的。
    #[serde(default)]
    pub tiktoken_prompt_tokens: Option<TiktokenPromptTokensSnapshot>,
}

/// 一条 OpenAI 兼容消息（只取展示所需字段）。
#[derive(Debug, Clone, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default, alias = "reasoning")]
    pub reasoning_content: Option<String>,
    /// 服务端给的展示文本（正文）。
    #[serde(default)]
    pub display_content: Option<String>,
    /// 服务端给的展示文本（思考）。
    #[serde(default)]
    pub display_reasoning_content: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Value>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// 回放出来的一行（不含任何 UI 类型，调用方自行映射成自己的行模型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayRow {
    User(String),
    Thinking(String),
    Assistant(String),
    Tool { name: String, body: String },
}

impl ConversationMessagesResponse {
    /// 响应 → 有序回放行。
    ///
    /// 规则（与 Desktop 的可视结果对齐，但**不做**时间线卡片/折叠分组）：
    /// - `user`：跳过首轮工作区上下文注入条，其余照出；
    /// - `system`：跳过（`crabmate_ui_sep` / `crabmate_timeline` 是 Web 专用卡片）；
    /// - `assistant`：先思考后正文，两者皆空（只带 `tool_calls`）时不产行——工具结果
    ///   行会紧跟其后单独出现，再造一张「调用卡」在终端里只是噪声；
    /// - `tool`：`[工具] name` + 结果正文；
    /// - 未知 `role`：跳过（宁可少显示，也不把不认识的东西当正文印出来）。
    #[must_use]
    pub fn replay_rows(&self) -> Vec<ReplayRow> {
        // 工具**真实名字**只在 assistant 的 `tool_calls` 里保证有；`role=tool` 消息的
        // `name` 是服务端可选字段（实践中常常只带 `tool_call_id`）。先把调用侧的
        // id→name 收成一张表，否则回放会把 `call_xxx` 当工具名印出来。
        let tool_names = tool_call_names(&self.messages);
        let mut out = Vec::new();
        for msg in &self.messages {
            match msg.role.trim() {
                "user" => {
                    if msg.name.as_deref().map(str::trim) == Some(FIRST_TURN_WORKSPACE_CONTEXT_NAME)
                    {
                        continue;
                    }
                    let text = msg.display_text();
                    if !text.is_empty() {
                        out.push(ReplayRow::User(text));
                    }
                }
                "assistant" => {
                    let reasoning = msg.display_reasoning();
                    if !reasoning.is_empty() {
                        out.push(ReplayRow::Thinking(reasoning));
                    }
                    let text = msg.display_text();
                    if !text.is_empty() {
                        out.push(ReplayRow::Assistant(text));
                    }
                }
                "tool" => out.push(ReplayRow::Tool {
                    name: msg.tool_label(&tool_names),
                    body: msg.display_text(),
                }),
                _ => {}
            }
        }
        out
    }
}

impl ConversationMessage {
    /// 正文展示文本：`display_content` 优先，其次从 `content` 归一。
    #[must_use]
    fn display_text(&self) -> String {
        self.display_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| content_text(&self.content))
    }

    /// 思考展示文本：`display_reasoning_content` 优先，其次原始 `reasoning_content`。
    ///
    /// 关键在**不要重复、也不要丢**。服务端可能把思考合并进 `display_content`（前端
    /// `conversation_hydrate` 的注释就是这么写的，并且「勿再叠加 raw `reasoning_content`」），
    /// 也可能只是提供正文的展示版、思考仍单独给：
    ///
    /// 1. 有 `display_reasoning_content` → 用它，绝不叠加 raw（两者是同一段思考的两个版本）；
    /// 2. 否则 raw 已出现在 `display_content` 里 → 判定已合并，不再单出一行（否则同一段思考
    ///    出现两次，且合并处的粘连会打乱正文的 markdown 结构）；
    /// 3. 否则 raw 单独成行 —— 宁可重复一次可折叠，也不能把思考整段丢掉。
    #[must_use]
    fn display_reasoning(&self) -> String {
        if let Some(display) = self
            .display_reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return display.to_string();
        }
        let Some(raw) = self
            .reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return String::new();
        };
        if self.display_content_mentions(raw) {
            return String::new();
        }
        raw.to_string()
    }

    /// `display_content` 是否已把这段思考原文并进正文（服务端的展示层合并）。
    fn display_content_mentions(&self, reasoning: &str) -> bool {
        self.display_content
            .as_deref()
            .is_some_and(|body| body.contains(reasoning))
    }

    /// 工具名：`name` → 调用侧 `tool_calls` 的 id 查表 → 字面 `tool`。
    ///
    /// 最后一档**不**回落到 `tool_call_id`：`call_abc123` 这种 id 对读的人没有
    /// 信息量，印出来只会让工具行看起来像坏掉的数据（比缺名字更糟）。
    #[must_use]
    fn tool_label(&self, names: &HashMap<&str, &str>) -> String {
        if let Some(name) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return name.to_string();
        }
        self.tool_call_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|id| names.get(id))
            .map_or_else(|| "tool".to_string(), |name| (*name).to_string())
    }
}

/// 扫一遍 assistant 的 `tool_calls`，收集 `tool_call_id → function.name`。
///
/// 这是回放里唯一能拿到工具**真实名字**的地方。顶层 `name` 也接受：某些服务端
/// 把函数名平铺在调用项上，而不是放在 `function` 里。
fn tool_call_names(messages: &[ConversationMessage]) -> HashMap<&str, &str> {
    let mut names = HashMap::new();
    for msg in messages.iter().filter(|m| m.role.trim() == "assistant") {
        let Some(calls) = msg.tool_calls.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for call in calls {
            let Some(id) = call
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let name = call
                .get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| call.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(name) = name {
                names.insert(id, name);
            }
        }
    }
    names
}

/// `content` 归一：字符串原样；parts 数组取 `type=text` 拼接，图片附 `[图片]` 占位。
///
/// 终端画不出图片，但静默丢掉会让「用户发了图」这条消息看起来莫名其妙，
/// 故保留一个可见占位。
fn content_text(content: &Option<Value>) -> String {
    match content {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(parts)) => {
            let mut text = String::new();
            let mut images = 0usize;
            for part in parts {
                let Some(obj) = part.as_object() else {
                    continue;
                };
                match obj.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(t) = obj.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                    "image_url" => images += 1,
                    _ => {}
                }
            }
            for _ in 0..images {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("[图片]");
            }
            text.trim().to_string()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> ConversationMessagesResponse {
        serde_json::from_value(v).expect("response parses")
    }

    #[test]
    fn replays_roles_in_order() {
        let resp = parse(json!({
            "conversation_id": "c1",
            "revision": 7,
            "total_count": 4,
            "has_older": true,
            "window_start_index": 0,
            "messages": [
                {"role": "user", "content": "看下这个文件"},
                {"role": "assistant", "content": "", "reasoning_content": "先读文件",
                 "display_content": "读完了"},
                {"role": "tool", "name": "read_file", "content": "1|fn main() {}"},
                {"role": "assistant", "content": "结论：没问题"}
            ]
        }));
        assert_eq!(resp.revision, 7);
        assert_eq!(resp.total_count, 4);
        assert!(resp.has_older);
        assert_eq!(
            resp.replay_rows(),
            vec![
                ReplayRow::User("看下这个文件".into()),
                ReplayRow::Thinking("先读文件".into()),
                ReplayRow::Assistant("读完了".into()),
                ReplayRow::Tool {
                    name: "read_file".into(),
                    body: "1|fn main() {}".into(),
                },
                ReplayRow::Assistant("结论：没问题".into()),
            ]
        );
    }

    /// serve 常常只在 `role=tool` 里带 `tool_call_id`：名字必须从调用侧补回来，
    /// 而不是把 id 当名字印给用户（这是回放工具行显示成 `call_xxx` 的成因）。
    #[test]
    fn tool_name_falls_back_to_the_call_site() {
        let resp = parse(json!({
            "messages": [
                {"role": "assistant", "content": "", "tool_calls": [
                    {"id": "call_abc123", "type": "function",
                     "function": {"name": "read_file", "arguments": "{}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_abc123", "content": "1|fn main() {}"}
            ]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::Tool {
                name: "read_file".into(),
                body: "1|fn main() {}".into(),
            }],
            "tool 消息缺 name 时应从 assistant 的 tool_calls 补"
        );
    }

    /// 连调用侧都查不到：宁可显字面 `tool`，也不把 `call_xxx` 当名字。
    #[test]
    fn unknown_tool_id_does_not_leak_into_the_name() {
        let resp = parse(json!({
            "messages": [
                {"role": "tool", "tool_call_id": "call_zzz", "content": "结果"}
            ]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::Tool {
                name: "tool".into(),
                body: "结果".into(),
            }],
            "tool_call_id 对读的人没有信息量，不该当工具名"
        );
    }

    #[test]
    fn skips_workspace_context_injection() {
        let resp = parse(json!({
            "messages": [
                {"role": "user", "name": FIRST_TURN_WORKSPACE_CONTEXT_NAME,
                 "content": "<workspace dump>"},
                {"role": "user", "content": "真正的问题"}
            ]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::User("真正的问题".into())],
            "注入了工作区上下文的 user 消息不该显示成用户发言"
        );
    }

    #[test]
    fn skips_system_cards_and_unknown_roles() {
        let resp = parse(json!({
            "messages": [
                {"role": "system", "name": "crabmate_ui_sep", "content": "---"},
                {"role": "system", "name": "crabmate_timeline", "content": "{}"},
                {"role": "developer", "content": "谁？"}
            ]
        }));
        assert!(resp.replay_rows().is_empty());
    }

    #[test]
    fn assistant_with_only_tool_calls_yields_no_row() {
        let resp = parse(json!({
            "messages": [{
                "role": "assistant",
                "content": "",
                "tool_calls": [{"id": "t1", "function": {"name": "read_file"}}]
            }]
        }));
        assert!(
            resp.replay_rows().is_empty(),
            "只有 tool_calls 的助手消息不该在终端再造一张调用卡"
        );
    }

    #[test]
    fn content_parts_join_text_and_mark_images() {
        let resp = parse(json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "第一段"},
                    {"type": "image_url", "image_url": {"url": "data:..."}},
                    {"type": "text", "text": "第二段"}
                ]
            }]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::User("第一段\n第二段\n[图片]".into())]
        );
    }

    #[test]
    fn display_content_wins_over_raw_content() {
        let resp = parse(json!({
            "messages": [{
                "role": "assistant",
                "content": "原始很长很长",
                "display_content": "展示文本"
            }]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::Assistant("展示文本".into())]
        );
    }

    #[test]
    fn empty_response_replays_nothing() {
        let resp = parse(json!({}));
        assert!(resp.replay_rows().is_empty());
        assert_eq!(resp.revision, 0);
    }

    /// 助手有 `display_content` 时**不再叠加** raw `reasoning_content`：服务端已把思考合并进
    /// 正文（与 Web Hydrate 同一规则）。旧行为会多出一行 Thinking，让同一段思考出现两次，
    /// 且合并处的粘连会打乱正文的 markdown 结构。
    #[test]
    fn assistant_with_display_content_does_not_double_reasoning() {
        let resp = parse(json!({
            "messages": [{
                "role": "assistant",
                "content": "正文",
                "display_content": "思考\n\n正文",
                "reasoning_content": "思考"
            }]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![ReplayRow::Assistant("思考\n\n正文".into())],
            "此时正文已含思考，不应再产 Thinking 行"
        );
    }

    /// 服务端**同时**给了 `display_reasoning_content` 时，思考行用它（不是 raw）。
    #[test]
    fn assistant_with_display_content_uses_display_reasoning() {
        let resp = parse(json!({
            "messages": [{
                "role": "assistant",
                "display_content": "正文",
                "display_reasoning_content": "展示版思考",
                "reasoning_content": "原始思考"
            }]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![
                ReplayRow::Thinking("展示版思考".into()),
                ReplayRow::Assistant("正文".into())
            ]
        );
    }

    /// 没有 `display_content` 时保持旧行为：raw `reasoning_content` 单独出思考行。
    #[test]
    fn assistant_without_display_content_still_replays_raw_reasoning() {
        let resp = parse(json!({
            "messages": [{
                "role": "assistant",
                "content": "正文",
                "reasoning_content": "原始思考"
            }]
        }));
        assert_eq!(
            resp.replay_rows(),
            vec![
                ReplayRow::Thinking("原始思考".into()),
                ReplayRow::Assistant("正文".into())
            ]
        );
    }

    /// 水合响应里的上下文快照：Web 端靠它在新会话就显示初始用量，TUI 走同一条路。
    #[test]
    fn hydrate_response_carries_tiktoken_snapshot() {
        let resp = parse(json!({
            "conversation_id": "conv-1",
            "revision": 43,
            "messages": [],
            "tiktoken_prompt_tokens": {
                "prompt_tokens": 1200,
                "tiktoken_model": "gpt-4o",
                "used_input_tokens": 1500,
                "max_input_tokens": 9000
            }
        }));
        let snap = resp.tiktoken_prompt_tokens.expect("应解析出快照");
        assert_eq!(snap.prompt_tokens, 1200);
        assert_eq!(snap.used_input_tokens, Some(1500));
    }

    /// 老服务端/不带该字段时不能报错，只是没有快照。
    #[test]
    fn hydrate_response_without_snapshot_is_fine() {
        let resp = parse(json!({"conversation_id": "conv-1", "revision": 1}));
        assert!(resp.tiktoken_prompt_tokens.is_none());
    }
}
