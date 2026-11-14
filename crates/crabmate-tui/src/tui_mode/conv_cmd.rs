//! `/conv` 子命令的落地：`list` / `use` / `new` / `export` / `filter` / `retry` / `edit` / 未知子命令。
//!
//! 从 `mod.rs` 的 `apply_control` 搬出来：那个 match 的臂已经很多，而该文件贴着 920 行
//! 门禁。每条命令的**拒绝理由**都在这里当面说清楚 —— 命令已经从输入框消失，不留痕等于
//! 什么都没发生。
//!
//! `retry` / `edit`（issue #422）的服务端半步是 `POST /chat/branch`：先让服务端把会话截到
//! 这一轮之前，再重发同一句话。**不能**只截本地视图就重发 —— Server 会持久化这一轮，
//! 于是会话里多出一条重复的用户消息（Desktop 的 CHANGELOG 记过这次分叉）。

use super::LaneApp;
use super::controls::{Control, conv_unknown_message};
use super::export;
use super::state::LineKind;
use super::worker::WorkerJob;
use crabmate_client_api::chat_branch::BranchFailure;

/// `/conv retry` 与 `/conv edit` 两个动作。
///
/// 只留 `retry` 一个名字（不做 `regen` 别名）：两件事在实现上完全同一条路径，
/// 差别只在回执文案 —— 同一件事两个名字，回执与文档迟早只更新一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCmd {
    /// 重发上一轮的用户消息（失败 / 被中断 / 不满意都走这条）。
    Retry,
    /// 把上一轮用户消息放进输入框，改完提交时再重跑。
    Edit,
}

/// `/conv retry` / `/conv edit` 的待办状态（随 `UiState` 走 = 每条 lane 一份）。
/// 一发在路上的 branch：**带上发起时的 `conversation_id`**。
///
/// 回包要跟当前会话比一次：branch 是几十毫秒到几秒的网络往返，期间切了会话，
/// 直接照回包截断就会把新会话截到别人的位置上。
#[derive(Debug, Clone)]
pub struct PendingTurnSend {
    pub conversation_id: String,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct PendingTurn {
    /// 待重发的那一发：`branch` 成功后由 [`LaneApp::on_turn_branched`] 取走。
    ///
    /// `Some` 同时充当「已经有一发 branch 在路上」的闸门，避免连按两次命令。
    pub send: Option<PendingTurnSend>,
    /// `/conv edit` 已把上一轮文本放进输入框，等用户改完回车。
    pub edit: bool,
}

impl PendingTurn {
    /// 切会话 / 新建会话 / 用户明确放弃时作废。
    pub fn clear(&mut self) {
        self.send = None;
        self.edit = false;
    }
}

impl LaneApp<'_> {
    /// `/conv …` 各分支的落地（调用方只把 `/conv` 的变体转进来）。
    pub(super) fn apply_conv_control(&mut self, control: Control) {
        match control {
            Control::ConvNew => self.new_session(),
            Control::ConvRefresh => {
                self.refresh_sessions();
                self.st.push_line(LineKind::System, "正在刷新会话列表…");
            }
            Control::ConvFilter(term) => self.apply_session_filter(term),
            Control::ConvTurn(TurnCmd::Retry) => self.retry_last_turn(),
            Control::ConvTurn(TurnCmd::Edit) => self.edit_last_turn(),
            Control::ConvUse(id) => {
                // 命令点名切到某条会话时顺手清掉过滤：那是「我知道要去哪」，
                // 留着过滤只会让切过去的行在左栏里看不见。
                self.st.clear_session_filter();
                self.st.pending_turn.clear();
                self.switch_to_conv(id, None);
            }
            Control::ConvExport(cmd) => export::run_command(self, cmd),
            Control::ConvUnknown(arg) => {
                let message = conv_unknown_message(&arg);
                self.st.push_line(LineKind::System, &message);
            }
            // 调用方只转 `/conv` 的变体（见 `mod.rs::apply_control` 的那条 `other @ (…)`）。
            _ => {}
        }
    }

    /// `/conv filter [<词>|off]`（`/conv list <词>` 同义）：设置 / 清除左栏标题过滤。
    ///
    /// 回执必须写清「剩几条」：设完词人一般直接按 Tab 去左栏，若那一步列表是空的，
    /// 一句「已过滤」等于让人自己猜是不是词打错了。
    fn apply_session_filter(&mut self, term: Option<String>) {
        let had_filter = self.st.session_filter_active();
        let visible = self.st.set_session_filter(term);
        let total = self.st.sessions.len();
        let text = match self.st.filter_term() {
            Some(term) => format!("会话过滤：「{term}」→ 可见 {visible}/{total} 条"),
            None if had_filter => "已清除会话过滤".to_string(),
            None => format!("会话过滤本就未启用（共 {total} 条）"),
        };
        self.st.push_line(LineKind::System, &text);
    }

    /// 三个动作共用的前置检查：返回 `Some(conversation_id)` 才允许继续。
    ///
    /// 三条拒绝都写清「为什么现在不行」，而不是按下去没反应 —— 这是 issue #422 的第 3 条。
    fn conv_turn_gate(&mut self) -> Option<String> {
        if self.st.running {
            self.st
                .push_line(LineKind::System, "回合进行中：先停止（Ctrl+C）再动上一轮");
            return None;
        }
        if self.st.has_queued() {
            self.st
                .push_line(LineKind::System, "有排队消息：先按 Esc 清掉再动上一轮");
            return None;
        }
        if self.st.pending_turn.send.is_some() {
            self.st
                .push_line(LineKind::System, "上一次重跑还在路上：稍候");
            return None;
        }
        let cid = self.st.conversation_id.clone();
        if cid.is_none() {
            self.st.push_line(
                LineKind::System,
                "这一轮还没在服务端落盘（没有 conversation_id）：直接重发即可，不必重跑",
            );
        }
        cid
    }

