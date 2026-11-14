//! `/ctl` 的输出格式化：把控制面实例列表排成人能读的几行。
//!
//! 只做排版，不做判定 —— 可连性来自 [`CtlInstance::can_connect`]（`observed == ready`）。
//! 单独成模块与成文件：这段要能单测（不该只能靠起一个真 socket 才验证得到文案）。

use crabmate_client_api::ctl::{CtlInstance, CtlObserved, RestartOutcome};

use super::state::LineKind;
use super::worker::CtlReport;
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
/// 说清两件事 —— 为什么没反应（ctl 是单写者、逐条执行），以及在等什么。两个会长时间
/// 占位的调用都要点名：`/lane new` 的就绪轮询（最长 20 s）与 `/lane restart`（**同步**
/// op，最长约 50 s）；只说前者会让一次重启看起来像卡死。
fn queue_notice(ahead: usize) -> String {
    format!(
        "控制面调用排队中：前面还有 {ahead} 条未返回（ctl 是单写者，逐条执行）——`/lane new` 的就绪轮询最长 20 s、`/lane restart` 最长约 {} s，通常在等它们之一",
        crabmate_client_api::ctl::RESTART_BUDGET_SECS
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

/// `CtlReport::Status`：把**单条**实例的详情写进发起查询的那条 lane。
///
/// 详情走 ctl 的 `status` op（不再 `list` 取全量自己筛）：失败只有一句，带上问的是哪条
/// 实例 —— `instance_not_found` 是「没有状态记录」，不是「名字写错」，两者都说清才不会
/// 让人去改一个本来没写错的名字。
pub(super) fn on_ctl_status(
    lane: &mut super::LaneApp<'_>,
    name: String,
    result: Result<CtlInstance, String>,
) {
    match result {
        Ok(item) => {
            for line in status_lines(&item) {
                lane.st.push_line(LineKind::System, &line);
            }
        }
        Err(message) => lane.st.push_line(
            LineKind::System,
            &format!("实例 `{name}` 的详情查询失败：{message}"),
        ),
    }
}

/// `CtlReport::Restarted`：写重启报告，并在「连的就是它」时说清连接已失效。
///
/// 失败带错误码（`CtlCallError::user_message` 已含 code + Server 原文）：这正是不让用户
/// 对着一次重启「等一个没有回音的界面」的那句话。
pub(super) fn on_ctl_restarted(
    lane: &mut super::LaneApp<'_>,
    name: String,
    result: Result<RestartOutcome, String>,
) {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(message) => {
            lane.st
                .push_line(LineKind::System, &format!("重启 `{name}` 失败：{message}"));
            return;
        }
    };
    for line in restart_report_lines(&outcome) {
        lane.st.push_line(LineKind::System, &line);
    }
    if let Some(hint) = self_lane_hint(lane, &outcome) {
        lane.st.push_line(LineKind::System, &hint);
    }
}

/// `UiEvent::CtlReport`：按载荷分派到「详情」或「重启报告」。
///
/// 两者在 `mod.rs` 的 `on_msg` 里共用**一条**分派（那个文件贴着 920 行门禁），差别只在
/// 排版，所以分支留在这里 —— 两条渲染函数各自可单测。
pub(super) fn on_ctl_report(lane: &mut super::LaneApp<'_>, name: String, report: CtlReport) {
    match report {
        CtlReport::Status(result) => on_ctl_status(lane, name, result),
        CtlReport::Restarted(result) => on_ctl_restarted(lane, name, result),
    }
}

/// 单条实例的详情（`/lane status <name>`）。
///
/// 字段按「能不能连 / 连到哪 / 声明里写了什么」分三块，缺的字段写成 `—` 而不是省略：
/// 详情视图的用处正是「对照 Server 到底说了什么」，行数随字段忽多忽少反而更难核。
#[must_use]
pub fn status_lines(item: &CtlInstance) -> Vec<String> {
    let observed = item.observed_state();
    let mut out = vec![
        format!("实例 `{}`（ctl `status`）", item.name),
        format!(
            "  状态：state {} · observed {}（{}）",
            field(item.state.as_deref()),
            observed_label(&observed),
            if observed.can_connect() {
                "可连"
            } else {
                "不可连"
            },
        ),
    ];
    out.extend(endpoint_line(item));
    out.extend(declaration_line(item));
    if let Some(err) = non_empty(item.last_error.as_deref()) {
        out.push(format!("  上次错误：{err}"));
    }
    out
}

/// 「连到哪」那一行：地址 / 端口 / pid（三者都缺时整行不出现）。
fn endpoint_line(item: &CtlInstance) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(url) = non_empty(item.base_url.as_deref()) {
        parts.push(format!("地址 {url}"));
    }
    if let Some(port) = item.port {
        parts.push(format!("端口 {port}"));
    }
    if let Some(pid) = item.pid {
        parts.push(format!("pid {pid}"));
    }
    (!parts.is_empty()).then(|| format!("  {}", parts.join(" · ")))
}

