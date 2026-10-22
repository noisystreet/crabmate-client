//! 切换工作区的三条入口（`/workspace` 命令 / 项目池 `p` / 鼠标点击）与统一收尾。
//!
//! 从 `mod.rs` 拆出：`mod.rs` 顶着 920 行门禁，而且这几段逻辑本来就该放一处看——
//! 三条入口的差别只在「怎么发请求」，成功后都要走同一个
//! `reset_workspace_after_switch`（旧项目的目录结构与光标必须丢弃）。

use super::TuiApp;
use super::state::LineKind;
use super::worker::WorkerJob;
use crate::workspace_path::{self, is_default_word};

/// 切换成功的系统行：serve 用空路径表示「恢复默认根」，那时没有路径可打。
fn workspace_set_line(path: &str) -> String {
    if path.trim().is_empty() {
        "已切换工作区：serve 默认根".to_string()
    } else {
        format!("已切换到工作区：{path}")
    }
}

/// `/workspace <路径>` 请求结果的处理决定。
///
/// 抽成纯函数是为了能测——尤其 `Failed` 这条：只有服务端**确认**成功才允许重置目录树。
/// 请求失败时真相有两种（serve 拒绝了 / serve 改了但响应丢了，客户端无从判断），
/// 此时顺手也重置一次树，界面就会显示一个既不是旧根、也没被确认的新根。
#[derive(Debug, PartialEq, Eq)]
enum WsSetOutcome {
    /// 服务端确认切换：附带回显文案。
    Switched(String),
    /// 请求失败：附带回显文案，**不**重置目录树。
    Failed(String),
}

fn workspace_set_outcome(result: Result<String, String>) -> WsSetOutcome {
    match result {
        Ok(path) => WsSetOutcome::Switched(workspace_set_line(&path)),
        Err(e) => WsSetOutcome::Failed(format!("切换工作区失败：{e}")),
    }
}

/// 查询结果的系统行：未设置根时 serve 回空路径（不能只打一个冒号）。
fn workspace_query_line(result: &Result<String, String>) -> String {
    match result {
        Ok(path) if path.trim().is_empty() => "当前工作区：（未设置）".to_string(),
        Ok(path) => format!("当前工作区：{path}"),
        Err(e) => format!("查询工作区失败：{e}"),
    }
}

/// `/workspace <路径>` 提交前的处理结果。
#[derive(Debug, PartialEq, Eq)]
enum WsSetInput {
    /// 发出去的载荷（空串＝恢复 serve 默认根）与就地回显文案。
    Submit { shown: String, payload: String },
    /// 端内拦下：只打一行失败，**不发请求**。
    Rejected(String),
}

/// 把 `/workspace` 的参数变成要 POST 的载荷。
///
/// `resolve` 由调用方注入（生产用 [`workspace_path::resolve`]），这样「展不开就拦下」这条
/// 规则能脱离环境变量单测 —— 终端里的 `$HOME` 不该参与断言。
fn ws_set_input(raw: &str, resolve: impl FnOnce(&str) -> Result<String, String>) -> WsSetInput {
    if is_default_word(raw) {
        return WsSetInput::Submit {
            shown: "正在恢复工作区为 serve 默认根…".to_string(),
            payload: String::new(),
        };
    }
    match resolve(raw) {
        // 空白载荷在 serve 语义里等于「空路径＝默认根」，与用户想要的切换相反，先拦。
        // `parse_control` 已把空参归一成 `None`（走查询分支），所以这条只防手工构造的 `Some("  ")`。
        Ok(p) if p.trim().is_empty() => WsSetInput::Rejected(
            "切换工作区失败：路径为空（恢复 serve 默认根请用 /workspace off）".to_string(),
        ),
        Ok(p) => {
            // 回显展开后的绝对路径：serve 记的就是它，用户也能立刻看出 `~` 落到了哪。
            // 必须点明「本机」：远端部署时这个前缀是**客户端**猜的，不一定是 serve 的家目录。
            let shown = if p == raw.trim() {
                format!("正在切换到工作区：{p}")
            } else {
                format!("正在切换到工作区：{p}（按本机 HOME 展开 {}）", raw.trim())
            };
            WsSetInput::Submit { shown, payload: p }
        }
        Err(e) => WsSetInput::Rejected(format!("切换工作区失败：{e}")),
    }
}

