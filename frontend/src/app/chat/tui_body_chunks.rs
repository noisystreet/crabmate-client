//! TUI 正文 chunks 构造：从 [`StoredMessage`] + [`TuiRenderCtx`] 产出回合正文块
//! （工具行 / 用户文本含文件引用 chip / skill slash / 思维链折叠 + 终答）。

use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::Locale;
use crate::markdown::plaintext_to_safe_html;
use crate::storage::{StoredMessage, StoredMessageState};
use crate::stream_text_overlay::{
    StreamTextOverlay, message_text_for_display_including_stream_overlay,
};

use super::tui_body_parse_cache::{TuiBodyParseCache, parse_tui_body_chunks_incremental};
use super::tui_line_markdown::{
    TuiBodyChunks, open_active_block_class, parse_tui_body_chunks_with, render_open_active_html,
};
use super::tui_thinking_block::{build_think_block, message_think_answer_display_text};
use super::tui_tool_process::tool_process_body_html;
use super::tui_transcript_sync::TuiRenderCtx;

fn message_finalize_open_block(message: &StoredMessage) -> bool {
    !message
        .state
        .as_ref()
        .is_some_and(StoredMessageState::is_loading)
}

pub(crate) fn tool_live_overlay<'a>(
    message: &StoredMessage,
    tool_chunks: &'a HashMap<String, String>,
) -> Option<&'a str> {
    message
        .tool_call_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| tool_chunks.get(id))
        .map(String::as_str)
}

fn message_display_text(
    message: &StoredMessage,
    session_id: &str,
    overlay: Option<&StreamTextOverlay>,
    locale: Locale,
    apply_assistant_display_filters: bool,
) -> String {
    message_text_for_display_including_stream_overlay(
        message,
        overlay,
        session_id,
        locale,
        apply_assistant_display_filters,
    )
}

fn file_ref_chip_html(token: &str) -> String {
    let display = crate::message_format::file_ref_display::file_ref_visible_label(token);
    let title_esc = plaintext_to_safe_html(token);
    let display_esc = plaintext_to_safe_html(display);
    format!("<span class=\"msg-file-ref\" title=\"{title_esc}\">{display_esc}</span>")
}

/// 将内联 HTML 插入首个正文容器内（优先 `<p>` 内），避免 chip 与块级段落上下叠成两行。
fn prepend_inline_html_to_first_tui_line(line_html: &mut String, prefix: &str) {
    const P_OPEN: &str = "<p>";
    if let Some(i) = line_html.find(P_OPEN) {
        line_html.insert_str(i + P_OPEN.len(), prefix);
        return;
    }
    if let Some(p0) = line_html.find("<p ") {
        if let Some(gt) = line_html[p0..].find('>') {
            line_html.insert_str(p0 + gt + 1, prefix);
            return;
        }
    }
    if let Some(i) = line_html.find('>') {
        line_html.insert_str(i + 1, prefix);
    } else {
        line_html.insert_str(0, prefix);
    }
}