/// 「声明里写了什么」那一行：期望态 / `enabled` / 工作区 / profile / 谁拉起的。
fn declaration_line(item: &CtlInstance) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(desired) = non_empty(item.desired_state.as_deref()) {
        parts.push(format!("期望 {desired}"));
    }
    if let Some(enabled) = item.enabled {
        parts.push(format!("enabled = {enabled}"));
    }
    if let Some(ws) = non_empty(item.workspace.as_deref()) {
        parts.push(format!("工作区 {ws}"));
    }
    if let Some(profile) = non_empty(item.profile.as_deref()) {
        parts.push(format!("profile {profile}"));
    }
    if let Some(by) = non_empty(item.managed_by.as_deref()) {
        parts.push(format!("由 {by} 管理"));
    }
    (!parts.is_empty()).then(|| format!("  {}", parts.join(" · ")))
}

/// 重启报告：一句结论 + 新 pid / 地址 + Server 的 `notes`（逐条缩进）。
///
/// `notes` 必须露出来：它是「env 丢了 / profile 漂移 / 声明其实 `enabled = false`」的唯一
/// 出口，吞掉就等于重启看起来成功、行为却和预期不一样。
#[must_use]
pub fn restart_report_lines(outcome: &RestartOutcome) -> Vec<String> {
    let item = &outcome.instance;
    let head = if outcome.restarted {
        format!(
            "已重启 `{}`：state {} · observed {}",
            item.name,
            field(item.state.as_deref()),
            observed_label(&item.observed_state()),
        )
    } else {
        // `restarted = false` 不是失败，但也**不是**「重启成功」：照 Server 的原话说。
        format!(
            "`{}` 未重启（Server 回 restarted = false）：state {}",
            item.name,
            field(item.state.as_deref()),
        )
    };
    let mut out = vec![head];
    if let Some(pid) = item.pid {
        out.push(format!("  新 pid：{pid}"));
    }
    if let Some(url) = non_empty(item.base_url.as_deref()) {
        out.push(format!("  地址：{url}"));
    }
    for note in &outcome.notes {
        out.push(format!("  ! {note}"));
    }
    out
}

