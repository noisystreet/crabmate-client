//! `crabmatectl --socket` 控制面的客户端：unix socket + **行分隔 JSON**，一问一答。
//!
//! 契约与安全边界全部在 Server 侧（`src/cm_ctl/control.rs`、`docs/命令行与路由.md`）：
//! socket 只在 `$XDG_RUNTIME_DIR/crabmate/ctl.sock`，**不监听 TCP**，鉴权只靠文件权限
//! （目录 `0700` / socket `0600`）。因此这条通道**只在 TUI 与 `crabmatectl` 同机同用户时
//! 可用**；跨机器请自己转发（`ssh -L`），本模块不提供任何网络传输。
//!
//! 本模块只做 IO 与「把一行换成一串 `CtlInstance`」，协议细节与可连性判定在
//! `crabmate_client_api::ctl`（纯逻辑、可单测）。

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crabmate_client_api::ctl::{
    AddOutcome, CtlError, CtlInstance, CtlRequest, parse_add_response, parse_list_response,
};

/// 单次读写超时：与 Server 侧 `IO_TIMEOUT`（5 s）对齐。
///
/// 不设成 60 s（那是 Server 的 `HANDLE_TIMEOUT`）：超时太长会把界面冻在一个已经不响应的
/// socket 上；`add` 起实例慢的那部分由 Server 自己等，客户端先放弃更安全（`list` 是只读的）。
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 单行响应上限：`list` 的响应是几十个短对象，1 MiB 已远超合理值。
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// 调用控制面失败的原因。
///
/// 分三类而不是一个字符串：**「ctl 没在跑」与「ctl 拒绝了请求」的处置完全不同**
/// （前者让用户去起 `supervise --socket`，后者要按 `code` 说话）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtlCallError {
    /// socket 不存在或连不上：ctl 没在跑，或 `--ctl-socket` 指错了路径。
    Connect {
        /// 被尝试的 socket 路径。
        path: PathBuf,
        /// 底层错误原文。
        detail: String,
    },
    /// 连上了但读写失败 / 超时。
    Io(String),
    /// Server 明确回了 `ok:false`。
    Rejected(CtlError),
}

impl CtlCallError {
    /// 面向用户的一行说明（TUI 直接显示）。
    ///
    /// 文案按「该让用户做什么」区分，不只是复述错误：连不上是要去起 supervise，
    /// 被拒绝要看 Server 的 `code`，IO 失败通常重试即可。
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            Self::Connect { path, .. } => format!(
                "控制面连不上：{}（需要先跑 `crabmatectl supervise --socket`，或核对 --ctl-socket）",
                path.display()
            ),
            Self::Io(detail) => format!("控制面读写失败：{detail}"),
            Self::Rejected(err) => {
                let code = err.code.as_deref().unwrap_or("(无 code)");
                if err.message.is_empty() {
                    format!("控制面拒绝请求：{code}")
                } else {
                    format!("控制面拒绝请求：{code} — {}", err.message)
                }
            }
        }
    }
}

/// 默认 socket 路径：`$XDG_RUNTIME_DIR/crabmate/ctl.sock`。
///
/// `XDG_RUNTIME_DIR` 缺失时返回 `None`（不猜 `/tmp`：Server 也只在有该变量时用默认路径，
/// 猜一个位置只会让「连不上」变成「连到别人的 socket」）。
#[must_use]
pub fn default_socket_path() -> Option<PathBuf> {
    socket_path_in(std::env::var_os("XDG_RUNTIME_DIR").as_deref())
}

/// 纯函数：由 `$XDG_RUNTIME_DIR` 的值算出 socket 路径。
///
/// 与环境变量分开是为了可测：edition 2024 下 `std::env::set_var` 是 `unsafe`
/// （进程内其它线程可能正在读环境），测试不去动它。
fn socket_path_in(runtime_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let dir = runtime_dir?;
    if dir.is_empty() {
        return None;
    }
    Some(Path::new(dir).join("crabmate").join("ctl.sock"))
}

