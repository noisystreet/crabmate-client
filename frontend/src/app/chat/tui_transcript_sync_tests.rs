use super::*;
use crate::storage::StoredMessageState;
use crate::stream_text_overlay::StreamTextOverlay;
use std::collections::{HashMap, HashSet};

fn sync(
    prev: Option<&TuiMountState>,
    messages: &[StoredMessage],
    session_id: &str,
    overlay: Option<&StreamTextOverlay>,
    tool_chunks: &HashMap<String, String>,
) -> TuiSyncPlan {
    sync_with_think(
        prev,
        messages,
        session_id,
        overlay,
        tool_chunks,
        &HashSet::new(),
    )
}

fn sync_with_think(
    prev: Option<&TuiMountState>,
    messages: &[StoredMessage],
    session_id: &str,
    overlay: Option<&StreamTextOverlay>,
    tool_chunks: &HashMap<String, String>,
    think_open: &HashSet<String>,
) -> TuiSyncPlan {
    let empty_jobs = HashMap::new();
    plan_tui_sync(PlanTuiSyncArgs {
        prev,
        messages,
        session_id,
        overlay,
        locale: Locale::ZhHans,
        apply_assistant_display_filters: false,
        markdown_render: true,
        show_turn_context_inject: false,
        tool_chunks,
        tool_jobs: &empty_jobs,
        think_open,
    })
}

fn message(id: &str, role: &str, text: &str) -> StoredMessage {
    StoredMessage {
        id: id.to_string(),
        role: role.to_string(),
        text: text.to_string(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    }
}

#[test]
fn streaming_tokens_incremental_open_plain() {
    let user = message("u1", "user", "hi");
    let mut assistant = message("a1", "assistant", "");
    assistant.state = Some(StoredMessageState::Loading);
    let messages = vec![user, assistant];
    let overlay1 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "**a".to_string(),
        reasoning: String::new(),
    };
    let empty = HashMap::new();
    let plan1 = sync(None, &messages, "s1", Some(&overlay1), &empty);
    assert!(plan1.full_html.is_some());

    let overlay2 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "**ab".to_string(),
        reasoning: String::new(),
    };
    let plan2 = sync(Some(&plan1.next), &messages, "s1", Some(&overlay2), &empty);
    assert!(plan2.full_html.is_none());
    assert!(plan2.append_sections.is_empty());
    let live = plan2.live.expect("live patch");
    assert_eq!(live.message_id, "a1");
    match live.patch {
        TuiBodyPatch::Incremental {
            append_closed,
            open_plain,
        } => {
            assert!(append_closed.is_empty());
            assert_eq!(open_plain.as_deref(), Some("**ab"));
        }
        other => panic!("expected Incremental, got {other:?}"),
    }
}

#[test]
fn empty_loading_shell_not_appended_until_overlay_has_text() {
    let user = message("u1", "user", "hi");
    let empty = HashMap::new();
    let plan1 = sync(None, std::slice::from_ref(&user), "s1", None, &empty);
    assert!(plan1.full_html.is_some());
    assert_eq!(plan1.next.mounted_ids, vec!["u1".to_string()]);

    let mut assistant = message("a1", "assistant", "");
    assistant.state = Some(StoredMessageState::Loading);
    let messages = vec![user.clone(), assistant];
    let empty_overlay = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: String::new(),
        reasoning: String::new(),
    };
    let plan2 = sync(
        Some(&plan1.next),
        &messages,
        "s1",
        Some(&empty_overlay),
        &empty,
    );
    assert!(plan2.full_html.is_none());
    assert!(
        plan2.append_sections.is_empty(),
        "empty loading shell must not mount"
    );
    assert_eq!(plan2.next.mounted_ids, vec!["u1".to_string()]);

    let with_text = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "你好".to_string(),
        reasoning: String::new(),
    };
    let plan3 = sync(Some(&plan2.next), &messages, "s1", Some(&with_text), &empty);
    assert!(plan3.full_html.is_none());
    assert_eq!(plan3.append_sections.len(), 1);
    assert!(plan3.append_sections[0].contains("data-tui-live=\"1\""));
    assert!(plan3.append_sections[0].contains("你好"));
    assert!(
        plan3.live.is_none(),
        "new live body already in section html"
    );
}

