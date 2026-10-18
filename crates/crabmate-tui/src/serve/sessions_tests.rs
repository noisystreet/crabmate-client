//! [`super`]（sessions.rs）的索引解析与变更单测。
//!
//! 独立成文件：与 md / render / state 同理，避免单文件行数越过 fn-nloc 门禁。

use super::*;
use serde_json::json;

fn stamp(local_id: &str) -> IndexStamp {
    IndexStamp {
        now_ms: 1_700_000_000_000,
        new_local_id: local_id.to_string(),
    }
}

#[test]
fn parses_rows() {
    let v = json!([
        {"id":"s1","title":"hi","server_conversation_id":"c1"},
        {"id":"s2","title":"","extra":true}
    ]);
    let rows = SessionListRow::parse_rows(&v);
    assert_eq!(rows.len(), 2);
    assert_eq!(conversation_id_for_resume(&rows[0]), Some("c1"));
    assert_eq!(conversation_id_for_resume(&rows[1]), None);
}

#[test]
fn new_blank_row_is_first_and_active() {
    // 旧行有正文（不是空白会话），所以这里必须真的插一行。
    let raw = json!([{"id":"s1","title":"old","messages":[{"role":"user","content":"hi"}]}]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::NewBlank {
            title: "新会话".into(),
        },
        &stamp("s_new"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], json!("s_new"));
    assert_eq!(rows[0]["title"], json!("新会话"));
    assert_eq!(rows[0]["messages"], json!([]));
    assert_eq!(rows[0]["draft"], json!(""));
    assert_eq!(rows[0]["server_conversation_id"], Value::Null, "新行未绑定");
    assert_eq!(rows[1]["id"], json!("s1"), "旧行仍在原位");
    assert_eq!(applied.active_session_id.as_deref(), Some("s_new"));
    assert_eq!(applied.local_id, "s_new");
}

#[test]
fn new_blank_keeps_other_rows_byte_identical() {
    // 服务端透传存储：别人的 messages / layout 字段一个都不能动。
    let other = json!({
        "id":"s1",
        "title":"keep",
        "messages":[{"role":"user","content":"hi"}],
        "layout_schema_version":2,
        "history_total":42,
        "workspace_root":"/w",
        "unknown_future_field":{"a":[1,2]}
    });
    let raw = json!([other.clone(), {"id":"s2","messages":[{"role":"user","content":"yo"}]}]);
    let applied = apply_index_op(
        &raw,
        None,
        &SessionIndexOp::NewBlank { title: "n".into() },
        &stamp("s_new"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows[1], other, "整行原样保留");
    assert_eq!(
        rows[2],
        json!({"id":"s2","messages":[{"role":"user","content":"yo"}]})
    );
    assert!(!applied.reused_blank, "两行都有正文：必须新插一行");
}

#[test]
fn bind_sets_conversation_id_only_on_target_row() {
    let raw = json!([
        {"id":"s_new","title":"新会话","messages":[]},
        {"id":"s1","title":"keep","messages":[{"role":"user","content":"hi"}]}
    ]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::BindServer {
            local_id: "s_new".into(),
            conversation_id: "c9".into(),
        },
        &stamp("ignored"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows[0]["server_conversation_id"], json!("c9"));
    assert_eq!(rows[0]["updated_at"], json!(1_700_000_000_000i64));
    assert_eq!(rows[0]["messages"], json!([]), "绑定不改正文");
    assert_eq!(
        rows[1],
        json!({"id":"s1","title":"keep","messages":[{"role":"user","content":"hi"}]}),
        "其他行原样"
    );
    assert_eq!(
        applied.active_session_id.as_deref(),
        Some("s1"),
        "绑定不动 active"
    );
    assert_eq!(applied.local_id, "s_new");
}

#[test]
fn bind_missing_row_is_an_error() {
    let err = apply_index_op(
        &json!([{"id":"s1"}]),
        None,
        &SessionIndexOp::BindServer {
            local_id: "gone".into(),
            conversation_id: "c9".into(),
        },
        &stamp("x"),
    )
    .expect_err("missing row must fail");
    assert!(err.to_string().contains("gone"), "错误文案带上行 id");
}

#[test]
fn non_array_raw_is_treated_as_empty() {
    let applied = apply_index_op(
        &Value::Null,
        None,
        &SessionIndexOp::NewBlank { title: "n".into() },
        &stamp("s_new"),
    )
    .expect("apply");
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(1));
}

#[test]
fn put_body_omits_absent_active_id() {
    let applied = AppliedIndex {
        sessions: json!([]),
        active_session_id: None,
        local_id: "s".into(),
        reused_blank: false,
    };
    let body = put_sessions_body(&applied);
    assert!(body.get("active_session_id").is_none(), "None 不写");
    assert_eq!(body["sessions"], json!([]));

    let applied = AppliedIndex {
        sessions: json!([]),
        active_session_id: Some("s1".into()),
        local_id: "s1".into(),
        reused_blank: false,
    };
    assert_eq!(
        put_sessions_body(&applied)["active_session_id"],
        json!("s1")
    );
}

#[test]
fn local_session_id_shape() {
    let id = new_local_session_id(UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123));
    assert_eq!(id, "s_1700000000000_123");
}

