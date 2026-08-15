//! 设置页 MCP 相关文案。

use super::Locale;

pub fn settings_section_mcp_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "MCP 服务器",
        Locale::En => "MCP Servers",
    }
}

pub fn settings_section_mcp_desc(_l: Locale) -> &'static str {
    ""
}

pub fn settings_mcp_global_enabled_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "启用 MCP 工具合并",
        Locale::En => "Enable MCP tool merging",
    }
}

pub fn settings_mcp_timeout_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工具调用超时（秒）",
        Locale::En => "Tool call timeout (seconds)",
    }
}

pub fn settings_mcp_enabled_missing_command(l: Locale, name: &str) -> String {
    match l {
        Locale::ZhHans => {
            format!("已启用的服务器「{name}」须含 command 或 url，请用 JSON 导入或先禁用后再保存。")
        }
        Locale::En => format!(
            "Enabled server \"{name}\" needs a command or url; import via JSON or disable it before saving."
        ),
    }
}

pub fn settings_mcp_import_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "从 MCP JSON 导入",
        Locale::En => "Import from MCP JSON",
    }
}

pub fn settings_mcp_import_placeholder(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "{\n  \"mcpServers\": {\n    \"my-server\": {\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"mcp-server\"]\n    }\n  }\n}"
        }
        Locale::En => {
            "{\n  \"mcpServers\": {\n    \"my-server\": {\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"mcp-server\"]\n    }\n  }\n}"
        }
    }
}

pub fn settings_mcp_import_paste_detected(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已检测到 MCP 配置 JSON",
        Locale::En => "Detected MCP config JSON",
    }
}

pub fn settings_mcp_import_apply(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "解析并添加到列表",
        Locale::En => "Parse and add to list",
    }
}

pub fn settings_mcp_import_merged_on_save(l: Locale) -> String {
    match l {
        Locale::ZhHans => "已合并粘贴的 MCP JSON，正在保存…",
        Locale::En => "Merged pasted MCP JSON; saving…",
    }
    .to_string()
}

pub fn settings_mcp_import_success(l: Locale, n: usize) -> String {
    match l {
        Locale::ZhHans => format!("已导入 {n} 个 MCP 服务器（启动命令已写入本机配置）"),
        Locale::En => format!("Imported {n} MCP server(s) (start commands saved locally)"),
    }
}

pub fn settings_mcp_import_skipped_remote(l: Locale, names: &str) -> String {
    match l {
        Locale::ZhHans => format!("已跳过远程 MCP（仅支持 stdio）：{names}"),
        Locale::En => format!("Skipped remote MCP (stdio only): {names}"),
    }
}

pub fn settings_mcp_save(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存 MCP 配置",
        Locale::En => "Save MCP settings",
    }
}

pub fn settings_mcp_probe(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "探测",
        Locale::En => "Probe",
    }
}

pub fn settings_mcp_probe_all(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "探测全部",
        Locale::En => "Probe all",
    }
}

pub fn settings_mcp_remove(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "删除",
        Locale::En => "Remove",
    }
}

pub fn settings_mcp_name_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "名称",
        Locale::En => "Name",
    }
}

pub fn settings_mcp_transport_stdio(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "本地 stdio",
        Locale::En => "Local stdio",
    }
}

pub fn settings_mcp_transport_remote(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "远程 HTTP",
        Locale::En => "Remote HTTP",
    }
}

pub fn settings_mcp_bearer_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "Bearer 令牌",
        Locale::En => "Bearer token",
    }
}

pub fn settings_mcp_bearer_hint_set(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已配置 Bearer",
        Locale::En => "Bearer is set",
    }
}

pub fn settings_mcp_bearer_hint_unset(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "尚未配置 Bearer",
        Locale::En => "Bearer not set",
    }
}

pub fn settings_mcp_bearer_save(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存 Bearer",
        Locale::En => "Save Bearer",
    }
}

pub fn settings_mcp_bearer_saved(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "Bearer 已保存",
        Locale::En => "Bearer saved",
    }
}

pub fn settings_mcp_bearer_cleared(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "Bearer 已清除",
        Locale::En => "Bearer cleared",
    }
}

pub fn settings_mcp_enabled_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "启用",
        Locale::En => "Enabled",
    }
}

pub fn settings_mcp_tools_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "可用工具",
        Locale::En => "Available tools",
    }
}

pub fn settings_mcp_tools_col_name(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "工具名称",
        Locale::En => "Tool name",
    }
}

pub fn settings_mcp_tools_col_description(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "说明",
        Locale::En => "Description",
    }
}

pub fn settings_mcp_tools_desc_empty(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "—",
        Locale::En => "—",
    }
}

pub fn settings_mcp_tools_probing(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "正在连接并拉取工具列表…",
        Locale::En => "Connecting and fetching tools…",
    }
}

pub fn settings_mcp_tools_none(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已连接，但远端未返回任何工具。",
        Locale::En => "Connected, but the server returned no tools.",
    }
}

pub fn settings_mcp_tools_server_disabled(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未启用",
        Locale::En => "Disabled",
    }
}

pub fn settings_mcp_tools_toggle_label(l: Locale, tool_count: Option<usize>) -> String {
    let base = settings_mcp_tools_title(l);
    match tool_count {
        Some(n) => match l {
            Locale::ZhHans => format!("{base}（{n}）"),
            Locale::En => format!("{base} ({n})"),
        },
        None => base.to_string(),
    }
}

pub fn settings_mcp_tools_expand_aria(l: Locale, expanded: bool) -> &'static str {
    match (l, expanded) {
        (Locale::ZhHans, true) => "折叠可用工具列表",
        (Locale::ZhHans, false) => "展开可用工具列表",
        (Locale::En, true) => "Collapse available tools",
        (Locale::En, false) => "Expand available tools",
    }
}

pub fn settings_mcp_disconnected(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未连接",
        Locale::En => "Not connected",
    }
}
