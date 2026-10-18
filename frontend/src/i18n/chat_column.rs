use super::Locale;

// --- 聊天列空态 / 输入区 ---

pub fn chat_tui_empty(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "CrabMate 已就绪。输入消息开始纯文本流式对话。",
        Locale::En => "CrabMate ready. Send a message to start the plain-text stream.",
    }
}

/// 助手气泡内「思维链折叠块」的摘要文案。
pub fn chat_thinking_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "思考",
        Locale::En => "Thinking",
    }
}

pub fn chat_tui_tool_status_done(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "完成",
        Locale::En => "done",
    }
}

pub fn chat_tui_tool_status_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "失败",
        Locale::En => "failed",
    }
}

pub fn composer_ph(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "输入消息，Enter 发送 / Shift+Enter 换行；/ 选用技能；@相对路径 展开文件…"
        }
        Locale::En => {
            "Message: Enter send / Shift+Enter newline; / for skills; @rel/path expands file…"
        }
    }
}

/// 工作区树双击插入文件引用时路径含空白。
pub fn composer_ws_path_whitespace_err(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "该文件路径含空格，无法自动生成 @ 引用，请手动输入相对路径。",
        Locale::En => {
            "This path contains spaces; cannot auto-insert @ ref — type the relative path manually."
        }
    }
}

/// 侧栏工作区文件行：双击 / 拖放到输入框的提示。
pub fn workspace_tree_insert_file_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "双击或拖放到输入框：插入 @相对路径（发送时由服务端展开；气泡只显示路径）"
        }
        Locale::En => {
            "Double-click or drag onto composer to insert @relative-path (expanded server-side; bubble shows path only)"
        }
    }
}

/// 从系统文件管理器拖入非图片且无法映射到工作区路径。
pub fn composer_drop_need_workspace_tree(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "无法从系统拖放解析工作区路径；请从侧栏工作区树拖入文件，或双击插入 @ 引用。"
        }
        Locale::En => {
            "Cannot map OS drop to a workspace path — drag from the workspace tree, or double-click to insert an @ ref."
        }
    }
}

pub fn composer_stop(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "停止",
        Locale::En => "Stop",
    }
}

pub fn composer_send_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "发送",
        Locale::En => "Send",
    }
}

pub fn composer_send_queue_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "排队下一句",
        Locale::En => "Queue next message",
    }
}

pub fn composer_queued_chip(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "下一句已排队",
        Locale::En => "Next message queued",
    }
}

pub fn composer_queued_dismiss(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "取消排队",
        Locale::En => "Dismiss queued message",
    }
}

pub fn composer_attach_image_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "附加图片",
        Locale::En => "Attach image",
    }
}

pub fn chat_image_attachment_alt(l: Locale, filename: &str) -> String {
    match l {
        Locale::ZhHans => format!("附图 {filename}"),
        Locale::En => format!("Attachment {filename}"),
    }
}

pub fn chat_image_unavailable(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "图片无法加载（可能已过期）",
        Locale::En => "Image unavailable (it may have expired)",
    }
}

pub fn chat_image_lightbox_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "图片预览",
        Locale::En => "Image preview",
    }
}

pub fn chat_image_lightbox_close(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "关闭预览",
        Locale::En => "Close preview",
    }
}

pub fn chat_image_lightbox_copy(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "复制图像",
        Locale::En => "Copy image",
    }
}

pub fn chat_image_lightbox_save(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存图像",
        Locale::En => "Save image",
    }
}

pub fn chat_image_save_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存图像失败。",
        Locale::En => "Could not save the image.",
    }
}

pub fn composer_slash_menu_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "斜杠命令与技能",
        Locale::En => "Slash commands and skills",
    }
}

pub fn composer_slash_menu_loading(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "加载技能列表…",
        Locale::En => "Loading skills…",
    }
}

pub fn composer_slash_menu_empty(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "无匹配项（可试 `/skills`，或在 `.crabmate/skills` 添加技能）",
        Locale::En => "No matches (try `/skills`, or add under `.crabmate/skills`)",
    }
}