#[test]
fn new_blank_reuses_existing_blank_row() {
    // 对齐 Web `prefer_or_create_empty_session`：已有空白会话就复用，不再插行。
    let raw = json!([
        {"id":"s1","title":"keep","messages":[{"role":"user","content":"hi"}]},
        {"id":"s_blank","title":"新会话","messages":[],"draft":""}
    ]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::NewBlank {
            title: "新会话".into(),
        },
        &stamp("s_new"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows.len(), 2, "不该插出第三行");
    assert_eq!(applied.local_id, "s_blank");
    assert_eq!(applied.active_session_id.as_deref(), Some("s_blank"));
    assert!(applied.reused_blank);
}

#[test]
fn bound_but_empty_row_is_not_reused() {
    // messages 空但已有 server_conversation_id：Web 侧不算空白会话，不能复用。
    let raw = json!([{"id":"s1","messages":[],"server_conversation_id":"c1"}]);
    let applied = apply_index_op(
        &raw,
        None,
        &SessionIndexOp::NewBlank { title: "n".into() },
        &stamp("s_new"),
    )
    .expect("apply");
    assert_eq!(applied.local_id, "s_new");
    assert!(!applied.reused_blank);
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(2));
}

#[test]
fn remove_blank_drops_disposable_row() {
    let raw = json!([
        {"id":"s_blank","title":"新会话","messages":[],"draft":"","pinned":false,"starred":false},
        {"id":"s1","title":"keep","messages":[{"role":"user","content":"hi"}]}
    ]);
    let applied = apply_index_op(
        &raw,
        Some("s_blank"),
        &SessionIndexOp::RemoveBlank {
            local_id: "s_blank".into(),
            next_active: Some("s1".into()),
        },
        &stamp("x"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], json!("s1"));
    assert_eq!(
        applied.active_session_id.as_deref(),
        Some("s1"),
        "active 要跟着落到目标行"
    );
}

#[test]
fn remove_blank_keeps_row_with_content_or_draft() {
    // 有正文 / 有草稿 / 置顶 / 收藏：一律不动（别的端可能正在用）。
    for row in [
        json!({"id":"s1","messages":[{"role":"user","content":"hi"}]}),
        json!({"id":"s1","messages":[],"draft":"写了一半"}),
        json!({"id":"s1","messages":[],"pinned":true}),
        json!({"id":"s1","messages":[],"starred":true}),
    ] {
        let applied = apply_index_op(
            &json!([row]),
            None,
            &SessionIndexOp::RemoveBlank {
                local_id: "s1".into(),
                next_active: None,
            },
            &stamp("x"),
        )
        .expect("apply");
        assert_eq!(
            applied.sessions.as_array().map(Vec::len),
            Some(1),
            "不该删：{row}"
        );
    }
}

