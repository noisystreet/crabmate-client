//! Linux 上解析 OS 明暗，供主窗 `theme(Some(...))` 使用。
//!
//! WebKitGTK / tao 在 `theme(None)` 时不会打开 `gtk-application-prefer-dark-theme`，
//! WebView 的 `prefers-color-scheme` 会恒为浅色，导致前端 `theme=system` 失效。
//! 探测顺序：xdg-desktop-portal → GNOME gsettings → `GTK_THEME` / gtk settings.ini → KDE kdeglobals。

use tauri::{AppHandle, Theme};

#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Read};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;
#[cfg(target_os = "linux")]
use tauri::Manager;

/// 创建主窗时使用的主题：Linux 显式 Dark/Light；其它平台 `None` 交 OS。
#[must_use]
pub fn initial_window_theme() -> Option<Theme> {
    #[cfg(target_os = "linux")]
    {
        Some(detect_linux_color_scheme_theme())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 供前端 invoke：Linux 上是否应视为深色；其它平台返回 `None`（由 WebView matchMedia 决定）。
#[must_use]
pub fn os_prefers_dark_theme() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        Some(matches!(detect_linux_color_scheme_theme(), Theme::Dark))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Portal `org.freedesktop.appearance color-scheme`：0 无偏好、1 深、2 浅。
/// 无法解析或无偏好时 `None`（继续回退）。
#[cfg(any(test, target_os = "linux"))]
fn parse_portal_color_scheme(stdout: &str) -> Option<bool> {
    match first_uint32_after_label(stdout, "uint32")? {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

#[cfg(any(test, target_os = "linux"))]
fn first_uint32_after_label(stdout: &str, label: &str) -> Option<u32> {
    let lower = stdout.to_ascii_lowercase();
    let idx = lower.find(label)?;
    let rest = &lower[idx + label.len()..];
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(any(test, target_os = "linux"))]
fn parse_gsettings_color_scheme(raw: &str) -> Option<bool> {
    let s = raw.to_ascii_lowercase();
    if s.contains("prefer-dark") {
        Some(true)
    } else if s.contains("prefer-light") {
        Some(false)
    } else {
        None
    }
}

#[cfg(any(test, target_os = "linux"))]
fn gtk_theme_name_is_dark(raw: &str) -> bool {
    let s = raw.trim().trim_matches('\'').to_ascii_lowercase();
    s.contains("-dark") || s.contains(":dark") || s.ends_with("darker")
}

#[cfg(any(test, target_os = "linux"))]
fn ini_first_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let v = rest.trim().trim_matches(['"', '\'']);
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(any(test, target_os = "linux"))]
fn parse_gtk_settings_ini(text: &str) -> Option<bool> {
    if let Some(name) = ini_first_value(text, "gtk-theme-name")
        && gtk_theme_name_is_dark(&name)
    {
        return Some(true);
    }
    let flag = ini_first_value(text, "gtk-application-prefer-dark-theme")?;
    match flag.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        // false/0 是 GTK 常见默认值，不能当成「确定浅色」，否则会挡住 kdeglobals。
        _ => None,
    }
}

#[cfg(any(test, target_os = "linux"))]
fn kde_token_is_dark(token: &str) -> bool {
    token == "dark" || token.ends_with("dark")
}

#[cfg(any(test, target_os = "linux"))]
fn kde_token_is_light(token: &str) -> bool {
    token == "light"
        || (token.ends_with("light") && !token.ends_with("highlight") && token != "delight")
}

#[cfg(any(test, target_os = "linux"))]
fn parse_kdeglobals(text: &str) -> Option<bool> {
    let color = ini_first_value(text, "ColorScheme").unwrap_or_default();
    let look = ini_first_value(text, "LookAndFeelPackage").unwrap_or_default();
    let blob = format!("{color} {look}").to_ascii_lowercase();
    let mut dark = false;
    let mut light = false;
    for token in blob.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        if kde_token_is_dark(token) {
            dark = true;
        }
        if kde_token_is_light(token) {
            light = true;
        }
    }
    if dark {
        Some(true)
    } else if light {
        Some(false)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
const OS_THEME_CMD_TIMEOUT: Duration = Duration::from_millis(1500);

#[cfg(target_os = "linux")]
fn detect_linux_color_scheme_theme() -> Theme {
    let dark = portal_prefers_dark()
        .or_else(gsettings_color_scheme_prefers_dark)
        .or_else(gsettings_gtk_theme_prefers_dark)
        .or_else(gtk_theme_env_prefers_dark)
        .or_else(gtk_settings_ini_prefers_dark)
        .or_else(kdeglobals_prefers_dark)
        .unwrap_or(false);
    if dark { Theme::Dark } else { Theme::Light }
}

#[cfg(target_os = "linux")]
fn command_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child_stdout_timeout(child, OS_THEME_CMD_TIMEOUT)
}

#[cfg(target_os = "linux")]
fn child_stdout_timeout(mut child: std::process::Child, limit: Duration) -> Option<String> {
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(limit) {
        Ok(buf) => finish_child_ok(&mut child, buf),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn finish_child_ok(child: &mut std::process::Child, buf: String) -> Option<String> {
    let status = child.wait().ok()?;
    if !status.success() {
        return None;
    }
    let text = buf.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(target_os = "linux")]
fn portal_prefers_dark() -> Option<bool> {
    let stdout = command_stdout(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            "color-scheme",
        ],
    )?;
    parse_portal_color_scheme(&stdout)
}

#[cfg(target_os = "linux")]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    command_stdout("gsettings", &["get", schema, key])
}

#[cfg(target_os = "linux")]
fn gsettings_color_scheme_prefers_dark() -> Option<bool> {
    parse_gsettings_color_scheme(&gsettings_get(
        "org.gnome.desktop.interface",
        "color-scheme",
    )?)
}

#[cfg(target_os = "linux")]
fn gsettings_gtk_theme_prefers_dark() -> Option<bool> {
    let name = gsettings_get("org.gnome.desktop.interface", "gtk-theme")?;
    gtk_theme_name_is_dark(&name).then_some(true)
}

#[cfg(target_os = "linux")]
fn gtk_theme_env_prefers_dark() -> Option<bool> {
    let name = std::env::var("GTK_THEME").ok()?;
    gtk_theme_name_is_dark(&name).then_some(true)
}

#[cfg(target_os = "linux")]
fn xdg_config_home() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("XDG_CONFIG_HOME")
        && !p.is_empty()
    {
        return Some(PathBuf::from(p));
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".config"))
}