pub fn composer_slash_menu_skills_disabled(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "技能已关闭（skills_enabled=false）；仍可选用下方内建命令",
        Locale::En => "Skills disabled (skills_enabled=false); built-in commands still listed",
    }
}

pub fn composer_slash_section_commands(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "命令",
        Locale::En => "Commands",
    }
}

pub fn composer_slash_section_skills(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "技能",
        Locale::En => "Skills",
    }
}

pub fn composer_slash_builtin_skills(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "查看当前工作区 skills 概览（不调用模型）",
        Locale::En => "List workspace skills overview (no model call)",
    }
}

/// 用户气泡内联：显式 `/<skill-id>` 的展示前缀（后接 skill id）。
pub fn msg_skill_invoke_prefix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "使用",
        Locale::En => "Using",
    }
}

/// 用户气泡内联：显式 skill 的展示后缀（接在 id 后）。
pub fn msg_skill_invoke_suffix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "skill",
        Locale::En => "skill",
    }
}

/// 聊天主列 skill 调用卡：标题（明确告知本轮确实调用了 skill）。
pub fn msg_skill_inject_card_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已调用 skill",
        Locale::En => "Skill invoked",
    }
}

/// 聊天主列 skill 调用卡：用户显式 `/<skill-id>` 选用标记。
pub fn msg_skill_inject_card_forced(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "显式",
        Locale::En => "explicit",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：服务端未给 `title` 时的兜底名称。
pub fn msg_context_trim_card_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上下文压缩",
        Locale::En => "Context compacted",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：状态符的无障碍标签（中性语义，避免复用工具卡的「完成」）。
pub fn msg_context_trim_status_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上下文压缩",
        Locale::En => "Context compaction",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：单行摘要（已用 / 可用上下文 + 占用百分比）。
pub fn msg_context_trim_used_available(
    l: Locale,
    used: &str,
    available: &str,
    used_pct: u32,
) -> String {
    match l {
        Locale::ZhHans => format!("已用 {used} tokens ({used_pct}%) · 可用 {available} tokens"),
        Locale::En => format!("Used {used} tokens ({used_pct}%) · available {available} tokens"),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：单行摘要（仅有已用量时）。
pub fn msg_context_trim_used_tokens(l: Locale, used: &str) -> String {
    match l {
        Locale::ZhHans => format!("已用 {used} tokens"),
        Locale::En => format!("Used {used} tokens"),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：单行摘要（消息条数 before → after）。
pub fn msg_context_trim_message_range(l: Locale, before: u32, after: u32) -> String {
    match l {
        Locale::ZhHans => format!("{before} → {after} 条消息"),
        Locale::En => format!("{before} → {after} messages"),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：折叠详情 summary 的 tooltip（展开后仍成立）。
pub fn msg_context_trim_detail_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上下文压缩详情",
        Locale::En => "Context compaction details",
    }
}

/// 上下文压缩 / 裁剪卡展开层的结构化字段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextTrimFact {
    /// 压缩前后 token 数。
    Tokens,
    /// 上下文窗口与预留输出。
    Window,
    /// 消息条数 before → after。
    Messages,
    /// 移除的轮次 / 消息数。
    Removed,
    /// 消息 / 工具 schema / 附件的 token 构成。
    Breakdown,
    /// 压缩方式与触发条件。
    Method,
    /// token 计数来源（服务端标识）。
    CountingSource,
    /// 压缩原因（服务端标识）。
    Reason,
}

/// 聊天主列上下文压缩 / 裁剪卡：结构化字段标签。
pub fn msg_context_trim_fact_label(l: Locale, fact: ContextTrimFact) -> &'static str {
    match (l, fact) {
        (Locale::ZhHans, ContextTrimFact::Tokens) => "Tokens",
        (Locale::En, ContextTrimFact::Tokens) => "Tokens",
        (Locale::ZhHans, ContextTrimFact::Window) => "上下文窗口",
        (Locale::En, ContextTrimFact::Window) => "Context window",
        (Locale::ZhHans, ContextTrimFact::Messages) => "消息",
        (Locale::En, ContextTrimFact::Messages) => "Messages",
        (Locale::ZhHans, ContextTrimFact::Removed) => "移除",
        (Locale::En, ContextTrimFact::Removed) => "Removed",
        (Locale::ZhHans, ContextTrimFact::Breakdown) => "构成",
        (Locale::En, ContextTrimFact::Breakdown) => "Breakdown",
        (Locale::ZhHans, ContextTrimFact::Method) => "方式",
        (Locale::En, ContextTrimFact::Method) => "Method",
        (Locale::ZhHans, ContextTrimFact::CountingSource) => "计数来源",
        (Locale::En, ContextTrimFact::CountingSource) => "Counting source",
        (Locale::ZhHans, ContextTrimFact::Reason) => "原因",
        (Locale::En, ContextTrimFact::Reason) => "Reason",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：上下文窗口（含预留输出）。
pub fn msg_context_trim_window(l: Locale, cap: &str, reserved: Option<&str>) -> String {
    match (l, reserved) {
        (Locale::ZhHans, Some(reserved)) => format!("{cap} tokens（预留输出 {reserved}）"),
        (Locale::En, Some(reserved)) => format!("{cap} tokens (reserved output {reserved})"),
        (Locale::ZhHans, None) => format!("{cap} tokens"),
        (Locale::En, None) => format!("{cap} tokens"),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：移除的轮次 / 消息数。
pub fn msg_context_trim_removed(l: Locale, groups: u32, messages: u32) -> String {
    match (l, groups > 0, messages > 0) {
        (Locale::ZhHans, true, true) => format!("{groups} 轮 · {messages} 条消息"),
        (Locale::En, true, true) => format!("{groups} turn groups · {messages} messages"),
        (Locale::ZhHans, true, false) => format!("{groups} 轮"),
        (Locale::En, true, false) => format!("{groups} turn groups"),
        (Locale::ZhHans, false, true) => format!("{messages} 条消息"),
        (Locale::En, false, true) => format!("{messages} messages"),
        (Locale::ZhHans, false, false) => String::new(),
        (Locale::En, false, false) => String::new(),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：token 构成（消息 / 工具 schema / 附件）。
pub fn msg_context_trim_breakdown(
    l: Locale,
    messages: Option<&str>,
    schema: Option<&str>,
    attachments: Option<&str>,
) -> String {
    let (messages_label, schema_label, attachments_label) = match l {
        Locale::ZhHans => ("消息", "工具定义", "附件"),
        Locale::En => ("messages", "tool schema", "attachments"),
    };
    [
        messages.map(|v| format!("{messages_label} {v}")),
        schema.map(|v| format!("{schema_label} {v}")),
        attachments.map(|v| format!("{attachments_label} {v}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// 聊天主列上下文压缩 / 裁剪卡：摘要方式（`tail_kept` 为保留的末尾条数）。
pub fn msg_context_trim_method_summary(l: Locale, tail_kept: Option<u32>) -> String {
    match (l, tail_kept) {
        (Locale::ZhHans, Some(n)) => format!("摘要（保留末尾 {n} 条）"),
        (Locale::En, Some(n)) => format!("summary (kept last {n})"),
        (Locale::ZhHans, None) => "摘要".to_string(),
        (Locale::En, None) => "summary".to_string(),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：按阈值裁剪历史。
pub fn msg_context_trim_method_trim(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "裁剪历史",
        Locale::En => "history trimmed",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：工具输出压缩次数。
pub fn msg_context_trim_method_compress(l: Locale, hits: u32) -> String {
    match l {
        Locale::ZhHans => format!("压缩工具输出 ×{hits}"),
        Locale::En => format!("tool output compressed ×{hits}"),
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：token 预算触发。
pub fn msg_context_trim_method_token_trigger(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "token 预算触发",
        Locale::En => "token budget triggered",
    }
}

/// 聊天主列上下文压缩 / 裁剪卡：原始 JSON 折叠区标题。
pub fn msg_context_trim_raw_json_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "原始 JSON",
        Locale::En => "Raw JSON",
    }
}

pub fn composer_slash_builtin_skills_list(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "列出可 `/id` 调用的技能明细",
        Locale::En => "List callable `/id` skills in detail",
    }
}

/// Web 内建 `/btw`：旁路提问（结合上下文回答，不写入历史、不调用工具）。
pub fn composer_slash_builtin_btw(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "旁路提问（结合上下文，不写入历史、不调用工具）",
        Locale::En => "Side question (uses context; not saved, no tools)",
    }
}

pub fn composer_slash_builtin_help(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "控制命令帮助（不调用模型）",
        Locale::En => "Control slash help (no model call)",
    }
}

pub fn composer_slash_builtin_workspace(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "显示或切换工作区",
        Locale::En => "Show or switch workspace",
    }
}

pub fn composer_slash_builtin_agent(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "列出或设置 Agent 角色",
        Locale::En => "List or set agent role",
    }
}

pub fn composer_slash_builtin_model(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "显示或设置模型（写 user-data）",
        Locale::En => "Show or set model (persists to user-data)",
    }
}

pub fn composer_slash_builtin_api_base(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "显示或设置 api_base",
        Locale::En => "Show or set api_base",
    }
}

pub fn composer_slash_builtin_export(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "导出当前会话（JSON / Markdown）",
        Locale::En => "Export current session (JSON / Markdown)",
    }
}

pub fn composer_slash_builtin_config(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "配置摘要或 /config reload",
        Locale::En => "Config summary or /config reload",
    }
}

pub fn composer_slash_builtin_context(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "上下文 token / 字符预算",
        Locale::En => "Context token / char budget",
    }
}

pub fn composer_slash_builtin_clear(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "清空当前会话消息",
        Locale::En => "Clear current session messages",
    }
}

pub fn composer_slash_builtin_api_key(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "密钥 status/set/clear（不进对话）",
        Locale::En => "API key status/set/clear (never in chat)",
    }
}

pub fn composer_remove_image_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "移除图片",
        Locale::En => "Remove image",
    }
}

pub fn clarification_panel_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "需要补充信息",
        Locale::En => "More information needed",
    }
}