#[test]
fn remove_blank_refuses_active_row_without_next() {
    // 被删行正是 active 且没有落点：宁可不删，不能让服务端 active 指向已删除的行。
    let raw = json!([{"id":"s_blank","messages":[]}]);
    let applied = apply_index_op(
        &raw,
        Some("s_blank"),
        &SessionIndexOp::RemoveBlank {
            local_id: "s_blank".into(),
            next_active: None,
        },
        &stamp("x"),
    )
    .expect("apply");
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(1));
    assert_eq!(applied.active_session_id.as_deref(), Some("s_blank"));
}

#[test]
fn remove_missing_row_is_idempotent() {
    let raw = json!([{"id":"s1","messages":[{"role":"user","content":"hi"}]}]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::RemoveBlank {
            local_id: "gone".into(),
            next_active: None,
        },
        &stamp("x"),
    )
    .expect("已被别端删掉时不该报错");
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(1));
    assert_eq!(applied.active_session_id.as_deref(), Some("s1"));
}

#[test]
fn title_from_prompt_matches_web_rules() {
    // 与 Web `title_from_user_prompt` 同值：压平换行、折叠连续空白。
    assert_eq!(title_from_user_prompt("  hello\nworld  "), "hello world");
    assert_eq!(title_from_user_prompt("a\t\tb"), "a b");
    // 全空白 → 默认标题（Web 侧同）。
    assert_eq!(title_from_user_prompt("  \n\t "), DEFAULT_SESSION_TITLE);
}

#[test]
fn title_from_prompt_truncates_at_48_chars() {
    let body = "a".repeat(60);
    let out = title_from_user_prompt(&body);
    assert!(out.ends_with('…'), "got {out:?}");
    assert_eq!(out.chars().count(), 48, "48 含省略号，与 Web 一致");
}

#[test]
fn default_title_detection_matches_web() {
    // Web `i18n::is_default_session_title`：中 / 英默认标题（大小写不敏感）都算没起名。
    for s in ["新会话", " New Chat ", "new chat", "NEW CHAT"] {
        assert!(is_default_session_title(s), "{s} 应算默认标题");
    }
    for s in ["随便聊聊", "新会话2", ""] {
        assert!(!is_default_session_title(s), "{s:?} 不应算默认标题");
    }
}

#[test]
fn set_title_renames_only_default_row() {
    let raw = json!([
        {"id":"s_new","title":"新会话","messages":[]},
        {"id":"s1","title":"已经起过的名字","messages":[{"role":"user","content":"hi"}]}
    ]);
    let applied = apply_index_op(
        &raw,
        None,
        &SessionIndexOp::SetTitle {
            local_id: "s_new".into(),
            title: "帮我看下这个报错".into(),
        },
        &stamp("x"),
    )
    .expect("apply");
    let rows = applied.sessions.as_array().expect("array");
    assert_eq!(rows[0]["title"], json!("帮我看下这个报错"));
    assert_eq!(rows[0]["updated_at"], json!(1_700_000_000_000i64));
    assert_eq!(rows[0]["messages"], json!([]), "起名不改正文");
    assert_eq!(rows[1], raw[1], "别的行原样");
}

#[test]
fn set_title_keeps_non_default_title() {
    // 别端已经起过名（或用户自己改过）：不能覆盖。
    let raw = json!([{"id":"s1","title":"别端起的名字","messages":[]}]);
    let applied = apply_index_op(
        &raw,
        None,
        &SessionIndexOp::SetTitle {
            local_id: "s1".into(),
            title: "TUI 想改的名字".into(),
        },
        &stamp("x"),
    )
    .expect("apply");
    assert_eq!(applied.sessions[0]["title"], json!("别端起的名字"));
}

