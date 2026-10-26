use super::Locale;

// --- 状态栏 ---

pub fn status_fetch_error(l: Locale, err: &str) -> String {
    match l {
        Locale::ZhHans => format!("无法加载状态（/status）：{err}"),
        Locale::En => format!("Failed to load status (/status): {err}"),
    }
}

pub fn status_retry(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "重试",
        Locale::En => "Retry",
    }
}

pub fn prefs_load_failed(l: Locale, err: &str) -> String {
    match l {
        Locale::ZhHans => format!("无法加载偏好（/user-data/prefs）：{err}"),
        Locale::En => format!("Failed to load preferences (/user-data/prefs): {err}"),
    }
}

pub fn prefs_save_failed(l: Locale, err: &str) -> String {
    match l {
        Locale::ZhHans => format!("偏好保存失败：{err}"),
        Locale::En => format!("Failed to save preferences: {err}"),
    }
}

pub fn status_open_web_api_settings(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "填写 Web Bearer",
        Locale::En => "Set Web Bearer",
    }
}

pub fn status_loading_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "加载状态",
        Locale::En => "Loading status",
    }
}

pub fn status_chip_model(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "模型",
        Locale::En => "Model",
    }
}

pub fn status_chip_context(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "会话",
        Locale::En => "Conversation",
    }
}

/// 状态栏「会话」芯片 `title`：说明 `used / max_input` 用量、灰段含义与水位语义。
pub fn status_chip_context_tooltip(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "当前用量 used_input_tokens（message + attachment + 每轮固定开销：工具 JSON + 网关开销，tiktoken 粗估）占模型原始上下文窗口 max_input_tokens 的比例。灰段为固定开销，brand 段为会话内容；分子分母都含固定开销，故百分比即「离窗口上限还有多远」。数值取会话内滚动峰值，故正常对话中单调不减，仅在切换会话或服务端压缩 / 裁剪历史后回落。与网关计费可能有偏差。点击查看用量明细。"
        }
        Locale::En => {
            "Current usage used_input_tokens (message + attachment + per-turn fixed overhead: tool JSON + provider overhead, tiktoken estimate) against the model's raw context window max_input_tokens. The grey bar is the fixed overhead, the brand bar is conversation content; both numerator and denominator include fixed overhead, so the percentage is literally how close the window is to its limit. The value is a per-session rolling peak, so it never decreases during normal chat and only drops on session switch or after the server compacts / trims the history. May differ from provider billing. Click to view the detailed breakdown."
        }
    }
}

pub fn status_role_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "角色",
        Locale::En => "Role",
    }
}

/// 状态栏权限等级段的标签（值本身走 `settings_permission_level_label`，同一套词表）。
pub fn status_perm_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "权限",
        Locale::En => "Perm",
    }
}

/// 权限菜单的无障碍名。
pub fn status_perm_menu_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "切换权限等级",
        Locale::En => "Switch permission level",
    }
}

/// 悬停说明：明确「点一下能切」，否则用户会以为它和模型名一样只是读数。
pub fn status_perm_title_attr(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "当前权限等级，点一下切换",
        Locale::En => "Current permission level — click to switch",
    }
}

/// 状态栏「思考模式」开关标签。
pub fn status_chip_thinking(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "思考",
        Locale::En => "Think",
    }
}

/// 状态栏「思考模式」开关 `title`（悬停提示）。
pub fn status_thinking_tooltip(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "思考模式：仅本机生效（on/off）；开启时随聊天请求发送 llm_thinking_mode=on"
        }
        Locale::En => {
            "Thinking mode: local-only (on/off); sends llm_thinking_mode=on with chat when on"
        }
    }
}

pub fn status_mode_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "模式",
        Locale::En => "Mode",
    }
}

pub fn status_session_mode_title(l: Locale, mode: &str) -> String {
    match (l, mode) {
        (Locale::ZhHans, "ask") => "Ask（只读）".into(),
        (Locale::ZhHans, "plan") => "Plan（只读规划）".into(),
        (Locale::ZhHans, "act") => "Act（执行）".into(),
        (Locale::En, "ask") => "Ask (read-only)".into(),
        (Locale::En, "plan") => "Plan (read-only)".into(),
        (Locale::En, "act") => "Act (execute)".into(),
        (Locale::ZhHans, _) => format!("模式：{mode}"),
        (Locale::En, _) => format!("Mode: {mode}"),
    }
}

pub fn status_session_mode_switched(l: Locale, mode: &str) -> String {
    let title = status_session_mode_title(l, mode);
    match l {
        Locale::ZhHans => format!("已切换会话模式为 {title}"),
        Locale::En => format!("Switched session mode to {title}"),
    }
}

pub fn status_role_title_attr(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "Agent 角色（对标 CLI /agent set）",
        Locale::En => "Agent role (same as CLI /agent set)",
    }
}

pub fn status_default_option(l: Locale, id: Option<&str>) -> String {
    match l {
        Locale::ZhHans => match id {
            Some(i) => format!("default ({i})"),
            None => "default".to_string(),
        },
        Locale::En => match id {
            Some(i) => format!("default ({i})"),
            None => "default".to_string(),
        },
    }
}

