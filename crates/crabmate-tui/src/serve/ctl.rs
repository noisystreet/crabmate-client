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
    AddOutcome, CtlError, CtlInstance, CtlRequest, ToggleOutcome, parse_add_response,
    parse_list_response, parse_toggle_response,
};

use super::url::normalize_api_base;

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
///
/// `profile` 只按**名字**引用 `profiles.toml` 里的预设（Server 的 `AddRequest.profile`
/// 同样是 `Option<String>`，且它只过存在性 / 名字校验，不接受内联配置）。名字写错时
/// Server 回的 `error` 原文会经 [`CtlCallError::user_message`] 透传出去 —— 本层不重复
/// 校验一遍，那只会让两处口径各自漂移。
pub fn add_instance(
    socket: &Path,
    name: &str,
    workspace: &str,
    profile: Option<&str>,
) -> Result<AddOutcome, CtlCallError> {
    let request = CtlRequest::Add {
        name: name.to_string(),
        workspace: workspace.to_string(),
        profile: profile.map(str::to_string),
        port: None,
        enabled: true,
    };
    let line = exchange(socket, &request.to_line())?;
    parse_add_response(&line).map_err(CtlCallError::Rejected)
}

/// 关闭 / 放开一条声明的 `enabled`（`{"op":"disable"|"enable","name":…}`）。
///
/// 这是 `/lane close` / `/lane open` 的落点：`enabled = false` 是 Server 文档里
/// **客户端软删 lane 的正式口径**（声明本身与手写的 `env` / `config` 都留着），
/// 而硬 `remove` 会把它们一起删掉 —— 那不是 `/lane` 该代劳的事。
///
/// 幂等：目标不存在也回 `ok:true`（`changed:false` + `note`），所以调用方不必先 `list`。
pub fn set_instance_enabled(
    socket: &Path,
    name: &str,
    enabled: bool,
) -> Result<ToggleOutcome, CtlCallError> {
    let request = CtlRequest::SetEnabled {
        name: name.to_string(),
        enabled,
    };
    let line = exchange(socket, &request.to_line())?;
    parse_toggle_response(&line).map_err(CtlCallError::Rejected)
}

/// 主 lane 的默认名字：ctl 里反查不到对应实例时用它。
///
/// 也是「`--api-base` 指向一台没有声明的 serve」时的唯一结果 —— 那种情况下
/// 用户自己起的进程不属于任何实例，叫 `local` 才不撒谎。
pub const LOCAL_LANE: &str = "local";

/// 启动期给主 lane 定名字：ctl 里有同根 URL 的实例就用其实例名，否则 [`LOCAL_LANE`]。
///
/// 这是「一台 serve 只该有一条 lane」的启动侧一半。主连接是裸 `--api-base`，label
/// 原本固定为 `local`；而运行期 `/lane attach <name>` 按 **label** 查重
/// （`tui_mode/shell.rs` 的 `attach_lane`），于是同一台 serve 会被连成两条 lane
/// （`local` + `server`）—— 左栏「连接 [N]」与 ctl 的进程数对不上，两条 lane 还各开
/// 一条 SSE。按 `base_url` 反查把主 lane 直接命名成声明里的实例名之后，
/// `/lane attach server` 会命中既有 lane、只切过去。
///
/// **失败一律回退**：ctl 没起、超时、回包读不懂都不该拦住 TUI 启动 —— 这条查询只负责
/// 命名，不是连接前提。匹配只看 `base_url`，不看 `observed`：暂时 `stale` /
/// `starting` 的实例仍是「这台 serve 的那条声明」，改名不会因此错位。
#[must_use]
pub fn primary_label(socket: Option<&Path>, api_base: &str) -> String {
    let Some(socket) = socket else {
        return LOCAL_LANE.to_string();
    };
    match list_instances(socket) {
        Ok(items) => name_for_base_url(&items, api_base).unwrap_or_else(|| LOCAL_LANE.to_string()),
        Err(_) => LOCAL_LANE.to_string(),
    }
}

/// 启动期把「主连接」认领成控制面里的实例：解析 `--ctl-socket` 并反查主 lane 的名字。
///
/// 两件事并成一处是因为它们只在启动时一起发生，且顺序有依赖：没有 socket 就无从反查。
/// socket 的解析沿用 [`resolve_socket_arg`] 的语义 —— 显式路径优先且 **不查存在性**
/// （先开 TUI、后起 `supervise --socket` 是常见顺序），未给时才在默认路径存在时启用。
#[must_use]
pub fn primary_binding(arg: Option<&str>, api_base: &str) -> (Option<PathBuf>, String) {
    let socket = resolve_socket_arg(arg);
    let label = primary_label(socket.as_deref(), api_base);
    (socket, label)
}

