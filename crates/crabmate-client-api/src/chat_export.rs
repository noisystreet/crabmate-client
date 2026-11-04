//! 会话导出（Desktop `projection=display` 同形）的纯逻辑：格式解析 / 跨页合并 / 正文渲染。
//!
//! 谁需要它：TUI（`crates/crabmate-tui`）的 `/conv export`。Desktop 的导出
//! （`frontend/src/session_export.rs`）导的是它**本地**那份完整 `ChatSession`，而 TUI 手上
//! 只有 `GET /conversation/messages` 的分页结果，所以「全量」必须先跨页合并。信封与
//! Markdown 骨架都来自 [`crabmate::cm_chat_export`]，两端产物因此同形。
//!
//! 过滤规则**不在这里新写一套**：逐条消息出哪一行由
//! [`crate::conversation_messages::ConversationMessagesResponse::replay_rows`] 决定
//! （跳过 `system` 卡片、跳过首轮工作区上下文注入、工具名从调用侧 `tool_calls` 回填），
//! 这里只做「回放行 → 导出消息」的最后一跳。

use crabmate::cm_chat_export::{
    DisplayChatSessionFile, DisplayExportMessage, ExportMdLocale, display_session_to_json_pretty,
    export_md_title_full, markdown_from_role_bodies,
};

use crate::conversation_messages::{ConversationMessagesResponse, ReplayRow};

/// 导出最多翻多少页（每页 [`crate::conversation_messages::CONVERSATION_MESSAGES_PAGE_LIMIT`] 条）。
///
/// 超上限时**报错**而不是悄悄截断：导出件被截掉一半，读的人不会知道少了什么。
pub const EXPORT_MAX_PAGES: u32 = 200;

/// 工具结果在导出件里折成一行摘要的最大字符数。
///
/// 一次 `read_file` 可能有上万字：导出件是给人存档 / 贴给别人的，全文既没人读，
/// 也会把这次排查的其余内容挤出视野（与 TUI 聊天区「工具只占一行摘要」同口径）。
pub const EXPORT_TOOL_SUMMARY_MAX_CHARS: usize = 200;

/// 导出格式（与 Web `/export [json|markdown|both]` 同名同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Markdown,
    Both,
}

impl ExportFormat {
    /// 用法提示：两端的 `Err(msg)` 分支共用这一句，免得措辞各自漂移。
    pub const USAGE: &'static str =
        "用法: /conv export [json|markdown|both] [--out <路径>] [--id <会话 id>]";

    /// 解析格式词：`json`（缺省）/ `markdown` `md` / `both`；其余返回 `None`。
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "json" => Some(Self::Json),
            "markdown" | "md" => Some(Self::Markdown),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    #[must_use]
    pub fn wants_json(self) -> bool {
        matches!(self, Self::Json | Self::Both)
    }

    #[must_use]
    pub fn wants_markdown(self) -> bool {
        matches!(self, Self::Markdown | Self::Both)
    }
}

/// 多页响应 → 有序导出消息。
///
/// `pages` 必须是**新 → 旧**（翻页就是从最新一页往回拉的），函数内部反过来 —— 调用方
/// 不必记住该不该 `reverse`，那是本函数唯一的不变式。
#[must_use]
pub fn export_messages(pages: &[ConversationMessagesResponse]) -> Vec<DisplayExportMessage> {
    let rows = pages
        .iter()
        .rev()
        .flat_map(ConversationMessagesResponse::replay_rows)
        .collect();
    merge_rows(rows)
}

/// 导出消息 → Markdown（标题与各段小标题都来自契约 crate，与 Desktop 同源）。
#[must_use]
pub fn export_markdown(messages: &[DisplayExportMessage]) -> String {
    let loc = ExportMdLocale::ZhHans;
    let pairs: Vec<(&str, &str)> = messages
        .iter()
        .map(|m| (m.role.as_str(), m.content.as_deref().unwrap_or("")))
        .collect();
    markdown_from_role_bodies(export_md_title_full(loc), pairs, loc)
}

/// 导出消息 → JSON（`projection=display` 信封，与 Desktop `chat_export_*.json` 同形）。
pub fn export_json(messages: &[DisplayExportMessage]) -> Result<String, String> {
    let file = DisplayChatSessionFile::new(messages.to_vec());
    display_session_to_json_pretty(&file).map_err(|e| format!("序列化失败：{e}"))
}

