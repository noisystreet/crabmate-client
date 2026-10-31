//! `/lane`：连接（lane）的**运行期**新增入口。
//!
//! `--connect name=url` 只能在启动时装配（见 `docs/design/tui_multi_server.md` §4.5），
//! 这里补上「进程已经跑起来之后再加一条」。两条路径都经控制面
//! （`crabmatectl supervise --socket`）：**声明的唯一写入者是 ctl** —— 客户端直接写
//! `instances.toml` 会与 `--watch` 构成同一文件的两个写入者，而让客户端自己 spawn
//! `crabmatectl` 又等于把进程管理搬进 TUI。
//!
//! - `/lane`（空参）/ `/lane list`：列出控制面实例（与 `/ctl` 同一实现，不再各写一份）。
//! - `/lane attach <name>`：连上**已就绪**的实例，不改声明。
//! - `/lane detach [<name>]`：只摘掉**本进程**的连接；声明与 serve 进程都不动（与 attach 对称）。
//! - `/lane new <name> [workspace] [profile=<name>]`：新增一条声明并等它就绪（写盘由 ctl 完成）。
//!
//! 失败一律写进**发起命令的那条 lane**的 transcript：用户正看着它，而新 lane 此刻
//! 还不存在，没有别的地方可写。

use super::inbound::{Inbound, ShellEvent};
use super::state::LineKind;
use super::worker_ctl::CtlJob;

/// `/lane` 的子动作。
#[derive(Debug)]
pub(crate) enum LaneCommand {
    /// 空参 / `list`：列出现有实例（与 `/ctl` 同一实现）。
    List,
    /// `attach <name>`：连上已就绪的实例。
    Attach(String),
    /// `detach [<name>]`：只摘掉本进程的连接（声明与 serve 进程都不动）。
    ///
    /// 名字缺省 = **当前 lane**：执行时由 `LaneApp` 的 `label` 兜底（解析层不知道
    /// 当前是哪条）。想断开的通常就是眼前这条，而它的名字还得先 `/lane` 查一遍才写得对；
    /// 写了名字仍按名字走 —— 「和当前同名」不是改语义的理由。
    Detach(Option<String>),
    /// `cancel <name>`：取消这条 lane 正在跑的回合（不必先切过去）。
    ///
    /// 后台 lane 的审批浮层画不到屏幕上，没有这条命令就只能干等 `APPROVAL_WAIT`
    /// 超时（600 s）才按拒绝收尾（见 issue #337）。
    Cancel(String),
    /// `close <name>`：把声明标成 `enabled = false`（Server 的软删口径）。
    ///
    /// 与 [`LaneCommand::Detach`] 不同：detach 只摘本进程的连接，声明还启用着、进程还在跑；
    /// close 让 ctl 把声明关掉并停掉实例，而声明本身与手写的 `env` / `config` 都留着。
    Close(String),
    /// `open <name>`：撤销 [`LaneCommand::Close`]（`enabled = true`）。
    Open(String),
    /// `new <name> [workspace] [profile=<name>]`：新增声明并等它就绪。
    New {
        /// 实例名（`instances.toml` 的键）。
        name: String,
        /// 显式给的工作区；`None` = 用当前 lane 的 serve 工作区兜底。
        workspace: Option<String>,
        /// `profiles.toml` 里的预设名；`None` = 不指定（Server 用默认）。
        ///
        /// 只按名字引用，**不**在这里校验、也不接受内联配置：`profiles.toml` 在 serve
        /// 那台机器上，客户端猜它有什么只会猜错。
        profile: Option<String>,
    },
}

/// 解析 `/lane` 的参数：`None` 与 `list` 等价，认不出的子命令返回一句可照抄的用法。
pub(crate) fn parse(arg: Option<&str>) -> Result<LaneCommand, String> {
    let Some(rest) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(LaneCommand::List);
    };
    let (sub, tail) = match rest.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (rest, ""),
    };
    match sub.to_ascii_lowercase().as_str() {
        // `list` 后面跟东西就是手误：静默忽略会让人以为参数生效了。
        "list" | "ls" | "show" if tail.is_empty() => Ok(LaneCommand::List),
        "attach" | "use" => one_name(tail, "attach").map(LaneCommand::Attach),
        "detach" => detach_tail(tail),
        "cancel" | "kill" | "interrupt" => one_name(tail, "cancel").map(LaneCommand::Cancel),
        // `disable` / `enable` 是 Server 侧的 op 名，认它们只是省事，语义与 close / open 一致。
        "close" | "disable" => one_name(tail, "close").map(LaneCommand::Close),
        "open" | "enable" => one_name(tail, "open").map(LaneCommand::Open),
        "new" | "add" => new_tail(tail),
        _ => Err(usage()),
    }
}