/// 在实例列表里按根 URL 找实例名。
///
/// 比较前两侧都过 [`normalize_api_base`]（去空白与尾 `/`）并转小写：`base_url` 不携带
/// 路径，scheme / host 本就不区分大小写。`localhost` 与 `127.0.0.1` **不**视为同一个
/// —— 那是两个 Origin，悄悄合并会让「我连的到底是哪台」失去答案。
#[must_use]
pub fn name_for_base_url(items: &[CtlInstance], api_base: &str) -> Option<String> {
    let want = url_key(api_base)?;
    items
        .iter()
        .find(|i| i.base_url.as_deref().and_then(url_key).as_deref() == Some(want.as_str()))
        .map(|i| i.name.clone())
}

/// 两个根 URL 是否指向同一台 serve（归一化后相等）。
///
/// `/lane attach` 的去重用它：**名字不同不代表是两台 serve** —— 启动 lane 的名字可能来自
/// 上面的反查，也可能是 `local`。重复连接会让同一台 serve 各开一条 SSE、各读一份会话，
/// 左栏的连接数还会与 ctl 的进程数对不上。
///
/// 与 [`name_for_base_url`] 同一套比较口径，且同样不把 `localhost` 与 `127.0.0.1` 合并。
#[must_use]
pub fn same_api_base(a: &str, b: &str) -> bool {
    match (url_key(a), url_key(b)) {
        // 空串（没有 URL / 非绝对 http(s)）不构成「同一台」的证据，宁可放行让调用方去连。
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 归一化后的比较键；空串（没有 `base_url` / 非绝对 http(s) URL）返回 `None`。
///
/// 先整体转小写再交给 [`normalize_api_base`]：后者用 `starts_with("http://")` 判绝对
/// URL，大写的 `HTTP://` 会被它当成非法并返回空串。base_url 不携带路径，整体小写不改变
/// 语义（scheme / host 本就不区分大小写）。
fn url_key(url: &str) -> Option<String> {
    let normalized = normalize_api_base(&url.trim().to_ascii_lowercase());
    (!normalized.is_empty()).then_some(normalized)
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
        let out = add_instance(&sock, "proj-a", "/home/u/proj-a", None).expect("应成功");
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

    /// `profile` 只传**名字**：请求行里有 `"profile"`，没有内联配置。
    ///
    /// Server 的 `add` 字段白名单是 `op` / `name` / `workspace` / `profile` / `port` /
    /// `enabled`，所以这里两头都不能错：漏了是「以为指定了预设、其实用了默认」，
    /// 夹带别的键则是 `bad_request`。
    #[test]
    fn add_instance_forwards_the_profile_name() {
        let (sock, handle) = fake_ctl(r#"{"ok":true,"name":"proj-a","created":true}"#);
        let out = add_instance(&sock, "proj-a", "/home/u/proj-a", Some("readonly-audit"))
            .expect("应成功");
        let request = handle.join().expect("假服务端线程");
        assert!(out.created, "新建声明应回 created=true");
        assert!(
            request.contains(r#""profile":"readonly-audit""#),
            "profile 必须按名字出现在请求行里：{request}"
        );
        assert!(!request.contains("config"), "绝不夹带 config：{request}");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// 关闭声明：发 `disable`，且只带 `name`（Server 的白名单就这两项）。
    #[test]
    fn set_instance_enabled_sends_the_matching_op() {
        let (sock, handle) =
            fake_ctl(r#"{"ok":true,"name":"proj-a","enabled":false,"changed":true}"#);
        let out = set_instance_enabled(&sock, "proj-a", false).expect("应成功");
        let request = handle.join().expect("假服务端线程");
        assert!(out.changed, "第一次关闭应真的改了声明");
        assert_eq!(out.enabled, Some(false));
        assert!(request.contains(r#""op":"disable""#), "{request}");
        assert!(request.contains(r#""name":"proj-a""#), "{request}");
        assert!(
            !request.contains("workspace") && !request.contains("config"),
            "绝不夹带别的字段：{request}"
        );
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// 放开声明：发 `enable`，并把 `cleared_down` 原样带回（否则 UI 会漏掉「顺手清了
    /// down 标记」这件事）。
    #[test]
    fn enabling_sends_enable_op_and_reports_cleared_down() {
        let (sock, handle) = fake_ctl(
            r#"{"ok":true,"name":"proj-a","enabled":true,"changed":true,"cleared_down":true}"#,
        );
        let out = set_instance_enabled(&sock, "proj-a", true).expect("应成功");
        let request = handle.join().expect("假服务端线程");
        assert!(out.cleared_down, "enable 顺手清 down 标记要带回");
        assert!(request.contains(r#""op":"enable""#), "{request}");
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

    /// 启动期命名：ctl 里有同根 URL 的实例就用实例名，尾斜杠不参与比较。
    #[test]
    fn primary_label_uses_ctl_instance_name_for_same_url() {
        let (sock, handle) = fake_ctl(
            r#"{"ok":true,"instances":[{"name":"cuda","observed":"ready","base_url":"http://127.0.0.1:8080/"},{"name":"server","observed":"ready","base_url":"http://127.0.0.1:8081/"}]}"#,
        );
        let label = primary_label(Some(&sock), "http://127.0.0.1:8081");
        handle.join().expect("假服务端线程");
        assert_eq!(label, "server", "主连接应直接叫声明里的名字");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// 反查不到就是 `local`：URL 未声明 / 没给 socket 都不该改名。
    #[test]
    fn primary_label_falls_back_to_local() {
        assert_eq!(
            primary_label(None, "http://127.0.0.1:8081"),
            LOCAL_LANE,
            "没有控制面时无从反查"
        );
        let (sock, handle) = fake_ctl(
            r#"{"ok":true,"instances":[{"name":"a","base_url":"http://127.0.0.1:8080"}]}"#,
        );
        let label = primary_label(Some(&sock), "http://127.0.0.1:8081");
        handle.join().expect("假服务端线程");
        assert_eq!(label, LOCAL_LANE, "URL 不在声明里就不改名");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// ctl 连不上只回退，不拦启动（一次命名查询不是连接前提）。
    #[test]
    fn primary_label_falls_back_when_ctl_is_down() {
        let dir = std::env::temp_dir().join(format!(
            "crabmate-ctl-down-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let label = primary_label(Some(&dir.join("nope.sock")), "http://127.0.0.1:8081");
        assert_eq!(label, LOCAL_LANE, "连不上就沿用 local");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 匹配只看 URL：`stale` 仍是「那台 serve 的声明」，空 `base_url` 不参与比较。
    ///
    /// 同 URL 判定（`same_api_base`）：尾斜杠与大小写不参与比较，`localhost` 不合并。
    #[test]
    fn name_for_base_url_ignores_observation_and_blank_urls() {
        let items = vec![
            CtlInstance {
                name: "stale-one".to_string(),
                observed: Some("stale".to_string()),
                base_url: Some("http://127.0.0.1:8081/".to_string()),
                ..CtlInstance::default()
            },
            CtlInstance {
                name: "no-url".to_string(),
                observed: Some("ready".to_string()),
                ..CtlInstance::default()
            },
        ];
        assert_eq!(
            name_for_base_url(&items, "http://127.0.0.1:8081").as_deref(),
            Some("stale-one")
        );
        assert_eq!(
            name_for_base_url(&items, ""),
            None,
            "空 URL 不能匹配到没有 base_url 的实例"
        );
        assert_eq!(
            name_for_base_url(&items, "http://localhost:8081"),
            None,
            "localhost 与 127.0.0.1 是两个 Origin，不合并"
        );
    }

    /// 同 URL 判定：尾斜杠与大小写不影响，端口不同 / `localhost` 与 `127.0.0.1` 不算同一台。
    #[test]
    fn same_api_base_ignores_cosmetic_differences_only() {
        assert!(same_api_base(
            "http://127.0.0.1:8081/",
            "http://127.0.0.1:8081"
        ));
        assert!(same_api_base(
            "HTTP://127.0.0.1:8081",
            "http://127.0.0.1:8081"
        ));
        assert!(!same_api_base(
            "http://127.0.0.1:8081",
            "http://127.0.0.1:8080"
        ));
        assert!(
            !same_api_base("http://127.0.0.1:8081", "http://localhost:8081"),
            "两个 Origin 不合并"
        );
        assert!(!same_api_base("", ""), "空串不是「同一台 serve」的证据");
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
