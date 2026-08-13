use super::Locale;

// --- 系统提示（alert / confirm）---

pub fn clipboard_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "复制失败：浏览器未授权剪贴板或不可用。",
        Locale::En => "Copy failed: clipboard permission denied or unavailable.",
    }
}

pub fn delete_session_confirm(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "确定删除此本地会话？此操作不可恢复。",
        Locale::En => "Delete this local session? This cannot be undone.",
    }
}

/// 流式进行中禁止删除绑定会话（否则 SSE 收尾找不到写入目标，内容静默丢失）。
pub fn delete_session_streaming_blocked(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "会话正在生成中，请先停止或等待生成完成后再删除。",
        Locale::En => {
            "This session is still generating. Stop or wait for it to finish before deleting."
        }
    }
}

/// 新建会话默认标题（写入 `ChatSession.title`）；与旧数据 **`新会话`** 等价判断见 [`is_default_session_title`]。
pub fn default_session_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "新会话",
        Locale::En => "New chat",
    }
}

/// 是否与当前语言下的默认会话标题等价（含历史中文默认值）。
pub fn is_default_session_title(s: &str) -> bool {
    let t = s.trim();
    t == default_session_title(Locale::ZhHans)
        || t == default_session_title(Locale::En)
        || t == "新会话"
        || t.eq_ignore_ascii_case("new chat")
}

/// 侧栏/列表展示用：默认标题随界面语言切换，其它标题保持原样。
pub fn session_title_for_display(stored: &str, loc: Locale) -> String {
    if is_default_session_title(stored) {
        default_session_title(loc).to_string()
    } else {
        stored.to_string()
    }
}
