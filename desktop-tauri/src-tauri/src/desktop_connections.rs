//! 多窗口连接的注册表与跨窗口广播。
//!
//! 为什么需要这一层：Desktop 的多连接走「每窗口一条连接」（见 #391），每个窗口是独立
//! 的 WebView / JS heap，所以窗口 A **看不见**窗口 B 存在。而「点连接列表切到那个窗口」
//! 要求先看见 —— 于是必须有一个进程内的协调者。它只能在这里（`desktop-tauri`）：
//! Tauri 的窗口 API 与 IPC 都属于平台能力，放进 `crabmate-client-api` 会被纯度门禁拒掉。
//!
//! 数据形状全部来自 `crabmate-client-api`：`ConnectionSnapshot` / `ConnectionStatus` /
//! `ConnectionTally`。这一层只负责「谁在、谁是哪条、广播出去」，不定义字段语义。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use crabmate_client_api::{ConnectionKey, ConnectionSnapshot, ConnectionStatus, ConnectionTally};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_notification::NotificationExt;

/// 名单变化的广播事件名。
///
/// 所有窗口都订阅它；接收方拿 `key` 与自己的比对，就知道哪条是「我」。
pub(crate) const CONNECTIONS_CHANGED: &str = "connections_changed";

/// 撞名时尝试的后缀上限。
const MAX_DEDUPE_SUFFIX: usize = 99;

/// `window label → 该窗口的连接快照`。
///
/// key 用 label 而不是 `ConnectionKey`：两者同值（见 `ConnectionKey` 的 doc），但 label 是
/// 平台身份、`key` 是契约身份 —— 用 label 做键，删除窗口时不必再从快照里反查。
#[derive(Debug, Default)]
pub(crate) struct ConnectionRegistry(Mutex<HashMap<String, ConnectionSnapshot>>);

impl ConnectionRegistry {
    /// 取锁；被 poison 时继续用里面的数据。
    ///
    /// 这里不 `unwrap`：注册表是 UI 的旁路数据，一次 panic 让整个应用的连接列表永久
    /// 不可用，比读到一份可能稍旧的名单糟得多。
    fn entries(&self) -> MutexGuard<'_, HashMap<String, ConnectionSnapshot>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 登记或更新一个窗口的连接。
    pub(crate) fn upsert(&self, label: &str, snapshot: ConnectionSnapshot) {
        self.entries().insert(label.to_string(), snapshot);
    }

    /// 窗口关掉了。
    pub(crate) fn remove(&self, label: &str) {
        self.entries().remove(label);
    }

    /// 分配一个与已有连接不撞的 key。
    ///
    /// `ConnectionKey::derive` 只保证「同样输入给同样输出」，**唯一性归这里** ——
    /// 这是注册表存在的理由之一：它知道已有集合，而纯函数不知道。两个窗口连同一台
    /// serve 会得到同一个 key，那时依次尝试 `-2`、`-3`……
    pub(crate) fn unique_key(&self, preferred: Option<&str>, api_base: &str) -> ConnectionKey {
        let base = ConnectionKey::derive(preferred, api_base);
        let guard = self.entries();
        if !guard.contains_key(base.as_str()) {
            return base;
        }
        for suffix in 2..=MAX_DEDUPE_SUFFIX {
            let candidate = ConnectionKey::from_raw(&format!("{}-{suffix}", base.as_str()));
            if !guard.contains_key(candidate.as_str()) {
                return candidate;
            }
        }
        // 99 条同基址还没撞开：退到长度兜底（不会与上面那些撞，因为都带 `-2..=99`）。
        ConnectionKey::from_raw(&format!("{}-{}", base.as_str(), guard.len() + 1))
    }