/// 用法提示：每条子命令各给一个可照抄的例子。
fn usage() -> String {
    "用法：/lane [list] · /lane attach <name> · /lane detach [<name>]（缺省摘当前这条） · /lane cancel <name> · /lane new <name> [workspace] [profile=<name>] · /lane close <name> · /lane open <name>"
        .to_string()
}

/// `detach` 的名字可选：缺省摘**当前 lane**（解析层不知道当前是哪条，交给 `LaneApp` 的
/// `label` 兜底）。这是它与 `attach` 唯一的语法分歧 —— `attach` 无参必然是手误（连哪条
/// 都得说清），而 `detach` 无参有天然默认值：你正看着的那条。
fn detach_tail(tail: &str) -> Result<LaneCommand, String> {
    if tail.is_empty() {
        return Ok(LaneCommand::Detach(None));
    }
    one_name(tail, "detach").map(|n| LaneCommand::Detach(Some(n)))
}

/// `attach` / `detach` 之后只能是**一个**实例名：含空白一律报错，不去猜哪个是名字。
///
/// `verb` 只参与报错文案：两条子命令的名字规则一样，但让人照抄的命令必须是他刚打错的
/// 那条 —— 对 `/lane detach foo bar` 回一句「`/lane attach <name>`」只会更糊涂。
fn one_name(tail: &str, verb: &str) -> Result<String, String> {
    if tail.is_empty() {
        return Err(format!(
            "缺实例名：`/lane {verb} <name>`（用 `/lane` 看现有连接与实例）"
        ));
    }
    if tail.split_whitespace().count() > 1 {
        return Err(format!(
            "实例名不能含空白：`{tail}`（用 `/lane` 看现有名称）"
        ));
    }
    Ok(tail.to_string())
}

/// `new` 之后是 `名字 [工作区]` 或 `名字 workspace=<路径> profile=<名>`。
///
/// 两种写法**不混用**：裸工作区只按第一个空白切（路径可以含空格），而一旦首 token 是
/// `key=value` 形式（判据见 [`looks_like_kv`]），整段尾参就按空白切成参数 —— 混着写时
/// 「这个空格属于路径还是参数」无解，猜错会悄悄换掉用户的工作区，所以宁可报用法。
fn new_tail(tail: &str) -> Result<LaneCommand, String> {
    let (name, rest) = match tail.split_once(char::is_whitespace) {
        Some((n, w)) => (n, w.trim()),
        None => (tail, ""),
    };
    if name.is_empty() {
        return Err("缺实例名：`/lane new <name> [workspace] [profile=<name>]`".to_string());
    }
    let (workspace, profile) = new_tail_args(rest)?;
    Ok(LaneCommand::New {
        name: name.to_string(),
        workspace,
        profile,
    })
}

/// 这段尾参是不是 `key=value` 写法。
///
/// 判据是**首个 token 的 `=` 左侧只含字母**（`workspace` / `profile`，以及用户误写的
/// `port` 之类）。`/data/profile=x` 的左侧带 `/`，仍按路径处理 —— 路径里出现 `=` 不该
/// 让一条合法命令变成语法错误；反过来 `port=8080` 若被当成路径，只会换来一句「工作区
/// 不存在」，说不清真正错在哪。
fn looks_like_kv(rest: &str) -> bool {
    let head = rest.split_whitespace().next().unwrap_or_default();
    match head.split_once('=') {
        Some((key, _)) => !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic()),
        None => false,
    }
}

