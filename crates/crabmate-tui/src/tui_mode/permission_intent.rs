//! 权限等级切换：`/perm` 与状态行点击共用的**写入路径**（对齐 Desktop 底栏 chip）。
//!
//! Desktop 的 chip 点一下就 `POST /conversation/state` 落盘；TUI 早先只写本地 override，
//! 退出进程就没了。两条入口（命令 / 状态行）收敛到同一处，避免「命令落盘、菜单不落盘」
//! 这种同功能不同语义 —— 那比没做更坏，用户会以为已经落盘。
//!
//! 与 Desktop 一致的几条：
//! - 选项集来自服务端 `available`（[`super::serve_defaults::ServeDefaults::available_levels`]），
//!   不用本仓五档词表兜底；
//! - 只发 `permission_level` 一列：服务端是部分更新，顺带回传 `session_mode` / `agent_role`
//!   会把过期值写回去；
//! - 没有已持久化 `conversation_id` 时**不落盘**，只提示「随下一条消息生效」
//!   （服务端不做 upsert，Desktop 同样在这里止步）；
//! - 允许意图高于生效：客户端**不**把意图改写成被钳制的低值，否则下一条 `/chat` 会把
//!   低档写回会话。
//!
//! 故意**没有**做的：清除（`/perm off`）只清本地 override，不去动服务端持久值。Desktop
//! 的 chip 也没有「清除」这一档 —— 那等于要求服务端支持删除意图列，属于契约变更。

use crabmate_client_api::conversation_state::ConversationStateOutcome;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;

use super::TuiApp;
use super::serve_defaults::ServeDefaults;
use super::worker::WorkerJob;
use crate::tui_mode::controls::clear_word;
use crate::tui_mode::state::LineKind;

/// 状态行 `perm` 段显示什么。`None` = 整段不显示。
///
/// 与 Desktop `chip_text`（`frontend/src/app/status_permission_chip.rs`）同一口径，
/// 只去掉本地化标签（TUI 一贯显示线格式）：
/// - 意图与生效相同 → 单值；
/// - 不同 → `意图→生效`（服务端允许意图高于生效，两个都要显示，否则用户以为切换没生效）；
/// - 没有意图 → 生效档；
/// - 两者都不知道 → 不显示（**不**拿 `permission_level_default` 顶替：那是「没意图时会
///   落到哪」，不是「此刻是哪」）。
///
/// `effective` 该传什么由 [`status_values`] 决定 —— 不是所有 `GET /permission`
/// 的读数都能当「此刻生效」。
#[must_use]
pub(super) fn perm_value(intent: Option<&str>, effective: Option<&str>) -> Option<String> {
    let intent = intent.map(str::trim).filter(|s| !s.is_empty());
    let effective = effective.map(str::trim).filter(|s| !s.is_empty());
    match (intent, effective) {
        (Some(i), Some(e)) if i == e => Some(i.to_string()),
        // 终端宽度紧张：`→` 两侧不留空格（`read→approved` 仍是 14 列，`read → approved` 是 16）。
        (Some(i), Some(e)) => Some(format!("{i}→{e}")),
        (Some(i), None) => Some(i.to_string()),
        (None, Some(e)) => Some(e.to_string()),
        (None, None) => None,
    }
}