#[test]
fn session_switch_forces_full_rebuild() {
    let messages = vec![message("u1", "user", "hi")];
    let empty = HashMap::new();
    let plan1 = sync(None, &messages, "s1", None, &empty);
    let plan2 = sync(Some(&plan1.next), &messages, "s2", None, &empty);
    assert!(plan2.full_html.is_some());
}

#[test]
fn finished_assistant_bold_becomes_strong() {
    let messages = vec![
        message("u1", "user", "你好"),
        message("a1", "assistant", "**原样**"),
    ];
    let output = build_html(&messages, None, &HashSet::new());
    assert!(output.contains("data-tui-msg-id"), "got {output}");
    assert!(output.contains("chat-tui-turn--user"), "got {output}");
    assert!(output.contains("chat-tui-turn--assistant"), "got {output}");
    assert!(output.contains("chat-tui-role-label"), "got {output}");
    for section in output.split("<section class=\"chat-tui-turn") {
        if let Some(end) = section.find("</section>") {
            assert!(
                !section[..end].contains("chat-tui-role-label"),
                "role label must be outside section card, got {output}"
            );
        }
    }
    assert!(!output.contains("chat-tui-turn-actions"), "got {output}");
    assert!(!output.contains("data-tui-action=\"copy\""), "got {output}");
    assert!(output.contains("data-tui-msg-idx"), "got {output}");
    assert!(!output.contains('❯'), "got {output}");
    assert!(
        output.contains("<strong>") || output.contains("<b>"),
        "got {output}"
    );
}

#[test]
fn tool_turn_uses_tool_modifier_without_generic_role_word() {
    let mut tool = message("t1", "assistant", "ok");
    tool.is_tool = true;
    tool.tool_name = Some("read_file".to_string());
    let output = build_html(&[tool], None, &HashSet::new());
    assert!(output.contains("chat-tui-turn--tool"), "got {output}");
    assert!(output.contains("chat-tui-tool-process"), "got {output}");
    assert!(output.contains("read_file"), "got {output}");
    assert!(!output.contains("工具:"), "got {output}");
}

#[test]
fn promote_finished_assistant_uses_incremental_not_replace_all() {
    let user = message("u1", "user", "hi");
    let mut assistant = message("a1", "assistant", "");
    assistant.state = Some(StoredMessageState::Loading);
    let messages_loading = vec![user.clone(), assistant.clone()];
    let overlay = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "hello\n\n**tail**".to_string(),
        reasoning: String::new(),
    };
    let empty = HashMap::new();
    let plan1 = sync(None, &messages_loading, "s1", Some(&overlay), &empty);
    assert!(plan1.next.live_id.as_deref() == Some("a1"));

    assistant.state = None;
    assistant.text = "hello\n\n**tail**".to_string();
    let messages_done = vec![user, assistant];
    let plan2 = sync(Some(&plan1.next), &messages_done, "s1", None, &empty);
    assert!(plan2.next.live_id.is_none());
    let live = plan2.live.expect("promote patch");
    assert_eq!(live.message_id, "a1");
    match live.patch {
        TuiBodyPatch::Incremental { .. } => {}
        TuiBodyPatch::ReplaceAll { .. } => {
            panic!("promote must not ReplaceAll whole body (jitter)")
        }
        other => panic!("unexpected patch {other:?}"),
    }
}

#[test]
fn live_tool_chunk_uses_tool_row_patch_not_replace_all() {
    let user = message("u1", "user", "hi");
    let mut tool = message("t1", "assistant", "");
    tool.is_tool = true;
    tool.tool_name = Some("read_file".to_string());
    tool.tool_call_id = Some("tc1".to_string());
    tool.state = Some(StoredMessageState::Loading);
    let messages = vec![user, tool];
    let mut chunks = HashMap::new();
    chunks.insert("tc1".to_string(), "part-a".to_string());
    let plan1 = sync(None, &messages, "s1", None, &chunks);
    assert!(plan1.full_html.is_some());
    assert_eq!(plan1.next.live_tool_has_details, Some(false));

    chunks.insert("tc1".to_string(), "part-a part-b".to_string());
    let plan2 = sync(Some(&plan1.next), &messages, "s1", None, &chunks);
    assert!(plan2.full_html.is_none());
    let live = plan2.live.expect("tool live patch");
    assert_eq!(live.message_id, "t1");
    match live.patch {
        TuiBodyPatch::ToolRow {
            status,
            status_label,
            one_line,
            detail,
        } => {
            assert_eq!(status, "⏳");
            assert!(
                status_label.contains("执行") || status_label.contains("Running"),
                "{status_label}"
            );
            assert!(one_line.contains("part-b"), "{one_line}");
            assert!(detail.is_none());
        }
        other => panic!("expected ToolRow, got {other:?}"),
    }
}

