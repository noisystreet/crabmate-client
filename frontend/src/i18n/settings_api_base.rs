//! 设置页：远程 serve API 基址文案（从 `settings` 拆出以控制单文件行数）。

use super::Locale;

pub fn settings_block_api_base(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "API 基址",
        Locale::En => "API base URL",
    }
}

pub fn settings_api_base_save(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "保存 API 基址",
        Locale::En => "Save API base",
    }
}

pub fn settings_api_base_saved(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已保存 API 基址；正在重新拉取状态…",
        Locale::En => "API base saved; refreshing status…",
    }
}

pub fn settings_api_base_cleared(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已清空；改回同 Origin 相对路径。",
        Locale::En => "Cleared; back to same-origin relative paths.",
    }
}

pub fn settings_api_base_invalid(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "无效地址：须为 http:// 或 https:// 开头（或留空）。",
        Locale::En => "Invalid URL: must start with http:// or https:// (or leave empty).",
    }
}

pub fn settings_save_ok_browser_insecure_key(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => {
            "已保存（本浏览器弱持久化：明文 localStorage，不如系统钥匙串安全）。正式使用请用 Desktop / Android 壳。"
        }
        Locale::En => {
            "Saved (plaintext localStorage in this browser; less safe than the system keyring). Prefer the official shell."
        }
    }
}
