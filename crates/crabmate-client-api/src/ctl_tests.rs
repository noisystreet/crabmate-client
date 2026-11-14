use super::*;

/// `disable` / `enable` 只发 `op` + `name`：Server 对这两个 op 的字段白名单就这两项
/// （多一个直接 `bad_request`），也不该夹带 `workspace` / `config`。
#[test]
fn set_enabled_request_carries_only_op_and_name() {
    let off = CtlRequest::SetEnabled {
        name: "proj-a".to_string(),
        enabled: false,
    }
    .to_line();
    let val: Value = serde_json::from_str(&off).expect("应是一行 JSON");
    let obj = val.as_object().expect("应是对象");
    assert_eq!(obj.len(), 2, "只能有 op / name：{off}");
    assert_eq!(val["op"].as_str(), Some("disable"));
    assert_eq!(val["name"].as_str(), Some("proj-a"));

    let on = CtlRequest::SetEnabled {
        name: "proj-a".to_string(),
        enabled: true,
    }
    .to_line();
    let val: Value = serde_json::from_str(&on).expect("应是一行 JSON");
    assert_eq!(val["op"].as_str(), Some("enable"));
}

/// 幂等命中不是失败：`changed:false` 与 `note` 要能原样带回（UI 据此说「本来就是这样」）。
#[test]
fn toggle_response_reports_idempotent_hit() {
    let out = parse_toggle_response(
        r#"{"ok":true,"name":"a","enabled":false,"changed":false,"note":"声明中不存在该实例"}"#,
    )
    .expect("应成功");
    assert_eq!(out.name, "a");
    assert_eq!(out.enabled, Some(false));
    assert!(!out.changed);
    assert!(!out.cleared_down);
    assert_eq!(out.note.as_deref(), Some("声明中不存在该实例"));
    assert!(out.warning.is_none());
}

/// `enable` 的 `cleared_down` 与 `warning` 都要露出来：后者是「只做成一半」的唯一线索。
#[test]
fn toggle_response_keeps_cleared_down_and_warning() {
    let out = parse_toggle_response(
        r#"{"ok":true,"name":"a","enabled":true,"changed":true,"cleared_down":true,"warning":"清理 down 标记失败（supervisor_gone）"}"#,
    )
    .expect("应成功");
    assert!(out.changed);
    assert!(out.cleared_down);
    assert_eq!(
        out.warning.as_deref(),
        Some("清理 down 标记失败（supervisor_gone）")
    );
}