#[test]
fn skill_slash_chip_stays_on_same_line_as_task() {
    let user = message("u1", "user", "/rust-style 分析一下");
    let html = build_html(std::slice::from_ref(&user), None, &HashSet::new());
    assert!(html.contains("msg-skill-invoke"), "{html}");
    assert!(html.contains("rust-style"), "{html}");
    assert!(html.contains("分析一下"), "{html}");
    // 旧实现：裸 chip 后再跟独立 chat-tui-line，视觉上多一换行。
    assert!(
        !html.contains("</span> <div class=\"chat-tui-line"),
        "chip must not sit outside the first line block: {html}"
    );
    assert!(
        html.contains("msg-skill-invoke") && html.contains("chat-tui-line"),
        "{html}"
    );
    let chip_at = html.find("msg-skill-invoke").expect("chip");
    let line_at = html.find("chat-tui-line").expect("line");
    assert!(
        line_at < chip_at,
        "chip should be inside a line wrapper: {html}"
    );
}

#[test]
fn file_ref_chip_stays_on_same_line_as_following_text() {
    let user = message("u1", "user", "@.gitignore 这个文件是什么");
    let html = build_html(std::slice::from_ref(&user), None, &HashSet::new());
    assert!(html.contains("msg-file-ref"), "{html}");
    assert!(html.contains(".gitignore"), "{html}");
    assert!(html.contains("这个文件是什么"), "{html}");
    assert!(
        !html.contains("</span><div class=\"chat-tui-line")
            && !html.contains("</span> <div class=\"chat-tui-line"),
        "file-ref chip must not sit outside the line block: {html}"
    );
    let chip_at = html.find("msg-file-ref").expect("chip");
    let line_at = html.find("chat-tui-line").expect("line");
    assert!(
        line_at < chip_at,
        "file-ref chip should be inside a line wrapper: {html}"
    );
    // 占位符不得泄漏到最终 HTML。
    assert!(!html.contains("CMFR"), "{html}");
}

#[test]
fn user_bubble_shows_chat_upload_images() {
    let mut user = message("u1", "user", "看图");
    user.image_urls = vec!["/uploads/u1_2_3.png".into(), "/uploads/../x".into()];
    let html = build_html(std::slice::from_ref(&user), None, &HashSet::new());
    assert!(html.contains("chat-tui-user-images"), "{html}");
    assert!(html.contains("src=\"/uploads/u1_2_3.png\""), "{html}");
    assert!(html.contains("alt=\"附图 u1_2_3.png\""), "{html}");
    assert!(!html.contains("/uploads/../x"), "{html}");
}

#[test]
fn committed_fingerprint_includes_user_image_urls() {
    let plain = message("u1", "user", "看图");
    let mut with_img = plain.clone();
    with_img.image_urls = vec!["/uploads/a.png".into()];
    let a = committed_fingerprint(&[(0, &plain)], None);
    let b = committed_fingerprint(&[(0, &with_img)], None);
    assert_ne!(a, b);
}

fn assistant_with_reasoning(id: &str, reasoning: &str, text: &str, loading: bool) -> StoredMessage {
    let mut m = message(id, "assistant", text);
    m.reasoning_text = reasoning.to_string();
    m.state = if loading {
        Some(StoredMessageState::Loading)
    } else {
        None
    };
    m
}

fn build_html(
    messages: &[StoredMessage],
    overlay: Option<&StreamTextOverlay>,
    think_open: &HashSet<String>,
) -> String {
    build_html_opts(messages, overlay, think_open, false, true)
}

