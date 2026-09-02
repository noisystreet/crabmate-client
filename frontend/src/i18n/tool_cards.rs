use super::Locale;

// --- 流式占位 / 非工具卡正文（工具卡 compact/detail 见 `crabmate_tool_card`）---

pub fn tool_card_prefix(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工具：",
        Locale::En => "Tool: ",
    }
}

pub fn tool_card_fallback(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工具输出",
        Locale::En => "Tool output",
    }
}

// --- 后台任务（run_command `async:true`）气泡 ---

/// 后台任务状态徽标可读文案（如「后台任务排队中」）。
pub fn tool_job_status_label(l: Locale, status: &str) -> String {
    let s = match status {
        "queued" => match l {
            Locale::ZhHans => "排队中",
            Locale::En => "queued",
        },
        "running" => match l {
            Locale::ZhHans => "运行中",
            Locale::En => "running",
        },
        "succeeded" => match l {
            Locale::ZhHans => "成功",
            Locale::En => "succeeded",
        },
        "failed" => match l {
            Locale::ZhHans => "失败",
            Locale::En => "failed",
        },
        "cancelled" => match l {
            Locale::ZhHans => "已取消",
            Locale::En => "cancelled",
        },
        "timed_out" => match l {
            Locale::ZhHans => "超时",
            Locale::En => "timed out",
        },
        other => other,
    };
    match l {
        Locale::ZhHans => format!("后台任务{s}"),
        Locale::En => format!("background job {s}"),
    }
}

/// 非终态后台任务的取消按钮文案。
pub fn tool_job_cancel_button(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "取消",
        Locale::En => "Cancel",
    }
}

/// 后台任务详情行（折叠展开后首行：任务 id 与状态）。
pub fn tool_job_detail_line(l: Locale, job_id: &str, status: &str) -> String {
    match l {
        Locale::ZhHans => format!("后台任务 {job_id}：{status}"),
        Locale::En => format!("background job {job_id}: {status}"),
    }
}

/// 实时输出区可读标签（无障碍 `role="log"`）。
pub fn tool_job_output_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "后台任务输出",
        Locale::En => "background job output",
    }
}

/// 实时输出因环形缓冲上限被丢弃早段的提示（`/output` 响应 `truncated=true`）。
pub fn tool_job_output_truncated_hint(l: Locale) -> String {
    match l {
        Locale::ZhHans => "输出过长已截断，仅保留最近一部分。".to_string(),
        Locale::En => "Output truncated; only the recent tail is kept.".to_string(),
    }
}

pub fn plan_generated(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已生成分阶段规划。",
        Locale::En => "Staged plan generated.",
    }
}

pub fn plan_no_new_tool_calls_note(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "（本轮不调用新工具）",
        Locale::En => "(No new tool calls this turn)",
    }
}

pub fn plan_step_no_desc(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "(未提供描述)",
        Locale::En => "(no description)",
    }
}

pub fn plan_step_placeholder_id() -> &'static str {
    "step"
}

pub fn plan_step_line(l: Locale, idx: usize, id: &str, desc: &str) -> String {
    let n = idx + 1;
    match l {
        Locale::ZhHans => format!("{n}. `{id}`: {desc}"),
        Locale::En => format!("{n}. `{id}`: {desc}"),
    }
}

/// 内置工具的固定种类符；长尾/MCP 名不返回（避免与行首状态符抢戏的随机哈希图标）。
#[must_use]
pub fn tool_kind_emoji_curated(name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    Some(match n {
        "run_command" => "⚡",
        "playbook_run_commands" => "📜",
        "read_file" => "📄",
        "read_binary_meta" => "💽",
        "extract_in_file" => "📤",
        "read_dir" => "📁",
        "list_tree" => "🌲",
        "glob_files" => "✴️",
        "file_exists" => "❔",
        "create_file" => "📝",
        "append_file" => "➕",
        "create_dir" => "📂",
        "modify_file" => "✏️",
        "search_replace" => "🔁",
        "apply_patch" => "🩹",
        "chmod_file" => "🛡️",
        "copy_file" => "📋",
        "move_file" => "🚚",
        "symlink_info" => "🔗",
        "delete_file" | "delete_dir" => "🗑️",
        "search_in_files" => "🔎",
        "codebase_semantic_search" => "🧭",
        "http_fetch" | "http_request" => "🌐",
        "web_search" => "🔍",
        "get_weather" => "🌤️",
        "calc" => "🧮",
        "date_calc" => "📆",
        "convert_units" => "↔️",
        "regex_test" => "🔣",
        "format_file" | "format_check_file" => "🎨",
        "get_current_time" => "🕐",
        "text_transform" => "🔤",
        "table_text" => "🔠",
        "text_diff" => "⚖️",
        "json_format" => "🗃️",
        "package_query" => "📦",
        "find_symbol" => "🔭",
        "find_references" => "📍",
        "call_graph_sketch" => "🕸️",
        "rust_file_outline" => "📑",
        "code_stats" => "📊",
        "dependency_graph" => "🪢",
        "coverage_report" => "📈",
        "hash_file" => "🔢",
        "port_check" => "🔌",
        "process_list" => "📟",
        "archive_pack" => "🗜️",
        "archive_unpack" => "📤",
        "archive_list" => "📃",
        "run_lints" | "quality_workspace" | "rust_backtrace_analyze" => "✅",
        "cargo_audit" => "🔒",
        "cargo_deny" => "🚧",
        "long_term_remember" => "💭",
        "long_term_forget" => "🧹",
        "long_term_memory_list" => "📚",
        "summarize_experience" => "📔",
        "add_reminder" | "complete_reminder" | "delete_reminder" => "⏰",
        "list_reminders" => "📒",
        "add_event" | "delete_event" | "update_event" => "🗓️",
        "list_events" => "📅",
        "diagnostic_summary" => "🩺",
        "error_output_playbook" => "📕",
        "present_clarification_questionnaire" => "❓",
        "changelog_draft" => "📰",
        "license_notice" => "🧾",
        "repo_overview_sweep" => "🗺️",
        "todo_scan" => "🗒️",
        "env_var_check" => "🔐",
        "structured_validate" => "✔️",
        "structured_query" => "🔬",
        "structured_diff" => "➖",
        "structured_patch" => "🧩",
        "markdown_check_links" => "📎",
        "workflow_execute" => "⚙️",
        "ci_pipeline_local" => "🛤️",
        "release_ready_check" => "🚀",
        "terminal_session" => "⌨️",
        _ => return None,
    })
}
