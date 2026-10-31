//! 控制面（`crabmatectl supervise --socket`）的**外壳级** worker：列实例 / 连一条 /
//! 新建一条 / 翻声明开关。
//!
//! 原先这四个 job 排在**发起命令那条 lane 的 worker** 上串行执行，`ctl_socket` 也因此
//! 存在每条 `LaneApp` 里。两处代价（issue #350）：`/lane new` 的 20 s 就绪轮询会把那条
//! lane 的所有拉取堵在队列里；8 条 lane 就是 8 条 unix 连接、8 个超时窗口，最终却都落到
//! 同一个 ctl 单写者。
//!
//! 现在是**一个**线程、**一份** socket、天然串行。每次调用带上发起它的 [`LaneId`]，
//! 结果按那个 id 回到**发起者**的 transcript —— 「谁发起的」与「谁执行的」解耦。
//!
//! 与 `worker.rs`（回合 / HTTP 拉取）分开：这边只经 unix socket、完全不碰 `reqwest`
//! 连接池，也不需要 tokio runtime。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crabmate_client_api::ctl::{CtlInstance, LaneReadiness, ToggleFailure, lane_readiness};

use super::inbound::{Inbound, ShellEvent};
use super::worker::{LaneId, LaneTx, UiEvent};
use crate::serve::{add_instance, list_instances, set_instance_enabled};

/// `/lane new` 等实例就绪的轮询间隔。
const LANE_READY_POLL: Duration = Duration::from_millis(300);

/// 等实例就绪的上限：`up` 自己的 readiness 超时是 10 s，这里留一倍余量。
///
/// 到点**不是**「实例起不来」的结论（它可能仍在起），所以文案分开说 ——
/// 把还在启动的实例报成失败，会让人去重试一件正在发生的事。
const LANE_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// 外壳派给控制面线程的一次调用。
///
/// 每个变体都带发起它的 [`LaneId`]：结果写回**发起者**，而不是「回包到达时聚焦的那条」
/// —— `/lane new` 的 20 s 轮询期间用户很可能已经切走了。
pub(super) enum CtlJob {
    /// `/ctl` 与 `/lane`（空参）：拉一次实例列表。
    List {
        /// 发起这次查询的 lane（回包写进它的 transcript）。
        from: LaneId,
    },
    /// `/lane attach <name>`：按名字在声明里找一条已就绪的实例，把 `base_url` 带回去。
    Attach {
        /// 发起这次连接的 lane。
        from: LaneId,
        /// 实例名（`instances.toml` 的键，也是将来那条 lane 的 `label`）。
        name: String,
    },
    /// `/lane new <name> [workspace] [profile=<name>]`：写声明 → 轮询到就绪。
    New {
        /// 发起这次新建的 lane。
        from: LaneId,
        /// 新实例名。
        name: String,
        /// 声明的 `workspace`（Server 的 socket `add` 要求必填）。
        workspace: String,
        /// `profiles.toml` 里的预设名；`None` = 用 Server 的默认（行为与加这个字段前一致）。
        ///
        /// 原样转发，不在客户端校验：名字写错由 Server 的 `add` 回 `bad_request`，
        /// 那句 `error` 原文才是用户要看的（`profiles.toml` 在 serve 那台机器上）。
        profile: Option<String>,
    },
    /// `/lane close|open`：翻声明里的 `enabled`（只写声明、不轮询）。
    Toggle {
        /// 发起这次翻转的 lane（被关的可能正是它）。
        from: LaneId,
        /// 实例名。
        name: String,
        /// `true` = `enable`，`false` = `disable`。
        enabled: bool,
    },
}

/// 外壳侧持有的控制面通道：发送端 + 「前面还有几条没返回」的计数。
///
/// 为什么要计数：控制面已收敛到**一个**线程（issue #350），于是 `/lane new` 的 20 s
/// 就绪轮询期间，别的 lane 敲的 `/ctl` 会排在那次轮询后面。用户那边看到的是自己那条
/// 命令迟迟没有回包（「正在查询…」早就写出来了），既没有进度也没有解释。
///
/// 计数让**发起方**在按下的那一刻就说清「为什么现在没反应」：ctl 线程只有轮到它才
/// 知道自己排第几，那时再说已经晚了。
#[derive(Clone)]
pub(super) struct CtlHandle {
    tx: Sender<CtlJob>,
    /// 已入队但还没执行完的条数（含正在执行的那一条）。只用于提示，不参与调度。
    ///
    /// [`CtlHandle::enqueue`] 加一、ctl 线程执行完减一。
    in_flight: Arc<AtomicUsize>,
}

