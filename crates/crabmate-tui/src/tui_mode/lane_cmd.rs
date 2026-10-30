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
//! - `/lane detach <name>`：只摘掉**本进程**的连接；声明与 serve 进程都不动（与 attach 对称）。
//! - `/lane new <name> [workspace]`：新增一条声明并等它就绪（写盘由 ctl 完成）。
//!
//! 失败一律写进**发起命令的那条 lane**的 transcript：用户正看着它，而新 lane 此刻
//! 还不存在，没有别的地方可写。

use super::state::LineKind;
use super::worker::{UiEvent, WorkerJob};

/// `/lane` 的子动作。
#[derive(Debug)]
pub(crate) enum LaneCommand {
    /// 空参 / `list`：列出现有实例（与 `/ctl` 同一实现）。
    List,
    /// `attach <name>`：连上已就绪的实例。
    Attach(String),
    /// `detach <name>`：只摘掉本进程的连接（声明与 serve 进程都不动）。
    Detach(String),
    /// `cancel <name>`：取消这条 lane 正在跑的回合（不必先切过去）。
    ///
    /// 后台 lane 的审批浮层画不到屏幕上，没有这条命令就只能干等 `APPROVAL_WAIT`
    /// 超时（600 s）才按拒绝收尾（见 issue #337）。
    Cancel(String),
    /// `new <name> [workspace]`：新增声明并等它就绪。
    New {
        /// 实例名（`instances.toml` 的键）。
        name: String,
        /// 显式给的工作区；`None` = 用当前 lane 的 serve 工作区兜底。
        workspace: Option<String>,
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
        "detach" => one_name(tail, "detach").map(LaneCommand::Detach),
        "cancel" | "kill" | "interrupt" => one_name(tail, "cancel").map(LaneCommand::Cancel),
        "new" | "add" => new_tail(tail),
        _ => Err(usage()),
    }
}

/// 用法提示：每条子命令各给一个可照抄的例子。
fn usage() -> String {
    "用法：/lane [list] · /lane attach <name> · /lane detach <name> · /lane cancel <name> · /lane new <name> [workspace]"
        .to_string()
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

/// `new` 之后是 `名字 [工作区]`：工作区可以含空格，所以只按**第一个**空白切。
fn new_tail(tail: &str) -> Result<LaneCommand, String> {
    let (name, workspace) = match tail.split_once(char::is_whitespace) {
        Some((n, w)) => (n, Some(w.trim()).filter(|w| !w.is_empty())),
        None => (tail, None),
    };
    if name.is_empty() {
        return Err("缺实例名：`/lane new <name> [workspace]`".to_string());
    }
    Ok(LaneCommand::New {
        name: name.to_string(),
        workspace: workspace.map(str::to_string),
    })
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

impl super::LaneApp<'_> {
    /// `/lane ...` 的分派（解析在 [`parse`]，执行全在 worker 线程）。
    pub(super) fn lane_command(&mut self, cmd: LaneCommand) {
        match cmd {
            LaneCommand::List => self.fetch_ctl_list(),
            LaneCommand::Attach(name) => self.lane_attach(name),
            LaneCommand::Detach(name) => self.lane_detach(name),
            LaneCommand::Cancel(name) => self.lane_cancel(name),
            LaneCommand::New { name, workspace } => self.lane_new(name, workspace),
        }
    }

    /// `/lane attach <name>`：只读查询，不动声明。
    fn lane_attach(&mut self, name: String) {
        let Some(socket) = self.ctl_socket_or_hint() else {
            return;
        };
        self.st
            .push_line(LineKind::System, &format!("正在连接 ctl 实例 `{name}`…"));
        if self
            .job_tx
            .send(WorkerJob::CtlAttach { socket, name })
            .is_err()
        {
            self.st
                .push_line(LineKind::System, "连接未发出：worker 线程已退出");
        }
    }

    /// `/lane detach <name>`：请外壳摘掉一条连接（`LaneApp` 没有 `lanes` 的 `&mut`）。
    ///
    /// 走**裸**事件（不带 `LaneId`）：被摘的可能正是发起命令的这条 lane，按 id 分发会
    /// 落空 —— 与 `CtlLaneReady` 同一个理由。不查 ctl：这一步不动声明、也不停进程。
    fn lane_detach(&mut self, name: String) {
        if self.tx.send(UiEvent::LaneDetach { name }).is_err() {
            self.st
                .push_line(LineKind::System, "断开未发出：事件通道已关闭");
        }
    }

    /// `/lane cancel <name>`：请外壳取消一条 lane 的回合（`LaneApp` 只能取消自己）。
    ///
    /// 走**裸**事件，理由同 [`Self::lane_detach`]：要取消的可能正是这条 lane，
    /// 按 id 分发会落回它自己、在 `TuiApp::on_msg` 里变成「自己取消自己」的二次请求。
    fn lane_cancel(&mut self, name: String) {
        if self.tx.send(UiEvent::LaneCancel { name }).is_err() {
            self.st
                .push_line(LineKind::System, "取消未发出：事件通道已关闭");
        }
    }

    /// `/lane new <name> [workspace]`：写声明 + 等就绪（`add` 由 worker 发）。
    fn lane_new(&mut self, name: String, workspace: Option<String>) {
        let Some(socket) = self.ctl_socket_or_hint() else {
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
            &format!("正在新建 ctl 实例 `{name}`（工作区 {workspace}）并等它就绪…"),
        );
        if self
            .job_tx
            .send(WorkerJob::CtlNew {
                socket,
                name,
                workspace,
            })
            .is_err()
        {
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

    /// `detach` 与 `attach` 的名字规则一致；报错要照抄用户刚打的那条命令。
    #[test]
    fn detach_takes_exactly_one_name() {
        assert!(matches!(
            parse(Some("detach proj-a")),
            Ok(LaneCommand::Detach(n)) if n == "proj-a"
        ));
        let err = parse(Some("detach")).unwrap_err();
        assert!(err.contains("/lane detach"), "要指回刚打的那条：{err}");
        assert!(parse(Some("detach a b")).is_err(), "两个词不猜哪个是名字");
    }

    #[test]
    fn new_takes_name_and_optional_workspace() {
        match parse(Some("new proj-b")) {
            Ok(LaneCommand::New { name, workspace }) => {
                assert_eq!(name, "proj-b");
                assert_eq!(workspace, None, "缺工作区时交给当前 lane 兜底");
            }
            _ => panic!("应解析为 new"),
        }
        // 工作区可以含空格：只按第一个空白切。
        match parse(Some("new proj-b /data/my proj")) {
            Ok(LaneCommand::New { name, workspace }) => {
                assert_eq!(name, "proj-b");
                assert_eq!(workspace.as_deref(), Some("/data/my proj"));
            }
            _ => panic!("应解析为 new"),
        }
        assert!(parse(Some("new")).is_err(), "缺名字要报错");
    }

    #[test]
    fn unknown_subcommand_returns_usage() {
        let err = parse(Some("close proj-a")).unwrap_err();
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