    /// `/conv retry`：把上一轮重跑一次。
    fn retry_last_turn(&mut self) {
        let Some(cid) = self.conv_turn_gate() else {
            return;
        };
        match self.st.last_user_turn() {
            Some((_, text)) => self.start_branch_then_send(text, cid),
            None => self
                .st
                .push_line(LineKind::System, "当前会话里没有可重跑的用户消息"),
        }
    }

    /// `/conv edit`：把上一轮的用户消息放回输入框；改完提交时**先 branch 再发**。
    ///
    /// 不在这一步 branch：那会让「打开来改、又决定不改」也把服务端历史截掉。
    fn edit_last_turn(&mut self) {
        if self.conv_turn_gate().is_none() {
            return;
        }
        match self.st.last_user_turn() {
            Some((_, text)) => {
                self.st.set_input(&text);
                self.st.pending_turn.edit = true;
                self.st.push_line(
                    LineKind::System,
                    "已把上一轮的用户消息放进输入框：改完回车即从这一轮重跑（不改则 Esc 放弃）",
                );
            }
            None => self
                .st
                .push_line(LineKind::System, "当前会话里没有可编辑的用户消息"),
        }
    }

    /// `Enter` 的统一入口：`/conv edit` 待办时先 branch，否则照常提交。
    ///
    /// 放在这里而不是在 `on_input_key` 里加一条带守卫的 arm：那条 match 已经贴着
    /// lizard 的 CCN 门禁（≤ 10），多一个守卫就顶过去。
    pub(super) fn submit_or_edit(&mut self) {
        if self.st.pending_turn.edit {
            let text = self.st.current_input();
            if self.on_edit_submit(text.trim()) {
                return;
            }
        }
        self.on_submit();
    }

    /// 提交时消费 `/conv edit`：返回 `true` = 已接管，不再走普通发送。
    pub(super) fn on_edit_submit(&mut self, text: &str) -> bool {
        if !self.st.pending_turn.edit {
            return false;
        }
        self.st.pending_turn.edit = false;
        // 提交前再走一遍闸门：输入框打开着的时候回合可能已经开跑了。
        if let Some(cid) = self.conv_turn_gate() {
            self.st.set_input("");
            self.start_branch_then_send(text.to_string(), cid);
        }
        true
    }

    /// 记下待重发的文本并入队 branch；任一步失败都把待办撤掉，免留下一个不会兑现的承诺。
    fn start_branch_then_send(&mut self, text: String, conversation_id: String) {
        self.st.pending_turn.send = Some(PendingTurnSend {
            conversation_id: conversation_id.clone(),
            text,
        });
        if self
            .job_tx
            .send(WorkerJob::ConvBranch { conversation_id })
            .is_err()
        {
            self.st.pending_turn.clear();
            self.st
                .push_line(LineKind::System, "回到上一轮失败：worker 已退出");
            return;
        }
        self.st
            .push_line(LineKind::System, "正在回到上一轮…（服务端截断后立即重跑）");
    }

    /// branch 回包：成功 → 本地截断 + 重发；失败按三分类分别交代。
    pub(super) fn on_turn_branched(&mut self, result: Result<u64, BranchFailure>) {
        let Some(pending) = self.st.pending_turn.send.take() else {
            return; // 待办已被撤销（Esc / 再次发起）：回包直接丢弃
        };
        if self.st.conversation_id.as_deref() != Some(pending.conversation_id.as_str()) {
            // 期间切了会话：不能拿这一发去截别人的 transcript。
            self.st.push_line(
                LineKind::System,
                "会话已切换：上一次「回到上一轮」的结果已丢弃",
            );
            return;
        }
        let text = pending.text;
        match result {
            Ok(_revision) => {
                // 本地半步：把 transcript 退到这一轮之前，让重跑的内容落在同一位置。
                self.st.truncate_to_last_user_turn();
                let head = first_line(&text);
                self.st.push_line(
                    LineKind::System,
                    &format!("已回到上一轮，正在重跑：「{head}」"),
                );
                self.start_turn(&text);
            }
            Err(BranchFailure::NotFound(msg)) => {
                // 会话在服务端没了（serve 重启 / 过期 / 别的端删了）：解开绑定，
                // 下一轮按新会话落盘，而不是继续往一个不存在的 id 上写。
                self.st.conversation_id = None;
                self.st.push_line(
                    LineKind::System,
                    &format!("会话在服务端不存在，已解除绑定：{msg}"),
                );
            }
            Err(e) => self.st.push_line(
                LineKind::System,
                &format!("未能回到上一轮：{}（本地记录未改动）", e.message()),
            ),
        }
    }
}

/// 回执里引用那句话时只取首行、最多 40 字符：整段贴进 transcript 会淹掉其它信息。
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    line.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_takes_one_trimmed_line() {
        assert_eq!(first_line("  第一行  \n第二行"), "第一行");
        assert_eq!(first_line(""), "");
        let long = "字".repeat(80);
        assert_eq!(first_line(&long).chars().count(), 40);
    }

    /// 待办的生命周期：`clear` 之后两个方向都不能再被兑现。
    #[test]
    fn pending_turn_clear_drops_both_halves() {
        let mut p = PendingTurn {
            send: Some(PendingTurnSend {
                conversation_id: "c1".into(),
                text: "x".into(),
            }),
            edit: true,
        };
        p.clear();
        assert!(p.send.is_none());
        assert!(!p.edit);
    }
}
