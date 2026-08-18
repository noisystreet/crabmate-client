use super::Locale;

// --- API / 存储错误（设置、分支、审批等回显）---

pub fn api_err_workspace_set_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "设置失败",
        Locale::En => "Workspace update failed",
    }
}

pub fn api_err_request_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "请求失败",
        Locale::En => "Request failed",
    }
}

fn api_err_http_401_guide(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "请在设置 →「Web API 共享密钥」填入与 CM_WEB_API_BEARER_TOKEN 相同的值（不是模型 API_KEY）"
        }
        Locale::En => {
            "Open Settings → “Web API shared secret” and enter the same value as CM_WEB_API_BEARER_TOKEN (not the LLM API_KEY)"
        }
    }
}

/// HTTP 非 2xx 时的回显：`detail` 为空且为 404 时提示重启 `serve`；401 引导 Web Bearer。
pub fn api_err_http_status(l: Locale, status: u16, detail: &str) -> String {
    let base = api_err_request_failed(l);
    let detail = detail.trim();
    if status == 401 {
        let guide = api_err_http_401_guide(l);
        if detail.is_empty() {
            return format!("{base} (401): {guide}");
        }
        return format!("{base} (401): {detail} — {guide}");
    }
    if detail.is_empty() {
        if status == 404 {
            return match l {
                Locale::ZhHans => {
                    format!("{base} (404)：接口不存在，请重新编译并重启 crabmate serve 后刷新页面")
                }
                Locale::En => {
                    format!(
                        "{base} (404): API route not found; rebuild and restart crabmate serve, then reload"
                    )
                }
            };
        }
        if status == 405 {
            return match l {
                Locale::ZhHans => {
                    format!(
                        "{base} (405)：HTTP 方法不被允许，请重新编译并重启 crabmate serve 后刷新页面"
                    )
                }
                Locale::En => {
                    format!(
                        "{base} (405): method not allowed; rebuild and restart crabmate serve, then reload"
                    )
                }
            };
        }
        return format!("{base} ({status})");
    }
    format!("{base} ({status}): {detail}")
}

pub fn api_err_no_response_body(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "无响应体",
        Locale::En => "Empty response body",
    }
}

/// `GET /conversation/messages` 失败：本地缓存可能仍显示，须让用户知道未与服务端对齐。
pub fn api_err_conversation_messages_fetch_failed(l: Locale, detail: &str) -> String {
    let detail = detail.trim();
    match l {
        Locale::ZhHans => {
            if detail.is_empty() {
                "拉取会话消息失败，仍显示本地缓存".to_string()
            } else {
                format!("拉取会话消息失败，仍显示本地缓存：{detail}")
            }
        }
        Locale::En => {
            if detail.is_empty() {
                "Failed to fetch conversation messages; still showing local cache".to_string()
            } else {
                format!(
                    "Failed to fetch conversation messages; still showing local cache: {detail}"
                )
            }
        }
    }
}

/// 服务端 `messages` 非空但客户端解析为 0 条：保留本地时间线并提示 ParseFailed。
pub fn api_err_conversation_messages_parse_failed(
    l: Locale,
    raw_count: usize,
    revision: u64,
    conversation_id: &str,
    sample_roles: &[String],
) -> String {
    let roles = if sample_roles.is_empty() {
        match l {
            Locale::ZhHans => "无".to_string(),
            Locale::En => "none".to_string(),
        }
    } else {
        sample_roles.join(", ")
    };
    let cid = conversation_id.trim();
    match l {
        Locale::ZhHans => {
            if cid.is_empty() {
                format!(
                    "会话消息解析失败（服务端 {raw_count} 条，客户端 0 条），仍显示本地缓存；revision={revision}；样本角色={roles}"
                )
            } else {
                format!(
                    "会话消息解析失败（服务端 {raw_count} 条，客户端 0 条），仍显示本地缓存；revision={revision}；conversation_id={cid}；样本角色={roles}"
                )
            }
        }
        Locale::En => {
            if cid.is_empty() {
                format!(
                    "Conversation message parse failed ({raw_count} server rows, 0 client rows); still showing local cache; revision={revision}; sample roles={roles}"
                )
            } else {
                format!(
                    "Conversation message parse failed ({raw_count} server rows, 0 client rows); still showing local cache; revision={revision}; conversation_id={cid}; sample roles={roles}"
                )
            }
        }
    }
}