/// 状态行 `perm` 段两个字段的取值：`(生效侧, 意图侧)`，交给 [`perm_value`] 组合。
///
/// `bound` = 有没有已持久化 `conversation_id`。这件事**必须**参与判断，因为同一句
/// `GET /permission` 在两种状态下回答的是两个问题：
/// - 有会话：`conversation_id=…` → 「这个会话此刻是哪一档」（会话意图 + min 链），
///   所以两侧可以并列成 `意图→生效`；
/// - 没有会话：裸请求 → 「配置默认档 + 无会话意图」（`level_source: config`）。把它摆在
///   `意图→生效` 的右侧，就把「这条意图随下一条消息生效」读成了「服务端把我钳制到了
///   approved」—— 完全不同的结论，而且那个档位与当前会话毫无关系（会话还不存在）。
///
/// 所以无会话时：有意图就只给意图；没有意图就把读数标成估算 `~approved`（与状态行
/// `~12k` 的上下文基线同一套记号：**不是**「此刻生效」），既不出现箭头，也不让该段
/// 消失 —— 它还得可点，否则新会话里唯一的选档入口就没了（菜单的 `available` 仍来自
/// 同一份裸投影，那是新会话唯一能拿到选项的地方）。
#[must_use]
pub(super) fn status_values(
    bound: bool,
    intent: Option<&str>,
    level: Option<&str>,
) -> (Option<String>, Option<String>) {
    let intent = intent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let level = level.map(str::trim).filter(|s| !s.is_empty());
    if bound {
        return (level.map(str::to_string), intent);
    }
    match intent {
        Some(i) => (None, Some(i)),
        None => (level.map(|l| format!("~{l}")), None),
    }
}

