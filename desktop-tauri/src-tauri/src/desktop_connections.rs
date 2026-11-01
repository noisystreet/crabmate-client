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

    /// 改某条连接的状态；返回是否真的存在这一条。
    pub(crate) fn set_status(&self, label: &str, status: ConnectionStatus) -> bool {
        self.entries().get_mut(label).is_some_and(|snapshot| {
            snapshot.status = status;
            true
        })
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
/// 由业务 UI 在连接成功后调用（拿得到 `api_base` 与用户取的名字）。`key` 由注册表分配
/// 并回传 —— 前端要把它存下来当作「我是谁」。
#[tauri::command]
pub(crate) fn register_connection(
    app: AppHandle,
    window: WebviewWindow,
    api_base: String,
    display_name: Option<String>,
) -> Result<ConnectionSnapshot, String> {
    let state = registry(&app).ok_or_else(|| "连接注册表未就绪".to_string())?;
    let base = crabmate_connect::normalize_base_url(&api_base)?;
    let preferred = display_name;
    let mut snapshot =
        ConnectionSnapshot::new(preferred.clone().unwrap_or_default(), base.as_str());
    // 以注册表的分配为准：`new` 派生的是基础 key，撞名时这里会带后缀。
    snapshot.key = state.unique_key(preferred.as_deref(), base.as_str());
    snapshot.active = true;
    state.upsert(window.label(), snapshot.clone());
    broadcast(&app);
    Ok(snapshot)
}

/// 窗口报告状态变化（回合开始 / 等审批 / 空闲）。
#[tauri::command]
pub(crate) fn report_connection_status(
    app: AppHandle,
    window: WebviewWindow,
    status: ConnectionStatus,
) {
    if let Some(state) = registry(&app)
        && state.set_status(window.label(), status)
    {
        broadcast(&app);
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
        assert!(registry.set_status("win-b", ConnectionStatus::WaitingApproval));

        let tally = registry.tally();
        assert_eq!(tally.connected, 2);
        assert_eq!(tally.pending_approvals, 1);

        // 退出中的连接仍占一个位置（与 `LaneTally::from_entries` 同口径）。
        assert!(registry.set_status("win-a", ConnectionStatus::Quitting));
        assert_eq!(registry.tally().connected, 2);
    }

    #[test]
    fn status_updates_for_an_unknown_window_are_ignored() {
        let registry = ConnectionRegistry::default();
        // 窗口已关（`Destroyed` 先到、状态后到）：静默忽略，不 panic。
        assert!(!registry.set_status("gone", ConnectionStatus::Running));
        assert!(!registry.set_unread("gone", true));
        assert!(!registry.rename("gone", "x"));
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