fn build_html_opts(
    messages: &[StoredMessage],
    overlay: Option<&StreamTextOverlay>,
    think_open: &HashSet<String>,
    apply_filters: bool,
    markdown_render: bool,
) -> String {
    let tool_chunks = HashMap::new();
    let tool_jobs = HashMap::new();
    build_tui_transcript_html(
        messages,
        &TuiRenderCtx {
            session_id: "s1",
            overlay,
            locale: Locale::ZhHans,
            apply_filters,
            markdown_render,
            show_turn_context_inject: false,
            tool_chunks: &tool_chunks,
            tool_jobs: &tool_jobs,
            think_open,
        },
    )
}

fn render_one(m: &StoredMessage, overlay: Option<&StreamTextOverlay>) -> String {
    render_one_with_think(m, overlay, &HashSet::new())
}

fn render_one_with_think(
    m: &StoredMessage,
    overlay: Option<&StreamTextOverlay>,
    think_open: &HashSet<String>,
) -> String {
    build_html(std::slice::from_ref(m), overlay, think_open)
}

#[test]
fn assistant_reasoning_renders_collapsed_thinking_details() {
    let m = assistant_with_reasoning("a1", "先检查类型\n再写实现", "这是答案", false);
    let html = render_one(&m, None);
    assert!(html.contains("chat-tui-think"), "got {html}");
    assert!(
        html.contains("<details class=\"chat-tui-think\">"),
        "finalized details must be collapsed (no open attr): {html}"
    );
    assert!(html.contains("思考"), "summary label, got {html}");
    assert!(html.contains("先检查类型"), "thinking body, got {html}");
    assert!(html.contains("这是答案"), "answer body, got {html}");
}

#[test]
fn loading_assistant_reasoning_details_collapsed_and_overlay_streams() {
    let m = assistant_with_reasoning("a1", "", "", true);
    let overlay = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "正在回答".to_string(),
        reasoning: "逐步推理中".to_string(),
    };
    let html = render_one(&m, Some(&overlay));
    assert!(
        html.contains("<details class=\"chat-tui-think\">"),
        "live details must stay collapsed by default: {html}"
    );
    assert!(
        !html.contains("<details class=\"chat-tui-think\" open>"),
        "must not auto-open while streaming: {html}"
    );
    assert!(
        html.contains("逐步推理中"),
        "streamed reasoning, got {html}"
    );
    assert!(html.contains("正在回答"), "streamed answer, got {html}");
    assert!(m.reasoning_text.is_empty(), "fixture sanity");
}

#[test]
fn user_message_has_no_thinking_block() {
    let u = message("u1", "user", "你好");
    let html = render_one(&u, None);
    assert!(!html.contains("chat-tui-think"), "got {html}");
}

#[test]
fn reasoning_streaming_uses_think_body_patch_not_replace_all() {
    // 思维链流式：只定向更新 `.chat-tui-think-body`，不再整块 ReplaceAll（消除每 token 重渲）。
    let m = assistant_with_reasoning("a1", "", "", true);
    let messages = vec![m.clone()];
    let overlay1 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: String::new(),
        reasoning: "推理第一步".to_string(),
    };
    let empty = HashMap::new();
    let plan1 = sync(None, &messages, "s1", Some(&overlay1), &empty);
    assert!(plan1.full_html.is_some());

    let overlay2 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: String::new(),
        reasoning: "推理第一步，第二步".to_string(),
    };
    let plan2 = sync(Some(&plan1.next), &messages, "s1", Some(&overlay2), &empty);
    assert!(plan2.full_html.is_none());
    let live = plan2.live.expect("live patch");
    match live.patch {
        TuiBodyPatch::ThinkBody {
            body_html,
            append_closed,
            open_plain,
        } => {
            assert!(body_html.contains("第二步"), "got {body_html}");
            assert!(append_closed.is_empty());
            assert_eq!(open_plain, None);
        }
        other => panic!("expected ThinkBody for streaming thinking, got {other:?}"),
    }
}