/// skill chip + 任务正文：chip 必须与首行同块，不能先单独塞一个裸 span 再跟 `div.chat-tui-line`。
fn skill_slash_body_chunks(
    skill_id: &str,
    task: &str,
    finalize_open_block: bool,
    markdown_render: bool,
    locale: Locale,
) -> TuiBodyChunks {
    let prefix = crate::i18n::msg_skill_invoke_prefix(locale);
    let suffix = crate::i18n::msg_skill_invoke_suffix(locale);
    let id_esc = plaintext_to_safe_html(skill_id);
    let title_esc = plaintext_to_safe_html(&format!("/{skill_id}"));
    let chip = format!(
        "<span class=\"msg-skill-invoke\" title=\"{title_esc}\">{prefix} <span class=\"msg-skill-invoke-id\">{id_esc}</span> {suffix}</span>"
    );
    if task.is_empty() {
        return TuiBodyChunks {
            think: None,
            closed: vec![Arc::new(format!(
                "<div class=\"chat-tui-line chat-tui-line--block\">{chip}</div>"
            ))],
            open_plain: None,
            markdown_render,
        };
    }
    let mut task_chunks = user_text_body_chunks(task, finalize_open_block, markdown_render);
    let chip_prefix = format!("{chip} ");
    if let Some(first) = task_chunks.closed.first_mut() {
        prepend_inline_html_to_first_tui_line(Arc::make_mut(first), &chip_prefix);
        return task_chunks;
    }
    if let Some(plain) = task_chunks.open_plain.take() {
        let class = open_active_block_class(&plain, markdown_render);
        let body = render_open_active_html(&plain, markdown_render);
        task_chunks.closed.push(Arc::new(format!(
            "<div class=\"{class}\">{chip_prefix}{body}</div>"
        )));
        return task_chunks;
    }
    TuiBodyChunks {
        think: None,
        closed: vec![Arc::new(format!(
            "<div class=\"chat-tui-line chat-tui-line--block\">{chip}</div>"
        ))],
        open_plain: None,
        markdown_render,
    }
}

fn user_text_body_chunks(
    text: &str,
    finalize_open_block: bool,
    markdown_render: bool,
) -> TuiBodyChunks {
    use crate::message_format::file_ref_display::{UserTextSeg, split_user_file_ref_segs};
    let segs = split_user_file_ref_segs(text);
    if segs.iter().all(|s| matches!(s, UserTextSeg::Plain(_))) {
        return parse_tui_body_chunks_with(text, finalize_open_block, markdown_render);
    }

    // 占位符整段解析后再换回 chip，避免裸 span 落在 `chat-tui-line` 外造成额外换行。
    const MARK_L: &str = "\u{2060}⟦CMFR";
    const MARK_R: &str = "⟧\u{2060}";
    let mut rebuilt = String::with_capacity(text.len());
    let mut chips: Vec<String> = Vec::new();
    for seg in segs {
        match seg {
            UserTextSeg::Plain(p) => rebuilt.push_str(&p),
            UserTextSeg::FileRef(tok) => {
                let i = chips.len();
                chips.push(file_ref_chip_html(&tok));
                rebuilt.push_str(MARK_L);
                rebuilt.push_str(&i.to_string());
                rebuilt.push_str(MARK_R);
            }
        }
    }

    let mut chunks = parse_tui_body_chunks_with(&rebuilt, finalize_open_block, markdown_render);
    let replace_marks = |s: &mut String| {
        for (i, chip) in chips.iter().enumerate() {
            let mark = format!("{MARK_L}{i}{MARK_R}");
            if s.contains(&mark) {
                *s = s.replace(&mark, chip);
                continue;
            }
            let esc = plaintext_to_safe_html(&mark);
            if esc != mark {
                *s = s.replace(&esc, chip);
            }
        }
    };
    for c in &mut chunks.closed {
        replace_marks(Arc::make_mut(c));
    }
    // open_plain 是源文本不能嵌 HTML：有引用时折成闭合行并替换占位符。
    if let Some(plain) = chunks.open_plain.take() {
        let class = open_active_block_class(&plain, markdown_render);
        let mut body = render_open_active_html(&plain, markdown_render);
        replace_marks(&mut body);
        chunks
            .closed
            .push(Arc::new(format!("<div class=\"{class}\">{body}</div>")));
    }
    chunks
}

