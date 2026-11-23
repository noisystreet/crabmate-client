//! [`LaneApp::status_info`]：把这条 lane 的实况 + override + serve 默认收敛成一份显示值。
//!
//! 单独成文件的两个理由：
//!
//! 1. `mod.rs` 贴着 920 行的文件门禁 —— 这段本来就该让出去，而不是为它抬门禁；
//! 2. 它是**纯取值**：只读 `st` / `overrides` / `persisted` / `serve_defaults`，不碰事件、
//!    不碰渲染，与 `permission_intent` / `status_thinking` 同一手法（`impl LaneApp` 扩展块）。
//!
//! 「值从哪来」的优先级（override ＞ user-data 已存值 ＞ serve 默认）是本模块唯一的规则，
//! 两端口径见 `normalize` 与 `permission_intent::perm_status_values`；`perm` 段的最终显示值
//! 由本模块的 [`perm_value`] 给出（与 Desktop chip 共用共享层同一函数）。

use super::render::StatusInfo;
use super::settings::normalize;
use super::{LaneApp, spinner, status, status_thinking};

impl LaneApp<'_> {
    /// 状态行显示值：override（`*`）＞ user-data 已存值 ＞ serve 默认。
    pub(super) fn status_info(&self) -> StatusInfo {
        let overrides = &self.overrides;
        let stored = self.persisted.as_ref();
        let defaults = self.st.serve_defaults.as_ref();
        // `perm` 段两侧：口径（含「无已绑定会话就不并列」）见 `perm_status_values`。
        let (perm_effective, perm_intent) = self.perm_status_values();
        let show =
            |local: &Option<String>, stored: Option<&str>, remote: Option<&str>| match normalize(
                local,
            ) {
                Some(v) => Some(format!("{v}*")),
                None => match stored.map(str::trim).filter(|s| !s.is_empty()) {
                    Some(v) => Some(v.to_string()),
                    None => remote
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                },
            };
        StatusInfo {
            // 视图归本 lane：`UiState` 自带 `editor`，外壳不需要覆盖（切 lane 就是换一份
            // `st`），所以这里直接读实况。
            editor_open: self.st.editor.open,
            api_base: self.client.config().api_base.clone(),
            // 上下文用量：段恒显示；没有实测时是占位 `—`，新会话则回落 `GET /status` 的
            // system 基线并带 `~`（见 `UiState::context_text`）。
            context: self.st.context_text(
                stored.and_then(|p| p.context_tokens.as_deref()),
                defaults
                    .and_then(|d| d.baseline_prompt_tokens(stored.and_then(|p| p.role.as_deref()))),
            ),
            model: show(
                &overrides.model,
                stored.and_then(|p| p.model.as_deref()),
                defaults.and_then(|d| d.model.as_deref()),
            ),
            role: show(
                &overrides.agent_role,
                stored.and_then(|p| p.role.as_deref()),
                defaults.and_then(|d| d.role.as_deref()),
            ),
            mode: show(
                &overrides.session_mode,
                stored.and_then(|p| p.session_mode.as_deref()),
                defaults.and_then(|d| d.mode.as_deref()),
            ),
            // 思考模式：`think` 段是**开关**（点一下就切，见 `status_thinking`），取值归一到
            // `开` / `关` 再交给渲染层 —— 判定口径与 Desktop 状态栏那颗 chip 是**同一条**
            // （只有 `on` 算开，`server` 也算关）。user-data 到手后段恒显（没存值 = 跟随
            // server = 关），否则开关会时隐时现；还没拉到才是 `None` = 不画段。
            thinking: status_thinking::switch_label(
                self.overrides.thinking.as_deref(),
                self.persisted.as_ref(),
            ),
            running: self.st.running,
            cancel_sent: self.st.cancel_sent,
            // 权限等级随 `ServeDefaults` 一起来（worker 同任务拉 `/permission`）；两侧怎么取
            // 见 `perm_status_values`，不同时渲染成 `意图→生效`（与 Desktop chip 同口径）。
            permission: perm_effective,
            permission_intent: perm_intent,
            permission_pending: self.st.permission_pending,
            spinner: spinner::frame_at(self.turn_elapsed()),
            view_offset: self.st.view_offset,
            search_term: self.st.search_term().map(str::to_string),
            search_total: self.st.search_total,
            // 窄屏工作区指示（issue #425）：左右栏一藏，右栏标题里的工作区名就没了。取值与
            // 「什么情况下才显示」都在 `status::narrow_workspace_text`，这里只传实况；
            // git 分支也一并传进去，括注成 `client(main ●3)`（不再单独成段）。
            workspace: status::narrow_workspace_text(
                self.st.sidebar_visible,
                self.st.workspace_path.as_deref(),
                defaults.and_then(|d| d.git_branch.as_deref()),
                defaults.and_then(|d| d.git_dirty.as_deref()),
            ),
        }
    }
}

/// 状态行 `perm` 段显示什么。`None` = 整段不显示。
///
/// 与 Desktop `chip_text`（`frontend/src/app/status_permission_chip.rs`）同一口径，
/// 只去掉本地化标签（TUI 一贯显示线格式）：
/// - 意图与生效相同 → 单值；
/// - 不同 → `意图→生效`（服务端允许意图高于生效，两个都要显示，否则用户以为切换没生效）；
/// - 没有意图 → 生效档；
/// - 两者都不知道 → 不显示（**不**拿 `permission_level_default` 顶替：那是「没意图时会
///   落到哪」，不是「此刻是哪」）。
///
/// `effective` 该传什么由 [`super::permission_intent::status_values`] 决定 —— 不是所有
/// `GET /permission` 的读数都能当「此刻生效」；`pending` 由写入路径给（意图已提交、
/// 尚未确认）。两者与 Desktop chip 共用共享层 `perm_display_with`。
#[must_use]
pub(super) fn perm_value(
    intent: Option<&str>,
    effective: Option<&str>,
    pending: bool,
) -> Option<String> {
    crabmate_client_api::permission::perm_display_with(true, intent, effective, pending)
        .map(|d| d.plain())
}