#[test]
fn reasoning_transition_to_answer_is_incremental_after_freeze() {
    // 思维链冻结后，终答增量恢复 Incremental（think 不再变化，closed/open_plain 走增量）。
    let m = assistant_with_reasoning("a1", "推理完毕", "", true);
    let messages = vec![m.clone()];
    let empty = HashMap::new();
    let overlay1 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "答".to_string(),
        reasoning: "推理完毕".to_string(),
    };
    let plan1 = sync(None, &messages, "s1", Some(&overlay1), &empty);
    assert!(plan1.full_html.is_some());

    let overlay2 = StreamTextOverlay {
        session_id: "s1".to_string(),
        message_id: "a1".to_string(),
        answer: "答案二".to_string(),
        reasoning: "推理完毕".to_string(),
    };
    let plan2 = sync(Some(&plan1.next), &messages, "s1", Some(&overlay2), &empty);
    assert!(plan2.full_html.is_none());
    let live = plan2.live.expect("live patch");
    match live.patch {
        TuiBodyPatch::Incremental {
            append_closed,
            open_plain,
        } => {
            assert!(append_closed.is_empty());
            assert_eq!(open_plain.as_deref(), Some("答案二"));
        }
        other => panic!("expected Incremental after thinking froze, got {other:?}"),
    }
}

#[test]
fn manually_opened_thinking_kept_open_after_turn_content_refresh() {
    // 用户手动展开历史回合的思考块后，`plan_refresh_bodies` 重建 body 必须保持 `open`。
    let a1 = assistant_with_reasoning("a1", "历史推理", "历史答案", false);
    let messages = vec![a1];
    let empty = HashMap::new();
    let plan1 = sync(None, &messages, "s1", None, &empty);
    assert!(plan1.full_html.is_some());
    assert!(
        plan1
            .full_html
            .as_ref()
            .unwrap()
            .contains("<details class=\"chat-tui-think\">"),
        "finalized turn must start collapsed: {plan1:?}"
    );

    let mut think_open = HashSet::new();
    think_open.insert("a1".to_string());
    // 内容变化（committed_key 改变）→ 走 refresh_bodies 而非结构 no-op。
    let a1_changed = assistant_with_reasoning("a1", "历史推理", "历史答案（更新）", false);
    let plan2 = sync_with_think(
        Some(&plan1.next),
        std::slice::from_ref(&a1_changed),
        "s1",
        None,
        &empty,
        &think_open,
    );
    assert!(plan2.full_html.is_none());
    let refreshed = plan2
        .refresh_bodies
        .iter()
        .find(|p| p.message_id == "a1")
        .expect("a1 body must be refreshed");
    match &refreshed.patch {
        TuiBodyPatch::ReplaceAll { chunks } => {
            let html = chunks.to_inner_html();
            assert!(
                html.contains("<details class=\"chat-tui-think\" open>"),
                "manually opened thinking must survive refresh: {html}"
            );
            assert!(html.contains("历史答案（更新）"), "got {html}");
        }
        other => panic!("expected ReplaceAll refresh, got {other:?}"),
    }
}

#[test]
fn non_opened_thinking_stays_collapsed_after_turn_content_refresh() {
    // 未手动展开的回合，刷新后仍保持收起（默认行为不回退）。
    // committed_fingerprint 按 text.len() 计，fixture 须让内容长度变化以触发 refresh。
    let a1 = assistant_with_reasoning("a1", "推理", "答案 v1", false);
    let empty = HashMap::new();
    let plan1 = sync(None, std::slice::from_ref(&a1), "s1", None, &empty);
    let a1_changed = assistant_with_reasoning("a1", "推理", "答案 v2 更长的尾巴", false);
    let plan2 = sync(
        Some(&plan1.next),
        std::slice::from_ref(&a1_changed),
        "s1",
        None,
        &empty,
    );
    let refreshed = plan2
        .refresh_bodies
        .iter()
        .find(|p| p.message_id == "a1")
        .expect("a1 body must be refreshed");
    match &refreshed.patch {
        TuiBodyPatch::ReplaceAll { chunks } => {
            let html = chunks.to_inner_html();
            assert!(
                html.contains("<details class=\"chat-tui-think\">"),
                "non-manual turn must stay collapsed: {html}"
            );
        }
        other => panic!("expected ReplaceAll refresh, got {other:?}"),
    }
}

#[test]
fn inline_think_extracted_into_thinking_block_when_filters_on() {
    // filters=true（默认）时，内联 `<think>`（Qwen / vLLM 网关）也要进折叠块。
    let mut m = message("a1", "assistant", "<think>步骤甲</think>\n\n这是答案");
    m.state = None;
    let html = build_html_opts(std::slice::from_ref(&m), None, &HashSet::new(), true, true);
    assert!(
        html.contains("<details class=\"chat-tui-think\">"),
        "inline think must render a thinking block: {html}"
    );
    assert!(html.contains("步骤甲"), "thinking body, got {html}");
    assert!(html.contains("这是答案"), "answer body, got {html}");
    assert!(!html.contains("<think>"), "raw tag must not leak: {html}");
}

