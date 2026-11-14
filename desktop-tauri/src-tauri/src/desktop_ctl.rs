//! `crabmatectl` 控制面（unix socket + 行分隔 JSON）的桌面壳传输层。
//!
//! **为什么这里再写一份 IO**：`crabmate-client-api` 是纯度层（该 crate 的 `ctl.rs`
//! 头注释写明「不允许 IO」），而 TUI 那份传输在 `crates/crabmate-tui/src/serve/ctl.rs`
//! —— 依赖方向不允许 desktop 引用 tui。协议形状、`observed` 词表与错误分类仍然只有
//! 一份（`crabmate_client_api::ctl`），本模块只做 socket 读写。
//!
//! **不做 `add` / `remove` / `enable`·`disable`**：那四个 op 会写 `instances.toml`，而
//! `crabmatectl --watch` 已经是那个文件的写者；本仓再写一次就是两个写者竞争。少一条连接
//! 由用户关窗口完成，不靠摘声明。
//!
//! **做 `list` 与 `restart`**：`list` 是只读查询；`restart` 是「**请 ctl** 重启一条实例」，
//! 声明仍由 ctl 写 —— 客户端只是它的调用方，不构成第二个写入者。它也是 TUI
//! `/lane restart` 的同一条通道（`serve::ctl::restart_instance`）。单条详情（`status`）
//! 只有 TUI 用得上（`/lane status`）：桌面左栏那几行本来就从 `list` 来，多一条不画任何
//! 东西的命令就是多一份没人调用的 IPC 面。

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crabmate_client_api::ctl::{
    CtlInstance, CtlRequest, RESTART_BUDGET_SECS, RestartOutcome, parse_list_response,
    parse_restart_response,
};

/// 单次读写超时：与 Server 侧 `IO_TIMEOUT`（5 s）及 TUI 同一值。
///
/// 不跟 Server 的 `HANDLE_TIMEOUT`（60 s）：超时太长会把界面冻在一个已经不响应的
/// socket 上，而 `list` / `status` 是只读的，客户端先放弃再重试更安全。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// `restart` 的读写超时：**必须**明显长于 [`RESTART_BUDGET_SECS`]。
///
/// 与上面那条相反，理由也相反：`restart` 是**同步** op（down → up → 等就绪才回一行），
/// 用 5 s 会把一次**成功**的重启读成超时，用户重试就等于把一条正在起的实例再打一次。
const RESTART_IO_TIMEOUT: Duration = Duration::from_secs(RESTART_BUDGET_SECS + 20);

/// 单行响应上限：`list` 是几十个短对象，1 MiB 已远超合理值（与 TUI 同值）。
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// `$XDG_RUNTIME_DIR/crabmate/ctl.sock`，**存在**才返回。
///
/// 缺失是正常状态（没跑 `crabmatectl supervise --socket`），所以这里既不是错误、
/// 也不回退去猜 `/tmp`：Server 同样只在有该变量时用默认路径，猜一个位置只会让
/// 「没有控制面」变成「连到别人的 socket」。
fn default_socket_path_if_present() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    if dir.is_empty() {
        return None;
    }
    let path = Path::new(&dir).join("crabmate").join("ctl.sock");
    path.exists().then_some(path)
}

/// 发一行请求并读一行回包。超时由调用方按 op 决定（见两个 `*_IO_TIMEOUT`）。
fn exchange(path: &Path, request: &str, io_timeout: Duration) -> Result<String, String> {
    let mut stream = UnixStream::connect(path).map_err(|e| format!("连接失败：{e}"))?;
    // 超时设不上不是致命错（某些平台/配置下会失败），继续尝试读写。
    let _ = stream.set_read_timeout(Some(io_timeout));
    let _ = stream.set_write_timeout(Some(io_timeout));
    writeln!(stream, "{request}").map_err(|e| format!("写入失败：{e}"))?;

    let mut reader = BufReader::new(stream).take(MAX_RESPONSE_BYTES);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("读取失败：{e}"))?;
    if line.trim().is_empty() {
        return Err("控制面返回空响应".to_string());
    }
    Ok(line)
}

