//! 壳级偏好与 **`document.documentElement`** 同步（持久化在 **`/user-data/prefs`**，见 [`crate::user_prefs_sync`]）。
//!
//! # 与其它模块分工
//!
//! - **主题 slug 白名单**：[`crate::app_prefs::THEME_SLUGS`] / [`normalize_theme_slug`]；DOM 用 [`resolve_data_theme_slug`]（`system` → `crabmate-dark`/`crabmate-light`）。加载偏好时在 [`crate::user_prefs_sync`] 中规范化。
//! - **首屏壳 UI 快照**：[`read_shell_ui_initial_snapshot`] 聚合主题/语言/侧栏宽度等读路径，供 [`super::app_signals::ShellUISignals::new`] 单点消费。
//! - **首帧主题镜像**：[`THEME_PREF_MIRROR_KEY`] 把**偏好 slug**（含 `system`）落 `localStorage`，供 `frontend/index.html` 的预绘脚本与 [`read_shell_ui_initial_snapshot`] 在 `/user-data/prefs` 到达前还原主题；主题非机密，与本仓「官方壳禁明文 `localStorage`」的 Bearer / model key 规则无关。
//! - **会话 JSON**：[`crate::storage`] / [`crate::app::chat::session_storage`]。
//! - **`client_llm.*` / Bearer**：[`crate::api::client_llm_storage`]。
//!
//! 新增「首屏就读 / Effect 里写磁盘或改 DOM」的壳偏好时，优先在本模块加函数，避免在多个 `wire_*` 文件里散落写偏好逻辑。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::app_signals::AppSignals;
use crate::app_prefs::{DEFAULT_SIDE_WIDTH, SidePanelView};
use crate::i18n::Locale;
use crate::session_typography_prefs::{session_chat_font_stack_css, session_ui_font_stack_css};

/// 首屏默认（真实偏好由 [`crate::user_prefs_sync::wire_load_user_prefs_from_server`] 异步覆盖）。
#[derive(Clone)]
pub(crate) struct ShellUiInitialSnapshot {
    pub theme: String,
    pub bg_decor: bool,
    /// 聊天主列是否展示本轮注入 / 裁剪时间线（首屏默认关）。
    pub show_turn_context_inject: bool,
    pub locale: Locale,
    pub side_panel_view: SidePanelView,
    pub side_width: f64,
    pub editor_layout_mode: bool,
    pub session_ui_font: String,
    pub session_chat_font: String,
    pub session_chat_font_size: f64,
}

/// 按端决定进入壳层后的工作区侧栏视图（纯函数，便于单测）。
///
/// - 移动 / 窄屏：强制收起（仅右缘左划或显式操作再开）
/// - 宽屏（桌面 Tauri / 浏览器）：`None` 时展开为 Workspace（不沿用移动端写入的 `none`）
#[must_use]
pub(crate) fn platform_side_panel_on_entry(
    narrow_or_mobile: bool,
    current: SidePanelView,
) -> SidePanelView {
    if narrow_or_mobile {
        return SidePanelView::None;
    }
    if matches!(current, SidePanelView::None) {
        return SidePanelView::Workspace;
    }
    current
}

/// 移动 / 窄屏时侧栏仅本端生效，不得经 `/user-data/prefs` 写回覆盖其它客户端。
#[must_use]
pub(crate) fn narrow_or_mobile_shell_layout(narrow_viewport: bool) -> bool {
    crate::mobile_remote::mobile_remote_client() || narrow_viewport
}

/// 侧栏视图是否应与 `/user-data/prefs` 同步。
#[must_use]
pub(crate) fn shell_layout_prefs_sync_to_server(narrow_or_mobile: bool) -> bool {
    !narrow_or_mobile
}

/// 进入壳层后按端调整工作区侧栏。
pub(crate) fn apply_platform_shell_ui_on_entry(app: &AppSignals) {
    let narrow = app.shell_ui.is_narrow_viewport.get_untracked();
    let narrow_or_mobile = narrow_or_mobile_shell_layout(narrow);
    let next = platform_side_panel_on_entry(
        narrow_or_mobile,
        app.shell_ui.side_panel_view.get_untracked(),
    );
    if next != app.shell_ui.side_panel_view.get_untracked() {
        app.shell_ui.side_panel_view.set(next);
    }
}