/// 本 lane 连的正是被重启的那条实例时，说明「连接已换进程」该怎么办。
///
/// `restart` 会**杀掉并重起**进程：本进程那条 SSE 已经断了；若新实例还换了端口，
/// 这条 lane 的 `api_base` 就指向一个死地址。两种都不会自己好转 —— 不说出来，用户看到的
/// 就是「聊着聊着不回了」。**不自动重连**：那会擅自改掉用户在用的连接，而 `detach` +
/// `attach` 是他自己能看清后果的两步。
#[must_use]
fn self_lane_hint(lane: &super::LaneApp<'_>, outcome: &RestartOutcome) -> Option<String> {
    if lane.label != outcome.instance.name {
        return None;
    }
    let base = non_empty(outcome.instance.base_url.as_deref())?;
    let name = &outcome.instance.name;
    let same = crate::serve::same_api_base(&lane.client.config().api_base, base);
    Some(if same {
        format!(
            "本 lane 连的就是它：进程已换、SSE 已断开 —— `/lane detach {name}` 再 `/lane attach {name}` 重连"
        )
    } else {
        format!(
            "本 lane 连的就是它，但新实例换了地址（{base}）：先 `/lane detach {name}`，再 `/lane attach {name}` 连新地址"
        )
    })
}

/// 可选文本字段的非空取值（空白也算缺省）。
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// 详情里缺省字段的占位：字段位要保持稳定，不能一会儿有一会儿没有。
fn field(value: Option<&str>) -> &str {
    non_empty(value).unwrap_or("—")
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

    /// 详情走 `status`：状态 / 连到哪 / 声明三块都要能对照，可连性仍取自 `observed`。
    #[test]
    fn status_lines_cover_state_endpoint_and_declaration() {
        let item = CtlInstance {
            name: "proj-a".to_string(),
            state: Some("starting".to_string()),
            desired_state: Some("running".to_string()),
            observed: Some("starting".to_string()),
            base_url: Some("http://127.0.0.1:8081/".to_string()),
            port: Some(8081),
            pid: Some(4242),
            workspace: Some("/srv/a".to_string()),
            profile: Some("heavy".to_string()),
            enabled: Some(true),
            managed_by: Some("supervise".to_string()),
            ..CtlInstance::default()
        };
        let text = status_lines(&item).join("\n");
        assert!(text.contains("state starting"), "{text}");
        assert!(text.contains("不可连"), "starting 不该说成可连：{text}");
        assert!(text.contains("127.0.0.1:8081"), "{text}");
        assert!(text.contains("pid 4242"), "{text}");
        assert!(text.contains("期望 running"), "{text}");
        assert!(text.contains("由 supervise 管理"), "{text}");
    }

    /// 字段全缺时仍保留字段位（写 `—`）：详情视图要能看出「Server 就是没给这个值」。
    #[test]
    fn status_lines_never_drop_a_field_slot() {
        let lines = status_lines(&inst("a", None));
        assert!(lines[1].contains("state —"), "{:?}", lines);
        assert!(lines[1].contains("observed 无 observed"), "{:?}", lines);
    }

    /// 重启报告要把 `notes` 逐条摆出来：那是「env 丢了」唯一能露面的地方。
    #[test]
    fn restart_report_surfaces_notes_and_the_new_pid() {
        let outcome = RestartOutcome {
            instance: CtlInstance {
                name: "proj-a".to_string(),
                state: Some("ready".to_string()),
                observed: Some("ready".to_string()),
                pid: Some(4243),
                base_url: Some("http://127.0.0.1:8081/".to_string()),
                ..CtlInstance::default()
            },
            restarted: true,
            notes: vec!["声明里的 env 已丢失（3 项）".to_string()],
        };
        let text = restart_report_lines(&outcome).join("\n");
        assert!(text.contains("已重启 `proj-a`"), "{text}");
        assert!(text.contains("新 pid：4243"), "{text}");
        assert!(text.contains("env 已丢失"), "notes 不能吞掉：{text}");
    }

    /// `restarted = false` 不写成「已重启」：那是 Server 的说法，别替它加戏。
    #[test]
    fn restart_report_says_so_when_nothing_was_restarted() {
        let outcome = RestartOutcome {
            instance: inst("a", Some("stopped")),
            restarted: false,
            notes: Vec::new(),
        };
        let text = restart_report_lines(&outcome).join("\n");
        assert!(text.contains("未重启"), "{text}");
        assert!(!text.contains("已重启"), "{text}");
    }
}