/// skill 调用卡：语义独立于工具卡，明确告知本轮确实调用了 skill（不得伪装成 `tool_call`）。
fn skill_inject_card_html(info: &crate::timeline_scan::SkillInjectInfo, locale: Locale) -> String {
    let label = plaintext_to_safe_html(crate::i18n::msg_skill_inject_card_label(locale));
    let forced = if info.forced {
        format!(
            "<span class=\"chat-tui-skill-inject-forced\">{}</span>",
            plaintext_to_safe_html(crate::i18n::msg_skill_inject_card_forced(locale))
        )
    } else {
        String::new()
    };
    let mut chips = String::new();
    for id in &info.skills {
        let esc = plaintext_to_safe_html(id);
        chips.push_str(&format!(
            "<span class=\"chat-tui-skill-inject-chip\" title=\"{esc}\">{esc}</span>"
        ));
    }
    format!(
        "<div class=\"chat-tui-skill-inject\" data-testid=\"chat-tui-skill-inject\">\
         <span class=\"chat-tui-skill-inject-mark\" aria-hidden=\"true\">✦</span>\
         <span class=\"chat-tui-skill-inject-label\">{label}</span>\
         {forced}{chips}</div>"
    )
}

/// 上下文压缩 / 裁剪卡的单行摘要：优先已用 / 可用上下文，其次消息条数区间。
fn context_trim_one_line(info: &crate::timeline_scan::ContextTrimInfo, locale: Locale) -> String {
    if let Some(used_tokens) = info.after_tokens {
        let used = super::super::status_context::format_compact_tokens(used_tokens);
        if let Some(cap) = info.max_input_tokens {
            let available = super::super::status_context::format_compact_tokens(
                cap.saturating_sub(used_tokens),
            );
            let used_pct = (u64::from(used_tokens) * 100 / u64::from(cap.max(1))).min(100) as u32;
            return crate::i18n::msg_context_trim_used_available(
                locale, &used, &available, used_pct,
            );
        }
        return crate::i18n::msg_context_trim_used_tokens(locale, &used);
    }
    match (info.n_before, info.n_after) {
        (Some(before), Some(after)) => {
            crate::i18n::msg_context_trim_message_range(locale, before, after)
        }
        _ => String::new(),
    }
}

/// 结构化字段行（`dl > div > dt + dd`）：空值不渲染。
fn trim_fact_row(label: &str, value: &str) -> String {
    if value.trim().is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"chat-tui-trim-fact\">\
         <dt class=\"chat-tui-trim-fact-label\">{}</dt>\
         <dd class=\"chat-tui-trim-fact-value\">{}</dd></div>",
        plaintext_to_safe_html(label),
        plaintext_to_safe_html(value),
    )
}

fn trim_fact_label(locale: Locale, fact: crate::i18n::ContextTrimFact) -> &'static str {
    crate::i18n::msg_context_trim_fact_label(locale, fact)
}

/// 压缩前 → 压缩后 token 数。
fn context_trim_token_fact(info: &crate::timeline_scan::ContextTrimInfo, locale: Locale) -> String {
    let (Some(before), Some(after)) = (info.before_tokens, info.after_tokens) else {
        return String::new();
    };
    let before = super::super::status_context::format_compact_tokens(before);
    let after = super::super::status_context::format_compact_tokens(after);
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Tokens),
        &format!("{before} → {after} tokens"),
    )
}

/// 上下文窗口与预留输出。
fn context_trim_window_fact(
    info: &crate::timeline_scan::ContextTrimInfo,
    locale: Locale,
) -> String {
    let Some(cap) = info.max_input_tokens else {
        return String::new();
    };
    let cap = super::super::status_context::format_compact_cap_tokens(cap);
    let reserved = info
        .reserved_output_tokens
        .map(super::super::status_context::format_compact_tokens);
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Window),
        &crate::i18n::msg_context_trim_window(locale, &cap, reserved.as_deref()),
    )
}

/// 消息条数 before → after。
fn context_trim_message_fact(
    info: &crate::timeline_scan::ContextTrimInfo,
    locale: Locale,
) -> String {
    let (Some(before), Some(after)) = (info.n_before, info.n_after) else {
        return String::new();
    };
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Messages),
        &crate::i18n::msg_context_trim_message_range(locale, before, after),
    )
}