/// 首帧主题镜像键（`localStorage`）：只存**偏好 slug**（含 `system`），由
/// [`persist_theme_to_storage_and_dom`] 单点写入，`frontend/index.html` 的 `<head>` 预绘脚本
/// 与 [`read_shell_ui_initial_snapshot`] 两处读取——三处必须保持同键同语义。
///
/// 主题是可公开的观感偏好，不涉及凭据：本仓「官方壳禁明文 `localStorage`」的约束只针对
/// Bearer / model key（见 `api::web_api_bearer_local`），与此处无关。
pub(crate) const THEME_PREF_MIRROR_KEY: &str = "crabmate.theme.pref";

/// 镜像缺失 / 非法时的首帧主题基线：与 `tokens.css` 的 `:root` 深色及启动页内联深色一致。
///
/// 不能用 `DEFAULT_THEME_SLUG`（`shadcn-light`）兜底：偏好要等 `GET /user-data/prefs` 才到，
/// 浅色基线会让深色用户开屏闪一帧浅色；`system` 在此刻也拿不到可靠的 OS 明暗。
pub(crate) const FIRST_FRAME_THEME_BASELINE: &str = "crabmate-dark";

/// 镜像原始值 → 首帧偏好 slug（纯函数，便于单测）。
///
/// 合法值原样返回（含 `system`，交给 [`crate::app_prefs::resolve_data_theme_slug`] 在使用点解析）；
/// 缺失 / 空白 / 不在 [`crate::app_prefs::THEME_SLUGS`] 内一律回退 [`FIRST_FRAME_THEME_BASELINE`]。
#[must_use]
pub(crate) fn theme_pref_from_mirror(raw: Option<&str>) -> String {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return FIRST_FRAME_THEME_BASELINE.to_string();
    };
    if crate::app_prefs::THEME_SLUGS.contains(&value) {
        value.to_string()
    } else {
        FIRST_FRAME_THEME_BASELINE.to_string()
    }
}

/// 读镜像；存储不可用 / 读失败一律 `None`（镜像只是首帧优化，不得阻塞首绘）。
fn read_theme_pref_mirror() -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    storage.get_item(THEME_PREF_MIRROR_KEY).ok().flatten()
}

/// 写镜像；存储不可用（隐私模式 / 配额）时静默放弃——镜像不是真源。
fn write_theme_pref_mirror(theme_pref: &str) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return;
    };
    let _ = storage.set_item(THEME_PREF_MIRROR_KEY, theme_pref);
}

#[must_use]
pub(crate) fn read_shell_ui_initial_snapshot() -> ShellUiInitialSnapshot {
    ShellUiInitialSnapshot {
        // 首帧主题来自本机镜像（`index.html` 预绘脚本读同一个键）：水合后的 DOM 同步 Effect
        // 会立刻把信号值写回 `data-theme`，若此处仍用固定基线就会把预绘的浅色冲回深色，
        // 闪帧反而变成「浅 → 深 → 浅」。镜像缺失 / 非法时回退深色基线，与 `tokens.css`
        // 的 `:root` 及启动页内联深色一致。首帧值不会写回服务端覆盖偏好——
        // `UserPrefsSyncPhase` 在偏好加载完成前禁止 PUT。
        theme: theme_pref_from_mirror(read_theme_pref_mirror().as_deref()),
        bg_decor: true,
        show_turn_context_inject: false,
        locale: Locale::ZhHans,
        side_panel_view: SidePanelView::Workspace,
        side_width: DEFAULT_SIDE_WIDTH,
        editor_layout_mode: false,
        session_ui_font: "default".to_string(),
        session_chat_font: "default".to_string(),
        session_chat_font_size: crate::session_typography_prefs::DEFAULT_SESSION_CHAT_FONT_SIZE,
    }
}

