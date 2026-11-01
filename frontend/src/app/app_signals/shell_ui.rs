//! 壳 UI：主题、语言、侧栏视图、Web UI 展示偏好等。

use leptos::prelude::*;

use crate::app::ide_menu_bar::IdeMenuBarBridge;
use crate::app::shell_prefs_storage;
use crate::app_prefs::SidePanelView;
use crate::i18n::Locale;

#[derive(Clone, Copy)]
pub struct ShellUISignals {
    pub theme: RwSignal<String>,
    pub bg_decor: RwSignal<bool>,
    /// 聊天主列是否展示本轮注入 / 窗口裁剪 `timeline_log`（默认关；经 `/user-data/prefs` 持久化）。
    pub show_turn_context_inject: RwSignal<bool>,
    pub locale: RwSignal<Locale>,
    pub view_menu_open: RwSignal<bool>,
    /// 顶栏「项目 / 编辑 / 视图」任一下拉打开时为 `true`（对话与 IDE 共用；供全局 Escape 关闭）。
    pub ide_menubar_dropdown_open: RwSignal<bool>,
    /// IDE 布局挂载后由 [`IdeLayoutView`](crate::app::ide_layout::IdeLayoutView) 写入，供统一顶栏渲染菜单。
    pub ide_menu_bar_bridge: RwSignal<Option<IdeMenuBarBridge>>,
    pub side_panel_view: RwSignal<SidePanelView>,
    pub side_width: RwSignal<f64>,
    pub web_ui_config_loaded: RwSignal<bool>,
    pub markdown_render: RwSignal<bool>,
    pub apply_assistant_display_filters: RwSignal<bool>,
    /// `true` 时主区为 IDE 式（工作区树 + 编辑器），隐藏对话列与右列。
    pub editor_layout_mode: RwSignal<bool>,
    /// 递增时 IDE 布局保存当前活动标签（`Ctrl/Cmd+S`）。
    pub ide_save_active_nonce: RwSignal<u64>,
    /// 递增时 IDE 布局保存全部脏标签（`Ctrl/Cmd+Shift+S`）。
    pub ide_save_all_nonce: RwSignal<u64>,
    /// 递增时 IDE 布局从磁盘重载已打开文件（`workspace_changed` 等）。
    pub ide_sync_disk_nonce: RwSignal<u64>,
    /// 递增时 IDE 布局打开 [`Self::ide_open_file_path`] 指向的工作区相对文件（工具卡「打开此文件」）。
    pub ide_open_file_nonce: RwSignal<u64>,
    /// 与 [`Self::ide_open_file_nonce`] 配对的目标路径；消费后清空，防布局切换重放。
    pub ide_open_file_path: RwSignal<Option<String>>,
    /// 设置中保存/清除 Web API Bearer 后递增，触发 `/status` 等恢复拉取。
    pub web_api_bearer_save_nonce: RwSignal<u64>,
    /// Device Flow 授权成功或断开后递增，触发侧栏 `GET /github/repo-context` 刷新。
    pub github_auth_refresh_nonce: RwSignal<u64>,
    /// 会话模式壳层 UI 字体 slug（`default` 表示随主题 `--font-sans`）。
    pub session_ui_font: RwSignal<String>,
    /// 聊天列消息与输入框正文字体 slug（`code`/`pre` 仍用 `--font-mono`）。
    pub session_chat_font: RwSignal<String>,
    /// 聊天列消息与输入框正文字号（px）。
    pub session_chat_font_size: RwSignal<f64>,
    /// 视口宽度 ≤ [`crate::app_prefs::MOBILE_LAYOUT_BREAKPOINT_PX`]（由 `matchMedia` 维护）。
    pub is_narrow_viewport: RwSignal<bool>,
    /// 当前这一屏连的 `serve` 基址（[`api_base_url`](crate::api::api_base_url) 的**响应式镜像**）。
    ///
    /// 基址的唯一真源仍是 `crate::api::browser` 的 thread_local（所有请求都读那里）；
    /// 这里只是让界面能在「设置 → 连接」改完基址后立刻换字。两处各持一份**独立**状态
    /// 就会得到「改了基址、状态栈还写着旧 host」——那恰好是这个字段要回答的问题。
    ///
    /// 写入面因此只有一处：`save_api_base_click`（`settings_sections.rs`）在
    /// `set_api_base_url` 之后写本字段。启动交接（URL hash）**不需要**额外写 ——
    /// `bootstrap_app_shell` 在 [`AppSignals::new`](super::AppSignals::new) 之前就消费完它，
    /// 所以下面 `new()` 里读到的 thread_local 已经是终值，不必先空后闪。
    pub api_base: RwSignal<String>,
}

impl ShellUISignals {
    pub fn new() -> Self {
        let s = shell_prefs_storage::read_shell_ui_initial_snapshot();
        Self {
            theme: RwSignal::new(s.theme),
            bg_decor: RwSignal::new(s.bg_decor),
            show_turn_context_inject: RwSignal::new(s.show_turn_context_inject),
            locale: RwSignal::new(s.locale),
            view_menu_open: RwSignal::new(false),
            ide_menubar_dropdown_open: RwSignal::new(false),
            ide_menu_bar_bridge: RwSignal::new(None),
            side_panel_view: RwSignal::new(s.side_panel_view),
            side_width: RwSignal::new(s.side_width),
            web_ui_config_loaded: RwSignal::new(false),
            markdown_render: RwSignal::new(true),
            apply_assistant_display_filters: RwSignal::new(true),
            editor_layout_mode: RwSignal::new(s.editor_layout_mode),
            ide_save_active_nonce: RwSignal::new(0),
            ide_save_all_nonce: RwSignal::new(0),
            ide_sync_disk_nonce: RwSignal::new(0),
            ide_open_file_nonce: RwSignal::new(0),
            ide_open_file_path: RwSignal::new(None),
            web_api_bearer_save_nonce: RwSignal::new(0),
            github_auth_refresh_nonce: RwSignal::new(0),
            session_ui_font: RwSignal::new(s.session_ui_font),
            session_chat_font: RwSignal::new(s.session_chat_font),
            session_chat_font_size: RwSignal::new(s.session_chat_font_size),
            is_narrow_viewport: RwSignal::new(false),
            // 启动初值直接取 thread_local：连接交接（URL hash）在挂载前就写完了，
            // 若这里给空串，首个「连接」chip 会先闪一下空态再出现。
            api_base: RwSignal::new(crate::api::api_base_url()),
        }
    }
}

impl Default for ShellUISignals {
    fn default() -> Self {
        Self::new()
    }
}