    /// 全部连接，按 key 排序（UI 顺序稳定）。
    ///
    /// `active` 一律 `false`：广播是给**所有**窗口的，同一份 payload 不可能对每个窗口
    /// 都对 —— 谁是自己那条由接收方比对 key 决定。在这里填只会让每个窗口都看到错的。
    /// （返回值那条路径是另一件事：`open_connection` / `register_connection` 回给调用方
    /// 的是「**你自己**在运行」，所以那里是 `true`。两句话，不是两套口径。）
    pub(crate) fn snapshots(&self) -> Vec<ConnectionSnapshot> {
        let mut out: Vec<ConnectionSnapshot> = self
            .entries()
            .values()
            .map(|snapshot| ConnectionSnapshot {
                active: false,
                ..snapshot.clone()
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    pub(crate) fn tally(&self) -> ConnectionTally {
        let statuses: Vec<ConnectionStatus> = self
            .entries()
            .values()
            .map(|snapshot| snapshot.status)
            .collect();
        ConnectionTally::from_statuses(statuses)
    }

    /// 改某条连接的状态；返回**旧状态**（`None` = 没有这一条）。
    ///
    /// 返回旧状态而不是 `bool`：调用方要靠「刚进入等审批」这个**边沿**决定发不发通知。
    /// 一直等审批时重复上报不该反复弹，而「当前是不是等审批」无法区分「刚进入」。
    pub(crate) fn set_status(
        &self,
        label: &str,
        status: ConnectionStatus,
    ) -> Option<ConnectionStatus> {
        self.entries().get_mut(label).map(|snapshot| {
            let previous = snapshot.status;
            snapshot.status = status;
            previous
        })
    }

    /// 读某条连接的快照（发通知时要用显示名与基址）。
    pub(crate) fn snapshot(&self, label: &str) -> Option<ConnectionSnapshot> {
        self.entries().get(label).cloned()
    }

    /// 改未读标记。
    pub(crate) fn set_unread(&self, label: &str, unread: bool) -> bool {
        self.entries().get_mut(label).is_some_and(|snapshot| {
            snapshot.unread = unread;
            true
        })
    }

    /// 改名（只动显示名，`key` 与 window label 不变）。
    pub(crate) fn rename(&self, label: &str, name: &str) -> bool {
        self.entries().get_mut(label).is_some_and(|snapshot| {
            snapshot.rename(name);
            true
        })
    }
}

fn registry(app: &AppHandle) -> Option<tauri::State<'_, ConnectionRegistry>> {
    app.try_state::<ConnectionRegistry>()
}

/// 把最新名单推给所有窗口。
pub(crate) fn broadcast(app: &AppHandle) {
    let Some(state) = registry(app) else {
        return;
    };
    let payload = state.snapshots();
    if let Err(error) = app.emit(CONNECTIONS_CHANGED, payload) {
        eprintln!("[crabmate-desktop] connections broadcast failed: {error}");
    }
}

/// 某条连接进入等审批、而它的窗口不在前台时，发一条系统通知。
///
/// 为什么需要：连接窗口是「铺满工作区」的（`apply_main_ui_geometry`），后开的窗口会
/// 完全盖住先开的 —— 被盖住的那个即使弹了审批框，用户也看不见它。TUI 侧是同一个问题
/// （#337，后台 lane 的审批画不到主屏），那边靠左栏的 `▲` 列解决；Desktop 的窗口会被
/// 整个遮住，所以必须有能跳出窗口的提醒。
///
/// 通知失败只记日志：Linux 上可能没有通知守护进程，而「审批本身」不该因此受影响。
fn notify_approval(app: &AppHandle, snapshot: &ConnectionSnapshot) {
    let body = if snapshot.authority.is_empty() {
        String::from("有一条审批在等你的决定")
    } else {
        format!("{} · {}", snapshot.display_name, snapshot.authority)
    };
    if let Err(error) = app
        .notification()
        .builder()
        .title("CrabMate：等待审批")
        .body(body)
        .show()
    {
        eprintln!("[crabmate-desktop] approval notification failed: {error}");
    }
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 当前所有连接（前端首帧用它补齐，之后靠 `connections_changed` 增量刷新）。
#[tauri::command]
pub(crate) fn list_connections(app: AppHandle) -> Vec<ConnectionSnapshot> {
    registry(&app).map_or_else(Vec::new, |state| state.snapshots())
}

/// 总连接数 / 等审批数（与 TUI 的 `连接 [n]`、`▲N` 同一份计数）。
#[tauri::command]
pub(crate) fn connections_tally(app: AppHandle) -> ConnectionTally {
    registry(&app).map_or_else(ConnectionTally::default, |state| state.tally())
}

/// 窗口报告「我连上了谁」。
///
/// 由业务 UI 在连接成功后调用（拿得到 `api_base` 与用户取的名字）。
///
/// `key` **恒等于 window label**（`ConnectionKey::from_raw` 只做 slug 化，label 本身
/// 已经是合规的）。这条等式是整套协调面的地基：`focus_connection`、`report_connection_status`、
/// `rename_connection`、窗口 `Destroyed` 清理全部按 label 查。窗口自报时 label 早已确定，
/// **不需要**再分配一次 key —— 再分配只会得到一个与 label 不同的值，于是
/// `focus_connection(key)` 永远找不到那个窗口。
#[tauri::command]
pub(crate) fn register_connection(
    app: AppHandle,
    window: WebviewWindow,
    api_base: String,
    display_name: Option<String>,
) -> Result<ConnectionSnapshot, String> {
    let state = registry(&app).ok_or_else(|| "连接注册表未就绪".to_string())?;
    let base = crabmate_connect::normalize_base_url(&api_base)?;
    let mut snapshot = ConnectionSnapshot::new(display_name.unwrap_or_default(), base.as_str());
    snapshot.key = ConnectionKey::from_raw(window.label());
    snapshot.active = true;
    state.upsert(window.label(), snapshot.clone());
    broadcast(&app);
    Ok(snapshot)
}

/// 开一条新连接（新窗口）。
///
/// 几何走「覆盖」：连接窗口都铺满工作区（`apply_main_ui_geometry`），后开的自然叠在
/// 先开的上面 —— 视觉上像切标签，实现上仍是各自独立的 WebView。这正是选多窗口的理由：
/// 不必给前端的 `thread_local` 做分区。
///
/// 覆盖也带来一个后果：被盖住的窗口看不见自己的审批框，所以 `report_connection_status`
/// 会在它进入等审批时发系统通知（见 `notify_approval`）。
#[tauri::command]
pub(crate) fn open_connection(
    app: AppHandle,
    api_base: String,
    display_name: Option<String>,
    bearer: Option<String>,
) -> Result<ConnectionSnapshot, String> {
    let state = registry(&app).ok_or_else(|| "连接注册表未就绪".to_string())?;
    let base = crabmate_connect::normalize_base_url(&api_base)?;
    let preferred = display_name;
    let key = state.unique_key(preferred.as_deref(), base.as_str());

    // 先**占位**再建窗。`unique_key` 返回时锁已经释放，两个并发的 `open_connection`
    // （用户在两个窗口里同时点「新连接」）会拿到同一个 key，而
    // `WebviewWindowBuilder::new` 遇到重名 label 会失败 —— 表现为「偶尔开不出来」。
    // 占位让第二个请求在 `unique_key` 里就看见它已存在，从而换到 `-2`。
    // 占位期间不广播，所以用户在列表里看不到这一条。
    let mut snapshot =
        ConnectionSnapshot::new(preferred.clone().unwrap_or_default(), base.as_str());
    snapshot.key = key.clone();
    state.upsert(key.as_str(), snapshot.clone());

    if let Err(error) = crate::desktop_main_window::create_connection_window(
        &app,
        key.as_str(),
        &base,
        preferred.as_deref().unwrap_or_default(),
        bearer.as_deref().unwrap_or_default(),
    ) {
        // 建窗失败：撤掉占位，别在列表里留一条打不开的连接。
        state.remove(key.as_str());
        return Err(error);
    }

    snapshot.active = true;
    // 注册表的键是 window label，而这里 label 就是 key（`create_connection_window`
    // 用的同一个值），所以两者一致。
    state.upsert(snapshot.key.as_str(), snapshot.clone());
    broadcast(&app);
    Ok(snapshot)
}

/// 开一条**待连接**窗口：只载连接页，连哪台 `serve` 由用户在新窗口里选。
///
/// 与 [`open_connection`] 的分工：那条已经有 `api_base`，建完直接交接进业务 UI；这条把
/// 选址交给连接页。设置页的按钮因此不必先读一个 draft 基址，也就不会和旁边「保存基址」
/// （改**当前窗口**连谁）的语义打架。
///
/// 连接成功后由业务 UI 的 `StatusConnectionChip` 调 `register_connection` 自报，
/// 那条占位快照随之被真实 `authority` 覆盖（同一个 window label）。在此之前它在名单里是
/// **空基址**的一条 —— chip 的单条分支对空基址整段不渲染，多连接列表里则显示为无名行。
///
/// 占位仍然必须先做：`unique_key` 只看注册表，不占位时连点两次会拿到同一个 label，
/// 而 `WebviewWindowBuilder` 遇重名 label 直接失败（与 [`open_connection`] 同一理由）。
#[tauri::command]
pub(crate) fn open_connect_window(
    app: AppHandle,
    display_name: Option<String>,
) -> Result<ConnectionSnapshot, String> {
    let state = registry(&app).ok_or_else(|| "连接注册表未就绪".to_string())?;
    let name = display_name.unwrap_or_default();
    // 空基址：`ConnectionKey::derive` 退到 `FALLBACK_KEY`，`unique_key` 在其上加序号。
    let key = state.unique_key(Some(name.as_str()), "");
    let mut snapshot = ConnectionSnapshot::new(name, "");
    snapshot.key = key.clone();
    state.upsert(key.as_str(), snapshot.clone());

    if let Err(error) = crate::desktop_main_window::create_pending_connection_window(
        &app,
        key.as_str(),
        &snapshot.display_name,
    ) {
        // 建窗失败：撤掉占位，别在列表里留一条打不开的连接。
        state.remove(key.as_str());
        return Err(error);
    }

    snapshot.active = true;
    state.upsert(snapshot.key.as_str(), snapshot.clone());
    broadcast(&app);
    Ok(snapshot)
}

/// 窗口报告状态变化（回合开始 / 等审批 / 空闲）。
///
/// 进入等审批时会按需发系统通知 —— 见 [`notify_approval`]。
#[tauri::command]
pub(crate) fn report_connection_status(
    app: AppHandle,
    window: WebviewWindow,
    status: ConnectionStatus,
) {
    let Some(state) = registry(&app) else {
        return;
    };
    let Some(previous) = state.set_status(window.label(), status) else {
        return;
    };
    broadcast(&app);

    // 只在**刚进入**等审批、且那个窗口不在前台时通知：
    // - 不在前台才需要提醒（用户正看着它的时候，审批框就在眼前，弹通知只是噪音）；
    // - 靠 `previous` 判边沿，一直等审批时重复上报不会反复弹。
    let entered_approval = status == ConnectionStatus::WaitingApproval
        && previous != ConnectionStatus::WaitingApproval;
    if entered_approval
        && !window.is_focused().unwrap_or(false)
        && let Some(snapshot) = state.snapshot(window.label())
    {
        notify_approval(&app, &snapshot);
    }
}

/// 窗口报告未读变化（后台产生了新输出）。
#[tauri::command]
pub(crate) fn report_connection_unread(app: AppHandle, window: WebviewWindow, unread: bool) {
    if let Some(state) = registry(&app)
        && state.set_unread(window.label(), unread)
    {
        broadcast(&app);
    }
}

/// 改名：只动显示名，窗口 label 与 key 不变（见 `ConnectionSnapshot::rename`）。
#[tauri::command]
pub(crate) fn rename_connection(app: AppHandle, window: WebviewWindow, name: String) {
    if let Some(state) = registry(&app)
        && state.rename(window.label(), &name)
    {
        broadcast(&app);
    }
}

/// 切到某个连接对应的窗口。
///
/// 复用 `desktop_lifecycle::show_and_focus`（单实例激活时用的同一个函数）：最小化过的
/// 窗口要先 `show` 再 `unminimize` 再 `set_focus`，少一步在部分窗口管理器上就没反应。
#[tauri::command]
pub(crate) fn focus_connection(app: AppHandle, key: String) -> Result<(), String> {
    let key = ConnectionKey::from_raw(&key);
    let window = app
        .get_webview_window(key.as_str())
        .ok_or_else(|| format!("没有这个连接: {key}"))?;
    crate::desktop_lifecycle::show_and_focus(&window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(name: &str, base: &str) -> ConnectionSnapshot {
        ConnectionSnapshot::new(name, base)
    }

    #[test]
    fn colliding_keys_get_a_suffix_instead_of_overwriting() {
        let registry = ConnectionRegistry::default();
        let first = registry.unique_key(Some("prod"), "http://127.0.0.1:8080");
        registry.upsert(first.as_str(), snapshot("prod", "http://127.0.0.1:8080"));
        // 第二个窗口连同一台 serve：derive 会给出同一个 key，注册表必须换一个。
        let second = registry.unique_key(Some("prod"), "http://127.0.0.1:8080");
        assert_ne!(first, second);
        assert_eq!(second.as_str(), "prod-2");
    }

    #[test]
    fn snapshots_are_sorted_and_never_mark_a_viewer_active() {
        let registry = ConnectionRegistry::default();
        registry.upsert("win-b", snapshot("b", "http://b:2"));
        registry.upsert("win-a", snapshot("a", "http://a:1"));

        let list = registry.snapshots();
        assert_eq!(list.len(), 2);
        // 排序稳定（按 key），否则列表会在每次广播时抖一下。
        assert!(list[0].key < list[1].key);
        // 谁是自己那条由接收方比对 key 决定，广播里不能替任何窗口做主。
        assert!(list.iter().all(|s| !s.active));
    }

    #[test]
    fn removing_a_window_drops_only_that_connection() {
        let registry = ConnectionRegistry::default();
        registry.upsert("win-a", snapshot("a", "http://a:1"));
        registry.upsert("win-b", snapshot("b", "http://b:2"));

        registry.remove("win-a");
        let list = registry.snapshots();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].display_name, "b");
    }

    #[test]
    fn tally_counts_pending_approvals_the_same_way_as_tui() {
        let registry = ConnectionRegistry::default();
        registry.upsert("win-a", snapshot("a", "http://a:1"));
        registry.upsert("win-b", snapshot("b", "http://b:2"));
        assert!(
            registry
                .set_status("win-b", ConnectionStatus::WaitingApproval)
                .is_some()
        );

        let tally = registry.tally();
        assert_eq!(tally.connected, 2);
        assert_eq!(tally.pending_approvals, 1);

        // 退出中的连接仍占一个位置（与 `LaneTally::from_entries` 同口径）。
        assert!(
            registry
                .set_status("win-a", ConnectionStatus::Quitting)
                .is_some()
        );
        assert_eq!(registry.tally().connected, 2);
    }

    #[test]
    fn status_updates_for_an_unknown_window_are_ignored() {
        let registry = ConnectionRegistry::default();
        // 窗口已关（`Destroyed` 先到、状态后到）：静默忽略，不 panic。
        assert!(
            registry
                .set_status("gone", ConnectionStatus::Running)
                .is_none()
        );
        assert!(!registry.set_unread("gone", true));
        assert!(!registry.rename("gone", "x"));
    }

    #[test]
    fn the_approval_edge_is_visible_to_the_caller() {
        // 通知只在「刚进入等审批」时发，所以 set_status 必须让调用方看得到旧状态：
        // 反复上报 `WaitingApproval` 不能反复弹通知。
        let registry = ConnectionRegistry::default();
        registry.upsert("win-a", snapshot("a", "http://a:1"));

        let first = registry.set_status("win-a", ConnectionStatus::WaitingApproval);
        assert_eq!(first, Some(ConnectionStatus::Idle));
        let again = registry.set_status("win-a", ConnectionStatus::WaitingApproval);
        assert_eq!(again, Some(ConnectionStatus::WaitingApproval));
        // 第二次上报的旧状态已是 WaitingApproval，调用方据此不再发通知。
    }

    #[test]
    fn renaming_keeps_the_key_so_focus_keeps_working() {
        let registry = ConnectionRegistry::default();
        let key = registry.unique_key(Some("prod"), "http://127.0.0.1:8080");
        registry.upsert(key.as_str(), snapshot("prod", "http://127.0.0.1:8080"));

        assert!(registry.rename(key.as_str(), "生产环境"));
        let list = registry.snapshots();
        assert_eq!(list[0].display_name, "生产环境");
        // key 不变 —— 否则窗口 label 与 focus_connection 的目标就对不上了。
        assert_eq!(list[0].key, key);
    }
}