/// `GET /user-data/prefs` 灌入信号后，将主题/语言/字体等反映到 DOM。
pub(crate) fn apply_loaded_prefs_to_dom(app: &AppSignals) {
    persist_theme_to_storage_and_dom(&app.shell_ui.theme.get_untracked());
    apply_locale_html_lang(app.shell_ui.locale.get_untracked());
    persist_bg_decor_to_storage_and_dom(app.shell_ui.bg_decor.get_untracked());
    persist_session_typography_to_storage_and_dom(
        &app.shell_ui.session_ui_font.get_untracked(),
        &app.shell_ui.session_chat_font.get_untracked(),
        app.shell_ui.session_chat_font_size.get_untracked(),
    );
    apply_shell_layout_dom_flags(app.shell_ui.editor_layout_mode.get_untracked());
}

/// 会话模式界面 / 聊天列字体与字号：在 `<html>` 上维护 `--crabmate-ui-font-family` /
/// `--crabmate-chat-font-family` / `--crabmate-chat-font-size`。
pub(crate) fn persist_session_typography_to_storage_and_dom(
    ui_slug: &str,
    chat_slug: &str,
    chat_font_size_px: f64,
) {
    let ui = crate::session_typography_prefs::normalize_session_ui_font(ui_slug);
    let chat = crate::session_typography_prefs::normalize_session_chat_font(chat_slug);
    let size = crate::session_typography_prefs::clamp_session_chat_font_size(chat_font_size_px);
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(root) = doc.document_element() else {
        return;
    };
    let Some(html_root) = root.dyn_ref::<web_sys::HtmlElement>() else {
        return;
    };
    let style = html_root.style();
    match session_ui_font_stack_css(&ui) {
        Some(stack) => {
            let _ = style.set_property("--crabmate-ui-font-family", stack);
        }
        None => {
            let _ = style.remove_property("--crabmate-ui-font-family");
        }
    }
    match session_chat_font_stack_css(&chat) {
        Some(stack) => {
            let _ = style.set_property("--crabmate-chat-font-family", stack);
        }
        None => {
            let _ = style.remove_property("--crabmate-chat-font-family");
        }
    }
    let size_css = format!("{size}px");
    let _ = style.set_property("--crabmate-chat-font-size", &size_css);
}

/// 设置 `data-theme` 为**解析后的 CSS slug**（`system` → `crabmate-dark`/`crabmate-light`），
/// 并把**偏好 slug** 写入首帧镜像 [`THEME_PREF_MIRROR_KEY`]（服务端持久化由 [`crate::user_prefs_sync`] 负责）。
pub(crate) fn persist_theme_to_storage_and_dom(theme_pref: &str) {
    // 写偏好而非解析结果：`system` 一旦被写成 `crabmate-*` 就永久丢掉「跟随系统」语义。
    write_theme_pref_mirror(theme_pref.trim());
    let css = crate::app_prefs::resolve_data_theme_slug(theme_pref);
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(root) = doc.document_element()
    {
        let _ = root.set_attribute("data-theme", &css);
    }
    // Android：状态栏透明后露出的是 Web 顶栏底色，系统栏图标须随主题明暗切换（否则浅色主题下白图标不可见）。
    crate::mobile_remote::apply_mobile_system_bar_icons(crate::app_prefs::theme_css_is_light(&css));
}

/// 将界面语言反映到 `<html lang>`（不写 `localStorage`；语言持久化在 i18n 路径）。
pub(crate) fn apply_locale_html_lang(locale: Locale) {
    let lang = locale.html_lang();
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let _ = doc
            .document_element()
            .map(|root| root.set_attribute("lang", lang));
    }
}

/// 背景装饰：维护 `data-bg-decor`。
pub(crate) fn persist_bg_decor_to_storage_and_dom(bg_decor: bool) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(root) = doc.document_element()
    {
        if bg_decor {
            let _ = root.remove_attribute("data-bg-decor");
        } else {
            let _ = root.set_attribute("data-bg-decor", "plain");
        }
    }
}

#[must_use]
pub(crate) fn read_agent_role_initial() -> Option<String> {
    None
}

/// Tauri 壳标记与 IDE 布局标记（`data-tauri-shell` / `data-ide-layout`：窗口装饰与菜单栏拖拽，不隐藏 Web 顶栏）。
pub(crate) fn apply_shell_layout_dom_flags(editor_layout_mode: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(root) = doc.document_element() else {
        return;
    };
    if crate::tauri_shell::tauri_shell_available() {
        let _ = root.set_attribute("data-tauri-shell", "");
    } else {
        let _ = root.remove_attribute("data-tauri-shell");
    }
    if editor_layout_mode {
        let _ = root.set_attribute("data-ide-layout", "");
    } else {
        let _ = root.remove_attribute("data-ide-layout");
    }
}