/// 拉一次实例列表。
///
/// `include_unmanaged: false` —— 与 TUI 同一口径：要的是**声明里的**实例
/// （= 可能是 lane 的那些），不是人工 `up` 起的进程。
fn list_at(path: &Path) -> Result<Vec<CtlInstance>, String> {
    let request = CtlRequest::List {
        include_unmanaged: false,
    }
    .to_line();
    let line = exchange(path, &request, IO_TIMEOUT)?;
    parse_list_response(&line).map_err(|e| format!("响应无法解析：{e:?}"))
}

/// 重启一条实例（`restart` op；同步，最长可占满 [`RESTART_BUDGET_SECS`]）。
fn restart_at(
    path: &Path,
    name: &str,
    grace: Option<u64>,
    timeout: Option<u64>,
) -> Result<RestartOutcome, String> {
    let request = CtlRequest::Restart {
        name: name.to_string(),
        grace,
        timeout,
    }
    .to_line();
    let line = exchange(path, &request, RESTART_IO_TIMEOUT)?;
    parse_restart_response(&line).map_err(|e| format!("响应无法解析：{e:?}"))
}

/// 控制面里**声明过的**实例（只读查询）。
///
/// 失败一律回空表而不是错误：socket 不存在、连不上、读写超时都是「本机现在没有
/// 可连实例」的常态，界面上那是一块空分组，不该弹错误；真出问题时壳日志里留一行。
/// 判可连性不要看 `observed` 字符串 —— 用 `CtlInstance::can_connect()`，它只认
/// `ready`（`stale` 会连到死端口）。
#[tauri::command]
pub(crate) async fn ctl_list_instances() -> Vec<CtlInstance> {
    let Some(path) = default_socket_path_if_present() else {
        return Vec::new();
    };
    // 阻塞读 5 s 是可能发生的，扔进阻塞线程池：别占住异步 worker。
    tauri::async_runtime::spawn_blocking(move || match list_at(&path) {
        Ok(items) => items,
        Err(detail) => {
            eprintln!("[ctl] list 失败（{path:?}）：{detail}");
            Vec::new()
        }
    })
    .await
    .unwrap_or_default()
}

/// 重启一条实例（`{"op":"restart",…}`），回重启后的状态行与 Server 的 `notes`。
///
/// **同步 op**：服务端跑完 down → up → 等就绪才回这一行（最长 [`RESTART_BUDGET_SECS`]），
/// 所以必须待在阻塞线程池里 —— 占住异步 worker 会让整个窗口（含其它窗口的调用）
/// 在后端排队。`grace` / `timeout` 省略即 Server 默认各 10 s；预算上限也在 Server
/// （`restart_budget_exceeded`），客户端不替它判。
#[tauri::command]
pub(crate) async fn ctl_restart_instance(
    name: String,
    grace: Option<u64>,
    timeout: Option<u64>,
) -> Result<RestartOutcome, String> {
    let path = default_socket_path_if_present().ok_or_else(|| {
        "控制面 socket 不存在（需要先跑 `crabmatectl supervise --socket`）".to_string()
    })?;
    tauri::async_runtime::spawn_blocking(move || restart_at(&path, &name, grace, timeout))
        .await
        .map_err(|e| format!("重启任务异常：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::{IO_TIMEOUT, RESTART_IO_TIMEOUT, default_socket_path_if_present};

    #[test]
    fn a_missing_default_socket_is_none_not_an_error() {
        // 没有 `XDG_RUNTIME_DIR` 时不猜路径：猜出来的位置只会连到别人的 socket。
        // 这里不设环境变量（edition 2024 下 `set_var` 是 unsafe），只验「变量缺失
        // 即 None」这条分支在真机上就是常态。
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            assert!(default_socket_path_if_present().is_none());
        }
    }

    /// `restart` 的读超时必须明显长于普通读：它是同步 op，用 5 s 会把**成功**读成超时。
    #[test]
    fn restart_waits_longer_than_a_read() {
        assert!(RESTART_IO_TIMEOUT > IO_TIMEOUT);
        assert!(
            RESTART_IO_TIMEOUT.as_secs() > crabmate_client_api::ctl::RESTART_BUDGET_SECS,
            "读超时要覆盖整个 grace + timeout 预算"
        );
    }
}