pub fn clarification_submit(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "提交澄清",
        Locale::En => "Submit answers",
    }
}

pub fn clarification_dismiss(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "关闭",
        Locale::En => "Dismiss",
    }
}

pub fn clarification_required_suffix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "（必填）",
        Locale::En => " (required)",
    }
}

pub fn clarification_missing_required(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "请填写所有必填项",
        Locale::En => "Please fill all required fields",
    }
}

pub fn clarification_user_bubble_stub(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "（已提交澄清问卷）",
        Locale::En => "(Clarification submitted)",
    }
}
// --- 聊天列空态 ---

pub fn chat_history_load_older(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "加载更早的消息",
        Locale::En => "Load older messages",
    }
}

pub fn chat_history_loading_older(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "正在加载更早的消息…",
        Locale::En => "Loading older messages…",
    }
}

pub fn debug_console_region_aria(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "思维与工具调试台",
        Locale::En => "Thinking and tool debug console",
    }
}

pub fn debug_console_empty_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "发起流式对话后，推理增量与工具上下文摘要会出现在此（若服务端用 `CM_THINKING_TRACE_ENABLED=0` 关闭则不会有事件）。"
        }
        Locale::En => {
            "After a streamed reply, reasoning deltas and tool context summaries appear here (unless the server disabled traces with `CM_THINKING_TRACE_ENABLED=0`)."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightbox_copy_is_bilingual() {
        assert!(chat_image_lightbox_aria(Locale::En).contains("preview"));
        assert!(chat_image_lightbox_close(Locale::ZhHans).contains("关闭"));
        assert!(chat_image_lightbox_copy(Locale::En).contains("Copy"));
        assert!(chat_image_lightbox_save(Locale::ZhHans).contains("保存"));
        assert!(chat_image_unavailable(Locale::En).contains("expired"));
    }
}