impl TuiApp<'_> {
    /// 状态行 `perm` 段的两侧值；口径见 [`status_values`]。
    ///
    /// 单独一个方法而不是在 `status_info` 里就地算：`status_info` 已贴着 `mod.rs` 的
    /// 920 行门禁，把「有没有已绑定会话」这条判断留在本模块也更好找。
    pub(super) fn perm_status_values(&self) -> (Option<String>, Option<String>) {
        status_values(
            self.st.conversation_id.is_some(),
            self.overrides.permission_level.as_deref(),
            self.st
                .serve_defaults
                .as_ref()
                .and_then(|d| d.effective_level()),
        )
    }

    /// `/perm [<level>]`：查询 / 设置 / 清除。
    ///
    /// 设置走 [`Self::apply_permission_level`]，与状态行点击是同一条路径。
    pub(super) fn submit_permission_slash(&mut self, arg: Option<String>) {
        match arg {
            Some(v) if clear_word(&v) => {
                self.overrides.permission_level = None;
                self.st
                    .push_line(LineKind::System, "permission level intent cleared");
            }
            Some(v) => self.apply_permission_level(&v),
            None => {
                let echo = match self.overrides.permission_level.as_deref() {
                    Some(p) if !p.trim().is_empty() => format!("permission level intent: {p}"),
                    _ => concat!(
                        "permission level: (no intent — using the conversation's persisted level; ",
                        "/perm <level> sets one, or click the status bar's perm segment)"
                    )
                    .to_string(),
                };
                self.st.push_line(LineKind::System, &echo);
            }
        }
    }

    /// 设一档：先写本地 override（下一条消息立刻带上），有已持久化会话再落盘。
    ///
    /// 两处都写的原因：override 让**当前这一轮**生效（不必等落盘回包），落盘让下次进同一
    /// 会话仍是这一档。顺序也与 Desktop `apply_level` 一致（先写信号，再 POST）。
    pub(super) fn apply_permission_level(&mut self, level: &str) {
        let level = level.trim();
        if level.is_empty() {
            self.st
                .push_line(LineKind::System, "permission level: (empty, ignored)");
            return;
        }
        self.overrides.permission_level = Some(level.to_string());
        // 不校验词表：可选集由服务端 `available` 决定，本地写死会在服务端新增档位时误拒；
        // 未知值由服务端按无效请求拒绝，报错比本地拦截更准确（`/perm` 原本就是这个口径）。
        self.st.push_line(
            LineKind::System,
            &format!("permission level intent: {level}"),
        );
        let Some(cid) = self.st.conversation_id.clone() else {
            self.st.push_line(
                LineKind::System,
                "permission level: no persisted conversation yet — the intent applies with the next message",
            );
            return;
        };
        self.st.permission_generation = self.st.permission_generation.wrapping_add(1);
        let generation = self.st.permission_generation;
        let _ = self.job_tx.send(WorkerJob::WritePermission {
            conversation_id: cid,
            level: level.to_string(),
            generation,
        });
    }

    /// 落盘回包：成功就用响应自带的投影刷新生效档，其余如实提示（不静默吞掉）。
    pub(super) fn on_permission_written(
        &mut self,
        level: &str,
        outcome: Result<ConversationStateOutcome, String>,
        generation: u64,
    ) {
        // 连选两档：只认最后一次（慢回包不能把界面刷回旧档）。
        if generation != self.st.permission_generation {
            return;
        }
        match outcome {
            Ok(ConversationStateOutcome::Ok(view)) => {
                // 用写后的投影，不再打 `/permission`（Desktop chip 同口径）。
                if let Some(defaults) = self.st.serve_defaults.as_mut() {
                    defaults.permission_view = Some(view.permission);
                }
                self.st.push_line(
                    LineKind::System,
                    &format!("permission level saved: {level}"),
                );
            }
            Ok(ConversationStateOutcome::NotFound) => {
                // 会话不存在 / 已被 TTL 淘汰：**不**重试（重试永远同一个结果），
                // 意图仍随下一条消息发送，那时服务端会建新会话。
                self.st.push_line(
                    LineKind::System,
                    "permission level: conversation not found on serve — the intent applies with the next message",
                );
            }
            Ok(ConversationStateOutcome::Unsupported) => {
                // 老 serve 没有这条路由：与 `/permission` 的裸 404 是同一件事（能力缺失）。
                self.st.push_line(
                    LineKind::System,
                    "permission level: this serve has no /conversation/state — the intent applies with the next message",
                );
            }
            Ok(ConversationStateOutcome::Failed { status, code }) => {
                let detail = code.unwrap_or_else(|| format!("HTTP {status}"));
                self.st.push_line(
                    LineKind::System,
                    &format!("permission level write failed: {detail}"),
                );
            }
            Err(e) => {
                self.st.push_line(
                    LineKind::System,
                    &format!("permission level write failed: {e}"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{perm_value, status_values};

    #[test]
    fn same_intent_and_effective_collapse_to_one_value() {
        assert_eq!(
            perm_value(Some("workspace"), Some("workspace")),
            Some("workspace".to_string())
        );
    }

    #[test]
    fn higher_intent_shows_both_sides() {
        // 意图被 ceiling 钳制：两个值都要出现，否则用户以为切换没生效。
        assert_eq!(
            perm_value(Some("unrestricted"), Some("read")),
            Some("unrestricted→read".to_string())
        );
    }

    #[test]
    fn no_intent_shows_effective_and_unknown_shows_nothing() {
        assert_eq!(
            perm_value(None, Some("approved")),
            Some("approved".to_string())
        );
        // 两者都不知道 → 整段不显示（不猜默认档）。
        assert_eq!(perm_value(None, None), None);
        // 空白意图等同没有。
        assert_eq!(
            perm_value(Some("  "), Some("read")),
            Some("read".to_string())
        );
        // 只有意图、投影还没回来：仍显示意图，别让状态行闪没一次。
        assert_eq!(perm_value(Some("read"), None), Some("read".to_string()));
    }

    /// 没有已绑定会话：裸 `/permission` 的档位不是「此刻生效」，绝不与意图并列。
    ///
    /// 回归用例：曾经在「切到新会话后 `/perm unrestricted`」时显示
    /// `unrestricted→approved` —— 左侧是用户刚选的意图，右侧是配置默认档，读起来像
    /// 「服务端把我钳制了」，实际只是「随下一条消息生效」（会话都还不存在）。
    #[test]
    fn no_bound_conversation_never_pairs_intent_with_a_default() {
        // 有意图：只显示意图；生效侧为空 → 不出现箭头。
        let (effective, intent) = status_values(false, Some("unrestricted"), Some("approved"));
        assert_eq!(effective, None);
        assert_eq!(intent.as_deref(), Some("unrestricted"));
        assert_eq!(
            perm_value(intent.as_deref(), effective.as_deref()),
            Some("unrestricted".to_string())
        );
        // 没意图：读数标成估算（同一个 `~` 记号，说明「下一条消息会落到哪」而不是
        // 「此刻生效」）；段不能消失——它还得可点开菜单。
        let (effective, intent) = status_values(false, None, Some("approved"));
        assert_eq!(intent, None);
        assert_eq!(
            perm_value(intent.as_deref(), effective.as_deref()),
            Some("~approved".to_string())
        );
        // 读数都没有：整段不显示。
        let (effective, intent) = status_values(false, Some("  "), None);
        assert_eq!(perm_value(intent.as_deref(), effective.as_deref()), None);
    }

    /// 已绑定会话：两侧照旧并列（这是箭头唯一有意义的场合）。
    #[test]
    fn bound_conversation_pairs_intent_with_the_effective_level() {
        let (effective, intent) = status_values(true, Some("unrestricted"), Some(" approved "));
        assert_eq!(effective.as_deref(), Some("approved"));
        assert_eq!(
            perm_value(intent.as_deref(), effective.as_deref()),
            Some("unrestricted→approved".to_string())
        );
        // 无意图：只显示生效档，且不带 `~`（会话已存在，这就是此刻的档）。
        let (effective, intent) = status_values(true, None, Some("approved"));
        assert_eq!(
            perm_value(intent.as_deref(), effective.as_deref()),
            Some("approved".to_string())
        );
        // 空白读数当未知（老 serve / 解析出空串），不当成一个档位。
        let (effective, intent) = status_values(true, Some("read"), Some("   "));
        assert_eq!(
            perm_value(intent.as_deref(), effective.as_deref()),
            Some("read".to_string())
        );
    }
}

/// 状态行 `perm` 段点开的档位菜单（模态：开着时键盘只在菜单内消费）。
///
/// 选项集来自服务端 `available`，不在本地补全 —— 菜单若要「列五档」，用户会去点一个
/// 服务端本就拒绝的档（角色上限 / 会话模式钳制），报错比不显示更困惑。
#[derive(Debug)]
pub(super) struct PermissionMenu {
    /// 自低到高（服务端顺序）。
    pub(super) options: Vec<String>,
    /// 光标（0-based）。打开时落在当前生效档上；当前档不在 `available` 里（被钳到更低
    /// 一档之外的情形）就落第一项。
    pub(super) selected: usize,
    /// `perm` 段在屏幕上的矩形（打开那一刻的）。
    ///
    /// 菜单贴着它向上展开，**不居中** —— 与 Desktop 底栏 chip 把菜单锚在 chip 上是同一个
    /// 位置关系，用户的视线不用从状态行挪到屏幕中央再挪回来。存下来而不是每帧重算：
    /// 状态栏排版要 `StatusInfo`，而渲染与命中都在这一层之上；打开后状态行内容变化（比如
    /// 一次 `/status` 刷新）也不该让已开的菜单跳位置。
    pub(super) anchor: Rect,
}

impl PermissionMenu {
    /// 当前光标指向的档。
    #[must_use]
    pub(super) fn current(&self) -> Option<&str> {
        self.options.get(self.selected).map(String::as_str)
    }

    /// 移动光标（`delta` 为 ±1）；到头不环绕 —— 五项的一来一回比绕圈更好预期。
    pub(super) fn move_by(&mut self, up: bool) {
        if up {
            self.selected = self.selected.saturating_sub(1);
        } else if self.selected + 1 < self.options.len() {
            self.selected += 1;
        }
    }
}

impl TuiApp<'_> {
    /// 丢掉按旧会话算出来的权限投影，并按当前会话重拉。
    ///
    /// 切会话 / 新建会话时用：投影里的生效档与 `available` 都属于**上一个**会话。留着它的
    /// 直接后果是菜单光标落在上一会话那档上（菜单按 `effective_level` 定初始项），回车一下
    /// 就把「为 A 选的档」写进了 B；状态行也会显示一个与 B 无关的档位。
    ///
    /// 先清再拉，而不是等新投影回来才换：清掉之后该段暂时不显示（说「不知道」，比说错一个
    /// 档位好），回包一到就恢复。
    pub(super) fn forget_permission_view(&mut self) {
        // 代数一起推进：切走前发出的 `POST /conversation/state` 回包可能还没回来，它带的是
        // **上一个**会话的写后投影；不推进就会被 `on_permission_written` 的代数校验放行，
        // 把刚清掉的 view 用旧会话的值填回来（菜单光标、状态行都会跟着错）。
        self.st.permission_generation = self.st.permission_generation.wrapping_add(1);
        // 已开的菜单同理：选项集是上一个会话的 `available`，留着让 Enter 一下就把那档写进新
        // 会话。（正常路径切不了会话 —— 模态菜单会吞掉按键 —— 但鼠标点左栏同样不该留下它。）
        self.st.permission_menu = None;
        if let Some(defaults) = self.st.serve_defaults.as_mut() {
            defaults.permission_view = None;
        }
        self.refresh_status();
    }

    /// 回合结束拿到 `conversation_id` 时收下它；**换了（或首次绑上）就重拉一次权限投影**。
    ///
    /// 为什么必须重拉：没有会话时 `/permission` 是裸请求，回的是「配置默认档 + 无会话意图」；
    /// 有了 id 才是「这个会话此刻哪一档」（会话意图 + min 链）。不重拉，状态行会一直拿裸投影
    /// 的默认档当这个会话的生效档 —— `status_values` 正是靠「有没有 id」区分这两件事的。
    /// 普通回合 id 不变，因此不额外打这两个请求。
    pub(super) fn adopt_turn_conversation(&mut self, conversation_id: &str) {
        let changed = self.st.conversation_id.as_deref() != Some(conversation_id);
        self.st.conversation_id = Some(conversation_id.to_string());
        if changed {
            self.refresh_status();
        }
    }

    /// `UiEvent::Status` 落地：只在**仍是同一个会话**时接受。
    ///
    /// 这次请求带着发起时的 `conversation_id` 走（`WorkerJob::RefreshStatus`），而 `/permission`
    /// 的生效档与 `available` 都是按会话算的；回包到达时用户可能已经切走或 `/conv new`，
    /// 收下就等于把上一个会话的档位写进当前状态行。丢弃而不是先收再清：切换 / 新建路径
    /// 已经带新 id 重发过刷新（见 [`Self::forget_permission_view`]）。
    pub(super) fn on_serve_status(
        &mut self,
        conversation_id: Option<String>,
        result: Result<ServeDefaults, String>,
    ) {
        if conversation_id.as_deref() != self.st.conversation_id.as_deref() {
            return;
        }
        match result {
            Ok(defaults) => self.st.serve_defaults = Some(defaults),
            Err(e) => self
                .st
                .push_line(LineKind::System, &format!("拉取 serve 状态失败：{e}")),
        }
    }

    /// 打开档位菜单，锚在 `anchor`（状态行 `perm` 段的矩形）。
    ///
    /// 没有投影就不开：`available` 是服务端算的，本地拿五档词表顶替会列出「不让选的档」。
    /// 这条提示同时也是「为什么点了没反应」的答案。
    pub(super) fn open_permission_menu(&mut self, anchor: Rect) {
        let Some(defaults) = self.st.serve_defaults.as_ref() else {
            self.st.push_line(
                LineKind::System,
                "permission level: serve status not loaded yet — try again in a moment",
            );
            return;
        };
        let options = defaults.available_levels().to_vec();
        if options.is_empty() {
            self.st.push_line(
                LineKind::System,
                "permission level: this serve reports no available levels (needs GET /permission)",
            );
            return;
        }
        let selected = defaults
            .effective_level()
            .and_then(|current| options.iter().position(|o| o == current))
            .unwrap_or(0);
        self.st.permission_menu = Some(PermissionMenu {
            options,
            selected,
            anchor,
        });
    }

    /// 菜单键：↑/↓ 移动、Enter 确认、Esc 取消。
    ///
    /// 返回 `false` 表示菜单没开（调用方继续后面的分发）。开着时**其它键一律吞掉**：
    /// 模态浮层期间让字符落进输入框，等于「看着菜单在打命令」。
    pub(super) fn on_permission_menu_key(&mut self, key: &KeyEvent) -> bool {
        let Some(menu) = self.st.permission_menu.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Up => menu.move_by(true),
            KeyCode::Down => menu.move_by(false),
            KeyCode::Enter => {
                let level = menu.current().map(str::to_string);
                self.st.permission_menu = None;
                if let Some(level) = level {
                    self.apply_permission_level(&level);
                }
            }
            KeyCode::Esc => self.st.permission_menu = None,
            _ => {}
        }
        true
    }

    /// 菜单打开时的点击：点在某一项上就确认，点菜单外关闭（与 Desktop 点遮罩关闭同义）。
    ///
    /// 返回 `true` 表示这次点击已被菜单消费 —— 否则同一次点击会继续落到下面的
    /// 「点侧栏 / 点聊天列」分支，表现为点一下菜单却切换了会话。
    pub(super) fn on_permission_menu_click(&mut self, column: u16, row: u16) -> bool {
        let Some(menu) = self.st.permission_menu.as_ref() else {
            return false;
        };
        let (width, height) = self.last_size;
        let rect = super::render::overlay::permission_menu_rect(
            Rect::new(0, 0, width, height),
            menu.anchor,
            menu.options.len(),
            menu.options
                .iter()
                .map(|o| o.chars().count())
                .max()
                .unwrap_or(0),
        );
        if !super::layout::contains(rect, column, row) {
            self.st.permission_menu = None;
            return true;
        }
        // 首行是边框：`row - rect.y - 1` 即选项下标。
        let idx = usize::from(row.saturating_sub(rect.y).saturating_sub(1));
        let level = self
            .st
            .permission_menu
            .as_ref()
            .and_then(|m| m.options.get(idx).cloned());
        let Some(level) = level else {
            // 框内但不在任何选项行上（标题行 / 下边框）：吞掉这次点击，菜单留着。
            // 按「点框外」关掉是不对的 —— 用户点的明明是框内，看到的是「点了却什么都没发生」，
            // 比菜单保持打开更难理解。
            return true;
        };
        self.st.permission_menu = None;
        self.apply_permission_level(&level);
        true
    }
}

#[cfg(test)]
mod menu_tests {
    use super::PermissionMenu;
    use ratatui::layout::Rect;

    /// 锚点随便给一个：这些用例只测光标，不测位置。
    fn anchor() -> Rect {
        Rect::new(0, 0, 4, 1)
    }

    fn menu() -> PermissionMenu {
        PermissionMenu {
            options: vec!["chat".into(), "read".into()],
            selected: 0,
            anchor: anchor(),
        }
    }

    #[test]
    fn cursor_stops_at_both_ends() {
        let mut m = menu();
        m.move_by(true);
        assert_eq!(m.selected, 0, "顶部不环绕");
        m.move_by(false);
        m.move_by(false);
        assert_eq!(m.selected, 1, "底部不越界");
        assert_eq!(m.current(), Some("read"));
    }

    #[test]
    fn empty_menu_has_no_current() {
        // `open_permission_menu` 会拒绝空选项集，这里只保证取值不 panic。
        let m = PermissionMenu {
            options: Vec::new(),
            selected: 0,
            anchor: anchor(),
        };
        assert_eq!(m.current(), None);
    }
}

#[cfg(test)]
mod session_guard_tests {
    use std::sync::mpsc;

    use super::super::TuiApp;
    use super::super::serve_defaults::ServeDefaults;
    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::super::worker::WorkerJob;
    use super::PermissionMenu;

    /// 造一份只带权限投影的 `ServeDefaults`（同 `mouse.rs` 的测试辅助：走 JSON 反序列化，
    /// 不把契约里的枚举/六轴变体名在本仓再抄一遍）。
    fn defaults_with_permission(level: &str) -> ServeDefaults {
        let json = format!(
            r#"{{"level":"{level}","level_source":"persisted","ceiling":"unrestricted",
                 "ceiling_source":"derived_loopback",
                 "available":["chat","read","workspace","approved","unrestricted"],
                 "clamped_by":[],
                 "caps":{{"tool_visibility":"full","external_tools":"allowed",
                          "path_scope":"any","command_scope":"any","net_scope":"any",
                          "approval":"audit_only"}}}}"#
        );
        ServeDefaults {
            permission_view: Some(serde_json::from_str(&json).expect("permission view")),
            ..Default::default()
        }
    }

    fn app<'a>(
        client: &'a super::super::super::serve::ServeClient,
        prefs: &'a mut crate::SessionPrefs,
        job_tx: mpsc::Sender<WorkerJob>,
    ) -> TuiApp<'a> {
        let (ui_tx, ui_rx) = mpsc::channel();
        test_app(client, prefs, ui_rx, ui_tx, job_tx)
    }

    /// 切会话后，上一个会话的写权限回包才到：代数已推进 → 不落盘、不提示、不覆盖新投影。
    #[test]
    fn switching_sessions_drops_an_in_flight_permission_write() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = app(&client, &mut prefs, job_tx);

        app.st.conversation_id = Some("A".to_string());
        app.st.serve_defaults = Some(defaults_with_permission("approved"));
        app.apply_permission_level("unrestricted");
        let stale = app.st.permission_generation;

        // 切会话 / `/conv new` 走的就是这一句（见 `switch_to_conv` / `reset_for_new_chat`）。
        app.forget_permission_view();
        assert_ne!(
            app.st.permission_generation, stale,
            "切换必须推进代数，否则旧回包会被当成合法的"
        );

        let lines = app.st.lines.len();
        app.on_permission_written("unrestricted", Err("boom".into()), stale);
        assert_eq!(
            app.st.lines.len(),
            lines,
            "过期回包不该写提示（它属于上一个会话）"
        );
        assert!(
            app.st
                .serve_defaults
                .as_ref()
                .expect("defaults")
                .permission_view
                .is_none(),
            "切换清掉的投影不能被旧会话的回包填回来"
        );
    }

    /// 切换时还开着的档位菜单一并关掉：它的选项集属于上一个会话。
    #[test]
    fn switching_sessions_closes_the_menu() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = app(&client, &mut prefs, job_tx);

        app.st.conversation_id = Some("A".to_string());
        app.st.permission_menu = Some(PermissionMenu {
            options: vec!["chat".into(), "read".into()],
            selected: 0,
            anchor: ratatui::layout::Rect::new(0, 0, 4, 1),
        });

        app.forget_permission_view();
        assert!(app.st.permission_menu.is_none(), "换会话后菜单不该留着");
    }

    /// 回包带着**别的**会话的 id：整份投影丢弃（`/permission` 按会话算）。
    #[test]
    fn stale_status_reply_is_dropped() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = app(&client, &mut prefs, job_tx);

        app.st.conversation_id = Some("B".to_string());
        let lines = app.st.lines.len();
        app.on_serve_status(
            Some("A".to_string()),
            Ok(defaults_with_permission("unrestricted")),
        );
        assert!(app.st.serve_defaults.is_none(), "A 的投影不能落到 B 上");

        // 失败提示同样只在「它还是当前会话」时才该出现。
        app.on_serve_status(Some("A".to_string()), Err("boom".into()));
        assert_eq!(app.st.lines.len(), lines, "过期回包连错误都不提示");
    }

    /// 会话对得上（包括「都还没有会话」）：照常落地。
    #[test]
    fn matching_status_reply_lands() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = app(&client, &mut prefs, job_tx);

        // 新会话：请求与当前都没有 cid，裸投影照收（`available` 就靠它，菜单要用）。
        app.on_serve_status(None, Ok(defaults_with_permission("approved")));
        assert!(app.st.serve_defaults.is_some());

        app.st.conversation_id = Some("B".to_string());
        app.on_serve_status(Some("B".to_string()), Ok(defaults_with_permission("read")));
        assert_eq!(
            app.st
                .serve_defaults
                .as_ref()
                .expect("defaults")
                .effective_level(),
            Some("read")
        );
    }

    /// 首次绑上（或换了）会话要带新 id 重拉一次；同一个 id 不重复打请求。
    #[test]
    fn adopting_a_turn_conversation_refreshes_the_projection_once() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, job_rx) = mpsc::channel();
        let mut app = app(&client, &mut prefs, job_tx);

        app.adopt_turn_conversation("c1");
        assert_eq!(app.st.conversation_id.as_deref(), Some("c1"));
        match job_rx.try_recv().expect("首次绑定要重拉投影") {
            WorkerJob::RefreshStatus { conversation_id } => {
                assert_eq!(conversation_id.as_deref(), Some("c1"));
            }
            _ => panic!("期望 RefreshStatus"),
        }

        app.adopt_turn_conversation("c1");
        assert!(job_rx.try_recv().is_err(), "同一个会话不该再打一次");
    }
}