/// `new` 的尾参：整体是裸工作区，或一列 `workspace=` / `profile=` 键值。
///
/// `profile` 只收**名字**：值的合法性由 Server 侧 `profiles.toml` 决定，客户端不预读
/// 那个文件（它在 serve 那台机器上，本机读到的那份未必是同一份）。
fn new_tail_args(rest: &str) -> Result<(Option<String>, Option<String>), String> {
    if rest.is_empty() {
        return Ok((None, None));
    }
    if !looks_like_kv(rest) {
        // 裸工作区：与 `/lane new <name> <路径>` 的旧写法逐字一致（可含空格）。
        return Ok((Some(rest.to_string()), None));
    }
    let mut workspace = None;
    let mut profile = None;
    for token in rest.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!(
                "认不出 `{token}`：`workspace=` / `profile=` 写法里每个参数都要是 `key=value`（例如 `/lane new <name> workspace=<路径> profile=<名>`）"
            ));
        };
        let slot = match key {
            "workspace" => &mut workspace,
            "profile" => &mut profile,
            other => {
                return Err(format!(
                    "不认识 `{other}=`：`/lane new` 只支持 `workspace=` / `profile=`"
                ));
            }
        };
        if value.is_empty() {
            return Err(format!("`{key}=` 后面缺值"));
        }
        // 重复给同一个键是手误：静默取最后一个会让人以为前一次生效了。
        if slot.replace(value.to_string()).is_some() {
            return Err(format!("`{key}=` 给了不止一次"));
        }
    }
    Ok((workspace, profile))
}

/// 新实例用哪个工作区：显式路径优先，否则拿当前 lane 的 serve 工作区兜底。
///
/// 空白（= serve 的待绑定态）不能拿来建声明：Server 会把它当成一个**空的**路径，
/// 起出来的实例在绑定前工具面不可用，而用户以为自己连上了某个项目。
fn resolve_workspace(explicit: Option<String>, current: Option<&str>) -> Option<String> {
    explicit.or_else(|| {
        current
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_string)
    })
}

/// 提示行里的 profile 片段；没给就是空串。
///
/// 拼成空串而不是换一套文案：**未指定 `profile` 时的提示与加这个参数前逐字一致**，
/// 既有的转录断言（和用户的 muscle memory）都不受影响。
fn profile_note(profile: Option<&str>) -> String {
    profile.map_or_else(String::new, |p| format!("，profile {p}"))
}