#[test]
fn set_title_missing_row_is_not_an_error() {
    // 行被别端删掉：起名失败不该让回合无法开始。
    let applied = apply_index_op(
        &json!([{"id":"s1","title":"新会话","messages":[]}]),
        None,
        &SessionIndexOp::SetTitle {
            local_id: "gone".into(),
            title: "t".into(),
        },
        &stamp("x"),
    )
    .expect("行不存在时不报错");
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(1));
}

#[test]
fn set_title_keeps_empty_title() {
    // 空串 / 缺字段按 Web 语义不算默认标题（`is_default_session_title("")` 是 false），
    // 不能因为读不出标题就覆盖。
    for row in [
        json!({"id":"s1","title":"","messages":[]}),
        json!({"id":"s1","messages":[]}),
    ] {
        let applied = apply_index_op(
            &json!([row.clone()]),
            None,
            &SessionIndexOp::SetTitle {
                local_id: "s1".into(),
                title: "t".into(),
            },
            &stamp("x"),
        )
        .expect("apply");
        assert_eq!(applied.sessions[0]["title"], row["title"], "不该改：{row}");
    }
}

#[test]
fn flush_exit_writes_draft_and_keeps_other_fields() {
    // 退出落盘只该动目标行的 `draft` / `updated_at`：`messages` 是写这一轮的那一端
    // 的正文，抹掉它等于把这段对话删了。
    let raw = json!([
        {
            "id": "s1",
            "title": "真名",
            "draft": "旧草稿",
            "messages": [{"id": "m1", "role": "user", "content": "正文"}],
            "server_conversation_id": "c1",
            "pinned": true,
            "updated_at": 1
        },
        {"id": "s2", "messages": []}
    ]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::FlushExit {
            local_id: "s1".into(),
            conversation_id: None,
            draft: Some("写一半的话".into()),
        },
        &stamp("x"),
    )
    .expect("apply");

    let row = &applied.sessions[0];
    assert_eq!(row["draft"], "写一半的话");
    assert_eq!(row["updated_at"], 1_700_000_000_000i64, "应刷新时间戳");
    assert_eq!(row["title"], "真名", "不该动标题");
    assert_eq!(row["messages"][0]["content"], "正文", "不该动正文");
    assert_eq!(row["server_conversation_id"], "c1", "不该动绑定");
    assert_eq!(row["pinned"], true, "不该动置顶");
    // 别的行逐字节不变。
    assert_eq!(applied.sessions[1], raw[1]);
    assert_eq!(applied.active_session_id.as_deref(), Some("s1"));
}

#[test]
fn flush_exit_writes_bind_and_draft_in_one_go() {
    // 合并成一次写入的意义：绑定与草稿在同一次 `GET → PUT` 里落地，中间没有
    // 让另一端插进来覆盖的窗口。
    let applied = apply_index_op(
        &json!([{"id": "s1", "messages": [], "updated_at": 1}]),
        None,
        &SessionIndexOp::FlushExit {
            local_id: "s1".into(),
            conversation_id: Some("c9".into()),
            draft: Some("写一半".into()),
        },
        &stamp("x"),
    )
    .expect("apply");

    assert_eq!(applied.sessions[0]["server_conversation_id"], "c9");
    assert_eq!(applied.sessions[0]["draft"], "写一半");
    assert_eq!(applied.sessions[0]["updated_at"], 1_700_000_000_000i64);
}

#[test]
fn flush_exit_missing_row_is_an_error() {
    // 与 `SetTitle` 的静默跳过不同：绑定丢失是用户可感知的后果（下次接不上），
    // 退出路径要能把它报出来，所以这里必须失败而不是假装成功。
    let err = apply_index_op(
        &json!([{"id": "s1", "messages": []}]),
        None,
        &SessionIndexOp::FlushExit {
            local_id: "gone".into(),
            conversation_id: Some("c9".into()),
            draft: Some("写一半".into()),
        },
        &stamp("x"),
    )
    .expect_err("行不存在时应报错");
    assert!(
        err.to_string().contains("会话行已不存在"),
        "错误信息应指明原因，got {err}"
    );
}