/// 窄屏布局标记（`data-narrow-viewport`：供 `mobile.css` 与状态栏收敛选择器使用）。
pub(crate) fn apply_narrow_viewport_dom_flag(narrow: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(root) = doc.document_element() else {
        return;
    };
    if narrow {
        let _ = root.set_attribute("data-narrow-viewport", "");
    } else {
        let _ = root.remove_attribute("data-narrow-viewport");
    }
}

#[cfg(test)]
mod platform_shell_ui_tests {
    use super::{
        narrow_or_mobile_shell_layout, platform_side_panel_on_entry,
        shell_layout_prefs_sync_to_server,
    };
    use crate::app_prefs::SidePanelView;

    #[test]
    fn mobile_or_narrow_forces_collapsed() {
        assert_eq!(
            platform_side_panel_on_entry(true, SidePanelView::Workspace),
            SidePanelView::None
        );
        assert_eq!(
            platform_side_panel_on_entry(true, SidePanelView::Tasks),
            SidePanelView::None
        );
    }

    #[test]
    fn wide_viewport_expands_hidden_prefs() {
        assert_eq!(
            platform_side_panel_on_entry(false, SidePanelView::None),
            SidePanelView::Workspace
        );
        assert_eq!(
            platform_side_panel_on_entry(false, SidePanelView::Tasks),
            SidePanelView::Tasks
        );
        assert_eq!(
            platform_side_panel_on_entry(false, SidePanelView::DebugConsole),
            SidePanelView::DebugConsole
        );
    }

    #[test]
    fn narrow_or_mobile_skips_server_layout_sync() {
        assert!(!shell_layout_prefs_sync_to_server(true));
        assert!(shell_layout_prefs_sync_to_server(false));
        assert!(!narrow_or_mobile_shell_layout(false));
        assert!(narrow_or_mobile_shell_layout(true));
    }
}

#[cfg(test)]
mod theme_pref_mirror_tests {
    use super::{FIRST_FRAME_THEME_BASELINE, theme_pref_from_mirror};

    #[test]
    fn missing_or_blank_mirror_falls_back_to_dark_baseline() {
        assert_eq!(theme_pref_from_mirror(None), FIRST_FRAME_THEME_BASELINE);
        assert_eq!(theme_pref_from_mirror(Some("")), FIRST_FRAME_THEME_BASELINE);
        assert_eq!(
            theme_pref_from_mirror(Some("  \n\t ")),
            FIRST_FRAME_THEME_BASELINE
        );
    }

    #[test]
    fn unknown_mirror_falls_back_to_dark_baseline_not_default_theme() {
        // 关键：非法值必须回落深色基线，而不是 `normalize_theme_slug` 的默认主题 `shadcn-light`，
        // 否则清过存储的深色用户会先闪一帧浅色。
        for raw in [
            "light",
            "dark",
            "material",
            "shadcn",
            "unknown-theme",
            "crabmate-light ",
        ] {
            let want = if raw.trim() == "crabmate-light" {
                "crabmate-light"
            } else {
                FIRST_FRAME_THEME_BASELINE
            };
            assert_eq!(theme_pref_from_mirror(Some(raw)), want, "raw {raw:?}");
        }
    }

    #[test]
    fn system_pref_is_preserved_for_later_resolution() {
        // `system` 必须原样保留：写死解析结果会把「跟随系统」静默改成显式主题。
        assert_eq!(theme_pref_from_mirror(Some("system")), "system");
        assert_eq!(theme_pref_from_mirror(Some(" system \n")), "system");
    }

    #[test]
    fn accepted_prefs_are_preserved_verbatim() {
        for slug in [
            "crabmate-dark",
            "crabmate-light",
            "material-dark",
            "high-contrast-dark",
            "shadcn-dark",
            "shadcn-light",
        ] {
            assert_eq!(theme_pref_from_mirror(Some(slug)), slug);
        }
    }
}
