//! 设置页 GitHub / Device Flow 相关文案。

use super::Locale;

pub fn settings_section_github_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "GitHub",
        Locale::En => "GitHub",
    }
}

pub fn settings_section_github_desc(_l: Locale) -> &'static str {
    ""
}

pub fn settings_github_block_title(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "GitHub",
        Locale::En => "GitHub",
    }
}

pub fn settings_github_client_id_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "OAuth / App Client ID",
        Locale::En => "OAuth / App Client ID",
    }
}

pub fn settings_github_client_id_unset(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未设置",
        Locale::En => "Not set",
    }
}

pub fn settings_github_client_id_set(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已设置",
        Locale::En => "Set",
    }
}

pub fn settings_github_client_id_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "保存在当前应用 / 浏览器站点的本机存储；仅连接 GitHub 时需要，不是启动必配项。"
        }
        Locale::En => {
            "Stored in this app/site's local storage; required only when connecting GitHub, not at startup."
        }
    }
}

pub fn settings_github_client_id_required(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "请先填写并保存 OAuth / App Client ID",
        Locale::En => "Save an OAuth / App Client ID first",
    }
}

pub fn settings_github_client_id_invalid(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "Client ID 格式无效：最多 128 字节，仅允许字母、数字、点、下划线和连字符",
        Locale::En => {
            "Invalid Client ID: use at most 128 bytes containing only letters, digits, dot, underscore, or hyphen"
        }
    }
}

pub fn settings_github_client_id_storage_failed(l: Locale, detail: &str) -> String {
    match l {
        Locale::ZhHans => format!("Client ID 本机存储失败：{detail}"),
        Locale::En => format!("Failed to store Client ID locally: {detail}"),
    }
}

pub fn settings_github_client_id_save(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存 Client ID",
        Locale::En => "Save Client ID",
    }
}

pub fn settings_github_client_id_clear(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "清除 Client ID",
        Locale::En => "Clear Client ID",
    }
}

pub fn settings_github_client_id_saved(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已保存",
        Locale::En => "Saved",
    }
}

pub fn settings_github_client_id_cleared(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已清除",
        Locale::En => "Cleared",
    }
}

pub fn settings_github_disconnected(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "未连接",
        Locale::En => "Not connected",
    }
}

pub fn settings_github_connected(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已连接",
        Locale::En => "Connected",
    }
}

pub fn settings_github_connection_label(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "连接状态",
        Locale::En => "Connection",
    }
}

pub fn settings_github_token_storage_hint(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "桌面/Android：token 存本机钥匙串；浏览器：由 HttpOnly Cookie 保存，页面脚本无法读取。"
        }
        Locale::En => {
            "Desktop/Android: token is stored in the OS keychain; browser: HttpOnly cookie (not readable by page scripts)."
        }
    }
}

pub fn settings_github_connect(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "连接 GitHub",
        Locale::En => "Connect GitHub",
    }
}

pub fn settings_github_disconnect(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "断开",
        Locale::En => "Disconnect",
    }
}

pub fn settings_github_disconnect_partial(l: Locale, detail: &str) -> String {
    match l {
        Locale::ZhHans => {
            format!("本地已断开（请求不再携带 token），但未能完全清除持久凭据：{detail}")
        }
        Locale::En => format!(
            "Signed out locally (requests no longer send the token), but durable credentials may remain: {detail}"
        ),
    }
}

pub fn settings_github_reopen(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "再次打开授权页",
        Locale::En => "Reopen authorization page",
    }
}

pub fn settings_github_device_expired(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "授权码已过期，请重新连接",
        Locale::En => "Device code expired; please connect again",
    }
}

pub fn settings_github_device_poll_retry(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "网络暂时中断，仍在等待授权…",
        Locale::En => "Network interrupted; still waiting for authorization…",
    }
}

pub fn settings_github_device_state(l: Locale, state: &str) -> String {
    match state {
        "pending" => match l {
            Locale::ZhHans => "等待在浏览器或 GitHub App 中完成授权…".into(),
            Locale::En => "Waiting for authorization in the browser or GitHub App…".into(),
        },
        "slow_down" => match l {
            Locale::ZhHans => "轮询过快，已自动降速…".into(),
            Locale::En => "Polling too fast; slowing down…".into(),
        },
        "success" => match l {
            Locale::ZhHans => "已授权".into(),
            Locale::En => "Authorized".into(),
        },
        "denied" => match l {
            Locale::ZhHans => "已拒绝授权".into(),
            Locale::En => "Authorization denied".into(),
        },
        "expired" => settings_github_device_expired(l).to_string(),
        "cancelled" => match l {
            Locale::ZhHans => "已取消".into(),
            Locale::En => "Cancelled".into(),
        },
        "error" => match l {
            Locale::ZhHans => "授权出错".into(),
            Locale::En => "Authorization error".into(),
        },
        other => other.to_string(),
    }
}