/// 默认 socket **存在**时才返回它（TUI 启动时的自动启用判据）。
///
/// 显式 `--ctl-socket` 不走这里：用户指了路径就照用，哪怕此刻还没起来
/// （先开 TUI、后起 supervise 也是常见顺序）。
#[must_use]
pub fn default_socket_path_if_present() -> Option<PathBuf> {
    default_socket_path().filter(|p| p.exists())
}

/// 把 `--ctl-socket` 的原始字符串解析成最终路径。
///
/// 显式给了就用（空串视为未给），**不**检查它此刻是否存在：先开 TUI、后起
/// `supervise --socket` 是常见顺序；等到 `/ctl` 真发请求时再报「连不上」更准确。
#[must_use]
pub fn resolve_socket_arg(arg: Option<&str>) -> Option<PathBuf> {
    arg.map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(default_socket_path_if_present)
}

/// 拉取控制面实例列表（`{"op":"list"}`）。
///
/// `include_unmanaged` 固定 `false`：TUI 要看的是**声明里的实例**（= 可能是 lane 的那些）；
/// 人工 `up` 起、声明里没有的实例属于 `ctl ls` 的视野，混进来只会让「哪条能当 lane」更模糊。
pub fn list_instances(socket: &Path) -> Result<Vec<CtlInstance>, CtlCallError> {
    let request = CtlRequest::List {
        include_unmanaged: false,
    };
    let line = exchange(socket, &request.to_line())?;
    // 解析失败也归 `Rejected`：那是「Server 回了没 ok:true 的东西」，不是传输故障。
    parse_list_response(&line).map_err(CtlCallError::Rejected)
}

/// 新增一条声明（`{"op":"add",...}`）。
///
/// `workspace` **必填**：Server 的 socket `add` 至今要求 `String`（`08291db6` 放开的是
/// TOML 声明里的省略，不是这个 op），所以调用方得先拿到一个路径 —— `/lane new <name>`
/// 用当前 lane 的工作区兜底，拿不到就明说要路径。
///
/// `created: false`（声明已存在）是幂等命中而不是失败：Server 不会覆盖既有声明，
/// 因为那会悄悄丢掉手工写进去的 `env`。
pub fn add_instance(
    socket: &Path,
    name: &str,
    workspace: &str,
) -> Result<AddOutcome, CtlCallError> {
    let request = CtlRequest::Add {
        name: name.to_string(),
        workspace: workspace.to_string(),
        profile: None,
        port: None,
        enabled: true,
    };
    let line = exchange(socket, &request.to_line())?;
    parse_add_response(&line).map_err(CtlCallError::Rejected)
}

/// 写一行、读一行。
fn exchange(socket: &Path, request_line: &str) -> Result<String, CtlCallError> {
    let mut stream = connect(socket)?;
    stream
        .write_all(format!("{request_line}\n").as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| CtlCallError::Io(format!("写入失败：{e}")))?;
    let mut line = String::new();
    // `take` 防一个不按契约行文的对端把内存吃光（多读的部分直接不读）。
    BufReader::new(&stream)
        .take(MAX_RESPONSE_BYTES)
        .read_line(&mut line)
        .map_err(|e| CtlCallError::Io(format!("读取失败：{e}")))?;
    if line.trim().is_empty() {
        return Err(CtlCallError::Io("控制面没有返回内容".to_string()));
    }
    Ok(line)
}