#[test]
fn markdown_off_thinking_block_escapes_markup() {
    let m = assistant_with_reasoning("a1", "**加粗** 与 <script>", "答案", false);
    let html = build_html_opts(
        std::slice::from_ref(&m),
        None,
        &HashSet::new(),
        false,
        false,
    );
    assert!(html.contains("chat-tui-think"), "got {html}");
    assert!(
        !html.contains("<strong>"),
        "markdown off must not render bold in thinking: {html}"
    );
    assert!(!html.contains("<script"), "must escape markup: {html}");
    assert!(
        html.contains("加粗") || html.contains("&#42;"),
        "literal thinking should remain: {html}"
    );
}

#[test]
fn echo_thinking_section_not_duplicated_in_answer_bubble() {
    // 模型在正文回显「### 思考过程」章节且与 reasoning 一致 → 正文段剥除，思考块只留一份。
    let mut m = message("a1", "assistant", "### 思考过程\n推理内容\n---\n这是答案");
    m.reasoning_text = "推理内容".to_string();
    let html = build_html_opts(std::slice::from_ref(&m), None, &HashSet::new(), true, true);
    assert!(
        html.contains("chat-tui-think"),
        "thinking block expected: {html}"
    );
    assert!(html.contains("推理内容"), "thinking in block: {html}");
    assert!(
        !html.contains("### 思考过程"),
        "echo section must be stripped from the answer body: {html}"
    );
    assert!(html.contains("这是答案"), "real answer kept: {html}");
}

#[test]
fn echo_only_answer_hides_empty_answer_section() {
    // 模型正文**只有**思考回显章节（无真实结论）：剥除后正文段保留 DOM（供流式定位）
    // 但整段 hidden，不渲染出空正文气泡，思考仍只出现一次。
    let mut m = message("a1", "assistant", "### 思考过程\n推理内容\n---\n");
    m.reasoning_text = "推理内容".to_string();
    let html = build_html_opts(std::slice::from_ref(&m), None, &HashSet::new(), true, true);
    assert!(
        html.contains("chat-tui-turn--think"),
        "think section: {html}"
    );
    assert!(
        html.contains("chat-tui-body--answer"),
        "answer section stays for streaming: {html}"
    );
    assert!(
        html.contains("chat-tui-turn--hidden"),
        "stripped-empty answer section must be hidden: {html}"
    );
    assert_eq!(
        html.matches("推理内容").count(),
        1,
        "thinking shown exactly once (in the block): {html}"
    );
    assert!(!html.contains("### 思考过程"), "echo stripped: {html}");
}

#[test]
fn thinking_rendered_as_own_section_above_answer() {
    // 思考过程单独成段（独立气泡），正文段紧随其后，同一 wrap 内两个 section。
    let m = assistant_with_reasoning("a1", "思考内容", "答案内容", false);
    let html = render_one(&m, None);
    assert_eq!(
        html.matches("<section class=\"chat-tui-turn").count(),
        2,
        "thinking + answer sections expected: {html}"
    );
    let think_at = html.find("chat-tui-turn--think").expect("think section");
    let answer_at = html.find("chat-tui-body--answer").expect("answer section");
    assert!(
        think_at < answer_at,
        "think bubble must sit above answer: {html}"
    );
    let think_part = &html[..answer_at];
    assert!(think_part.contains("思考内容"), "think body, got {html}");
    assert!(
        !think_part.contains("答案内容"),
        "answer must not be inside the think section: {html}"
    );
    assert!(html.contains("答案内容"), "answer body present: {html}");
    assert!(!html.contains("<think>"), "raw tag must not leak: {html}");

    // 无思考消息保持单段。
    let u = message("u1", "user", "你好");
    let html_u = render_one(&u, None);
    assert_eq!(
        html_u.matches("<section class=\"chat-tui-turn").count(),
        1,
        "no-think message keeps a single section: {html_u}"
    );
}
