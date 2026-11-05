//! `crabmatectl` 控制面（unix socket + 行分隔 JSON）的桌面壳传输层。
//!
//! **为什么这里再写一份 IO**：`crabmate-client-api` 是纯度层（该 crate 的 `ctl.rs`
//! 头注释写明「不允许 IO」），而 TUI 那份传输在 `crates/crabmate-tui/src/serve/ctl.rs`
//! —— 依赖方向不允许 desktop 引用 tui。协议形状、`observed` 词表与错误分类仍然只有
//! 一份（`crabmate_client_api::ctl`），本模块只做 socket 读写。
//!
//! **只做 `list`（只读）**：`add` / `remove` / `enable`·`disable` 会写 `instances.toml`，
//! 而 `crabmatectl --watch` 已经是那个文件的写者；本仓再写一次就是两个写者竞争。
//! 少一条连接由用户关窗口完成，不靠摘声明。

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crabmate_client_api::ctl::{CtlInstance, CtlRequest, parse_list_response};

/// 单次读写超时：与 Server 侧 `IO_TIMEOUT`（5 s）及 TUI 同一值。
///
/// 不跟 Server 的 `HANDLE_TIMEOUT`（60 s）：超时太长会把界面冻在一个已经不响应的
/// socket 上，而 `list` 是只读的，客户端先放弃再重试更安全。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

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

/// 发一条 `list` 并读一行回包。
fn list_at(path: &Path) -> Result<Vec<CtlInstance>, String> {
    let mut stream = UnixStream::connect(path).map_err(|e| format!("连接失败：{e}"))?;
    // 超时设不上不是致命错（某些平台/配置下会失败），继续尝试读写。
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

    // `include_unmanaged: false` —— 与 TUI 同一口径：要的是**声明里的**实例
    // （= 可能是 lane 的那些），不是人工 `up` 起的进程。
    let request = CtlRequest::List {
        include_unmanaged: false,
    }
    .to_line();
    writeln!(stream, "{request}").map_err(|e| format!("写入失败：{e}"))?;

    let mut reader = BufReader::new(stream).take(MAX_RESPONSE_BYTES);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("读取失败：{e}"))?;
    if line.trim().is_empty() {
        return Err("控制面返回空响应".to_string());
    }
    parse_list_response(&line).map_err(|e| format!("响应无法解析：{e:?}"))
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

#[cfg(test)]
mod tests {
    use super::default_socket_path_if_present;

    #[test]
    fn a_missing_default_socket_is_none_not_an_error() {
        // 没有 `XDG_RUNTIME_DIR` 时不猜路径：猜出来的位置只会连到别人的 socket。
        // 这里不设环境变量（edition 2024 下 `set_var` 是 unsafe），只验「变量缺失
        // 即 None」这条分支在真机上就是常态。
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            assert!(default_socket_path_if_present().is_none());
        }
    }
}