/// 建连并设置读写超时。
fn connect(socket: &Path) -> Result<UnixStream, CtlCallError> {
    let stream = UnixStream::connect(socket).map_err(|e| CtlCallError::Connect {
        path: socket.to_path_buf(),
        detail: e.to_string(),
    })?;
    // 超时设不上不该让一次只读请求失败，但也不能假装成功：失败即视为 IO 错误。
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|e| CtlCallError::Io(format!("设置超时失败：{e}")))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// 在临时目录里起一个只回一次的假控制面：拿到请求行后回 `reply`。
    fn fake_ctl(reply: &'static str) -> (PathBuf, thread::JoinHandle<String>) {
        let dir = std::env::temp_dir().join(format!(
            "crabmate-ctl-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let sock = dir.join("ctl.sock");
        let listener = UnixListener::bind(&sock).expect("绑定 socket");
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            reader.read_line(&mut request).expect("读请求");
            let mut writer = BufWriter::new(&stream);
            writer.write_all(reply.as_bytes()).expect("写响应");
            writer.write_all(b"\n").expect("写换行");
            writer.flush().expect("flush");
            request
        });
        (sock, handle)
    }

    #[test]
    fn list_instances_sends_list_op_and_parses_reply() {
        // **必须单行**：协议是行分隔 JSON，`read_line` 只会读第一个 `\n` 之前的内容
        // （Server 侧 `Response::to_line` 也只序列化一行）。
        let (sock, handle) = fake_ctl(
            r#"{"ok":true,"instances":[{"name":"proj-a","observed":"ready","base_url":"http://127.0.0.1:8721"},{"name":"proj-b","observed":"starting"}]}"#,
        );
        let items = list_instances(&sock).expect("应成功");
        let request = handle.join().expect("假服务端线程");
        assert_eq!(request.trim(), r#"{"op":"list"}"#);
        assert_eq!(items.len(), 2);
        assert!(items[0].can_connect());
        assert!(!items[1].can_connect(), "starting 不可连");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// `add` 只发 Server 白名单里的字段，并且把 `created` 原样带回给调用方。
    #[test]
    fn add_instance_sends_add_op_and_reports_created() {
        let (sock, handle) = fake_ctl(r#"{"ok":true,"name":"proj-a","created":true}"#);
        let out = add_instance(&sock, "proj-a", "/home/u/proj-a").expect("应成功");
        let request = handle.join().expect("假服务端线程");
        assert!(out.created, "新建声明应回 created=true");
        assert!(request.contains(r#""op":"add""#), "{request}");
        assert!(request.contains(r#""name":"proj-a""#), "{request}");
        assert!(
            request.contains(r#""workspace":"/home/u/proj-a""#),
            "{request}"
        );
        // 多一个键就会被 Server 的字段白名单拒成 bad_request。
        assert!(!request.contains("config"), "绝不夹带 config：{request}");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    #[test]
    fn rejected_reply_becomes_rejected_with_code() {
        let (sock, handle) = fake_ctl(r#"{"ok":false,"code":"supervisor_gone","error":"已退出"}"#);
        let err = list_instances(&sock).unwrap_err();
        handle.join().expect("假服务端线程");
        match err {
            CtlCallError::Rejected(e) => {
                assert_eq!(
                    e.kind,
                    crabmate_client_api::ctl::CtlErrorKind::SupervisorGone
                );
                assert_eq!(e.code.as_deref(), Some("supervisor_gone"));
            }
            other => panic!("应归 Rejected，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// ctl 没在跑（路径下没有 socket）：必须是 `Connect`，不能报成「被拒绝」。
    #[test]
    fn missing_socket_is_a_connect_error() {
        let dir = std::env::temp_dir().join(format!(
            "crabmate-ctl-missing-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let sock = dir.join("nope.sock");
        let err = list_instances(&sock).unwrap_err();
        assert!(matches!(err, CtlCallError::Connect { .. }), "{err:?}");
        // 文案要指向「去起 supervise」，而不是让人重试。
        assert!(err.user_message().contains("supervise --socket"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空响应（对端直接关连接）算 IO 失败，不算「解析出 0 条」。
    #[test]
    fn empty_reply_is_io_error() {
        let (sock, handle) = fake_ctl("");
        let err = list_instances(&sock).unwrap_err();
        handle.join().expect("假服务端线程");
        assert!(matches!(err, CtlCallError::Io(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// 默认路径只从 `XDG_RUNTIME_DIR` 派生，不猜 `/tmp`。
    #[test]
    fn default_path_uses_xdg_runtime_dir() {
        let dir = std::ffi::OsStr::new("/run/user/1000");
        assert_eq!(
            socket_path_in(Some(dir)),
            Some(PathBuf::from("/run/user/1000/crabmate/ctl.sock"))
        );
        // 变量缺失与空串都不猜位置。
        assert_eq!(socket_path_in(None), None);
        assert_eq!(socket_path_in(Some(std::ffi::OsStr::new(""))), None);
    }
}
