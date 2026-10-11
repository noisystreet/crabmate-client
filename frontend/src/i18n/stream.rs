use super::Locale;
use crabmate::cm_sse_protocol::StreamEndReason;

// --- 流式 / 停止 ---

pub fn stream_empty_reply(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "(无回复)",
        Locale::En => "(No reply)",
    }
}

pub fn stream_empty_reply_no_answer_phase(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "(无回复：未进入正文阶段)",
        Locale::En => "(No reply: answer phase not entered)",
    }
}

pub fn stream_empty_reply_no_delta(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "(无回复：未收到正文片段)",
        Locale::En => "(No reply: no answer delta received)",
    }
}

/// 契约钉尚未包含、但服务端已可下发的 `stream_ended.reason` 本地化兜底。
///
/// 服务端在 `StreamEndReason` 上新增 `Truncated`（`"truncated"`）后，客户端若仍钉旧版
/// crate，`parse::<StreamEndReason>()` 会失败、把该 reason 当未知原样回显。此表在契约钉
/// 升级（`parse` 能命中）之前提供同一文案，升级后可整表删除。
fn future_stream_end_reason_label(l: Locale, raw: &str) -> Option<&'static str> {
    match raw {
        "truncated" => Some(match l {
            Locale::ZhHans => "输出被截断",
            Locale::En => "truncated",
        }),
        _ => None,
    }
}

/// 将 `stream_ended.reason` 原始字符串转成当前语言的本地化标签。
///
/// 顺序：契约钉枚举解析 → 本地向前兼容表 → 原样回显；`None` 回退为 `"unknown"`。
pub fn stream_end_reason_label_from_raw(l: Locale, raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return "unknown".to_string();
    };
    if let Ok(reason) = raw.parse::<StreamEndReason>() {
        return stream_end_reason_label(l, reason);
    }
    future_stream_end_reason_label(l, raw)
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

pub fn stream_empty_reply_diag_line(
    l: Locale,
    stream_end_reason: Option<&str>,
    answer_phase: bool,
    delta_chars: usize,
) -> String {
    let reason = stream_end_reason_label_from_raw(l, stream_end_reason);
    match l {
        Locale::ZhHans => format!(
            "诊断：stream_ended={reason}, answer_phase={answer_phase}, delta_chars={delta_chars}"
        ),
        Locale::En => format!(
            "Diagnostic: stream_ended={reason}, answer_phase={answer_phase}, delta_chars={delta_chars}"
        ),
    }
}

pub fn stream_end_reason_label(l: Locale, reason: StreamEndReason) -> String {
    match l {
        Locale::ZhHans => reason.label_zh_hans().to_string(),
        Locale::En => reason.label_en().to_string(),
    }
}

pub fn stream_stopped_suffix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "\n\n[已停止]",
        Locale::En => "\n\n[Stopped]",
    }
}

pub fn stream_stopped_inline(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已停止",
        Locale::En => "Stopped",
    }
}

pub fn chat_failed_banner(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "对话失败",
        Locale::En => "Chat failed",
    }
}

/// 流式已生成正文但 `stream_ended` 仍为 unknown 时的用户提示首段。
pub fn stream_partial_finalize_missing_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "本轮回复已生成，但流式收尾信号缺失（stream_ended=unknown）。请点击“重试”获取完整收尾。"
        }
        Locale::En => {
            "Reply content was generated, but stream finalization signal is missing (stream_ended=unknown). Click Retry to finish cleanly."
        }
    }
}

pub fn stream_completed_missing_final_summary_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "本轮执行已完成，但最终总结消息缺失。你可以点击“重试”让助手补发最终汇总。"
        }
        Locale::En => {
            "Execution finished, but final summary message is missing. Click Retry to regenerate the final summary."
        }
    }
}

pub fn stream_err_impact_api_key(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "当前回合无法调用模型，助手不会继续生成。",
        Locale::En => "The model call cannot proceed for this turn.",
    }
}

pub fn stream_err_hint_api_key(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "检查设置中「模型」分区的 API 密钥（client_llm）是否已填写且有效，然后点击“重试”。勿与「Web API 共享密钥」混淆。"
        }
        Locale::En => {
            "Verify the model API key (client_llm) under Settings → Model, then click Retry. Do not confuse it with the Web API shared secret."
        }
    }
}

pub fn stream_err_impact_web_api_bearer(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "浏览器未通过 CrabMate Web API 鉴权，受保护接口被拒绝。",
        Locale::En => "The browser failed CrabMate Web API auth; protected routes were rejected.",
    }
}

pub fn stream_err_hint_web_api_bearer(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "打开设置 →「Web API 共享密钥」，填入与 CM_WEB_API_BEARER_TOKEN 相同的值并保存（不是模型 API_KEY），然后重试。"
        }
        Locale::En => {
            "Open Settings → “Web API shared secret”, enter the same value as CM_WEB_API_BEARER_TOKEN (not the LLM API_KEY), save, then retry."
        }
    }
}

pub fn stream_err_impact_timeout(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "本次请求超时中断，当前回复未完整生成。",
        Locale::En => "The request timed out and the reply is incomplete.",
    }
}

pub fn stream_err_hint_timeout(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "稍后重试，或缩小本次请求范围后再发送。",
        Locale::En => "Retry later or send a narrower request.",
    }
}

pub fn stream_err_impact_generic(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "本轮流式对话已中止，后续步骤未执行。",
        Locale::En => "The streaming turn stopped and follow-up steps were skipped.",
    }
}

pub fn stream_err_hint_generic(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "点击“重试”；若仍失败，请补充报错上下文以便排查。",
        Locale::En => "Click Retry; if it persists, share more error context.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `"truncated"` 在旧契约钉下须仍本地化，文案与服务端 `label_*` 一致，
    /// 避免回落成未知 reason 原样回显。
    #[test]
    fn truncated_reason_is_localized_locally() {
        assert_eq!(
            stream_end_reason_label_from_raw(Locale::ZhHans, Some("truncated")),
            "输出被截断"
        );
        assert_eq!(
            stream_end_reason_label_from_raw(Locale::En, Some("truncated")),
            "truncated"
        );
    }

    /// 契约钉已能解析的 reason 仍走枚举标签（此处与服务端语义比对，不写死文案）。
    #[test]
    fn known_reason_uses_enum_label() {
        for l in [Locale::ZhHans, Locale::En] {
            assert_eq!(
                stream_end_reason_label_from_raw(l, Some("completed")),
                stream_end_reason_label(l, StreamEndReason::Completed)
            );
        }
    }

    #[test]
    fn unknown_reason_is_echoed_and_none_is_unknown() {
        assert_eq!(
            stream_end_reason_label_from_raw(Locale::ZhHans, Some("future_reason")),
            "future_reason"
        );
        assert_eq!(
            stream_end_reason_label_from_raw(Locale::En, None),
            "unknown"
        );
    }

    /// 诊断行是 truncated 唯一用户可见落点，须带上本地化标签而非原始串。
    #[test]
    fn diag_line_shows_localized_truncated_label() {
        let line = stream_empty_reply_diag_line(Locale::ZhHans, Some("truncated"), true, 0);
        assert!(line.contains("输出被截断"), "{line}");
        assert!(!line.contains("truncated"), "{line}");
    }
}
