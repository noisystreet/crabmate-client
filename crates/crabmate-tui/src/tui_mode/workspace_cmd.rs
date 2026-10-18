//! 切换工作区的三条入口（`/workspace` 命令 / 项目池 `p` / 鼠标点击）与统一收尾。
//!
//! 从 `mod.rs` 拆出：`mod.rs` 顶着 920 行门禁，而且这几段逻辑本来就该放一处看——
//! 三条入口的差别只在「怎么发请求」，成功后都要走同一个
//! `reset_workspace_after_switch`（旧项目的目录结构与光标必须丢弃）。

use super::TuiApp;
use super::controls::clear_word;
use super::state::LineKind;
use super::worker::WorkerJob;

/// `/workspace` 的「恢复 serve 默认根」词：与 `/model` 等 override 的清除词一致，
/// 另加直观的 `default`。serve 侧以**空路径**表示默认根，所以这里必须有个词能表达它——
/// 空参已被「查询当前根」占用（`parse_control` 把空白参数归一成 `None`）。
fn is_default_word(arg: &str) -> bool {
    clear_word(arg) || arg.eq_ignore_ascii_case("default")
}

/// 切换成功的系统行：serve 用空路径表示「恢复默认根」，那时没有路径可打。
fn workspace_set_line(path: &str) -> String {
    if path.trim().is_empty() {
        "已切换工作区：serve 默认根".to_string()
    } else {
        format!("已切换到工作区：{path}")
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

impl TuiApp<'_> {
    /// `/workspace`（别名 `/cd`）：空参查当前根，带参按路径切换。
    ///
    /// 与项目池的差别只在**入口怎么选**：项目池走 `…/projects`，只能挑 serve 池内已有
    /// 项目；这里把路径原样 POST 给 `/workspace`，客户端不做任何白名单校验（本 crate 里
    /// 没有任何 allowed-roots 概念），所以池外路径**能不能**落地由 serve 侧策略决定——
    /// 拒绝会以 `ok:false` + `error` 回来，照原样打给用户。
    ///
    /// 回合进行中直接拒绝（同 `switch_to_conv`）：工具正跑着时换根，「这一轮用哪个根」
    /// 没有确定答案，交给 serve 去猜不如客户端先说清楚。
    pub(super) fn workspace_command(&mut self, arg: Option<String>) {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：结束后再切换工作区");
            return;
        }
        match arg {
            None => {
                if self.job_tx.send(WorkerJob::WorkspaceQuery).is_err() {
                    self.st
                        .push_line(LineKind::System, "查询工作区失败：worker 已退出");
                }
            }
            Some(path) => {
                // 清除词（off / none / clear / default）→ 发空路径，恢复 serve 默认根。
                let restore = is_default_word(&path);
                let shown = if restore {
                    "正在恢复工作区为 serve 默认根…".to_string()
                } else {
                    format!("正在切换到工作区：{path}")
                };
                self.st.push_line(LineKind::System, &shown);
                let payload = if restore { String::new() } else { path };
                if self.job_tx.send(WorkerJob::WorkspaceSet(payload)).is_err() {
                    self.st
                        .push_line(LineKind::System, "切换工作区失败：worker 已退出");
                }
            }
        }
    }

    /// `GET /workspace` 结果：只报当前根（顶栏与目录树由随后的根列表刷新负责）。
    pub(super) fn on_workspace_query(&mut self, result: Result<String, String>) {
        let line = workspace_query_line(&result);
        self.st.push_line(LineKind::System, &line);
    }

    /// `POST /workspace` 结果：成功后与项目池切换走同一收尾。
    pub(super) fn on_workspace_set(&mut self, result: Result<String, String>) {
        match result {
            Ok(path) => {
                let line = workspace_set_line(&path);
                self.st.push_line(LineKind::System, &line);
                self.reset_workspace_after_switch();
            }
            Err(e) => self
                .st
                .push_line(LineKind::System, &format!("切换工作区失败：{e}")),
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
    /// 顶栏的 `workspace_path` 随根列表回来时更新（见 `ws_root_replace`）。
    fn reset_workspace_after_switch(&mut self) {
        self.st.ws_begin_root_fetch();
        self.refresh_workspace(false);
    }

    pub(super) fn fetch_ws_projects(&mut self) {
        if self.job_tx.send(WorkerJob::WsProjects).is_err() {
            self.st
                .push_line(LineKind::System, "拉取项目池失败：worker 已退出");
        }
    }

    pub(super) fn switch_ws_project(&mut self, name: String) {
        if self.job_tx.send(WorkerJob::WsSwitchProject(name)).is_err() {
            self.st
                .push_line(LineKind::System, "切换项目失败：worker 已退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_default_word, workspace_query_line, workspace_set_line};

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
}
