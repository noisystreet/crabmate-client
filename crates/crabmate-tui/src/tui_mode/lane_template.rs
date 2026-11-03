//! lane 的运行期装配：模板 + `LaneApp::assemble` / `bootstrap`。
//!
//! 从 `shell.rs` 拆出：那边顶着 920 行/文件的 `fn-nloc-ratchet` 上限，而这里的两件事
//! 只跟「新 lane 从哪份初值开始」有关，与终端外壳（raw mode / 备用屏幕 / 事件循环）
//! 无关。字段清单只此一份：启动装配与 `/lane attach|new` 共用同一个模板。

use std::borrow::Cow;
use std::sync::mpsc;

use crate::SessionPrefs;
use crate::serve::{BearerSource, ServeClient};

// 本模块挂在 `shell` 下（`#[path]` 声明也在那边），所以 tui_mode 里的兄弟模块要走两级。
use super::super::inbound::Inbound;
use super::super::pane_store;
use super::super::pane_width::PaneWidths;
use super::super::state::UiState;
use super::super::worker::{JobCancel, LaneId, WorkerJob};
use super::super::{LaneApp, body_cache, drafts, session_index, worker_ctl};

/// 运行期新增 lane 的装配模板（`/lane attach` / `/lane new`）。
///
/// 存**启动时的**表达层初值，而不是「当前 lane 的现值」：当前 lane 的 `/model`
/// 覆盖只属于它，复制给新 lane 等于把 A 的临时选择静默带进 B。
///
/// 定义在这里而不是 `mod.rs`：那边贴着 920 行门禁，而它只被外壳（装配 + 新增 lane）
/// 用到。
pub(crate) struct LaneTemplate {
    /// 启动时的 `/model` `/mode` `/role` `/perm` 初值（CLI / env）。
    pub(crate) overrides: SessionPrefs,
    /// `--yes`：审批自动放行。
    pub(crate) yes: bool,
    /// `--no-keyring`：设置面板禁写密钥。
    pub(crate) no_keyring: bool,
    /// 两栏列宽的初始值（本机落盘读到的，见 `pane_store`）；每条 lane 装配时各抄一份。
    ///
    /// 放进模板而不是让 `assemble` 自己读盘：`/lane attach|new` 在事件循环里跑，
    /// 每新增一条 lane 读一次文件没有意义，而「读到的值」与「启动时的值」也不该不同。
    pub(crate) pane: PaneWidths,
    /// 列宽落到哪个文件（同一次读盘解析出来的目的地，随 lane 持有）。
    pub(crate) pane_store: pane_store::PaneStore,
    /// Web API Bearer：控制面里所有实例同机同用户，第一版共用主连接那份
    /// （per-profile 槽需要牵动 Desktop / Android 的槽名约定，见设计稿 §4.5）。
    pub(crate) bearer: String,
    /// 上面那份 Bearer 的来源（`--bearer` / 钥匙串 / 未提供）：新 lane 的
    /// `ConnectionConfig` 与主连接同一个来源，设置面板才能如实回答「带了没」。
    pub(crate) bearer_source: BearerSource,
    /// 外壳级控制面通道（各 lane 相同：一个 socket 管所有实例）。
    /// 存通道句柄而不是 socket：socket 只由 `worker_ctl` 的线程持有，调用也在那里串行
    /// 发生（issue #350）。`None` = 没启用控制面（`/ctl` 会说明怎么启用）。
    pub(crate) ctl: Option<worker_ctl::CtlHandle>,
    /// 所有 lane 共用的事件通道发送端。
    pub(crate) tx: mpsc::Sender<Inbound>,
}

/// lane 的装配与首拉：启动装配与 `/lane attach|new` 共用。
///
/// 字段清单只此一份：两处各写一遍的话，加字段时必有一边漏掉，而漏的那条 lane
/// 会静默缺一项行为（例如没有 `ctl` 就连 `/lane` 都用不了）。
impl<'a> LaneApp<'a> {
    /// 按模板装配一条 lane（worker 由调用方先 spawn）。`pub(super)` 只为测试：`test_support` 直接构造会绕过它，而「落盘宽度真的进了 `st.pane`」正是 P2 的承诺（本文件贴着 920 行门禁，说明写在这里）。
    pub(crate) fn assemble(
        id: LaneId,
        label: String,
        client: Cow<'a, ServeClient>,
        template: &LaneTemplate,
        job_tx: mpsc::Sender<WorkerJob>,
        job_cancel: JobCancel,
    ) -> Self {
        Self {
            id,
            label,
            client,
            overrides: template.overrides.clone(),
            yes: template.yes,
            no_keyring: template.no_keyring,
            st: UiState::new().with_pane(template.pane),
            drafts: drafts::Drafts::default(),
            tx: template.tx.clone(),
            job_tx,
            job_cancel,
            cancel: None,
            exit: false,
            quit_pending: false,
            body: body_cache::BodyCache::default(),
            panel: None,
            persisted: None,
            last_size: (0, 0),
            pane_drag: None,
            ws_click: Default::default(),
            // 装配时的宽度就是「已经保存过的值」：它来自读盘（或默认），没拖过就不该写文件。
            // 夹取与 `with_pane` 同一条，否则「刚启动就已经和磁盘不一致」会立刻触发一次写。
            pane_saved: template.pane.clamped_to_ranges(),
            pane_save_warned: false,
            pane_store: template.pane_store.clone(),
            index: session_index::UiIndexState::default(),
            resume: None,
            sent_turn: None,
            bg: None,
            ctl: template.ctl.clone(),
            session_refresh_pending: false,
            unread: false,
            turn_started: None,
        }
    }

    /// 首次拉取：会话 / 工作区根 / serve 默认状态 / user-data 设置快照。
    ///
    /// 新增 lane 也要跑这一套：只刷当前 lane 的话，切过去会看到空的会话列表。
    pub(crate) fn bootstrap(&mut self) {
        self.refresh_sessions();
        self.st.ws_begin_root_fetch();
        self.refresh_workspace(false);
        self.refresh_status();
        self.load_user_settings();
    }
}
