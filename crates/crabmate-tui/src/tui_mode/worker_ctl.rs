//! 控制面（`crabmatectl supervise --socket`）的 worker 任务：列实例 / 连一条 / 新建一条。
//!
//! 从 `worker.rs` 拆出：那边的 job 派发已经贴着 `fn-nloc-ratchet` 的 920 行/文件上限，
//! 而这几件事只经 unix socket、完全不碰 `reqwest`，与 HTTP 拉取不是一类。

use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crabmate_client_api::ctl::{CtlInstance, LaneReadiness, lane_readiness};

use super::worker::{LaneTx, UiEvent};
use crate::serve::{add_instance, list_instances};

/// `/lane new` 等实例就绪的轮询间隔。
const LANE_READY_POLL: Duration = Duration::from_millis(300);

/// 等实例就绪的上限：`up` 自己的 readiness 超时是 10 s，这里留一倍余量。
///
/// 到点**不是**「实例起不来」的结论（它可能仍在起），所以文案分开说 ——
/// 把还在启动的实例报成失败，会让人去重试一件正在发生的事。
const LANE_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// `/ctl` 与 `/lane`（空参）：拉一次实例列表。
///
/// 回包属于**发起这次 `/ctl` 的 lane**，所以要打标签：裸发会把它写进当前聚焦的
/// 那条 transcript（切走之后再回来就看不到自己那条命令的结果了）。
pub(super) fn job_ctl_list(socket: &Path, out: &LaneTx) {
    let result = list_instances(socket).map_err(|e| e.user_message());
    let _ = out.send(UiEvent::CtlList(result));
}

/// `/lane attach`：按名字在声明里找一条已就绪的实例，把 `base_url` 带回去。
pub(super) fn job_ctl_attach(socket: &Path, name: &str, tx: &Sender<UiEvent>) {
    let result = list_instances(socket)
        .map_err(|e| e.user_message())
        .and_then(|items| ready_base_url(&items, name));
    let _ = tx.send(UiEvent::CtlLaneReady {
        name: name.to_string(),
        result,
    });
}

/// `/lane new`：写声明 → 轮询到就绪。
pub(super) fn job_ctl_new(socket: &Path, name: &str, workspace: &str, tx: &Sender<UiEvent>) {
    let result = add_then_wait_ready(socket, name, workspace);
    let _ = tx.send(UiEvent::CtlLaneReady {
        name: name.to_string(),
        result,
    });
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
fn add_then_wait_ready(socket: &Path, name: &str, workspace: &str) -> Result<String, String> {
    add_instance(socket, name, workspace).map_err(|e| e.user_message())?;
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

    fn inst(name: &str, observed: &str, url: Option<&str>) -> CtlInstance {
        CtlInstance {
            name: name.to_string(),
            observed: Some(observed.to_string()),
            base_url: url.map(str::to_string),
            ..CtlInstance::default()
        }
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
}
