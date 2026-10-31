//! `/ctl` 的输出格式化：把控制面实例列表排成人能读的几行。
//!
//! 只做排版，不做判定 —— 可连性来自 [`CtlInstance::can_connect`]（`observed == ready`）。
//! 单独成模块与成文件：这段要能单测（不该只能靠起一个真 socket 才验证得到文案）。

use crabmate_client_api::ctl::{CtlInstance, CtlObserved};

use super::state::LineKind;
use super::worker_ctl::{CtlHandle, CtlJob, Queued};

/// 控制面未启用时的提示（`/ctl` 与 `/lane` 共用：两处各写一份必然漂移）。
const CTL_DISABLED_HINT: &str = "ctl 控制面未启用：启动时加 `--ctl-socket <path>`，或让 $XDG_RUNTIME_DIR/crabmate/ctl.sock 存在";

impl super::LaneApp<'_> {
    /// 取控制面通道句柄；未启用时写一句「怎么启用」并返回 `None`。
    pub(super) fn ctl_handle_or_hint(&mut self) -> Option<CtlHandle> {
        let ctl = self.ctl.clone();
        if ctl.is_none() {
            self.st.push_line(LineKind::System, CTL_DISABLED_HINT);
        }
        ctl
    }

    /// 派一次控制面调用；排在别人后面时先解释一句。
    ///
    /// 返回 `false` = 没发出去（ctl 线程已退出）：失败文案留给调用方 —— 四条命令的措辞
    /// 不同（查询 / 连接 / 新建 / 翻转），这里替它们决定只会多一处漂移。
    ///
    /// 排队提示必须在**发送侧**写（issue #350）：控制面只有一个线程，`/lane new` 的
    /// 20 s 就绪轮询期间别人敲的命令就排在那次轮询后面，而「正在查询…」早已写到屏上
    /// 了 —— 不解释，用户看到的就是一条卡住的命令。
    pub(super) fn enqueue_ctl(&mut self, ctl: &CtlHandle, job: CtlJob) -> bool {
        match ctl.enqueue(job) {
            Ok(Queued::Immediate) => true,
            Ok(Queued::Behind(ahead)) => {
                self.st.push_line(LineKind::System, &queue_notice(ahead));
                true
            }
            Err(_) => false,
        }
    }

    /// `/ctl` 与 `/lane`（空参）：把一次控制面查询派给**外壳级**的 ctl 线程。
    ///
    /// 未启用时不发请求、也不报错，而是说清「怎么启用」——这条通道存在的全部前提是
    /// 同机同用户的 `crabmatectl supervise --socket`，静默什么都不做会让人以为命令写错了。
    ///
    /// 不排进**本 lane 的** worker（issue #350）：那个队列装着回合与各种拉取，
    /// 一次最坏 5 s 的控制面 IO 排在后面，整条 lane 的界面就停更了。
    pub(super) fn fetch_ctl_list(&mut self) {
        let Some(ctl) = self.ctl_handle_or_hint() else {
            return;
        };
        self.st.push_line(LineKind::System, "正在查询 ctl 实例…");
        if !self.enqueue_ctl(&ctl, CtlJob::List { from: self.id }) {
            self.st
                .push_line(LineKind::System, "ctl 查询未发出：worker 线程已退出");
        }
    }
}

/// 「排在别人后面」的解释。
///
/// 纯函数：这句只在多条 lane 抢同一个 ctl 时出现，手敲两条命令几乎撞不到，只能单测文案。
/// 说清两件事 —— 为什么没反应（ctl 是单写者、逐条执行），以及在等什么（`/lane new`
/// 的就绪轮询最长 20 s，是唯一会占位那么久的调用）。
fn queue_notice(ahead: usize) -> String {
    format!(
        "控制面调用排队中：前面还有 {ahead} 条未返回（ctl 是单写者，逐条执行）——`/lane new` 的就绪轮询最长 20 s，通常在等它"
    )
}

/// `UiEvent::CtlList`：把查询结果写进发起它的那条 lane 的 transcript。
///
/// 失败只有一句（`CtlCallError::user_message` 已经把「该做什么」写进去了），
/// 成功则是摘要 + 每条实例一行 —— 都在 [`listing_lines`] 里定，便于单测。
pub(super) fn on_ctl_list(lane: &mut super::LaneApp<'_>, result: Result<Vec<CtlInstance>, String>) {
    match result {
        Ok(items) => {
            for line in listing_lines(&items) {
                lane.st.push_line(LineKind::System, &line);
            }
        }
        Err(message) => lane.st.push_line(LineKind::System, &message),
    }
}