/// 移除的轮次 / 消息数。
fn context_trim_removed_fact(
    info: &crate::timeline_scan::ContextTrimInfo,
    locale: Locale,
) -> String {
    if info.removed_turn_groups == 0 && info.removed_messages == 0 {
        return String::new();
    }
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Removed),
        &crate::i18n::msg_context_trim_removed(
            locale,
            info.removed_turn_groups,
            info.removed_messages,
        ),
    )
}

/// token 构成（消息 / 工具定义 / 附件）。
fn context_trim_breakdown_fact(
    info: &crate::timeline_scan::ContextTrimInfo,
    locale: Locale,
) -> String {
    let messages = info
        .message_tokens
        .map(super::super::status_context::format_compact_tokens);
    let schema = info
        .tool_schema_tokens
        .map(super::super::status_context::format_compact_tokens);
    let attachments = info
        .attachment_tokens
        .map(super::super::status_context::format_compact_tokens);
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Breakdown),
        &crate::i18n::msg_context_trim_breakdown(
            locale,
            messages.as_deref(),
            schema.as_deref(),
            attachments.as_deref(),
        ),
    )
}

/// 压缩方式与触发条件。
fn context_trim_method_fact(
    info: &crate::timeline_scan::ContextTrimInfo,
    locale: Locale,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if info.summarized {
        parts.push(crate::i18n::msg_context_trim_method_summary(
            locale,
            info.tail_kept,
        ));
    }
    if info.count_hit || info.char_hit {
        parts.push(crate::i18n::msg_context_trim_method_trim(locale).to_string());
    }
    if info.compress_hits > 0 {
        parts.push(crate::i18n::msg_context_trim_method_compress(
            locale,
            info.compress_hits,
        ));
    }
    if info.token_triggered {
        parts.push(crate::i18n::msg_context_trim_method_token_trigger(locale).to_string());
    }
    trim_fact_row(
        trim_fact_label(locale, crate::i18n::ContextTrimFact::Method),
        &parts.join(" · "),
    )
}

/// 展开层结构化字段；无任何可比字段时返回空串（仅保留原始 JSON）。
fn context_trim_facts_html(info: &crate::timeline_scan::ContextTrimInfo, locale: Locale) -> String {
    let mut rows = String::new();
    for row in [
        context_trim_token_fact(info, locale),
        context_trim_window_fact(info, locale),
        context_trim_message_fact(info, locale),
        context_trim_removed_fact(info, locale),
        context_trim_breakdown_fact(info, locale),
        context_trim_method_fact(info, locale),
    ] {
        rows.push_str(&row);
    }
    if let Some(source) = &info.counting_source {
        rows.push_str(&trim_fact_row(
            trim_fact_label(locale, crate::i18n::ContextTrimFact::CountingSource),
            source,
        ));
    }
    if let Some(reason) = &info.compaction_reason {
        rows.push_str(&trim_fact_row(
            trim_fact_label(locale, crate::i18n::ContextTrimFact::Reason),
            reason,
        ));
    }
    if rows.is_empty() {
        return String::new();
    }
    format!("<dl class=\"chat-tui-trim-facts\">{rows}</dl>")
}

/// 原始 JSON 收进二级折叠，避免默认展开时淹没结构化字段。
fn context_trim_raw_html(info: &crate::timeline_scan::ContextTrimInfo, locale: Locale) -> String {
    if info.detail.trim().is_empty() {
        return String::new();
    }
    format!(
        "<details class=\"chat-tui-trim-raw\">\
         <summary class=\"chat-tui-trim-raw-summary\">{}</summary>\
         <pre class=\"chat-tui-trim-detail-body\">{}</pre></details>",
        plaintext_to_safe_html(crate::i18n::msg_context_trim_raw_json_label(locale)),
        plaintext_to_safe_html(&info.detail),
    )
}

