//! timeline_scan 单测（自 [`super`] 拆出，避免 lizard 误解析超长 `mod tests`）。

use super::*;
use crate::storage::{StoredMessage, StoredMessageState};

#[test]
fn context_inject_kind_and_json_body_detected() {
    assert!(is_turn_context_inject_kind("context_inject"));
    assert!(is_turn_context_inject_kind(" context_trim "));
    assert!(!is_turn_context_inject_kind("final_response"));
    assert!(is_turn_context_inject_timeline_text(
        r#"{"kind":"context_inject","title":"inject"}"#
    ));
    assert!(is_turn_context_inject_timeline_text(
        "本轮注入\n{\"skills\":[\"rust-style\"],\"forced\":false}"
    ));
    assert!(is_turn_context_inject_timeline_text(
        r#"{"count_hit":true,"n_before":40,"n_after":12}"#
    ));
    assert!(!is_turn_context_inject_timeline_text("hello"));
}

#[test]
fn legacy_local_snapshot_dropped_when_server_has_same_assistant_text() {
    let snap = StoredMessage {
        id: "legacy-fr".into(),
        role: "assistant".into(),
        text: "same answer".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(timeline_state_local_snapshot()),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let server = vec![StoredMessage {
        id: "srv".into(),
        role: "assistant".into(),
        text: "same answer".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 1,
    }];
    assert!(!should_preserve_local_timeline_on_hydrate(&snap, &server));
    assert!(is_ephemeral_timeline_assistant_for_export(
        &snap,
        &[snap.clone(), server[0].clone()]
    ));
}

#[test]
fn parses_context_trim_after_tokens() {
    // 正常 `after_tokens`。
    assert_eq!(
        context_trim_after_used_tokens(Some(r#"{"count_hit":true,"after_tokens":1234}"#)),
        Some(1234)
    );
    // 无 `after_tokens` / `null` / 缺 detail → 不重置。
    assert_eq!(
        context_trim_after_used_tokens(Some(r#"{"n_before":3}"#)),
        None
    );
    assert_eq!(
        context_trim_after_used_tokens(Some(r#"{"after_tokens":null}"#)),
        None
    );
    assert_eq!(context_trim_after_used_tokens(None), None);
    // 空串 / 非 JSON → 不重置。
    assert_eq!(context_trim_after_used_tokens(Some("  ")), None);
    assert_eq!(context_trim_after_used_tokens(Some("not json")), None);
}

#[test]
fn context_trim_watermark_reset_requires_all_conditions() {
    // 三条件齐备 → 返回归属与回落值（0 亦为合法回落）。
    assert_eq!(
        context_trim_watermark_reset(true, Some(18000), Some("cid-1")),
        Some(("cid-1", 18000))
    );
    assert_eq!(
        context_trim_watermark_reset(true, Some(0), Some("cid-1")),
        Some(("cid-1", 0))
    );
    // 后台流（本轮不被查看）→ 不重置，避免污染活跃会话。
    assert_eq!(
        context_trim_watermark_reset(false, Some(18000), Some("cid-1")),
        None
    );
    // 解析不出 after_tokens → 不重置。
    assert_eq!(
        context_trim_watermark_reset(true, None, Some("cid-1")),
        None
    );
    // 无服务端 conversation_id → 不重置。
    assert_eq!(context_trim_watermark_reset(true, Some(18000), None), None);
}

#[test]
fn context_trim_parsed_from_hydrate_stream_and_detail_forms() {
    // 水合：外层 `{kind,title,detail}`，detail 为 JSON 字符串。
    let hydrated = r#"{"kind":"context_trim","title":"已裁剪历史","detail":"{\"count_hit\":true,\"n_before\":40,\"n_after\":12,\"after_tokens\":3456,\"max_input_tokens\":160000}"}"#;
    let info = context_trim_from_timeline_text(hydrated).expect("hydrate form");
    assert_eq!(info.title, "已裁剪历史");
    assert_eq!(info.after_tokens, Some(3456));
    assert_eq!(info.max_input_tokens, Some(160_000));
    assert_eq!(info.n_before, Some(40));
    assert_eq!(info.n_after, Some(12));
    assert!(info.detail.contains("\"n_before\": 40"));

    // 流式：title + `\n` + detail JSON（黄金 fixture 的仅 `n_before` 形态）。
    let streamed = "已压缩工具输出\n{\"compress_hits\":2,\"n_before\":3}";
    let info = context_trim_from_timeline_text(streamed).expect("stream form");
    assert_eq!(info.title, "已压缩工具输出");
    assert_eq!(info.n_before, Some(3));
    assert_eq!(info.after_tokens, None);

    // 纯 detail JSON：无标题，展示层回退 i18n。
    let info = context_trim_from_timeline_text(r#"{"n_before":3}"#).expect("detail form");
    assert!(info.title.is_empty());

    // 水合 detail 已是对象（非字符串）时，直接取该对象，不落回外层。
    let object_detail = r#"{"kind":"context_trim","title":"已裁剪历史","detail":{"n_before":40,"after_tokens":3456}}"#;
    let info = context_trim_from_timeline_text(object_detail).expect("object detail form");
    assert_eq!(info.title, "已裁剪历史");
    assert_eq!(info.n_before, Some(40));
    assert_eq!(info.after_tokens, Some(3456));

    // `context_inject`（含 skills）不得误判为 trim。
    let inject = r#"{"kind":"context_inject","title":"本轮已注入上下文","detail":"{\"kinds\":[\"skill\"],\"skills\":[\"rust-style\"],\"forced\":false}"}"#;
    assert!(context_trim_from_timeline_text(inject).is_none());
    assert!(context_trim_from_timeline_text("hello").is_none());
}

#[test]
fn local_trim_snapshot_dropped_when_server_has_equivalent_trim() {
    let local = StoredMessage {
        id: "local-trim".into(),
        role: "assistant".into(),
        text: "已裁剪历史\n{\"count_hit\":true,\"n_before\":40,\"n_after\":12}".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(timeline_state_local_snapshot()),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    // 服务端落盘 detail 计数可能继续累加，`n_before` / 标题仍稳定。
    let server = vec![StoredMessage {
        id: "srv-trim".into(),
        role: "system".into(),
        text: r#"{"kind":"context_trim","title":"已裁剪历史","detail":"{\"count_hit\":true,\"n_before\":40,\"n_after\":8}"}"#
            .into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 1,
    }];
    assert!(is_context_trim_timeline_message(&local));
    assert!(!should_preserve_local_timeline_on_hydrate(&local, &server));
}

fn stored_trim_row(id: &str, role: &str, text: &str, local_snapshot: bool) -> StoredMessage {
    StoredMessage {
        id: id.into(),
        role: role.into(),
        text: text.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: local_snapshot.then(timeline_state_local_snapshot),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    }
}

#[test]
fn trim_dedupe_requires_a_comparable_stable_key() {
    let local = stored_trim_row(
        "local",
        "assistant",
        "已裁剪历史\n{\"after_tokens\":3456}",
        true,
    );
    // 同标题、after_tokens 相同、两侧都无 n_before → 同一事件（e2e fixture 形态）。
    // 普通转义字符串：含 `\"` 的 raw string 会让 lizard 的 token 配对失准，误报超长函数。
    let same = vec![stored_trim_row(
        "srv-same",
        "system",
        "{\"kind\":\"context_trim\",\"title\":\"已裁剪历史\",\"detail\":\"{\\\"after_tokens\\\":3456}\"}",
        false,
    )];
    assert!(!should_preserve_local_timeline_on_hydrate(&local, &same));

    // 同标题、无共同键（本地只有 n_before，服务端只有 after_tokens）→ 保守保留。
    let local_n = stored_trim_row(
        "local-n",
        "assistant",
        "已裁剪历史\n{\"n_before\":40}",
        true,
    );
    assert!(should_preserve_local_timeline_on_hydrate(&local_n, &same));
}

#[test]
fn trim_dedupe_same_title_without_n_before_is_not_equivalent() {
    // e2e fixture 形态：detail 只有 `after_tokens`，两侧都推不出 `n_before`。
    let local = stored_trim_row(
        "local",
        "assistant",
        "已裁剪历史\n{\"after_tokens\":3456}",
        true,
    );
    let server = vec![stored_trim_row(
        "srv",
        "system",
        "{\"kind\":\"context_trim\",\"title\":\"已裁剪历史\",\"detail\":\"{\\\"after_tokens\\\":9000}\"}",
        false,
    )];
    // 同标题但 after_tokens 不同 → 不是同一事件，宁可保留本地快照。
    assert!(should_preserve_local_timeline_on_hydrate(&local, &server));
}

#[test]
fn parses_final_response_snapshot_without_msg_field() {
    let state =
        StoredMessageState::from_wire(r#"{"k":"cm_tl","t":"final_response_snapshot"}"#.into());
    assert_eq!(
        timeline_ui_snapshot_type(&state).as_deref(),
        Some("final_response_snapshot")
    );
}

#[test]
fn final_response_snapshot_dropped_when_server_has_same_text() {
    let snap = StoredMessage {
        id: "fr1".into(),
        role: "assistant".into(),
        text: "hello world".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(timeline_state_final_response_snapshot()),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let server = vec![StoredMessage {
        id: "srv".into(),
        role: "assistant".into(),
        text: "hello world".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 1,
    }];
    assert!(is_ephemeral_timeline_assistant_for_export(&snap, &server));
    assert!(!should_preserve_local_timeline_on_hydrate(&snap, &server));
}

#[test]
fn planner_tool_call_rejected_timeline_dropped_for_export() {
    let m = StoredMessage {
        id: "reject".into(),
        role: "assistant".into(),
        text: "规划轮工具调用已拒绝\ncode=PLANNER_TOOL_CALL_REJECTED".to_string(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(timeline_state_local_snapshot()),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn commentary_before_tools_dropped_for_export() {
    let m = StoredMessage {
        id: "c1".into(),
        role: "assistant".into(),
        text: String::new(),
        reasoning_text: "先写总结\n".into(),
        image_urls: vec![],
        state: Some(crate::storage::StoredMessageState::CommentaryBeforeTools),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_commentary_before_tools_assistant(&m));
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn staged_planner_round_dropped_for_export() {
    let m = StoredMessage {
        id: "plan".into(),
        role: "assistant".into(),
        text: r#"1. `rerun-demo`: 重新运行 demo

```json
{"type":"agent_reply_plan","version":1,"steps":[{"id":"rerun-demo","description":"重新运行 demo"}],"no_task":false}
```"#
            .into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn tool_argument_array_residue_dropped_for_export() {
    let m = StoredMessage {
        id: "args".into(),
        role: "assistant".into(),
        text: r#"["-c","tar xzf hpcg-HPCG-release-3-1-0.tar.gz"]"#.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn bare_shell_command_residue_dropped_for_export() {
    let m = StoredMessage {
        id: "cmd".into(),
        role: "assistant".into(),
        text: "tar -xzf hpcg-HPCG-release-3-1-0.tar.gz".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn bare_archive_path_argument_residue_dropped_for_export() {
    let m = StoredMessage {
        id: "path-arg".into(),
        role: "assistant".into(),
        text: "hpcg-HPCG-release-3-1-0.tar.gz".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn truncated_install_path_residue_dropped_for_export() {
    let m = StoredMessage {
        id: "install-trunc".into(),
        role: "assistant".into(),
        text: "hpcg-HPCG-release-3-1-0/INSTALLe".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn makefile_path_residue_not_dropped_for_export() {
    let m = StoredMessage {
        id: "makefile-path".into(),
        role: "assistant".into(),
        text: "hpcg-HPCG-release-3-1-0/Makefile".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(!is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn explanatory_shell_command_answer_is_kept_for_export() {
    let m = StoredMessage {
        id: "cmd-explain".into(),
        role: "assistant".into(),
        text: "可以执行：\n\n```bash\ntar -xzf archive.tar.gz\n```".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(!is_ephemeral_timeline_assistant_for_export(&m, &[]));
}

#[test]
fn context_inject_skills_parses_three_shapes_and_dedupes() {
    // 流式：`title\n<detail>`，id 与 `.md` 路径归一为同一展示名。
    let streamed = "本轮注入\n{\"kinds\":[\"skill\"],\"skills\":[\"rust-style\",\".crabmate/skills/rust-style.md\"],\"forced\":true}";
    let info = context_inject_skills(streamed).expect("streamed");
    assert_eq!(info.skills, vec!["rust-style".to_string()]);
    assert!(info.forced);

    // 水合：外层 `{kind,title,detail}`，`detail` 是 JSON 字符串。
    let hydrated = r#"{"kind":"context_inject","title":"本轮注入","detail":"{\"skills\":[\"rust-style\"],\"forced\":false}"}"#;
    let info = context_inject_skills(hydrated).expect("hydrated");
    assert_eq!(info.skills, vec!["rust-style".to_string()]);
    assert!(!info.forced);

    // 纯 detail JSON。
    let pure = r#"{"skills":["a","b"],"forced":false}"#;
    assert_eq!(
        context_inject_skills(pure).expect("pure").skills,
        vec!["a".to_string(), "b".to_string()]
    );

    // 无 skill / 非法输入 → None（不得画空卡）。
    assert!(context_inject_skills("hello").is_none());
    assert!(context_inject_skills(r#"{"skills":[],"forced":true}"#).is_none());
    assert!(context_inject_skills(r#"{"count_hit":true,"n_before":40}"#).is_none());
    // `.md` 后缀大小写不敏感。
    assert_eq!(
        context_inject_skills(r#"{"skills":["foo.MD"],"forced":false}"#)
            .expect("uppercase suffix")
            .skills,
        vec!["foo".to_string()]
    );
}

#[test]
fn plain_message_with_skills_json_is_not_skill_inject() {
    // 普通消息正文恰为 `{"skills":[...]}`：detail 解析能取到，但注入门控必须拒绝，
    // 否则会被画成 skill 卡并丢掉角色标签 / 原文。
    let raw = r#"{"skills":["rust-style"]}"#;
    assert!(context_inject_skills(raw).is_some());
    assert!(skill_inject_from_timeline_text(raw).is_none());
    let m = StoredMessage {
        id: "u1".into(),
        role: "user".into(),
        text: raw.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    assert!(!is_skill_inject_timeline_message(&m));
}

#[test]
fn local_skill_inject_snapshot_dropped_when_server_has_equivalent() {
    let snap = StoredMessage {
        id: "local-skill".into(),
        role: "assistant".into(),
        text: "本轮注入\n{\"skills\":[\"rust-style\"],\"forced\":false}".into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: Some(timeline_state_local_snapshot()),
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 0,
    };
    let server = vec![StoredMessage {
        id: "srv-skill".into(),
        role: "system".into(),
        text: r#"{"kind":"context_inject","title":"本轮注入","detail":"{\"skills\":[\"rust-style\"],\"forced\":false}"}"#.into(),
        reasoning_text: String::new(),
        image_urls: vec![],
        state: None,
        is_tool: false,
        tool_call_id: None,
        tool_name: None,
        created_at: 1,
    }];
    assert!(is_skill_inject_timeline_message(&snap));
    assert!(is_skill_inject_timeline_message(&server[0]));
    assert!(!should_preserve_local_timeline_on_hydrate(&snap, &server));
    // 服务端没有等价 skill 注入时保留本地快照。
    assert!(should_preserve_local_timeline_on_hydrate(&snap, &[]));
}