impl TuiApp<'_> {
    /// `/workspace`（别名 `/cd`）：空参查当前根，带参按路径切换。
    ///
    /// 与项目池的差别只在**入口怎么选**：项目池走 `…/projects`，只能挑 serve 池内已有
    /// 项目；这里把路径 POST 给 `/workspace`，客户端不做任何白名单校验（本 crate 里
    /// 没有任何 allowed-roots 概念），所以池外路径**能不能**落地由 serve 侧策略决定——
    /// 拒绝会以 `ok:false` + `error` 回来，照原样打给用户。
    ///
    /// 端内只做两件能确定判断的事（见 [`crate::workspace_path`]）：把 `~` / `~/…` 展开成
    /// 绝对路径；`~user` 或取不到家目录时就地拦下、**不发请求** —— 否则 serve 收到的
    /// 只是一个字面量 `~`，它报的错与用户的真实问题无关。
    ///
    /// 回合进行中直接拒绝（同 `switch_to_conv`）：工具正跑着时换根，「这一轮用哪个根」
    /// 没有确定答案，交给 serve 去猜不如客户端先说清楚。
    pub(super) fn workspace_command(&mut self, arg: Option<String>) {
        self.workspace_command_with(arg, workspace_path::resolve);
    }

    /// [`Self::workspace_command`] 的可测内核：`resolve` 由参数注入。
    ///
    /// 分开的理由只有一个：`ws_set_input` 的返回值能证明「算出来了什么」，但「被拦下时
    /// 一个请求都没发」只有 `job_tx` 能证明，而生产路径用的是读环境变量的
    /// [`workspace_path::resolve`]，测试无从注入。
    fn workspace_command_with(
        &mut self,
        arg: Option<String>,
        resolve: impl FnOnce(&str) -> Result<String, String>,
    ) {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：结束后再切换工作区");
            return;
        }
        let Some(path) = arg else {
            if self.job_tx.send(WorkerJob::WorkspaceQuery).is_err() {
                self.st
                    .push_line(LineKind::System, "查询工作区失败：worker 已退出");
            }
            return;
        };
        match ws_set_input(&path, resolve) {
            WsSetInput::Rejected(line) => self.st.push_line(LineKind::System, &line),
            WsSetInput::Submit { shown, payload } => {
                self.st.push_line(LineKind::System, &shown);
                if self.job_tx.send(WorkerJob::WorkspaceSet(payload)).is_err() {
                    self.st
                        .push_line(LineKind::System, "切换工作区失败：worker 已退出");
                }
            }
        }
    }

    /// `GET /workspace` 结果：只报当前根（右栏标题与目录树由随后的根列表刷新负责）。
    pub(super) fn on_workspace_query(&mut self, result: Result<String, String>) {
        let line = workspace_query_line(&result);
        self.st.push_line(LineKind::System, &line);
    }

    /// `POST /workspace` 结果：成功后与项目池切换走同一收尾。
    pub(super) fn on_workspace_set(&mut self, result: Result<String, String>) {
        match workspace_set_outcome(result) {
            // 只有服务端确认成功才重置树：失败时真实根未知（可能已生效也可能没），
            // 此时重置会让界面显示一个既非旧根、也未经确认的树。
            WsSetOutcome::Switched(line) => {
                self.st.push_line(LineKind::System, &line);
                self.reset_workspace_after_switch();
            }
            WsSetOutcome::Failed(line) => self.st.push_line(LineKind::System, &line),
        }
    }

    /// 项目池切换结果：成功打印路径、关闭选择视图并刷新根列表；失败留在视图提示。
    pub(super) fn on_ws_project_switch(&mut self, result: Result<String, String>) {
        if let Ok(path) = &result {
            self.st
                .push_line(LineKind::System, &format!("已切换到工作区：{path}"));
        }
        self.st.ws_pick_switch_result(result);
        if self.st.ws_pick.is_none() {
            self.reset_workspace_after_switch();
        }
    }

    /// 切到新工作区后：重置树与展开态（不能保留上一个项目的目录结构）并重拉根列表；
    /// 右栏标题的工作区名（`workspace_path`）随根列表回来时更新（见 `ws_root_replace`）。
    ///
    /// 左栏会话列表同样是**按工作区分桶**的（`…/workspaces/current/sessions`），也要重拉：
    /// 否则右栏标题与目录树已换到新项目，左栏还列着上一个项目的会话，而且要等到下一轮结束
    /// （`on_turn_done` 的例行刷新）才会更新。当前 `conversation_id` 保持不动（serve 契约：
    /// 切换工作区不打断正在进行的对话），只是列表换成新桶。
    ///
    /// 旧桶的会话索引记账（`/conv new` 行 id、待绑定 cid / 标题、待选中行）也必须丢弃：
    /// 那些行 id 只存在于上一个桶，留着会让新桶首轮结束时的 `BindServer` 落到
    /// 「会话行已不存在」并让新会话登记不上。
    fn reset_workspace_after_switch(&mut self) {
        self.index.local_new_session = None;
        self.index.pending_select_session = None;
        self.index.pending_bind_conversation = None;
        self.index.pending_session_title = None;
        self.index.new_session_pending = false;
        // skill 目录是按工作区（`.crabmate/skills`）解析的：换根后旧候选不该继续出现在菜单里。
        self.st.clear_skills();
        self.st.ws_begin_root_fetch();
        self.refresh_workspace(false);
        self.refresh_sessions();
    }

    pub(super) fn fetch_ws_projects(&mut self) {
        if self.job_tx.send(WorkerJob::WsProjects).is_err() {
            self.st
                .push_line(LineKind::System, "拉取项目池失败：worker 已退出");
        }
    }

    pub(super) fn switch_ws_project(&mut self, name: String) {
        // 与 `/workspace <路径>` 同一条守卫：回合中换根，飞行中的工具没有确定的根可依。
        // 调用方已把选择视图置 busy；这里借结果回调就地解锁并显示原因，否则 Enter 会
        // 被 busy 永久挡住且用户看不到任何反馈。
        if self.st.running {
            self.st
                .ws_pick_switch_result(Err("回合进行中：结束后再切换工作区".to_string()));
            return;
        }
        if self.job_tx.send(WorkerJob::WsSwitchProject(name)).is_err() {
            self.st
                .push_line(LineKind::System, "切换项目失败：worker 已退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::super::TuiApp;
    use super::super::spinner::Spinner;
    use super::super::state::UiState;
    use super::super::worker::{UiEvent, WorkerJob};
    use super::super::workspace_tree::WsPickState;
    use super::{
        WsSetInput, WsSetOutcome, is_default_word, workspace_query_line, workspace_set_line,
        workspace_set_outcome, ws_set_input,
    };
    use crate::SessionPrefs;
    use crate::serve::{ConnectionConfig, ServeClient};

    fn test_prefs() -> SessionPrefs {
        SessionPrefs {
            api_key: None,
            model: None,
            api_base: None,
            agent_role: None,
            session_mode: None,
        }
    }

    fn test_app<'a>(
        client: &'a ServeClient,
        prefs: &'a mut SessionPrefs,
        rx: mpsc::Receiver<UiEvent>,
        tx: mpsc::Sender<UiEvent>,
        job_tx: mpsc::Sender<WorkerJob>,
    ) -> TuiApp<'a> {
        // `tx` 必须与 `rx` 是同一对：单测没有 tokio runtime，`fetch_ws_dir` 会退回队列，这条
        // tx 眼下用不到；但若将来某个测试跑在 `#[tokio::test]` 下，读请求就真的会 `spawn`，
        // 那时发到一个没人读的通道里会让断言假通过。
        TuiApp {
            client,
            overrides: prefs,
            yes: true,
            no_keyring: true,
            st: UiState::new(),
            spinner: Spinner::new(),
            rx,
            tx,
            job_tx,
            cancel: None,
            exit: false,
            quit_pending: false,
            prepared: Vec::new(),
            body_key: None,
            panel: None,
            persisted: None,
            last_size: (0, 0),
            index: Default::default(),
            resume: None,
            sent_turn: None,
            bg: None,
        }
    }

    #[test]
    fn default_words_restore_serve_default() {
        // 空参不可表达「恢复默认根」（`parse_control` 把它归一成查询），故必须有词。
        for w in ["off", "none", "clear", "default", "Default"] {
            assert!(is_default_word(w), "{w} 应视为恢复默认根");
        }
        assert!(!is_default_word("/data/proj"));
        assert!(!is_default_word("defaults"));
    }

    #[test]
    fn set_line_reports_default_root_for_empty_path() {
        assert!(workspace_set_line("").contains("默认根"));
        assert!(workspace_set_line("   ").contains("默认根"));
        assert_eq!(
            workspace_set_line("/data/proj"),
            "已切换到工作区：/data/proj"
        );
    }

    #[test]
    fn query_line_covers_unset_and_error() {
        assert!(workspace_query_line(&Ok(String::new())).contains("未设置"));
        assert!(workspace_query_line(&Ok("/data/proj".into())).contains("/data/proj"));
        assert!(workspace_query_line(&Err("busy".into())).contains("busy"));
    }

    /// 服务端拒绝时，回显必须读作「失败」，不能半个字像成功。
    #[test]
    fn failed_set_never_reads_as_success() {
        let out = workspace_set_outcome(Err(
            "工作区路径无效或不存在: No such file or directory (os error 2)".into(),
        ));
        match out {
            WsSetOutcome::Failed(line) => {
                assert!(line.contains("切换工作区失败"), "应明说失败：{line}");
                assert!(
                    line.contains("工作区路径无效或不存在"),
                    "应透传 serve 原文：{line}"
                );
                assert!(!line.contains("已切换"), "失败行不能含成功措辞：{line}");
            }
            WsSetOutcome::Switched(l) => panic!("Err 走了成功分支：{l}"),
        }
    }

    /// 只有 `Switched` 才允许重置树：失败分支必须可区分，不能与成功混为一谈。
    #[test]
    fn only_confirmed_set_switches() {
        assert_eq!(
            workspace_set_outcome(Ok("/data/proj".into())),
            WsSetOutcome::Switched("已切换到工作区：/data/proj".into())
        );
        // serve 用空串表示恢复默认根：回显不能是空路径。
        assert_eq!(
            workspace_set_outcome(Ok(String::new())),
            WsSetOutcome::Switched("已切换工作区：serve 默认根".into())
        );
        assert!(matches!(
            workspace_set_outcome(Err("boom".into())),
            WsSetOutcome::Failed(_)
        ));
        // 失败行与成功行必须互斥（防止有人后来把两分支合并）。
        assert_ne!(
            workspace_set_outcome(Err("x".into())),
            workspace_set_outcome(Ok("x".into()))
        );
    }

    /// 切换工作区后必须立刻重拉左栏，并丢掉只属于旧桶的索引记账。
    #[test]
    fn workspace_switch_refetches_sessions_and_drops_old_bucket_index() {
        let client =
            ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client");
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        app.index.local_new_session = Some("old-row".into());
        app.index.pending_select_session = Some("old-row".into());
        app.index.pending_bind_conversation = Some("old-cid".into());
        app.index.pending_session_title = Some("旧标题".into());
        app.index.new_session_pending = true;

        app.reset_workspace_after_switch();

        // 旧桶行 id / 待绑定状态带进新桶会让首轮 BindServer 报「会话行已不存在」。
        assert!(app.index.local_new_session.is_none());
        assert!(app.index.pending_select_session.is_none());
        assert!(app.index.pending_bind_conversation.is_none());
        assert!(app.index.pending_session_title.is_none());
        assert!(!app.index.new_session_pending);

        // 顺序固定（worker 串行）：先根列表，再会话列表。
        assert!(matches!(
            job_rx.try_recv(),
            Ok(WorkerJob::RefreshWorkspace { keep: false })
        ));
        assert!(matches!(job_rx.try_recv(), Ok(WorkerJob::RefreshSessions)));
        assert!(job_rx.try_recv().is_err(), "不应再排队其它任务");
    }

    /// 项目池切换与 `/workspace <路径>` 共用同一条回合守卫，且拒绝后要解锁视图。
    #[test]
    fn project_switch_is_refused_while_a_turn_is_running() {
        let client =
            ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client");
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        app.st.running = true;
        app.st.ws_pick = Some(WsPickState {
            enabled: true,
            projects: vec!["proj".into()],
            busy: true,
            ..Default::default()
        });

        app.switch_ws_project("proj".into());

        assert!(job_rx.try_recv().is_err(), "回合中不得发切换请求");
        let pick = app.st.ws_pick.as_ref().expect("应留在选择视图");
        assert!(!pick.busy, "拒绝后必须解锁，否则 Enter 会被 busy 永久挡住");
        assert!(
            pick.error
                .as_deref()
                .is_some_and(|e| e.contains("回合进行中")),
            "原因要在原地可见：{:?}",
            pick.error
        );
    }

    /// 清除词不参与展开：`/workspace off` 必须仍是「恢复默认根」，不能被当成路径。
    #[test]
    fn default_words_skip_tilde_expansion() {
        for w in ["off", "none", "clear", "default"] {
            match ws_set_input(w, |_| panic!("清除词不应调用展开：{w}")) {
                WsSetInput::Submit { shown, payload } => {
                    assert_eq!(payload, "");
                    assert!(
                        shown.contains("默认根"),
                        "{w} 回显应说明恢复默认根：{shown}"
                    );
                }
                WsSetInput::Rejected(l) => panic!("{w} 被拦下了：{l}"),
            }
        }
    }

    /// `~` 展开后的绝对路径才是载荷；回显要带出原输入，否则用户看不出落到了哪个目录。
    #[test]
    fn tilde_is_expanded_before_post() {
        match ws_set_input("~/proj", |_| Ok("/home/t/proj".into())) {
            WsSetInput::Submit { shown, payload } => {
                assert_eq!(payload, "/home/t/proj");
                assert!(shown.contains("/home/t/proj"), "应回显展开结果：{shown}");
                assert!(
                    shown.contains("按本机 HOME 展开 ~/proj"),
                    "应点明是本机 HOME 展开的（远端部署时这不是 serve 的家目录）：{shown}"
                );
            }
            WsSetInput::Rejected(l) => panic!("不该被拦下：{l}"),
        }
    }

    #[test]
    fn unchanged_path_is_echoed_without_expand_note() {
        match ws_set_input("/data/proj", |_| Ok("/data/proj".into())) {
            WsSetInput::Submit { shown, payload } => {
                assert_eq!(payload, "/data/proj");
                assert_eq!(shown, "正在切换到工作区：/data/proj");
            }
            WsSetInput::Rejected(l) => panic!("不该被拦下：{l}"),
        }
    }

    /// 展不开（`~user` / 无家目录）时**不发请求**，且文案必须读作失败。
    #[test]
    fn unexpandable_tilde_is_rejected_without_request() {
        match ws_set_input("~root", |_| Err("不支持 `~user` 形式…".into())) {
            WsSetInput::Rejected(line) => {
                assert!(line.contains("切换工作区失败"), "失败措辞：{line}");
                assert!(line.contains("~user"), "应透传原因：{line}");
                assert!(!line.contains("已切换"), "不能像成功：{line}");
            }
            WsSetInput::Submit { payload, .. } => panic!("拦下失败，仍会 POST `{payload}`"),
        }
    }

    /// 空白载荷在 serve 语义里等于「空路径＝恢复默认根」，与用户想要的切换相反。
    #[test]
    fn blank_payload_is_rejected_not_treated_as_default_root() {
        match ws_set_input("   ", |_| Ok("  ".into())) {
            WsSetInput::Rejected(line) => assert!(line.contains("路径为空"), "{line}"),
            WsSetInput::Submit { payload, .. } => panic!("空白载荷不该 POST：`{payload}`"),
        }
    }

    /// 代数不符的回包必须被丢弃，否则旧工作区的目录内容会被写进新工作区的缓存。
    #[test]
    fn stale_workspace_dir_reply_is_dropped() {
        let client =
            ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client");
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        use crate::serve::{WorkspaceDirData, WorkspaceDirEntry};
        let dir = |path: &str, name: &str| WorkspaceDirData {
            path: path.to_string(),
            entries: vec![WorkspaceDirEntry {
                name: name.to_string(),
                is_dir: false,
            }],
            error: None,
        };
        app.st.ws_root_replace(WorkspaceDirData {
            path: "/b".to_string(),
            entries: vec![WorkspaceDirEntry {
                name: "src".to_string(),
                is_dir: true,
            }],
            error: None,
        });
        let current = app.st.ws_generation;

        // 旧工作区的回包（代数小一号）：必须被丢弃。
        app.on_msg(UiEvent::WorkspaceDir {
            rel: "src".to_string(),
            generation: current.wrapping_sub(1),
            result: Ok(dir("/a/src", "stale.rs")),
        });
        assert!(
            !app.st.ws_dir_cache.contains_key("src"),
            "旧代数的回包应被丢弃"
        );

        // 当前代数的回包：正常写入。
        app.on_msg(UiEvent::WorkspaceDir {
            rel: "src".to_string(),
            generation: current,
            result: Ok(dir("/b/src", "fresh.rs")),
        });
        assert!(
            app.st.ws_dir_cache.contains_key("src"),
            "同代数的回包应生效"
        );
    }

    /// 被拦下时**一个 worker 任务都不发**：这条只有 job 通道能证明，返回值看不出来。
    #[test]
    fn rejected_input_never_reaches_the_worker() {
        let client =
            ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client");
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.workspace_command_with(Some("~root".into()), |_| Err("不支持 `~user` 形式".into()));

        assert!(
            job_rx.try_recv().is_err(),
            "被拦下时不该发任何 worker 任务（含 WorkspaceSet）"
        );
        let last = app.st.lines.last().expect("应留一行失败回显");
        assert!(last.text.contains("切换工作区失败"), "{:?}", last.text);
    }

    /// 放行时发出去的是**展开后**的绝对路径，不是用户输入原文。
    #[test]
    fn expanded_path_is_what_gets_posted() {
        let client =
            ServeClient::new(ConnectionConfig::new("http://127.0.0.1:1", "")).expect("client");
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        app.workspace_command_with(Some("~/proj".into()), |_| Ok("/home/t/proj".into()));

        match job_rx.try_recv() {
            Ok(WorkerJob::WorkspaceSet(payload)) => assert_eq!(payload, "/home/t/proj"),
            _ => panic!("应发出 WorkspaceSet"),
        }
    }
}