#[cfg(target_os = "linux")]
fn read_xdg_config(rel: &str) -> Option<String> {
    let path = xdg_config_home()?.join(rel);
    std::fs::read_to_string(path).ok()
}

#[cfg(target_os = "linux")]
fn gtk_settings_ini_prefers_dark() -> Option<bool> {
    for rel in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Some(parsed) = read_xdg_config(rel).and_then(|t| parse_gtk_settings_ini(&t)) {
            return Some(parsed);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn kdeglobals_prefers_dark() -> Option<bool> {
    parse_kdeglobals(&read_xdg_config("kdeglobals")?)
}

#[cfg(target_os = "linux")]
fn apply_detected_theme(app: &AppHandle) {
    let theme = detect_linux_color_scheme_theme();
    let handle = app.clone();
    // 主窗可能已关；忽略回调失败。
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window("main") {
            let _ = win.set_theme(Some(theme));
        }
    });
}

#[cfg(target_os = "linux")]
fn spawn_line_watcher(
    name: &'static str,
    bin: &'static str,
    args: &'static [&str],
    app: AppHandle,
) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let Ok(mut child) = Command::new(bin)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return;
            };
            let Some(stdout) = child.stdout.take() else {
                return;
            };
            let filter_portal = name.contains("portal");
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if filter_portal && !line.to_ascii_lowercase().contains("color-scheme") {
                    continue;
                }
                apply_detected_theme(&app);
            }
        })
        .ok();
}