#[test]
fn toggle_response_classifies_errors_by_code() {
    let err = parse_toggle_response(
        r#"{"ok":false,"code":"bad_request","error":"`disable` 不接受字段 `workspace`"}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::BadRequest);
    assert_eq!(err.code.as_deref(), Some("bad_request"));
}

#[test]
fn list_request_omits_the_default_flag() {
    assert_eq!(
        CtlRequest::List {
            include_unmanaged: false
        }
        .to_line(),
        r#"{"op":"list"}"#
    );
    assert_eq!(
        CtlRequest::List {
            include_unmanaged: true
        }
        .to_line(),
        r#"{"include_unmanaged":true,"op":"list"}"#
    );
}

/// `add` 的字段集必须与 Server 的白名单对齐：多一个键就会被 `bad_request` 拒。
#[test]
fn add_request_carries_only_the_allowed_fields() {
    let line = CtlRequest::Add {
        name: "proj-a".to_string(),
        workspace: "/home/u/proj-a".to_string(),
        profile: None,
        port: None,
        enabled: true,
    }
    .to_line();
    assert_eq!(
        line,
        r#"{"name":"proj-a","op":"add","workspace":"/home/u/proj-a"}"#
    );
    // `enabled` 默认 true 时省略，false 时必须写出（否则语义丢失）。
    let disabled = CtlRequest::Add {
        name: "proj-a".to_string(),
        workspace: "/w".to_string(),
        profile: Some("heavy".to_string()),
        port: Some(8721),
        enabled: false,
    }
    .to_line();
    assert!(disabled.contains(r#""enabled":false"#), "{disabled}");
    assert!(disabled.contains(r#""profile":"heavy""#), "{disabled}");
    assert!(disabled.contains(r#""port":8721"#), "{disabled}");
    assert!(!disabled.contains("config"), "绝不夹带 config：{disabled}");
}

#[test]
fn remove_request_carries_op_and_name() {
    assert_eq!(
        CtlRequest::Remove {
            name: "proj-a".to_string()
        }
        .to_line(),
        r#"{"name":"proj-a","op":"remove"}"#
    );
}

/// 可连性只认 `observed`，且只有 `ready` 为真 —— 这是本模块存在的首要理由。
#[test]
fn only_ready_is_connectable() {
    assert!(CtlObserved::parse(Some("ready")).can_connect());
    for other in ["starting", "stale", "stopped", "failed", "held", "???"] {
        assert!(
            !CtlObserved::parse(Some(other)).can_connect(),
            "{other} 不该被判为可连"
        );
    }
    assert!(!CtlObserved::parse(None).can_connect());
    assert!(!CtlObserved::parse(Some("   ")).can_connect());
}

/// `stale`：记录说在跑、实际连不上。看 `state` 会误判成可连，看 `observed` 才拦得住。
#[test]
fn stale_record_is_not_connectable_even_when_state_looks_ready() {
    let inst = CtlInstance {
        name: "proj-a".to_string(),
        state: Some("ready".to_string()),
        observed: Some("stale".to_string()),
        ..CtlInstance::default()
    };
    assert!(!inst.can_connect());
    assert_eq!(inst.observed_state(), CtlObserved::Stale);
}

#[test]
fn unknown_observed_is_kept_verbatim() {
    assert_eq!(
        CtlObserved::parse(Some("draining")),
        CtlObserved::Unknown("draining".to_string())
    );
}

#[test]
fn list_response_parses_instances_and_tolerates_new_fields() {
    let line = r#"{"ok":true,"instances":[
        {"name":"proj-a","state":"running","observed":"ready",
         "base_url":"http://127.0.0.1:8721","port":8721,"pid":4242,
         "last_error":null,"workspace":"/home/u/proj-a","profile":null,
         "enabled":true,"declared":true,"managed_by":null,"brand_new_field":1}
    ]}"#;
    let items = parse_list_response(line).expect("应能解析");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "proj-a");
    assert_eq!(items[0].base_url.as_deref(), Some("http://127.0.0.1:8721"));
    assert_eq!(items[0].port, Some(8721));
    assert!(items[0].can_connect());
}

#[test]
fn list_response_without_instances_is_empty() {
    assert!(parse_list_response(r#"{"ok":true}"#).unwrap().is_empty());
}

#[test]
fn error_response_is_classified_by_code_not_by_text() {
    let err = parse_list_response(
        r#"{"ok":false,"code":"socket_in_use","error":"控制面已在使用：/run/u/c.sock"}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::SocketInUse);
    assert_eq!(err.code.as_deref(), Some("socket_in_use"));
    // 文案原样透传，但分类绝不依赖它。
    assert!(err.message.contains("控制面已在使用"));
}

#[test]
fn unknown_code_does_not_fall_into_a_known_bucket() {
    let err = parse_list_response(r#"{"ok":false,"code":"brand_new","error":"x"}"#).unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::Other);
    assert_eq!(err.code.as_deref(), Some("brand_new"));
}

/// 连 `code` 都没有的错误体：分类为 `Other`，不猜成某个已知原因。
#[test]
fn error_without_code_is_other() {
    let err = parse_list_response(r#"{"ok":false,"error":"boom"}"#).unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::Other);
    assert_eq!(err.code, None);
    assert_eq!(err.message, "boom");
}

#[test]
fn non_json_response_is_malformed() {
    let err = parse_list_response("not json at all").unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::Malformed);
    assert_eq!(err.code, None);
}

#[test]
fn add_response_reports_created_and_note() {
    let created =
        parse_add_response(r#"{"ok":true,"name":"proj-a","created":true}"#).expect("应能解析");
    assert_eq!(created.name, "proj-a");
    assert!(created.created);
    assert_eq!(created.note, None);

    // 幂等命中：不是失败，但 note 要说清「没改动」。
    let existing = parse_add_response(
        r#"{"ok":true,"name":"proj-a","created":false,"note":"已存在，未改动"}"#,
    )
    .expect("应能解析");
    assert!(!existing.created);
    assert_eq!(existing.note.as_deref(), Some("已存在，未改动"));
}

#[test]
fn add_error_is_classified_by_code() {
    let err = parse_add_response(
        r#"{"ok":false,"code":"bad_workspace","error":"workspace 不存在或不是目录：/nope"}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::BadWorkspace);
    assert_eq!(err.code.as_deref(), Some("bad_workspace"));
}

#[test]
fn add_response_without_ok_is_an_error_not_a_silent_success() {
    // 缺 `ok` 一律按失败：把「没 ok」当成功会让 `/lane new` 报一个不存在的连接。
    let err = parse_add_response(r#"{"name":"proj-a","created":true}"#).unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::Other);
}

fn ready_inst(name: &str, url: Option<&str>, last_error: Option<&str>) -> CtlInstance {
    CtlInstance {
        name: name.to_string(),
        observed: Some("ready".to_string()),
        base_url: url.map(str::to_string),
        last_error: last_error.map(str::to_string),
        ..CtlInstance::default()
    }
}

#[test]
fn readiness_needs_ready_and_a_base_url() {
    let items = vec![ready_inst("a", Some("http://127.0.0.1:8721/"), None)];
    assert_eq!(
        lane_readiness(&items, "a"),
        LaneReadiness::Ready("http://127.0.0.1:8721/".to_string())
    );
    // 就绪但没 URL：不许按端口猜。
    let no_url = vec![ready_inst("a", None, None)];
    assert_eq!(lane_readiness(&no_url, "a"), LaneReadiness::Pending);
    let blank_url = vec![ready_inst("a", Some("   "), None)];
    assert_eq!(lane_readiness(&blank_url, "a"), LaneReadiness::Pending);
}

#[test]
fn readiness_treats_starting_as_pending_and_missing_as_missing() {
    let starting = vec![CtlInstance {
        name: "a".to_string(),
        observed: Some("starting".to_string()),
        ..CtlInstance::default()
    }];
    assert_eq!(lane_readiness(&starting, "a"), LaneReadiness::Pending);
    assert_eq!(lane_readiness(&starting, "nope"), LaneReadiness::Missing);
    assert_eq!(lane_readiness(&[], "a"), LaneReadiness::Missing);
}

/// `stopped` / `held` 不是中间态：继续轮询只会把结论拖成超时。
#[test]
fn readiness_fails_fast_on_terminal_states() {
    for observed in ["failed", "stopped", "held"] {
        let items = vec![CtlInstance {
            name: "a".to_string(),
            observed: Some(observed.to_string()),
            ..CtlInstance::default()
        }];
        match lane_readiness(&items, "a") {
            LaneReadiness::Failed(why) => assert!(!why.is_empty(), "{observed} 要给原因"),
            other => panic!("{observed} 应判失败，实际 {other:?}"),
        }
    }
}

/// 失败原因优先 Server 的 `last_error`：那是唯一知道真正原因的地方。
#[test]
fn failure_prefers_server_last_error() {
    let items = vec![CtlInstance {
        name: "a".to_string(),
        observed: Some("failed".to_string()),
        last_error: Some("工作区不在白名单".to_string()),
        ..CtlInstance::default()
    }];
    assert_eq!(
        lane_readiness(&items, "a"),
        LaneReadiness::Failed("工作区不在白名单".to_string())
    );
}

/// 声明被 close 是**事实**：即便 `observed` 还写着 `starting`（Server 对「有声明、
/// 无状态记录」的回法）也要立刻判失败，并指出回到可用状态的那一步（issue #373）。
///
/// `ready` 也在表里：`disable` 会停掉实例，两者同时成立说明状态记录是陈旧的，按
/// 「可连」处理会连到一个即将被停掉的实例。
#[test]
fn closed_declaration_fails_fast_whatever_observed_says() {
    for observed in ["starting", "", "ready"] {
        let items = vec![CtlInstance {
            name: "a".to_string(),
            observed: Some(observed.to_string()),
            base_url: Some("http://127.0.0.1:8721/".to_string()),
            enabled: Some(false),
            ..CtlInstance::default()
        }];
        match lane_readiness(&items, "a") {
            LaneReadiness::Failed(why) => {
                assert!(
                    why.contains("enabled = false"),
                    "observed={observed}: {why}"
                );
                assert!(why.contains("/lane open a"), "要给出下一步命令：{why}");
            }
            other => panic!("observed={observed} 且 enabled=false 应判失败，实际 {other:?}"),
        }
    }
}

/// `enabled` 缺失（旧版 ctl 不返回该字段）或为 `true` 时，判读与加这个短路之前逐字
/// 一致 —— 这是本次改动唯一的兼容性边界。
#[test]
fn absent_or_true_enabled_keeps_the_old_readiness() {
    for enabled in [None, Some(true)] {
        let starting = vec![CtlInstance {
            name: "a".to_string(),
            observed: Some("starting".to_string()),
            enabled,
            ..CtlInstance::default()
        }];
        assert_eq!(lane_readiness(&starting, "a"), LaneReadiness::Pending);

        let mut ready = ready_inst("a", Some("http://127.0.0.1:8721/"), None);
        ready.enabled = enabled;
        assert_eq!(
            lane_readiness(&[ready], "a"),
            LaneReadiness::Ready("http://127.0.0.1:8721/".to_string())
        );
    }
}

/// `status` 只发 `op` + `name`：它是查询，Server 对它的字段白名单就这两项。
#[test]
fn status_request_carries_only_op_and_name() {
    let line = CtlRequest::Status {
        name: "proj-a".to_string(),
    }
    .to_line();
    let val: Value = serde_json::from_str(&line).expect("应是一行 JSON");
    let obj = val.as_object().expect("应是对象");
    assert_eq!(obj.len(), 2, "只能有 op / name：{line}");
    assert_eq!(val["op"].as_str(), Some("status"));
    assert_eq!(val["name"].as_str(), Some("proj-a"));
}

/// `restart` 的 `grace` / `timeout` 省略时要**真的不写**（Server 按缺省 10 s），
/// 显式给了才写出 —— 多一个键就是 `bad_request`。
#[test]
fn restart_request_omits_absent_budget_fields() {
    let defaulted = CtlRequest::Restart {
        name: "proj-a".to_string(),
        grace: None,
        timeout: None,
    }
    .to_line();
    let val: Value = serde_json::from_str(&defaulted).expect("应是一行 JSON");
    let obj = val.as_object().expect("应是对象");
    assert_eq!(obj.len(), 2, "缺省时只能有 op / name：{defaulted}");
    assert_eq!(val["op"].as_str(), Some("restart"));

    let explicit = CtlRequest::Restart {
        name: "proj-a".to_string(),
        grace: Some(10),
        timeout: Some(30),
    }
    .to_line();
    assert!(explicit.contains(r#""grace":10"#), "{explicit}");
    assert!(explicit.contains(r#""timeout":30"#), "{explicit}");
    assert!(!explicit.contains("config"), "绝不夹带 config：{explicit}");
}

/// `status` 的成功响应就是一个 `status_row`（外加 `ok`）：**不**为它另写一套字段，
/// 所以这里连同 `desired_state` 与列表行共有的字段一起验，并顺手确认新字段不会
/// 让解析失败。
#[test]
fn status_response_parses_the_shared_status_row() {
    let inst = parse_status_response(
        r#"{"ok":true,"name":"proj-a","state":"ready","desired_state":"running",
            "observed":"ready","base_url":"http://127.0.0.1:8081/","pid":4242,
            "port":8081,"workspace":"/srv/a","enabled":true,"brand_new_field":1}"#,
    )
    .expect("应能解析");
    assert_eq!(inst.name, "proj-a");
    assert_eq!(inst.state.as_deref(), Some("ready"));
    assert_eq!(inst.desired_state.as_deref(), Some("running"));
    assert_eq!(inst.pid, Some(4242));
    assert!(inst.can_connect());
}

/// `restart` = 同一份 status_row + `restarted` + `notes`；`notes` 恒为数组且要原样带回
/// （「env 丢了」这类提示只有 Server 知道，吞掉就等于静默）。
#[test]
fn restart_response_keeps_restarted_and_notes() {
    let out = parse_restart_response(
        r#"{"ok":true,"restarted":true,"name":"proj-a","state":"ready","pid":4243,
            "notes":["声明里的 env 已丢失（3 项）","profile 漂移：heavy → default"]}"#,
    )
    .expect("应能解析");
    assert!(out.restarted);
    assert_eq!(out.instance.name, "proj-a");
    assert_eq!(out.instance.pid, Some(4243));
    assert_eq!(out.notes.len(), 2);
    assert!(out.notes[0].contains("env"), "{:?}", out.notes);

    // 空数组（契约里的常态）与缺字段都归一成空列表，不是错误。
    let empty = parse_restart_response(
        r#"{"ok":true,"restarted":true,"name":"proj-a","state":"ready","notes":[]}"#,
    )
    .expect("应能解析");
    assert!(empty.notes.is_empty());
    let absent = parse_restart_response(r#"{"ok":true,"name":"proj-a"}"#).expect("应能解析");
    assert!(absent.notes.is_empty());
    assert!(!absent.restarted, "缺 restarted 不猜成「重启了」");
}

/// 重启路径上的错误码都要落进自己的分类，而不是一起进 `Other`。
#[test]
fn restart_errors_classify_by_code() {
    let cases = [
        ("instance_not_found", CtlErrorKind::InstanceNotFound),
        (
            "restart_budget_exceeded",
            CtlErrorKind::RestartBudgetExceeded,
        ),
        ("port_in_use", CtlErrorKind::PortInUse),
        ("identity_mismatch", CtlErrorKind::IdentityMismatch),
    ];
    for (code, want) in cases {
        let line = format!(r#"{{"ok":false,"code":"{code}","error":"x"}}"#);
        let err = parse_restart_response(&line).unwrap_err();
        assert_eq!(err.kind, want, "{code}");
        assert_eq!(err.code.as_deref(), Some(code));
    }
    // `status` 与 `restart` 共用同一套解析，同一个码不该在两条路径上分到两处。
    let err = parse_status_response(
        r#"{"ok":false,"code":"instance_not_found","error":"没有该实例的状态记录"}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind, CtlErrorKind::InstanceNotFound);
}