/// 服务端无此会话（过期 / mock SSE 假 id 等）。自动水合应软忽略并保留本地缓存，勿钉死状态栏。
#[must_use]
pub fn conversation_messages_err_is_not_found(detail: &str) -> bool {
    detail.contains("CONVERSATION_NOT_FOUND")
}

pub fn api_err_branch_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "分支请求未成功",
        Locale::En => "Branch request did not succeed",
    }
}

pub fn api_err_approval_failed(l: Locale, status: u16) -> String {
    match l {
        Locale::ZhHans => format!("审批请求失败 {status}"),
        Locale::En => format!("Approval request failed ({status})"),
    }
}

// --- 流式 / SSE 错误 ---

pub fn api_err_stream_gone(l: Locale) -> String {
    match l {
        Locale::ZhHans => "流式任务已结束或不在服务端内存中，无法重连".to_string(),
        Locale::En => {
            "Stream job has ended or is no longer in server memory; cannot resume".to_string()
        }
    }
}

pub fn api_err_stream_reader(l: Locale) -> String {
    match l {
        Locale::ZhHans => "流读取器初始化失败".to_string(),
        Locale::En => "Failed to initialize stream reader".to_string(),
    }
}

pub fn api_err_stream_stopped(l: Locale) -> String {
    match l {
        Locale::ZhHans => "流已停止".to_string(),
        Locale::En => "Stream stopped".to_string(),
    }
}

/// 判断是否为「流已停止」哨兵错误（解析器在 `RUN_ERROR` 等事件后中断流的错误）。
///
/// 该错误由 [`api_err_stream_stopped`] 按语言本地化（"流已停止" / "Stream stopped"），
/// 消费方据此静默收尾、避免按普通错误二次触发 `on_error` 覆盖已写好的错误文案。
/// 历史上消费方与硬编码小写英文 `"stream stopped"` 比较，任何语言下都不匹配
/// （英文为 `"Stream stopped"`），导致二次触发 `on_error`；此处按大小写不敏感收敛。
pub fn is_stream_stopped_error(e: &str) -> bool {
    let t = e.trim();
    t == "流已停止" || t.eq_ignore_ascii_case("stream stopped")
}

pub fn api_err_stream_read(e: &wasm_bindgen::JsValue) -> String {
    format!("read await: {:?}", e)
}

// --- HTTP 通用错误 ---

pub fn api_err_no_window(l: Locale) -> String {
    match l {
        Locale::ZhHans => "window 对象不可用".to_string(),
        Locale::En => "window object unavailable".to_string(),
    }
}

pub fn api_err_response_type(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "响应类型错误",
        Locale::En => "Unexpected response type",
    }
}

pub fn api_err_body_type(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "响应体类型错误",
        Locale::En => "Unexpected body type",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 哨兵识别须与 [`api_err_stream_stopped`] 的两种语言一致，否则中文环境下
    /// 消费方按普通错误二次触发 `on_error`，覆盖已写好的错误文案。
    #[test]
    fn stream_stopped_sentinel_matches_both_locales() {
        assert!(is_stream_stopped_error(&api_err_stream_stopped(
            Locale::ZhHans
        )));
        assert!(is_stream_stopped_error(&api_err_stream_stopped(Locale::En)));
        assert!(!is_stream_stopped_error("other error"));
        assert!(!is_stream_stopped_error(""));
    }
}