/// 上下文压缩 / 裁剪卡：仿工具卡（状态符 + 名称 + 单行摘要 + 折叠详情），语义独立。
///
/// Server `context_trim` 合并「裁剪历史」与「压缩工具输出」两类标题，卡片以 `title` 区分。
fn context_trim_card_html(info: &crate::timeline_scan::ContextTrimInfo, locale: Locale) -> String {
    let name = if info.title.trim().is_empty() {
        crate::i18n::msg_context_trim_card_label(locale).to_string()
    } else {
        info.title.trim().to_string()
    };
    let name_esc = plaintext_to_safe_html(&name);
    let one_line = plaintext_to_safe_html(&context_trim_one_line(info, locale));
    let status_aria = plaintext_to_safe_html(crate::i18n::msg_context_trim_status_label(locale));
    let expand_title = plaintext_to_safe_html(crate::i18n::msg_context_trim_detail_title(locale));
    format!(
        "<div class=\"chat-tui-trim-card\" data-testid=\"chat-tui-trim-card\">\
         <details class=\"chat-tui-trim-details\">\
         <summary class=\"chat-tui-trim-row\" title=\"{expand_title}\">\
         <span class=\"chat-tui-trim-status\" role=\"img\" aria-label=\"{status_aria}\" title=\"{status_aria}\">📉</span>\
         <span class=\"chat-tui-trim-mark\" aria-hidden=\"true\">🗜️</span>\
         <span class=\"chat-tui-trim-name\" title=\"{name_esc}\">{name_esc}</span>\
         <span class=\"chat-tui-trim-one-line\">{one_line}</span>\
         <span class=\"chat-tui-trim-expand\" aria-hidden=\"true\">▸</span>\
         </summary>\
         {facts}{raw}\
         </details></div>",
        facts = context_trim_facts_html(info, locale),
        raw = context_trim_raw_html(info, locale),
    )
}

/// timeline 旁注卡（skill 调用 / 上下文压缩）：非旁注 → `None`，交回常规正文渲染。
fn timeline_card_body_chunks(
    message: &StoredMessage,
    locale: Locale,
    markdown_render: bool,
) -> Option<TuiBodyChunks> {
    if let Some(info) = crate::timeline_scan::skill_inject_from_timeline_text(&message.text) {
        return Some(TuiBodyChunks {
            think: None,
            closed: vec![Arc::new(skill_inject_card_html(&info, locale))],
            open_plain: None,
            markdown_render,
        });
    }
    let info = crate::timeline_scan::context_trim_from_timeline_text(&message.text)?;
    Some(TuiBodyChunks {
        think: None,
        closed: vec![Arc::new(context_trim_card_html(&info, locale))],
        open_plain: None,
        markdown_render,
    })
}

pub(crate) fn message_body_chunks(
    message: &StoredMessage,
    ctx: &TuiRenderCtx<'_>,
) -> TuiBodyChunks {
    message_body_chunks_cached(message, ctx, None)
}