#[cfg(test)]
mod click_tests {
    use std::sync::mpsc;

    use ratatui::layout::Rect;

    use super::super::TuiApp;
    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::PermissionMenu;

    /// 在 80×24 屏幕上开一个锚在右下角、3 个选项的菜单，并把渲染用的矩形交给断言。
    ///
    /// 矩形由 `permission_menu_rect`（绘制与命中**共用**的那一个）算出，测试不另算一份。
    fn with_menu(f: impl FnOnce(&mut TuiApp<'_>, Rect)) {
        let client = test_client();
        let mut prefs = test_prefs();
        let (tx, rx) = mpsc::channel();
        let (job_tx, _job_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, rx, tx, job_tx);
        app.last_size = (80, 24);
        let anchor = Rect::new(50, 22, 28, 1);
        app.st.permission_menu = Some(PermissionMenu {
            options: vec!["chat".into(), "read".into(), "approved".into()],
            selected: 0,
            anchor,
        });
        let rect = crate::tui_mode::render::overlay::permission_menu_rect(
            Rect::new(0, 0, 80, 24),
            anchor,
            3,
            8,
        );
        f(&mut app, rect);
    }

    /// 点下边框：不确认、也**不**关闭。框内的点击被当成「点外面」关掉，用户看到的是
    /// 「点了却什么都没发生」。
    #[test]
    fn clicking_the_border_keeps_the_menu_open() {
        with_menu(|app, rect| {
            let bottom = rect.y + rect.height - 1;
            assert!(
                app.on_permission_menu_click(rect.x + 1, bottom),
                "框内点击要被菜单消费"
            );
            assert!(app.st.permission_menu.is_some(), "菜单应保持打开");
            assert!(
                app.overrides.permission_level.is_none(),
                "边框不是选项，不该写入任何档位"
            );
        });
    }

    /// 点第一项：确认并关闭。
    #[test]
    fn clicking_an_item_confirms_it() {
        with_menu(|app, rect| {
            assert!(app.on_permission_menu_click(rect.x + 1, rect.y + 1));
            assert!(app.st.permission_menu.is_none(), "确认后关闭");
            assert_eq!(app.overrides.permission_level.as_deref(), Some("chat"));
        });
    }

    /// 点框外：关闭且不写入。
    #[test]
    fn clicking_outside_closes_without_picking() {
        with_menu(|app, rect| {
            assert!(app.on_permission_menu_click(0, rect.y.saturating_sub(2)));
            assert!(app.st.permission_menu.is_none(), "点框外关闭");
            assert!(app.overrides.permission_level.is_none());
        });
    }
}