/// 导出文件名主干：`chat_export_YYYYmmdd_HHMMSS`。
///
/// 时间戳取 **UTC**：本端不引时区库，而一个差 8 小时的文件名远好过「因为算时区而多一个
/// 依赖」。名字只用于区分两次导出，内容不受影响。
#[must_use]
pub fn export_filename_stem(secs_since_epoch: i64) -> String {
    let (y, mo, d, h, mi, s) = utc_parts(secs_since_epoch);
    format!("chat_export_{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
}

/// 回放行 → 导出消息。
///
/// 关键一步是**思考行的归属**：`DisplayExportMessage.role` 只有 user / assistant / tool
/// （Desktop 的展示投影就这么窄），所以思考不能单占一个 role（那会掉进 Markdown 的
/// 「其它」段，JSON 形状也和 Desktop 不一致）。规则是并进**紧随其后**的助手正文；
/// 后面没有正文（只带 `tool_calls` 的那条 assistant）时单独成一段 assistant ——
/// 排查过程正是导出件的主要用途，丢了就白导了。
fn merge_rows(rows: Vec<ReplayRow>) -> Vec<DisplayExportMessage> {
    let mut out = Vec::new();
    let mut thinking = String::new();
    for row in rows {
        match row {
            ReplayRow::Thinking(text) => append_block(&mut thinking, &text),
            ReplayRow::Assistant(text) => {
                let body = if thinking.is_empty() {
                    text
                } else {
                    format!("{}\n\n{text}", take_thinking(&mut thinking))
                };
                out.push(message("assistant", body, None));
            }
            ReplayRow::User(text) => {
                flush_thinking(&mut out, &mut thinking);
                out.push(message("user", text, None));
            }
            ReplayRow::Tool { name, body } => {
                flush_thinking(&mut out, &mut thinking);
                // 空结果不出一行：Markdown 按 role 分段渲染，一个没有正文的小节只是噪音
                // （`role=tool` 的 `content` 可能是空数组或纯空白）。
                let summary = tool_summary(&body);
                if !summary.is_empty() {
                    out.push(message("tool", summary, Some(name)));
                }
            }
        }
    }
    flush_thinking(&mut out, &mut thinking);
    out
}

/// 追加一段思考（连续两段之间空一行；服务端理论上一条 assistant 只有一段）。
fn append_block(acc: &mut String, text: &str) {
    if !acc.is_empty() {
        acc.push_str("\n\n");
    }
    acc.push_str(text);
}

fn take_thinking(thinking: &mut String) -> String {
    std::mem::take(thinking)
}

/// 思考没有紧随其后的正文：单独成一段，不丢内容。
fn flush_thinking(out: &mut Vec<DisplayExportMessage>, thinking: &mut String) {
    if !thinking.is_empty() {
        out.push(message("assistant", take_thinking(thinking), None));
    }
}

fn message(role: &str, content: String, name: Option<String>) -> DisplayExportMessage {
    DisplayExportMessage {
        role: role.to_string(),
        content: Some(content),
        name,
    }
}

/// 工具结果折成一行：第一个非空行，超长按**字符**截（按字节切会把 CJK 切出半个字）。
fn tool_summary(body: &str) -> String {
    let first = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if first.chars().count() > EXPORT_TOOL_SUMMARY_MAX_CHARS {
        let head: String = first.chars().take(EXPORT_TOOL_SUMMARY_MAX_CHARS).collect();
        format!("{head}…")
    } else {
        first.to_string()
    }
}

/// `i64` 秒 → UTC 的 `(年, 月, 日, 时, 分, 秒)`。
fn utc_parts(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    (
        y,
        mo,
        d,
        (rem / 3_600) as u32,
        ((rem % 3_600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// Howard Hinnant 的 `civil_from_days`（UTC、不含闰秒）。
///
/// 只用于文件名，所以不做历法抽象 —— 但它是纯整数运算，能被下面的单测钉住。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_messages::FIRST_TURN_WORKSPACE_CONTEXT_NAME;
    use serde_json::json;

    fn page(v: serde_json::Value) -> ConversationMessagesResponse {
        serde_json::from_value(v).expect("response parses")
    }

    fn roles(messages: &[DisplayExportMessage]) -> Vec<(String, String)> {
        messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone().unwrap_or_default()))
            .collect()
    }

    #[test]
    fn parses_the_web_format_words() {
        for raw in ["json", "JSON", " json ", ""] {
            assert_eq!(ExportFormat::parse(raw), Some(ExportFormat::Json), "{raw}");
        }
        for raw in ["markdown", "MD", "Markdown"] {
            assert_eq!(
                ExportFormat::parse(raw),
                Some(ExportFormat::Markdown),
                "{raw}"
            );
        }
        assert_eq!(ExportFormat::parse("both"), Some(ExportFormat::Both));
        assert_eq!(ExportFormat::parse("pdf"), None);
    }

    /// 两页必须按「旧 → 新」接起来：`pages` 是新→旧传进来的，顺序反了导出件就倒着读。
    #[test]
    fn merges_pages_oldest_first() {
        let newest = page(json!({
            "messages": [
                {"role": "user", "content": "第二个问题"},
                {"role": "assistant", "content": "第二个回答"}
            ]
        }));
        let older = page(json!({
            "messages": [
                {"role": "user", "content": "第一个问题"},
                {"role": "assistant", "content": "第一个回答"}
            ]
        }));
        let merged = export_messages(&[newest, older]);
        assert_eq!(
            roles(&merged),
            vec![
                ("user".to_string(), "第一个问题".to_string()),
                ("assistant".to_string(), "第一个回答".to_string()),
                ("user".to_string(), "第二个问题".to_string()),
                ("assistant".to_string(), "第二个回答".to_string()),
            ]
        );
    }

    /// 思考并进紧随其后的助手正文（Desktop 的展示投影里没有独立的 thinking role）。
    #[test]
    fn thinking_merges_into_the_next_assistant_body() {
        let resp = page(json!({
            "messages": [
                {"role": "user", "content": "看下这个文件"},
                {"role": "assistant", "content": "读完了", "reasoning_content": "先读文件"}
            ]
        }));
        let merged = export_messages(&[resp]);
        assert_eq!(
            roles(&merged),
            vec![
                ("user".to_string(), "看下这个文件".to_string()),
                ("assistant".to_string(), "先读文件\n\n读完了".to_string()),
            ]
        );
    }

    /// 思考后面没有正文（只带 `tool_calls` 的 assistant）：单独成段，不能丢。
    #[test]
    fn orphan_thinking_becomes_its_own_assistant_block() {
        let resp = page(json!({
            "messages": [
                {"role": "assistant", "content": "", "reasoning_content": "先查一下",
                 "tool_calls": [{"id": "t1", "function": {"name": "read_file"}}]},
                {"role": "tool", "name": "read_file", "content": "1|fn main() {}"}
            ]
        }));
        assert_eq!(
            roles(&export_messages(&[resp])),
            vec![
                ("assistant".to_string(), "先查一下".to_string()),
                ("tool".to_string(), "1|fn main() {}".to_string()),
            ]
        );
    }

    /// 工具结果只留一行摘要：全文会把导出件变成日志文件。
    #[test]
    fn tool_bodies_collapse_to_one_summary_line() {
        let long = "x".repeat(EXPORT_TOOL_SUMMARY_MAX_CHARS + 10);
        let resp = page(json!({
            "messages": [
                {"role": "tool", "name": "read_file", "content": format!("\n  {long}\n第二行")}
            ]
        }));
        let merged = export_messages(&[resp]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].role, "tool");
        assert_eq!(merged[0].name.as_deref(), Some("read_file"));
        let body = merged[0].content.clone().unwrap_or_default();
        assert_eq!(body.chars().count(), EXPORT_TOOL_SUMMARY_MAX_CHARS + 1);
        assert!(body.ends_with('…'));
        assert!(!body.contains("第二行"), "只保留第一条非空行");
    }

    /// 服务端卡片（`system`）与首轮工作区上下文注入都不进导出件。
    #[test]
    fn system_cards_and_workspace_injection_are_skipped() {
        let resp = page(json!({
            "messages": [
                {"role": "system", "name": "crabmate_ui_sep", "content": "---"},
                {"role": "user", "name": FIRST_TURN_WORKSPACE_CONTEXT_NAME, "content": "<dump>"},
                {"role": "user", "content": "真正的问题"}
            ]
        }));
        assert_eq!(
            roles(&export_messages(&[resp])),
            vec![("user".to_string(), "真正的问题".to_string())]
        );
    }

    #[test]
    fn markdown_uses_the_contract_skeleton() {
        let resp = page(json!({
            "messages": [{"role": "user", "content": "你好"}]
        }));
        let md = export_markdown(&export_messages(&[resp]));
        assert!(md.starts_with(export_md_title_full(ExportMdLocale::ZhHans)));
        assert!(md.contains("你好"));
    }

    #[test]
    fn json_uses_the_display_projection_envelope() {
        let resp = page(json!({
            "messages": [{"role": "user", "content": "你好"}]
        }));
        let raw = export_json(&export_messages(&[resp])).expect("json");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("parses");
        assert_eq!(v["projection"], "display");
        assert_eq!(v["schema"], "crabmate.chat_session");
        assert_eq!(v["messages"][0]["role"], "user");
    }

    /// 空结果的工具不进导出件：Markdown 按 role 分段，空小节只是噪音。
    #[test]
    fn empty_tool_bodies_are_skipped() {
        let resp = page(json!({
            "messages": [
                {"role": "user", "content": "跑一下"},
                {"role": "tool", "name": "run_command", "content": "  \n\t\n"}
            ]
        }));
        assert_eq!(
            roles(&export_messages(&[resp])),
            vec![("user".to_string(), "跑一下".to_string())]
        );
    }

    #[test]
    fn filename_stem_is_utc() {
        assert_eq!(export_filename_stem(0), "chat_export_19700101_000000");
        // 2023-11-14T22:13:20Z
        assert_eq!(
            export_filename_stem(1_700_000_000),
            "chat_export_20231114_221320"
        );
        // 闰日 + 跨世纪（2000-02-29T12:34:56Z）
        assert_eq!(
            export_filename_stem(951_827_696),
            "chat_export_20000229_123456"
        );
    }
}