/// [`message_body_chunks`] 的缓存版：助手终答可传增量解析缓存（`None` 等价全量）。
pub(crate) fn message_body_chunks_cached(
    message: &StoredMessage,
    ctx: &TuiRenderCtx<'_>,
    cache: Option<&mut TuiBodyParseCache>,
) -> TuiBodyChunks {
    if message.is_tool {
        let live = tool_live_overlay(message, ctx.tool_chunks);
        let job = message
            .tool_call_id
            .as_deref()
            .and_then(|tid| ctx.tool_jobs.get(tid));
        let open_path = ctx.tool_open_file_path(message);
        return TuiBodyChunks {
            think: None,
            closed: vec![Arc::new(tool_process_body_html(
                message, ctx.locale, live, job, open_path,
            ))],
            open_plain: None,
            // 工具 HTML 不走 MD，仍记录全局开关以免与 Incremental 前缀比较漂移。
            markdown_render: ctx.markdown_render,
        };
    }
    if let Some(chunks) = timeline_card_body_chunks(message, ctx.locale, ctx.markdown_render) {
        return chunks;
    }
    if message.role == "user" {
        let text = message_display_text(
            message,
            ctx.session_id,
            ctx.overlay,
            ctx.locale,
            ctx.apply_filters,
        );
        if let Some((skill_id, task)) = crate::message_format::parse_user_skill_slash(&text) {
            let mut chunks = skill_slash_body_chunks(
                &skill_id,
                &task,
                message_finalize_open_block(message),
                ctx.markdown_render,
                ctx.locale,
            );
            super::user_upload_images::append_user_upload_images(
                &mut chunks,
                &message.image_urls,
                ctx.locale,
            );
            return chunks;
        }
        let mut chunks = user_text_body_chunks(
            &text,
            message_finalize_open_block(message),
            ctx.markdown_render,
        );
        super::user_upload_images::append_user_upload_images(
            &mut chunks,
            &message.image_urls,
            ctx.locale,
        );
        return chunks;
    }
    // 助手/其他：思维链折叠块 + 终答正文（非助手角色思维链为空，行为不变）。
    let (thinking, answer) = message_think_answer_display_text(
        message,
        ctx.session_id,
        ctx.overlay,
        ctx.locale,
        ctx.apply_filters,
    );
    let mut chunks = match cache {
        Some(cache) => {
            if cache.message_id() != message.id.as_str()
                || cache.markdown_render() != ctx.markdown_render
            {
                *cache = TuiBodyParseCache::new(&message.id, ctx.markdown_render);
            }
            parse_tui_body_chunks_incremental(cache, &answer, message_finalize_open_block(message))
        }
        None => parse_tui_body_chunks_with(
            &answer,
            message_finalize_open_block(message),
            ctx.markdown_render,
        ),
    };
    if !thinking.trim().is_empty() {
        // 默认折叠：流式生成时也不自动展开，避免渲染过程视觉跳动；用户点击 summary 展开。
        let open = ctx.think_open.contains(&message.id);
        chunks.think = Some(build_think_block(
            &thinking,
            open,
            ctx.markdown_render,
            ctx.locale,
        ));
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline_scan::ContextTrimInfo;

    fn trim_info(after_tokens: Option<u32>, max_input_tokens: Option<u32>) -> ContextTrimInfo {
        ContextTrimInfo {
            title: "已裁剪历史".into(),
            before_tokens: None,
            after_tokens,
            max_input_tokens,
            reserved_output_tokens: None,
            message_tokens: None,
            tool_schema_tokens: None,
            attachment_tokens: None,
            n_before: None,
            n_after: None,
            count_hit: false,
            char_hit: false,
            compress_hits: 0,
            summarized: false,
            tail_kept: None,
            token_triggered: false,
            removed_turn_groups: 0,
            removed_messages: 0,
            counting_source: None,
            compaction_reason: None,
            detail: String::new(),
        }
    }

    #[test]
    fn context_trim_one_line_shows_used_percentage_and_available() {
        let info = trim_info(Some(3_456), Some(160_000));
        assert_eq!(
            context_trim_one_line(&info, Locale::ZhHans),
            "已用 3456 tokens (2%) · 可用 156.5k tokens"
        );
        assert_eq!(
            context_trim_one_line(&info, Locale::En),
            "Used 3456 tokens (2%) · available 156.5k tokens"
        );

        // 已用超过窗口时百分比封顶 100%，可用量不为负。
        let over = trim_info(Some(1_000), Some(500));
        assert_eq!(
            context_trim_one_line(&over, Locale::ZhHans),
            "已用 1000 tokens (100%) · 可用 0 tokens"
        );
    }

    #[test]
    fn context_trim_one_line_falls_back_without_window_or_tokens() {
        let no_cap = trim_info(Some(3_456), None);
        assert_eq!(
            context_trim_one_line(&no_cap, Locale::ZhHans),
            "已用 3456 tokens"
        );

        let mut range = trim_info(None, None);
        range.n_before = Some(40);
        range.n_after = Some(12);
        assert_eq!(
            context_trim_one_line(&range, Locale::ZhHans),
            "40 → 12 条消息"
        );
    }

    #[test]
    fn context_trim_expanded_layer_renders_structured_facts_and_raw_json() {
        let mut info = trim_info(Some(3_456), Some(160_000));
        info.before_tokens = Some(12_000);
        info.reserved_output_tokens = Some(8_000);
        info.message_tokens = Some(3_100);
        info.tool_schema_tokens = Some(2_000);
        info.attachment_tokens = Some(0);
        info.n_before = Some(40);
        info.n_after = Some(12);
        info.count_hit = true;
        info.compress_hits = 3;
        info.summarized = true;
        info.token_triggered = true;
        info.removed_turn_groups = 2;
        info.removed_messages = 5;
        info.counting_source = Some("matched_tokenizer".into());
        info.compaction_reason = Some("token_budget_turn_groups".into());
        info.detail = "{\n  \"n_before\": 40\n}".into();

        let html = context_trim_card_html(&info, Locale::ZhHans);
        assert!(html.contains("data-testid=\"chat-tui-trim-card\""));
        // 折叠层保持关闭；状态符用中性语义，不复用工具卡的「完成」。
        assert!(html.contains("<details class=\"chat-tui-trim-details\">"));
        assert!(!html.contains("<details class=\"chat-tui-trim-details\" open"));
        assert!(html.contains("📉"));
        // `aria-label` 不能命名 generic role 的裸 span，`role="img"` 让标签生效。
        assert!(html.contains("role=\"img\" aria-label=\"上下文压缩\""));
        assert!(html.contains("aria-label=\"上下文压缩\""));
        assert!(!html.contains("aria-label=\"完成\""));
        // summary tooltip 展开后仍成立（不写「展开」）。
        assert!(html.contains("title=\"上下文压缩详情\""));
        assert!(html.contains("<dl class=\"chat-tui-trim-facts\">"));
        assert!(html.contains("12.0k → 3456 tokens"));
        assert!(html.contains("160k tokens（预留输出 8000）"));
        assert!(html.contains("40 → 12 条消息"));
        assert!(html.contains("2 轮 · 5 条消息"));
        assert!(html.contains("消息 3100 · 工具定义 2000 · 附件 0"));
        assert!(html.contains("摘要 · 裁剪历史 · 压缩工具输出 ×3 · token 预算触发"));
        assert!(html.contains("matched_tokenizer"));
        assert!(html.contains("token_budget_turn_groups"));
        // 原始 JSON 收进二级折叠，而不是直接铺在展开层。
        assert!(html.contains("<details class=\"chat-tui-trim-raw\">"));
        assert!(html.contains("原始 JSON"));
        assert!(html.contains("&quot;n_before&quot;"));
    }

    #[test]
    fn context_trim_expanded_layer_skips_facts_when_detail_has_no_extra_keys() {
        // 黄金 fixture 的旧形态：只有 `n_before`，没有可结构化的其余字段。
        let mut info = trim_info(None, None);
        info.n_before = Some(3);
        info.detail = "{\n  \"n_before\": 3\n}".into();
        let html = context_trim_card_html(&info, Locale::ZhHans);
        assert!(!html.contains("chat-tui-trim-facts"));
        assert!(html.contains("<details class=\"chat-tui-trim-raw\">"));
    }

    #[test]
    fn context_trim_card_escapes_user_visible_text() {
        let mut info = trim_info(Some(3_456), None);
        info.title = "<b>压缩</b>".into();
        info.detail = "{\"x\":\"<script>\"}".into();
        let html = context_trim_card_html(&info, Locale::ZhHans);
        assert!(html.contains("&lt;b&gt;压缩&lt;/b&gt;"));
        assert!(!html.contains("<b>压缩</b>"));
        assert!(html.contains("&quot;x&quot;"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