#[test]
fn remove_blank_refuses_unknown_next_active() {
    // 落点是调用方按本地快照反查的，可能已被别端删掉：这里必须按最新数组再校验。
    let raw = json!([
        {"id":"s_blank","messages":[]},
        {"id":"s1","messages":[{"role":"user","content":"hi"}]}
    ]);
    let applied = apply_index_op(
        &raw,
        Some("s_blank"),
        &SessionIndexOp::RemoveBlank {
            local_id: "s_blank".into(),
            next_active: Some("gone".into()),
        },
        &stamp("x"),
    )
    .expect("apply");
    assert_eq!(
        applied.sessions.as_array().map(Vec::len),
        Some(2),
        "落点不在数组里：宁可不删"
    );
    assert_eq!(applied.active_session_id.as_deref(), Some("s_blank"));
}

#[test]
fn remove_blank_refuses_self_next_active() {
    // 落点等于被删行本身：删完 active 就悬空了，同样不删。
    let raw = json!([{"id":"s_blank","messages":[]}]);
    let applied = apply_index_op(
        &raw,
        Some("s_blank"),
        &SessionIndexOp::RemoveBlank {
            local_id: "s_blank".into(),
            next_active: Some("s_blank".into()),
        },
        &stamp("x"),
    )
    .expect("apply");
    assert_eq!(applied.sessions.as_array().map(Vec::len), Some(1));
    assert_eq!(applied.active_session_id.as_deref(), Some("s_blank"));
}

#[test]
fn malformed_row_is_never_treated_as_blank() {
    // 判定改为按 `ChatSession` 反序列化后，字段类型不对会解析失败。
    // 失败必须一律当作「不是空白行」——宁可少认一条空行去复用 / 回收，
    // 也不能把别的端写过的行当成空壳删掉。
    let bad_messages = json!({"id": "s1", "messages": {"0": "x"}});
    assert!(!row_is_blank_chat(&bad_messages));
    assert!(!row_is_disposable_blank(&bad_messages));

    let bad_cid = json!({"id": "s2", "messages": [], "server_conversation_id": 42});
    assert!(!row_is_blank_chat(&bad_cid));

    let bad_flags = json!({"id": "s3", "messages": [], "pinned": "yes"});
    assert!(!row_is_disposable_blank(&bad_flags));

    // 非空消息行：长度快筛直接短路，不看消息体是否畸形——都不算空行。
    let with_messages = json!({"id": "s5", "messages": [{"id": "m1", "role": "user"}]});
    assert!(!row_is_blank_chat(&with_messages));
    assert!(!row_is_disposable_blank(&with_messages));

    // 对照：同样内容、类型正确时确实是空白行。
    let well_formed = json!({"id": "s4", "messages": [], "server_conversation_id": null});
    assert!(row_is_blank_chat(&well_formed));
    assert!(row_is_disposable_blank(&well_formed));
}

#[test]
fn malformed_row_is_not_dropped_by_remove_blank() {
    // 端到端：形状不对的行即使 id 命中，也不能被 RemoveBlank 删掉。
    let raw = json!([{"id": "s1", "messages": {"0": "x"}}]);
    let applied = apply_index_op(
        &raw,
        Some("s1"),
        &SessionIndexOp::RemoveBlank {
            local_id: "s1".into(),
            next_active: None,
        },
        &stamp("x"),
    )
    .expect("apply");
    assert_eq!(
        applied.sessions.as_array().map(Vec::len),
        Some(1),
        "解析失败的行不可回收"
    );
}
