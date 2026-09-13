//! TUI 正文 chunks 构造：从 [`StoredMessage`] + [`TuiRenderCtx`] 产出回合正文块
//! （工具行 / 用户文本含文件引用 chip / skill slash / 思维链折叠 + 终答）。

use std::collections::HashMap;

use crate::i18n::Locale;
use crate::markdown::plaintext_to_safe_html;
use crate::storage::{StoredMessage, StoredMessageState};
use crate::stream_text_overlay::{
    StreamTextOverlay, message_text_for_display_including_stream_overlay,
};

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
            closed: vec![format!(
                "<div class=\"chat-tui-line chat-tui-line--block\">{chip}</div>"
            )],
            open_plain: None,
            markdown_render,
        };
    }
    let mut task_chunks = user_text_body_chunks(task, finalize_open_block, markdown_render);
    let chip_prefix = format!("{chip} ");
    if let Some(first) = task_chunks.closed.first_mut() {
        prepend_inline_html_to_first_tui_line(first, &chip_prefix);
        return task_chunks;
    }
    if let Some(plain) = task_chunks.open_plain.take() {
        let class = open_active_block_class(&plain, markdown_render);
        let body = render_open_active_html(&plain, markdown_render);
        task_chunks
            .closed
            .push(format!("<div class=\"{class}\">{chip_prefix}{body}</div>"));
        return task_chunks;
    }
    TuiBodyChunks {
        think: None,
        closed: vec![format!(
            "<div class=\"chat-tui-line chat-tui-line--block\">{chip}</div>"
        )],
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
        replace_marks(c);
    }
    // open_plain 是源文本不能嵌 HTML：有引用时折成闭合行并替换占位符。
    if let Some(plain) = chunks.open_plain.take() {
        let class = open_active_block_class(&plain, markdown_render);
        let mut body = render_open_active_html(&plain, markdown_render);
        replace_marks(&mut body);
        chunks
            .closed
            .push(format!("<div class=\"{class}\">{body}</div>"));
    }
    chunks
}

pub(crate) fn message_body_chunks(
    message: &StoredMessage,
    ctx: &TuiRenderCtx<'_>,
) -> TuiBodyChunks {
    if message.is_tool {
        let live = tool_live_overlay(message, ctx.tool_chunks);
        let job = message
            .tool_call_id
            .as_deref()
            .and_then(|tid| ctx.tool_jobs.get(tid));
        let open_path = if ctx.open_file_enabled {
            message
                .tool_call_id
                .as_deref()
                .and_then(|tid| ctx.tool_file_paths.get(tid))
                .map(String::as_str)
        } else {
            None
        };
        return TuiBodyChunks {
            think: None,
            closed: vec![tool_process_body_html(
                message, ctx.locale, live, job, open_path,
            )],
            open_plain: None,
            // 工具 HTML 不走 MD，仍记录全局开关以免与 Incremental 前缀比较漂移。
            markdown_render: ctx.markdown_render,
        };
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
    let mut chunks = parse_tui_body_chunks_with(
        &answer,
        message_finalize_open_block(message),
        ctx.markdown_render,
    );
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