pub fn status_unavailable(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "/status 不可用",
        Locale::En => "/status unavailable",
    }
}

pub fn status_error_prefix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "错误: ",
        Locale::En => "Error: ",
    }
}

pub fn status_tool_running(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工具执行中…",
        Locale::En => "Running tools…",
    }
}

/// 用户点击「停止」后，工具占位气泡上替代「执行中」的短标签。
pub fn status_tool_stopped_user(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已终止",
        Locale::En => "Stopped",
    }
}

/// 无在途 SSE 时收口僵尸「执行中」工具卡（流已结束未收到 `tool_result`、或重启后从 user-data 恢复）。
pub fn status_tool_interrupted_stale(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已中断",
        Locale::En => "Interrupted",
    }
}

pub fn status_model_running(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "生成中",
        Locale::En => "Generating",
    }
}

pub fn status_ready(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "就绪",
        Locale::En => "Ready",
    }
}

/// Android 13+ 未授予通知权限：前台保活通知 / 审批 heads-up 不可见。
pub fn android_stream_keepalive_notify_needed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未授予通知权限：后台保活与命令审批提醒不可用。可在系统设置中允许通知。",
        Locale::En => {
            "Notifications are off: background keep-alive and approval alerts are unavailable. Enable notifications in system settings."
        }
    }
}

// --- 上下文用量可视化页 ---

pub fn context_usage_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上下文用量",
        Locale::En => "Context usage",
    }
}

pub fn context_usage_back(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "返回",
        Locale::En => "Back",
    }
}

pub fn context_usage_badge(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "只读",
        Locale::En => "Read-only",
    }
}

pub fn context_usage_no_data(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "无数据",
        Locale::En => "No data",
    }
}

pub fn context_usage_empty(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "当前会话暂无上下文用量数据。",
        Locale::En => "No context usage data for the current session.",
    }
}

pub fn context_usage_note_refresh(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "数据随每轮对话刷新；固定开销（工具 JSON / 网关开销）为 tiktoken 粗估，可能与网关计费有偏差。"
        }
        Locale::En => {
            "Refreshed each turn; fixed overhead (tool JSON / provider overhead) is a tiktoken estimate and may differ from provider billing."
        }
    }
}

pub fn context_usage_note_estimate(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "当前为基线估算（尚无精确用量），条以虚线标示。",
        Locale::En => "Showing a baseline estimate (no precise usage yet); the bar is dashed.",
    }
}

pub fn context_usage_note_reverse(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "条为当前构成；虚线标出会话内滚动峰值，顶部数字取该峰值，正常对话中单调不减，仅在切换会话或服务端压缩 / 裁剪历史后回落。"
        }
        Locale::En => {
            "The bar shows the current composition; the dashed line marks the session's rolling peak and the headline figure is that peak — monotonically non-decreasing during normal chat and only dropping on session switch or after the server compacts / trims the history."
        }
    }
}

pub fn context_usage_reserved_note(l: Locale, tokens: u32) -> String {
    match l {
        Locale::ZhHans => {
            format!("已预留输出预算 {tokens} tokens（与窗口占用关系未确认，未计入条内）。")
        }
        Locale::En => format!(
            "Reserved output budget {tokens} tokens (its relation to window occupancy is unconfirmed and not counted in the bar)."
        ),
    }
}

/// 条 `aria-label`：说明条为「当前构成」，并在存在峰值虚线时一并说明峰值。
pub fn context_usage_bar_aria(l: Locale, current: &str, peak: Option<&str>) -> String {
    match (l, peak) {
        (Locale::ZhHans, Some(p)) => {
            format!("上下文用量条：当前构成 {current}；虚线标出会话内峰值 {p}")
        }
        (Locale::ZhHans, None) => format!("上下文用量条：当前构成 {current}"),
        (Locale::En, Some(p)) => format!(
            "Context usage bar: current composition {current}; the dashed line marks the session peak {p}"
        ),
        (Locale::En, None) => format!("Context usage bar: current composition {current}"),
    }
}

/// 分段名称（按 key 查表；未知 key 回退到「其他」）。
pub fn context_usage_bucket_name(l: Locale, key: &str) -> String {
    match l {
        Locale::ZhHans => context_usage_bucket_name_zh(key),
        Locale::En => context_usage_bucket_name_en(key),
    }
    .to_string()
}

fn context_usage_bucket_name_zh(key: &str) -> &'static str {
    match key {
        "tool-schema" => "工具定义",
        "other-fixed" => "其他固定开销",
        "message" => "会话内容",
        "attachment" => "附件",
        "used" => "已用",
        "remaining" => "剩余",
        _ => "其他",
    }
}

fn context_usage_bucket_name_en(key: &str) -> &'static str {
    match key {
        "tool-schema" => "Tool schema",
        "other-fixed" => "Other fixed",
        "message" => "Messages",
        "attachment" => "Attachments",
        "used" => "Used",
        "remaining" => "Remaining",
        _ => "Other",
    }
}
