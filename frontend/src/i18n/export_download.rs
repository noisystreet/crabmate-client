//! 浏览器 / Tauri 导出与下载相关提示。

use super::Locale;

pub fn export_tauri_save_cancelled_alert(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "已取消保存。",
        Locale::En => "Save cancelled.",
    }
}

pub fn export_tauri_save_failed_alert(l: Locale, err: &str) -> String {
    match l {
        Locale::ZhHans => format!("导出失败（Tauri 保存对话框）：{err}"),
        Locale::En => format!("Export failed (Tauri save dialog): {err}"),
    }
}

pub fn export_android_save_failed(l: Locale) -> &'static str {
    match l {
        Locale::ZhHans => "无法打开系统保存对话框，文件未保存到本机。",
        Locale::En => "Could not open the system save dialog; the file was not saved.",
    }
}

/// Android 另存为失败提示：按原生回传的稳定原因码选文案。
///
/// 码表见 `ChatImageShare` / `PendingDeviceSave` 的 `ERR_*` 常量（`origin` / `busy` / `chunk` /
/// `not_begun` / `too_large` / `decode` / `write`）。未知码退回 [`export_android_save_failed`]。
pub fn export_android_save_failed_alert(l: Locale, code: &str) -> String {
    match code {
        "too_large" => match l {
            Locale::ZhHans => {
                "文件超过 16 MB，未能保存到本机；请改用更短的会话或分次导出。".to_string()
            }
            Locale::En => {
                "The file is over 16 MB and was not saved; export a shorter conversation."
                    .to_string()
            }
        },
        "write" => match l {
            Locale::ZhHans => "写入失败，文件未保存到本机。".to_string(),
            Locale::En => "Writing the file failed; nothing was saved to this device.".to_string(),
        },
        "busy" => match l {
            Locale::ZhHans => "上一次保存还没有结束，请稍后再试。".to_string(),
            Locale::En => {
                "The previous save has not finished yet; try again in a moment.".to_string()
            }
        },
        "decode" => match l {
            Locale::ZhHans => "文件内容解码失败，未保存到本机。".to_string(),
            Locale::En => "The file contents could not be decoded; nothing was saved.".to_string(),
        },
        "origin" => match l {
            Locale::ZhHans => "当前页面不允许保存到本机，请在 App 内重试。".to_string(),
            Locale::En => "This page may not save to the device; retry inside the app.".to_string(),
        },
        _ => export_android_save_failed(l).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 码表这一侧由 `ChatImageShare` / `PendingDeviceSave` 的 `ERR_*` 常量决定，
    /// 改任一侧都要让这个测试与原生常量同时移动。
    #[test]
    fn android_save_alert_maps_known_codes() {
        assert!(export_android_save_failed_alert(Locale::ZhHans, "too_large").contains("16 MB"));
        assert!(export_android_save_failed_alert(Locale::En, "too_large").contains("16 MB"));
        assert!(export_android_save_failed_alert(Locale::ZhHans, "write").contains("写入失败"));
        assert_eq!(
            export_android_save_failed_alert(Locale::En, "write"),
            "Writing the file failed; nothing was saved to this device."
        );
        assert!(export_android_save_failed_alert(Locale::ZhHans, "busy").contains("还没有结束"));
        assert!(export_android_save_failed_alert(Locale::ZhHans, "decode").contains("解码"));
        assert!(export_android_save_failed_alert(Locale::ZhHans, "origin").contains("App"));
    }

    #[test]
    fn android_save_alert_falls_back_for_unknown_code() {
        assert_eq!(
            export_android_save_failed_alert(Locale::ZhHans, "native folder picker unavailable"),
            export_android_save_failed(Locale::ZhHans)
        );
        assert_eq!(
            export_android_save_failed_alert(Locale::En, ""),
            export_android_save_failed(Locale::En)
        );
    }
}