/// 监听 OS 明暗变化并更新主窗主题（使 WebView matchMedia 跟随）。
pub fn spawn_linux_color_scheme_watcher(app: AppHandle) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
    }
    #[cfg(target_os = "linux")]
    {
        spawn_line_watcher(
            "cm-os-theme-gsettings",
            "gsettings",
            &["monitor", "org.gnome.desktop.interface", "color-scheme"],
            app.clone(),
        );
        spawn_line_watcher(
            "cm-os-theme-portal",
            "gdbus",
            &[
                "monitor",
                "--session",
                "--dest",
                "org.freedesktop.portal.Desktop",
            ],
            app,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_uint32_1_is_dark() {
        assert_eq!(parse_portal_color_scheme("(<'uint32 1'>,)"), Some(true));
        assert_eq!(parse_portal_color_scheme("(<uint32 1>,)"), Some(true));
        assert_eq!(
            parse_portal_color_scheme("variant       uint32 1"),
            Some(true)
        );
    }

    #[test]
    fn portal_uint32_2_is_light() {
        assert_eq!(parse_portal_color_scheme("(<'uint32 2'>,)"), Some(false));
    }

    #[test]
    fn portal_uint32_0_or_garbage_is_none() {
        assert_eq!(parse_portal_color_scheme("(<'uint32 0'>,)"), None);
        assert_eq!(parse_portal_color_scheme(""), None);
        assert_eq!(parse_portal_color_scheme("error"), None);
    }

    #[test]
    fn gsettings_prefer_dark_and_light() {
        assert_eq!(parse_gsettings_color_scheme("'prefer-dark'"), Some(true));
        assert_eq!(parse_gsettings_color_scheme("prefer-light"), Some(false));
        assert_eq!(parse_gsettings_color_scheme("'default'"), None);
    }

    #[test]
    fn gtk_theme_name_dark_markers() {
        assert!(gtk_theme_name_is_dark("Adwaita-dark"));
        assert!(gtk_theme_name_is_dark("'Adwaita:dark'"));
        assert!(gtk_theme_name_is_dark("Yaru-darker"));
        assert!(!gtk_theme_name_is_dark("Adwaita"));
    }

    #[test]
    fn gtk_settings_ini_prefer_dark_and_theme_name() {
        assert_eq!(
            parse_gtk_settings_ini("[Settings]\ngtk-application-prefer-dark-theme=true\n"),
            Some(true)
        );
        assert_eq!(
            parse_gtk_settings_ini("[Settings]\ngtk-theme-name=Adwaita-dark\n"),
            Some(true)
        );
        assert_eq!(
            parse_gtk_settings_ini("[Settings]\ngtk-application-prefer-dark-theme=false\n"),
            None
        );
    }

    #[test]
    fn gtk_ini_false_does_not_override_kde_dark() {
        assert_eq!(
            parse_gtk_settings_ini("[Settings]\ngtk-application-prefer-dark-theme=0\n"),
            None
        );
        assert_eq!(
            parse_kdeglobals("[General]\nColorScheme=BreezeDark\n"),
            Some(true)
        );
    }

    #[test]
    fn kdeglobals_breeze_dark_and_light() {
        assert_eq!(
            parse_kdeglobals("[General]\nColorScheme=BreezeDark\n"),
            Some(true)
        );
        assert_eq!(
            parse_kdeglobals("[KDE]\nLookAndFeelPackage=org.kde.breezedark.desktop\n"),
            Some(true)
        );
        assert_eq!(
            parse_kdeglobals("[General]\nColorScheme=BreezeLight\n"),
            Some(false)
        );
        assert_eq!(parse_kdeglobals("[General]\nColorScheme=Breeze\n"), None);
        assert_eq!(parse_kdeglobals("[General]\nColorScheme=Highlight\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detect_returns_dark_or_light() {
        let t = detect_linux_color_scheme_theme();
        assert!(matches!(t, Theme::Dark | Theme::Light));
    }
}