impl super::LaneApp<'_> {
    /// `/lane ...` 的分派（解析在 [`parse`]，执行全在 worker 线程）。
    pub(super) fn lane_command(&mut self, cmd: LaneCommand) {
        match cmd {
            LaneCommand::List => self.fetch_ctl_list(),
            LaneCommand::Attach(name) => self.lane_attach(name),
            LaneCommand::Detach(name) => self.lane_detach(name),
            LaneCommand::Cancel(name) => self.lane_cancel(name),
            LaneCommand::Close(name) => self.lane_toggle(name, false),
            LaneCommand::Open(name) => self.lane_toggle(name, true),
            LaneCommand::New {
                name,
                workspace,
                profile,
            } => self.lane_new(name, workspace, profile),
        }
    }

    /// `/lane attach <name>`：只读查询，不动声明。
    fn lane_attach(&mut self, name: String) {
        let Some(ctl) = self.ctl_handle_or_hint() else {
            return;
        };
        self.st
            .push_line(LineKind::System, &format!("正在连接 ctl 实例 `{name}`…"));
        if !self.enqueue_ctl(
            &ctl,
            CtlJob::Attach {
                from: self.id,
                name,
            },
        ) {
            self.st
                .push_line(LineKind::System, "连接未发出：worker 线程已退出");
        }
    }

    /// `/lane detach [<name>]`：请外壳摘掉一条连接（`LaneApp` 没有 `lanes` 的 `&mut`）。
    ///
    /// 名字缺省用**本 lane 的 `label`**：`self` 就是发起命令的那条 lane，而外壳查表用的
    /// 正是 `label`（见 `TuiApp::detach_lane`）。写 `self` 的 `label` 而不是「当前 lane 的
    /// label」——二者在生产里恒等（命令只在自己 lane 的输入行里敲），而从这里看不到外壳。
    ///
    /// 走 [`ShellEvent`]（只到外壳，不进任何 lane）：被摘的可能正是发起命令的这条，
    /// 按 `LaneId` 分发会落空 —— 与 `LaneReady` 同一个理由。不查 ctl：这一步不动声明、
    /// 也不停进程。
    fn lane_detach(&mut self, name: Option<String>) {
        let name = name.unwrap_or_else(|| self.label.clone());
        if self
            .tx
            .send(Inbound::Shell(ShellEvent::Detach { name }))
            .is_err()
        {
            self.st
                .push_line(LineKind::System, "断开未发出：事件通道已关闭");
        }
    }

    /// `/lane cancel <name>`：请外壳取消一条 lane 的回合（`LaneApp` 只能取消自己）。
    ///
    /// 走 [`ShellEvent`]，理由同 [`Self::lane_detach`]：要取消的可能正是这条 lane，
    /// 按 `LaneId` 分发会落回它自己、在 `TuiApp::on_msg` 里变成「自己取消自己」。
    fn lane_cancel(&mut self, name: String) {
        if self
            .tx
            .send(Inbound::Shell(ShellEvent::Cancel { name }))
            .is_err()
        {
            self.st
                .push_line(LineKind::System, "取消未发出：事件通道已关闭");
        }
    }

    /// `/lane close|open`：请控制面翻声明里的 `enabled`（客户端不写 `instances.toml`）。
    ///
    /// 回执由 ctl 线程以 `ShellEvent::Toggled` 发出（理由同 [`Self::lane_detach`]：
    /// 被关的可能正是这条 lane），摘连接 / 措辞都在外壳层 `shell::on_lane_toggled`。
    fn lane_toggle(&mut self, name: String, enabled: bool) {
        let Some(ctl) = self.ctl.clone() else {
            self.st.push_line(
                LineKind::System,
                "控制面未启用：`/lane close` 需要 `crabmatectl supervise --socket`（或 `--ctl-socket <path>`）",
            );
            return;
        };
        let verb = if enabled { "启用" } else { "关闭" };
        // 先写「正在…」再入队：排队提示由 `enqueue_ctl` 追加在后面，两条的顺序才是
        // 「我在做什么 → 为什么还没回」。四条命令都沿用这个次序。
        self.st
            .push_line(LineKind::System, &format!("正在{verb}声明 `{name}`…"));
        let job = CtlJob::Toggle {
            from: self.id,
            name,
            enabled,
        };
        if !self.enqueue_ctl(&ctl, job) {
            self.st
                .push_line(LineKind::System, "请求未发出：worker 线程已退出");
        }
    }

    /// `/lane new <name> [workspace] [profile=<name>]`：写声明 + 等就绪（`add` 由 ctl 线程发）。
    ///
    /// 20 s 的就绪轮询在**外壳级的 ctl 线程**上跑（issue #350），所以这条 lane 的
    /// 会话 / 工作区 / 状态拉取不会排在它后面 —— 命令发出去就立刻返回。
    fn lane_new(&mut self, name: String, workspace: Option<String>, profile: Option<String>) {
        let Some(ctl) = self.ctl_handle_or_hint() else {
            return;
        };
        let Some(workspace) = resolve_workspace(workspace, self.st.workspace_path.as_deref())
        else {
            self.st.push_line(
                LineKind::System,
                "不知道新实例用哪个工作区：本 lane 还没取到 serve 的工作区根，请显式给路径 —— `/lane new <name> <路径>`",
            );
            return;
        };
        self.st.push_line(
            LineKind::System,
            &format!(
                "正在新建 ctl 实例 `{name}`（工作区 {workspace}{}）并等它就绪…",
                profile_note(profile.as_deref())
            ),
        );
        if !self.enqueue_ctl(
            &ctl,
            CtlJob::New {
                from: self.id,
                name,
                workspace,
                profile,
            },
        ) {
            self.st
                .push_line(LineKind::System, "新建未发出：worker 线程已退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_lane_lists_like_ctl() {
        assert!(matches!(parse(None), Ok(LaneCommand::List)));
        assert!(matches!(parse(Some("   ")), Ok(LaneCommand::List)));
        assert!(matches!(parse(Some("list")), Ok(LaneCommand::List)));
        assert!(matches!(parse(Some("LS")), Ok(LaneCommand::List)));
    }

    /// `list` 后面跟参数是手误：宁可报用法，也不要静默当成 list。
    #[test]
    fn list_with_trailing_words_is_rejected() {
        assert!(parse(Some("list proj-a")).is_err());
    }

    #[test]
    fn attach_takes_exactly_one_name() {
        assert!(matches!(
            parse(Some("attach proj-a")),
            Ok(LaneCommand::Attach(n)) if n == "proj-a"
        ));
        // `use` 只是别名，语义相同。
        assert!(matches!(
            parse(Some("use proj-a")),
            Ok(LaneCommand::Attach(n)) if n == "proj-a"
        ));
        assert!(parse(Some("attach")).is_err(), "缺名字要报错");
        assert!(
            parse(Some("attach proj-a extra")).is_err(),
            "两个词不猜哪个是名字"
        );
    }

    /// `detach` 的名字可省略：缺省摘**当前 lane**（执行侧用本 lane 的 `label` 兜底）。
    ///
    /// 显式名字仍按名字走；两个词照旧报错 —— 「空参合法」不等于可以糊弄多参。
    #[test]
    fn detach_defaults_to_the_current_lane() {
        assert!(matches!(
            parse(Some("detach")),
            Ok(LaneCommand::Detach(None))
        ));
        assert!(matches!(
            parse(Some("detach proj-a")),
            Ok(LaneCommand::Detach(Some(n))) if n == "proj-a"
        ));
        assert!(parse(Some("detach a b")).is_err(), "两个词不猜哪个是名字");
    }

    /// `close` / `open` 与 detach 一样只收一个名字；`disable` / `enable` 是同义别名。
    #[test]
    fn close_and_open_take_exactly_one_name() {
        assert!(matches!(
            parse(Some("close proj-a")),
            Ok(LaneCommand::Close(n)) if n == "proj-a"
        ));
        assert!(matches!(
            parse(Some("disable proj-a")),
            Ok(LaneCommand::Close(n)) if n == "proj-a"
        ));
        assert!(matches!(
            parse(Some("open proj-a")),
            Ok(LaneCommand::Open(n)) if n == "proj-a"
        ));
        assert!(matches!(
            parse(Some("enable proj-a")),
            Ok(LaneCommand::Open(n)) if n == "proj-a"
        ));
        let err = parse(Some("close")).unwrap_err();
        assert!(err.contains("/lane close"), "要指回刚打的那条：{err}");
        assert!(parse(Some("close a b")).is_err(), "两个词不猜哪个是名字");
    }

    /// 无参 `/lane detach` 在外壳侧摘的就是**这条 lane**：`LaneApp` 用自身 `label` 兜底。
    ///
    /// 解析层只能给出 `None`（它不知道当前是哪条），所以这条接线只能在 `LaneApp` 上验。
    #[test]
    fn detach_without_name_targets_this_lane() {
        use std::sync::mpsc;

        use crate::tui_mode::inbound::{Inbound, ShellEvent};
        use crate::tui_mode::test_support::{lane_with_tx, test_client, test_prefs};
        use crate::tui_mode::worker::LaneId;

        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut lane = lane_with_tx(LaneId(7), &client, &mut prefs, tx, job_tx);

        lane.lane_command(LaneCommand::Detach(None));

        match rx.recv().expect("应发出 Detach") {
            Inbound::Shell(ShellEvent::Detach { name }) => {
                assert_eq!(name, "lane-7", "缺省摘当前这条")
            }
            _ => panic!("应发 ShellEvent::Detach"),
        }
    }

    /// `/lane new` 不再占用本 lane 的 worker（issue #350）：它排进**外壳级** ctl 通道，
    /// `job_tx` 一个任务都收不到。
    ///
    /// 这是那个 issue 的核心验收：20 s 的就绪轮询原先排在本 lane 的 worker 队列里，
    /// 期间该 lane 的会话 / 工作区 / 状态刷新全都排队。现在「发出去就返回」。
    #[test]
    fn lane_new_goes_to_the_ctl_plane_not_the_lane_worker() {
        use std::sync::mpsc;

        use crate::tui_mode::test_support::{lane_with_tx, test_client, test_prefs};
        use crate::tui_mode::worker::LaneId;
        use crate::tui_mode::worker_ctl::CtlHandle;

        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, _rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel();
        let (ctl_tx, ctl_rx) = mpsc::channel();
        let mut lane = lane_with_tx(LaneId(4), &client, &mut prefs, tx, job_tx);
        lane.ctl = Some(CtlHandle::detached(ctl_tx));

        // 工作区给死：本 lane 还没拉过 `GET /workspace`。
        lane.lane_new(
            "proj-c".to_string(),
            Some("/home/u/proj-c".to_string()),
            None,
        );

        match ctl_rx.try_recv() {
            Ok(CtlJob::New {
                from,
                name,
                workspace,
                profile,
            }) => {
                assert_eq!(from, LaneId(4), "回执要回到发起它的 lane");
                assert_eq!(name, "proj-c");
                assert_eq!(workspace, "/home/u/proj-c");
                assert_eq!(profile, None, "没给 profile 就不该凭空造一个");
            }
            _ => panic!("应派 CtlJob::New 给外壳级 ctl 通道"),
        }
        assert!(
            job_rx.try_recv().is_err(),
            "本 lane 的 worker 不该收到任何任务"
        );
    }

    /// `profile` 原样进 `CtlJob::New`，并在「正在新建…」那句提示里露出来。
    ///
    /// 提示行必须带上它：否则用户敲了 `profile=readonly-audit` 却没有任何回显，
    /// 只能事后靠 `/ctl` 确认那条声明到底用了哪个预设。
    #[test]
    fn lane_new_forwards_profile_and_says_which_one() {
        use std::sync::mpsc;

        use crate::tui_mode::test_support::{lane_with_tx, test_client, test_prefs};
        use crate::tui_mode::worker::LaneId;
        use crate::tui_mode::worker_ctl::CtlHandle;

        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, _rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ctl_tx, ctl_rx) = mpsc::channel();
        let mut lane = lane_with_tx(LaneId(6), &client, &mut prefs, tx, job_tx);
        lane.ctl = Some(CtlHandle::detached(ctl_tx));

        lane.lane_new(
            "proj-d".to_string(),
            Some("/home/u/proj-d".to_string()),
            Some("readonly-audit".to_string()),
        );

        match ctl_rx.try_recv() {
            Ok(CtlJob::New { profile, .. }) => {
                assert_eq!(profile.as_deref(), Some("readonly-audit"));
            }
            _ => panic!("应派 CtlJob::New 给外壳级 ctl 通道"),
        }
        let texts: Vec<String> = lane.st.lines.iter().map(|l| l.text().to_string()).collect();
        assert!(
            texts.iter().any(|t| t.contains("readonly-audit")),
            "提示行要说清用了哪个 profile：{texts:?}"
        );
    }

    /// 控制面被占着时，第二条命令**按下那一刻**就说清自己在排队（issue #350）。
    ///
    /// 这个 refactor 把控制面收敛到一条线程，于是 `/lane new` 的 20 s 轮询期间别人敲的
    /// 命令会排在那次轮询之后：不解释，用户看到的就是一条没有回包的卡住命令。
    /// 提示只能由发送侧写 —— ctl 线程轮到它才知道自己排第几。
    #[test]
    fn second_control_call_says_it_is_queued() {
        use std::sync::mpsc;

        use crate::tui_mode::test_support::{lane_with_tx, test_client, test_prefs};
        use crate::tui_mode::worker::LaneId;
        use crate::tui_mode::worker_ctl::CtlHandle;

        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, _rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut lane = lane_with_tx(LaneId(5), &client, &mut prefs, tx, job_tx);
        let (ctl_tx, ctl_rx) = mpsc::channel();
        lane.ctl = Some(CtlHandle::detached(ctl_tx));

        lane.lane_attach("proj-a".to_string());
        let after_first = lane.st.lines.len();
        lane.lane_attach("proj-b".to_string());

        assert!(ctl_rx.try_recv().is_ok(), "第一条应已入队");
        assert!(ctl_rx.try_recv().is_ok(), "第二条也要入队，只是排在后面");
        let texts: Vec<String> = lane.st.lines.iter().map(|l| l.text().to_string()).collect();
        assert!(
            !texts[..after_first].iter().any(|t| t.contains("排队中")),
            "第一条前面没人，不该报排队：{texts:?}"
        );
        assert!(
            texts[after_first..]
                .iter()
                .any(|t| t.contains("排队中") && t.contains("前面还有 1 条")),
            "第二条要说明前面还压着一条：{texts:?}"
        );
    }

    #[test]
    fn new_takes_name_and_optional_workspace() {
        match parse(Some("new proj-b")) {
            Ok(LaneCommand::New {
                name,
                workspace,
                profile,
            }) => {
                assert_eq!(name, "proj-b");
                assert_eq!(workspace, None, "缺工作区时交给当前 lane 兜底");
                assert_eq!(profile, None, "缺 profile 时交给 Server 的默认");
            }
            _ => panic!("应解析为 new"),
        }
        // 工作区可以含空格：只按第一个空白切。
        match parse(Some("new proj-b /data/my proj")) {
            Ok(LaneCommand::New {
                name,
                workspace,
                profile,
            }) => {
                assert_eq!(name, "proj-b");
                assert_eq!(workspace.as_deref(), Some("/data/my proj"));
                assert_eq!(profile, None, "裸路径写法不带 profile");
            }
            _ => panic!("应解析为 new"),
        }
        assert!(parse(Some("new")).is_err(), "缺名字要报错");
    }

    /// `profile` 只按名字传；`workspace=` / `profile=` 与裸路径不混用，错法一律报用法。
    #[test]
    fn new_takes_profile_by_name_only() {
        match parse(Some(
            "new proj-b workspace=/data/proj-b profile=readonly-audit",
        )) {
            Ok(LaneCommand::New {
                name,
                workspace,
                profile,
            }) => {
                assert_eq!(name, "proj-b");
                assert_eq!(workspace.as_deref(), Some("/data/proj-b"));
                assert_eq!(profile.as_deref(), Some("readonly-audit"));
            }
            _ => panic!("应解析为带 profile 的 new"),
        }
        // 只给 profile：工作区仍交给当前 lane 兜底。
        match parse(Some("new proj-b profile=readonly-audit")) {
            Ok(LaneCommand::New {
                workspace, profile, ..
            }) => {
                assert_eq!(workspace, None);
                assert_eq!(profile.as_deref(), Some("readonly-audit"));
            }
            _ => panic!("应解析为 new"),
        }
        // 裸路径里带 `=` 也不算参数：整段都是工作区（存在性由 serve 判）。
        match parse(Some("new proj-b /data/profile=x")) {
            Ok(LaneCommand::New {
                workspace, profile, ..
            }) => {
                assert_eq!(workspace.as_deref(), Some("/data/profile=x"));
                assert_eq!(profile, None, "裸路径不该被拆成 profile");
            }
            _ => panic!("裸路径要整段当工作区"),
        }
        // 混用：进了 `key=value` 写法，裸词就没有位置了。
        assert!(parse(Some("new proj-b workspace=/a /b")).is_err());
        assert!(parse(Some("new proj-b profile=")).is_err(), "缺值要报错");
        assert!(
            parse(Some("new proj-b port=8080")).is_err(),
            "只认 workspace / profile 两个键"
        );
        assert!(
            parse(Some("new proj-b profile=a profile=b")).is_err(),
            "同一个键给两次是手误"
        );
    }

    #[test]
    fn unknown_subcommand_returns_usage() {
        let err = parse(Some("zzz proj-a")).unwrap_err();
        assert!(err.contains("/lane attach"), "{err}");
        assert!(err.contains("/lane new"), "{err}");
    }

    #[test]
    fn workspace_falls_back_to_the_current_lane_only_when_non_blank() {
        assert_eq!(
            resolve_workspace(Some("/explicit".to_string()), Some("/current")),
            Some("/explicit".to_string()),
            "显式路径优先"
        );
        assert_eq!(
            resolve_workspace(None, Some("/current")),
            Some("/current".to_string())
        );
        // 待绑定（空串）不能拿来建声明。
        assert_eq!(resolve_workspace(None, Some("   ")), None);
        assert_eq!(resolve_workspace(None, None), None);
    }
}