/// 列表摘要 + 每条实例一行。
///
/// 不可连的条目前缀用 `·` 而不是 `•`：一屏里「能连哪几条」要能一眼扫出来。每条都带
/// 观测态原文（`ready` / `stale` …），因为**只有 `observed` 说的才算**，`state` 不在
/// 这里显示 —— 两个都显示会让人以为可以二选一。
#[must_use]
pub fn listing_lines(items: &[CtlInstance]) -> Vec<String> {
    if items.is_empty() {
        return vec![
            "ctl: 声明里没有实例（`instances.toml` 为空）；新建请先改声明或用 `crabmatectl up`"
                .to_string(),
        ];
    }
    let connectable = items.iter().filter(|i| i.can_connect()).count();
    let mut out = Vec::with_capacity(items.len() + 1);
    out.push(format!("ctl 实例 {} 条（可连 {connectable}）", items.len()));
    out.extend(items.iter().map(item_line));
    out
}

/// 单条实例：`• 名字 [观测态] :端口 工作区`，失败原因附在末尾。
fn item_line(item: &CtlInstance) -> String {
    let observed = item.observed_state();
    let mark = if observed.can_connect() { "•" } else { "·" };
    let mut line = format!("{mark} {} [{}]", item.name, observed_label(&observed));
    if let Some(port) = item.port {
        line.push_str(&format!(" :{port}"));
    }
    if let Some(ws) = item
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
    {
        line.push_str(&format!(" {ws}"));
    }
    if let Some(err) = item
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        line.push_str(&format!("（上次错误：{err}）"));
    }
    line
}

/// 观测态的人读标签。
///
/// 认不出的取值**原样显示**并标注「未识别」：把它折成 `ready` 或 `starting` 都会
/// 让用户对着一个与实际不符的状态做决定。
fn observed_label(observed: &CtlObserved) -> String {
    match observed {
        CtlObserved::Ready => "ready".to_string(),
        CtlObserved::Starting => "starting 未就绪".to_string(),
        CtlObserved::Stale => "stale 连不上".to_string(),
        CtlObserved::Stopped => "stopped".to_string(),
        CtlObserved::Failed => "failed".to_string(),
        CtlObserved::Held => "held".to_string(),
        CtlObserved::Absent => "无 observed".to_string(),
        CtlObserved::Unknown(raw) => format!("{raw} 未识别"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(name: &str, observed: Option<&str>) -> CtlInstance {
        CtlInstance {
            name: name.to_string(),
            observed: observed.map(str::to_string),
            ..CtlInstance::default()
        }
    }

    #[test]
    fn empty_list_says_so_instead_of_printing_nothing() {
        let lines = listing_lines(&[]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("声明里没有实例"), "{:?}", lines[0]);
    }

    #[test]
    fn summary_counts_only_connectable_instances() {
        let items = vec![
            inst("a", Some("ready")),
            inst("b", Some("starting")),
            inst("c", Some("ready")),
        ];
        let lines = listing_lines(&items);
        assert!(lines[0].contains("3 条"), "{}", lines[0]);
        assert!(lines[0].contains("可连 2"), "{}", lines[0]);
        // 每条实例各一行（+1 摘要）。
        assert_eq!(lines.len(), 4);
    }

    /// `stale` 必须显眼：它的 `state` 可能写着 `ready`，只有 `observed` 说连不上。
    #[test]
    fn stale_is_marked_not_connectable() {
        let item = CtlInstance {
            name: "proj-a".to_string(),
            state: Some("ready".to_string()),
            observed: Some("stale".to_string()),
            ..CtlInstance::default()
        };
        let lines = listing_lines(&[item]);
        assert!(lines[1].starts_with("· "), "不可连用 · 前缀：{}", lines[1]);
        assert!(lines[1].contains("stale 连不上"), "{}", lines[1]);
        assert!(lines[0].contains("可连 0"), "{}", lines[0]);
    }

    #[test]
    fn missing_observed_is_reported_not_guessed() {
        let lines = listing_lines(&[inst("a", None)]);
        assert!(lines[1].contains("无 observed"), "{}", lines[1]);
        assert!(lines[0].contains("可连 0"), "{}", lines[0]);
    }

    #[test]
    fn unknown_observed_is_echoed_verbatim() {
        let lines = listing_lines(&[inst("a", Some("draining"))]);
        assert!(lines[1].contains("draining 未识别"), "{}", lines[1]);
    }

    #[test]
    fn port_workspace_and_last_error_are_appended() {
        let item = CtlInstance {
            name: "proj-a".to_string(),
            observed: Some("failed".to_string()),
            port: Some(8721),
            workspace: Some("/home/u/proj-a".to_string()),
            last_error: Some("端口被占用".to_string()),
            ..CtlInstance::default()
        };
        let line = &listing_lines(&[item])[1];
        assert!(line.contains(":8721"), "{line}");
        assert!(line.contains("/home/u/proj-a"), "{line}");
        assert!(line.contains("端口被占用"), "{line}");
    }

    /// 空工作区（待绑定）不该留下悬空的分隔符。
    #[test]
    fn blank_workspace_leaves_no_dangling_text() {
        let item = CtlInstance {
            name: "proj-a".to_string(),
            observed: Some("ready".to_string()),
            workspace: Some("   ".to_string()),
            ..CtlInstance::default()
        };
        let line = &listing_lines(&[item])[1];
        assert_eq!(line.trim_end(), "• proj-a [ready]");
    }
}
