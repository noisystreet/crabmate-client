//! `/conv` 子命令的落地：`list` / `use` / `new` / `export` / 未知子命令。
//!
//! 从 `mod.rs` 的 `apply_control` 搬出来：那个 match 的臂已经很多，而该文件贴着 920 行
//! 门禁。每条命令的**拒绝理由**都在这里当面说清楚 —— 命令已经从输入框消失，不留痕等于
//! 什么都没发生。

use super::LaneApp;
use super::controls::{Control, conv_unknown_message};
use super::export;
use super::state::LineKind;

impl LaneApp<'_> {
    /// `/conv …` 五个分支的落地（调用方只把 `/conv` 的变体转进来）。
    pub(super) fn apply_conv_control(&mut self, control: Control) {
        match control {
            Control::ConvNew => self.new_session(),
            Control::ConvRefresh => {
                self.refresh_sessions();
                self.st.push_line(LineKind::System, "正在刷新会话列表…");
            }
            Control::ConvFilter(term) => self.apply_session_filter(term),
            Control::ConvUse(id) => {
                // 命令点名切到某条会话时顺手清掉过滤：那是「我知道要去哪」，
                // 留着过滤只会让切过去的行在左栏里看不见。
                self.st.clear_session_filter();
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
}
