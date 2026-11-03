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
            Control::ConvUse(id) => self.switch_to_conv(id, None),
            Control::ConvExport(cmd) => export::run_command(self, cmd),
            Control::ConvUnknown(arg) => {
                let message = conv_unknown_message(&arg);
                self.st.push_line(LineKind::System, &message);
            }
            // 调用方只转 `/conv` 的变体（见 `mod.rs::apply_control` 的那条 `other @ (…)`）。
            _ => {}
        }
    }
}