/// 一次调用在队列里的位置（[`CtlHandle::enqueue`] 的返回）。
///
/// 不做成 `bool`：`Behind(2)` 里的数字能直接说清「前面还有几条」，`true` 只会让人
/// 再猜一次 —— 而这句提示存在的全部意义就是省掉那次猜。
pub(super) enum Queued {
    /// 前面没有未返回的调用：马上就会执行。
    Immediate,
    /// 前面还有 `ahead` 条没返回（含正在执行的那条）。
    Behind(usize),
}

impl CtlHandle {
    /// 派一次调用，并返回它排在第几个。
    ///
    /// 计数与入队必须挨在一起：`fetch_add` 的返回值正是「我之前有几条」，拆成两处就
    /// 多一个「读到 0、随即被人插队」的窗口，而提示的准确性全押在这个返回值上。
    pub(super) fn enqueue(&self, job: CtlJob) -> Result<Queued, mpsc::SendError<CtlJob>> {
        let ahead = self.in_flight.fetch_add(1, Ordering::SeqCst);
        match self.tx.send(job) {
            Ok(()) if ahead == 0 => Ok(Queued::Immediate),
            Ok(()) => Ok(Queued::Behind(ahead)),
            Err(e) => {
                // 没入队就不能占着计数：否则 ctl 线程已退出时，后面每条命令都会先报
                // 「前面还有 N 条」再报发不出去 —— 两句互相矛盾。
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// 只造句柄与接收端，不起线程（测试用）。
    ///
    /// 队列序只取决于计数与通道，而用例要看的是「提示何时出现」：起一个真线程（还得
    /// 配一个真 socket）只会让它变慢、变脆。
    #[cfg(test)]
    pub(super) fn detached(tx: Sender<CtlJob>) -> Self {
        Self {
            tx,
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// 起那条**唯一**的控制面线程；`socket` 从此只由它持有。
///
/// 所有 [`CtlHandle`] 都 drop（= TUI 退出）之后 `recv` 返回 `Err`，线程自行退出。
/// 与每条 lane 的 worker 线程同构（靠通道关闭收尾），不需要另发停止信号。
pub(super) fn spawn(socket: PathBuf, tx: Sender<Inbound>) -> CtlHandle {
    let (job_tx, job_rx) = mpsc::channel::<CtlJob>();
    let in_flight = Arc::new(AtomicUsize::new(0));
    let done = Arc::clone(&in_flight);
    thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            run(job, &socket, &tx);
            // 执行完才递减：计数要覆盖「排队 + 在执行」整段，否则提示会低估等待。
            // `run` panic 时这一行不会执行，但那时线程也死了 —— 之后的 `enqueue`
            // 直接报「发不出去」，不会停在假的排队状态上。
            done.fetch_sub(1, Ordering::SeqCst);
        }
    });
    CtlHandle {
        tx: job_tx,
        in_flight,
    }
}

/// 执行一次调用，并把结果送回发起它的 lane。
///
/// 两条出口沿用 #347 的类型划分：`/ctl` 的回包属于那条 lane 自己，走 [`LaneTx`]
/// 打 `LaneId` 标签；attach / new / toggle 是外壳动作（目标 lane 可能还不存在、
/// 也可能正是被关掉的那条），走 [`ShellEvent`]。
fn run(job: CtlJob, socket: &Path, tx: &Sender<Inbound>) {
    match job {
        CtlJob::List { from } => {
            let result = list_instances(socket).map_err(|e| e.user_message());
            let _ = LaneTx::new(from, tx.clone()).send(UiEvent::CtlList(result));
        }
        CtlJob::Attach { from, name } => {
            let result = list_instances(socket)
                .map_err(|e| e.user_message())
                .and_then(|items| ready_base_url(&items, &name));
            let _ = tx.send(Inbound::Shell(ShellEvent::Ready { from, name, result }));
        }
        CtlJob::New {
            from,
            name,
            workspace,
            profile,
        } => {
            let result = add_then_wait_ready(socket, &name, &workspace, profile.as_deref());
            let _ = tx.send(Inbound::Shell(ShellEvent::Ready { from, name, result }));
        }
        CtlJob::Toggle {
            from,
            name,
            enabled,
        } => {
            let result = set_instance_enabled(socket, &name, enabled).map_err(|e| ToggleFailure {
                name,
                enabled,
                reason: e.user_message(),
            });
            let _ = tx.send(Inbound::Shell(ShellEvent::Toggled { from, result }));
        }
    }
}

/// [`LaneReadiness`] → 「能不能据此建一条 lane」的结论（含该做什么）。
fn ready_base_url(items: &[CtlInstance], name: &str) -> Result<String, String> {
    match lane_readiness(items, name) {
        LaneReadiness::Ready(url) => Ok(url),
        LaneReadiness::Pending => Err(format!(
            "实例 {name} 还没就绪（`/ctl` 可复查；就绪后再 `/lane attach {name}`）"
        )),
        LaneReadiness::Failed(why) => Err(why),
        LaneReadiness::Missing => Err(format!(
            "`instances.toml` 里没有 {name}（`/ctl` 看现有实例名，或 `/lane new {name}` 新建）"
        )),
    }
}

/// `add` 之后轮询到就绪。
///
/// 必须轮询：`add` 的响应只有 `created`，端口与 URL 要等 ctl 收敛完才出现在 `list` 里。
fn add_then_wait_ready(
    socket: &Path,
    name: &str,
    workspace: &str,
    profile: Option<&str>,
) -> Result<String, String> {
    add_instance(socket, name, workspace, profile).map_err(|e| e.user_message())?;
    let deadline = Instant::now() + LANE_READY_TIMEOUT;
    loop {
        let items = list_instances(socket).map_err(|e| e.user_message())?;
        match lane_readiness(&items, name) {
            LaneReadiness::Ready(url) => return Ok(url),
            LaneReadiness::Failed(why) => return Err(why),
            // `Missing` 也继续等：声明刚落盘、ctl 还没读到它。
            LaneReadiness::Pending | LaneReadiness::Missing => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "实例 {name} 在 {} s 内未就绪（可能仍在启动）：`/ctl` 复查，就绪后 `/lane attach {name}`",
                LANE_READY_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(LANE_READY_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    fn inst(name: &str, observed: &str, url: Option<&str>) -> CtlInstance {
        CtlInstance {
            name: name.to_string(),
            observed: Some(observed.to_string()),
            base_url: url.map(str::to_string),
            ..CtlInstance::default()
        }
    }

    /// 在临时目录里起一个假控制面：按 `replies` 顺序，每个连接回一条响应，
    /// 并把收到的请求行按序收回来（好断言「壳层真的把字段发出去了」）。
    ///
    /// 与 `serve::ctl` 的测试各自成一份：那边的假服务端只回一次，而 `/lane new`
    /// 要「先 add 再 list」，两次请求必须由同一个假服务端接。
    fn fake_ctl(replies: &'static [&'static str]) -> (PathBuf, thread::JoinHandle<Vec<String>>) {
        let dir = std::env::temp_dir().join(format!(
            "crabmate-ctl-worker-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let sock = dir.join("ctl.sock");
        let listener = UnixListener::bind(&sock).expect("绑定 socket");
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut request = String::new();
                let _ = BufReader::new(&stream).read_line(&mut request);
                seen.push(request.trim().to_string());
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.write_all(b"\n");
                let _ = stream.flush();
            }
            seen
        });
        (sock, handle)
    }

    #[test]
    fn ready_instance_yields_its_base_url() {
        let items = vec![inst("a", "ready", Some("http://127.0.0.1:8721/"))];
        assert_eq!(
            ready_base_url(&items, "a"),
            Ok("http://127.0.0.1:8721/".to_string())
        );
    }

    /// 还没就绪时给的是「再等一下」的指引，而不是失败结论。
    #[test]
    fn pending_instance_points_at_ctl_and_attach() {
        let items = vec![inst("a", "starting", None)];
        let err = ready_base_url(&items, "a").unwrap_err();
        assert!(err.contains("还没就绪"), "{err}");
        assert!(err.contains("/lane attach a"), "要给出下一步命令：{err}");
    }

    /// 名字写错与实例起不来要分开说：前者是「查名字」，后者是「看原因」。
    #[test]
    fn missing_instance_suggests_new_and_ctl() {
        let err = ready_base_url(&[], "a").unwrap_err();
        assert!(err.contains("没有 a"), "{err}");
        assert!(err.contains("/lane new a"), "{err}");
    }

    #[test]
    fn failed_instance_reports_the_server_reason() {
        let mut failed = inst("a", "failed", None);
        failed.last_error = Some("工作区不在白名单".to_string());
        let err = ready_base_url(&[failed], "a").unwrap_err();
        assert_eq!(err, "工作区不在白名单");
    }

    /// `/ctl` 的回包必须按**发起它的**那条 lane 打标签。
    ///
    /// 执行已经搬到外壳级（issue #350），归属不能跟着搬走：裸发会落进当前聚焦的
    /// transcript，切回来就看不到自己那条命令的结果了。
    #[test]
    fn list_result_is_tagged_with_the_requesting_lane() {
        let (sock, handle) = fake_ctl(&[
            r#"{"ok":true,"instances":[{"name":"proj-a","observed":"ready","base_url":"http://127.0.0.1:8721"}]}"#,
        ]);
        let (tx, rx) = mpsc::channel();
        let ctl = spawn(sock.clone(), tx);
        ctl.enqueue(CtlJob::List { from: LaneId(3) }).expect("派发");
        // 通道关掉不影响已入队的 job：worker 仍会跑完这一次再退出。
        drop(ctl);
        match rx.recv_timeout(Duration::from_secs(5)).expect("应有回包") {
            Inbound::FromLane(LaneId(3), UiEvent::CtlList(Ok(items))) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].name, "proj-a");
            }
            _ => panic!("应按 LaneId(3) 分发 UiEvent::CtlList"),
        }
        handle.join().expect("假服务端线程");
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }

    /// 队列位置来自计数：第一条是立即，第二条已经知道自己前面还有人（issue #350）。
    ///
    /// 这句提示必须由**发起方**在按下时写出来 —— ctl 线程轮到它才知道自己排第几，
    /// 那时用户已经干等了几秒。
    #[test]
    fn enqueue_reports_how_many_calls_are_ahead() {
        let (tx, _rx) = mpsc::channel();
        let ctl = CtlHandle::detached(tx);
        assert!(
            matches!(
                ctl.enqueue(CtlJob::List { from: LaneId(1) }),
                Ok(Queued::Immediate)
            ),
            "前面没人时不该报排队"
        );
        match ctl.enqueue(CtlJob::List { from: LaneId(2) }) {
            Ok(Queued::Behind(ahead)) => assert_eq!(ahead, 1, "前面正好压着第一条"),
            _ => panic!("第二条应报排队"),
        }
    }

    /// 发不出去时不能占着计数：否则之后每条命令都会先报「前面还有 N 条」再报发不出去。
    #[test]
    fn failed_enqueue_does_not_leave_the_counter_raised() {
        let (tx, rx) = mpsc::channel();
        let ctl = CtlHandle::detached(tx);
        drop(rx);
        assert!(ctl.enqueue(CtlJob::List { from: LaneId(1) }).is_err());
        assert_eq!(ctl.in_flight.load(Ordering::SeqCst), 0);
    }

    /// `/lane new` 失败（ctl 报 `failed`）时，原因回到**发起命令那条 lane** 的外壳事件，
    /// 并且 `profile` 按名字进 `add` 的请求行（壳层只转发，不做校验）。
    #[test]
    fn new_failure_returns_to_the_requesting_lane() {
        let (sock, handle) = fake_ctl(&[
            r#"{"ok":true,"name":"proj-c","created":true}"#,
            r#"{"ok":true,"instances":[{"name":"proj-c","observed":"failed","last_error":"工作区不在白名单"}]}"#,
        ]);
        let (tx, rx) = mpsc::channel();
        let ctl = spawn(sock.clone(), tx);
        ctl.enqueue(CtlJob::New {
            from: LaneId(7),
            name: "proj-c".to_string(),
            workspace: "/home/u/proj-c".to_string(),
            profile: Some("readonly-audit".to_string()),
        })
        .expect("派发");
        drop(ctl);
        match rx.recv_timeout(Duration::from_secs(5)).expect("应有回包") {
            Inbound::Shell(ShellEvent::Ready { from, name, result }) => {
                assert_eq!(from, LaneId(7), "回执回到发起它的 lane");
                assert_eq!(name, "proj-c");
                assert_eq!(result.unwrap_err(), "工作区不在白名单");
            }
            _ => panic!("应回 ShellEvent::Ready"),
        }
        let seen = handle.join().expect("假服务端线程");
        assert!(
            seen.first()
                .is_some_and(|r| r.contains(r#""profile":"readonly-audit""#)),
            "profile 要原样进 add 的请求行：{seen:?}"
        );
        let _ = std::fs::remove_dir_all(sock.parent().unwrap());
    }
}
