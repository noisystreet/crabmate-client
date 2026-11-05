//! `/version` `/skills` `/context` `/config` `/agent`：信息类斜杠命令（issue #444）。
//!
//! 为什么单独成文件：`mod.rs` 贴着 920 行门禁，而这五条各自要写「数据从哪来、缺数据
//! 怎么提示」；它们的共同点只是「往 transcript 写几行」，没有共享状态要维护。
//!
//! 数据来源一律是**已有状态**（`SkillsCache` / `ServeDefaults` / 上下文水位），只有
//! `/config reload` 是真的发请求 —— 命令层再打一次已经拉过的接口，就是给「两处数字
//! 不一致」留门（`/skills` 尤其明显：菜单刚刷掉的 skill 会在命令输出里复活）。

use super::LaneApp;
use super::controls::InfoCmd;
use super::state::LineKind;
use super::worker::WorkerJob;

/// `/skills` 最多印多少条（与 Web `/skills` 的上限一致）。
const MAX_LISTED_SKILLS: usize = 24;

impl LaneApp<'_> {
    /// `apply_control` 的唯一入口：事件循环只认 `Control::Info`，子命令分发留在这里。
    ///
    /// 这一个间接层换来 `mod.rs` 五行（它贴着 920 行门禁），而五个 `cmd_*` 本来也只在
    /// 这里出现一次 —— 代价只是一个 `match`。
    pub(super) fn apply_info_control(&mut self, cmd: InfoCmd) {
        match cmd {
            InfoCmd::Version => self.cmd_version(),
            InfoCmd::Skills => self.cmd_skills(),
            InfoCmd::Context => self.cmd_context(),
            InfoCmd::Config(arg) => self.cmd_config(arg),
            InfoCmd::Agent(arg) => self.cmd_agent(arg),
        }
    }

    /// `/version`：本进程版本号。**不请求 serve** —— 那是 `/status` 的工作。
    pub(super) fn cmd_version(&mut self) {
        let line = format!(
            "crabmate-tui {}（本地版本；serve 侧版本与默认值看 /status）",
            env!("CARGO_PKG_VERSION")
        );
        self.st.push_line(LineKind::System, &line);
    }

    /// `/skills`：印 `/` 补全菜单那份缓存；没拉过就先拉一次。
    pub(super) fn cmd_skills(&mut self) {
        let Some(cache) = self.st.skills.clone() else {
            // 缓存还没有（启动时没拉 / 拉失败）：入队一次并说明，不在这里同步等。
            if self.job_tx.send(WorkerJob::Skills).is_err() {
                self.st
                    .push_line(LineKind::System, "拉取 skill 目录失败：worker 已退出");
            } else {
                self.st
                    .push_line(LineKind::System, "正在拉取 skill 目录，稍后再 /skills");
            }
            return;
        };
        if !cache.enabled {
            self.st.push_line(
                LineKind::System,
                "skills 已关闭（serve 侧 skills_enabled=false）",
            );
            return;
        }
        let mut lines = Vec::new();
        if let Some(err) = cache
            .err
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("skill 目录有错误：{err}"));
        }
        if cache.skills.is_empty() {
            lines.push("当前未发现 skill（检查 serve 侧的 skills 目录）".to_string());
        } else {
            lines.push(format!(
                "skills {} 个（用 /<id> 调用）：",
                cache.skills.len()
            ));
            for s in cache.skills.iter().take(MAX_LISTED_SKILLS) {
                let desc = s.description.trim();
                if desc.is_empty() {
                    lines.push(format!("  /{}", s.id));
                } else {
                    lines.push(format!("  /{} — {}", s.id, desc));
                }
            }
            if cache.skills.len() > MAX_LISTED_SKILLS {
                lines.push(format!(
                    "  …还有 {} 个（`/` 菜单里可以按前缀过滤）",
                    cache.skills.len() - MAX_LISTED_SKILLS
                ));
            }
        }
        self.push_lines(lines);
    }

    /// `/context`：上下文用量明细。与状态行 `ctx` 段共用同一条轴线。
    pub(super) fn cmd_context(&mut self) {
        let lines = {
            let stored = self.persisted.as_ref();
            let cap = stored.and_then(|p| p.context_tokens.as_deref());
            let baseline = self
                .st
                .serve_defaults
                .as_ref()
                .and_then(|d| d.baseline_prompt_tokens(stored.and_then(|p| p.role.as_deref())));
            self.st.context_detail_lines(cap, baseline)
        };
        self.push_lines(lines);
    }

    /// `/config [reload]`：`reload` 让 serve 重读磁盘配置；空参给用法。
    pub(super) fn cmd_config(&mut self, arg: Option<String>) {
        match arg.as_deref().map(str::trim) {
            Some("reload") => {
                if self.job_tx.send(WorkerJob::ConfigReload).is_err() {
                    self.st
                        .push_line(LineKind::System, "重载失败：worker 已退出");
                } else {
                    self.st
                        .push_line(LineKind::System, "正在让 serve 重读配置…（结果会写在这里）");
                }
            }
            _ => self.st.push_line(
                LineKind::System,
                "用法：/config reload（让 serve 重读磁盘配置；客户端 override 不受影响）",
            ),
        }
    }

    /// `/agent [list]`：列可用 role。设置走 `/role`，这里只读。
    pub(super) fn cmd_agent(&mut self, arg: Option<String>) {
        match arg.as_deref().map(str::trim) {
            None | Some("") | Some("list") | Some("ls") => self.list_agent_roles(),
            Some("set") => self.st.push_line(
                LineKind::System,
                "TUI 用 /role <id> 设置 agent role（/agent 只做列举）",
            ),
            Some(other) => {
                let msg =
                    format!("不支持的 /agent 子命令：{other}（支持 list；设置用 /role <id>）");
                self.st.push_line(LineKind::System, &msg);
            }
        }
    }

    /// `POST /config/reload` 的回包：成败各写一行，成功顺手刷新一次状态。
    pub(super) fn on_config_reloaded(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.st.push_line(
                    LineKind::System,
                    "serve 配置已重载（默认模型 / 模式等事实再刷新一次状态行即可见）",
                );
                // 重载改的正是状态行要显示的那些默认值，不刷新的话用户看到的是旧值，
                // 会以为重载没生效。
                self.refresh_status();
            }
            Err(e) => {
                let msg = format!("重载失败：{e}");
                self.st.push_line(LineKind::System, &msg);
            }
        }
    }

    fn list_agent_roles(&mut self) {
        let Some(defaults) = self.st.serve_defaults.as_ref() else {
            self.st.push_line(
                LineKind::System,
                "还没有拉取 serve 状态：先 /status，再 /agent list",
            );
            return;
        };
        if defaults.agent_role_ids.is_empty() {
            self.st.push_line(
                LineKind::System,
                "该 serve 未提供 agent_role_ids（老版本，或没配置 role）",
            );
            return;
        }
        // 先 clone 再 push：`defaults` 借着 `self.st`，与 `push_line` 的 `&mut` 冲突。
        let ids = defaults.agent_role_ids.clone();
        let serve_default = defaults.role.clone();
        let override_role = self.overrides.agent_role.clone();
        let mut lines = vec![format!(
            "agent roles {} 个（* = 本轮 override，~ = serve 默认）：",
            ids.len()
        )];
        for id in &ids {
            let mark = if override_role.as_deref() == Some(id.as_str()) {
                "*"
            } else if serve_default.as_deref() == Some(id.as_str()) {
                "~"
            } else {
                " "
            };
            lines.push(format!(" {mark} {id}"));
        }
        lines.push("设置：/role <id>（off 清除，只影响后续回合）".to_string());
        self.push_lines(lines);
    }

    fn push_lines(&mut self, lines: Vec<String>) {
        for line in &lines {
            self.st.push_line(LineKind::System, line);
        }
    }
}
