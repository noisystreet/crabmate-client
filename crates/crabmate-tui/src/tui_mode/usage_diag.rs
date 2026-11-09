//! 逐轮用量诊断：`CM_TUI_SSE_DUMP` 打开时，每轮结束往该文件写一条记录。
//!
//! 为什么需要它：状态行「冻住」有两种完全不同的成因 —— **本轮一帧都没带快照**（客户端没有
//! 任何可显示的新值），或**快照不高于水位**（`max` 压制，ADR 0005 的刻意行为）。前者在现有
//! 实现下**不会自愈**（`has_usage_snapshot` 是会话级门控，缺过一轮就不再补拉，见 issue #472），
//! 而现场唯一的线索是「数字不动」这件事本身 —— 靠人盯状态行区分不了这两者。
//!
//! 这条记录把判定落进 dump 文件，下一次复现不用再猜。
//!
//! 只在 `CM_TUI_SSE_DUMP` 设置时写盘；**不写 stderr**：raw mode 下任何直写都会破坏画面，
//! 本 crate 也没有接 `tracing`。

use serde_json::{Map, Value};

/// 一轮回合的用量诊断状态；由事件循环单线程触碰。
#[derive(Debug, Default)]
pub struct TurnUsageDiag {
    seq: u64,
    measured: bool,
    model_output: bool,
}

impl TurnUsageDiag {
    /// 起回合：序号 +1，两个标志复位。`dispatch_turn` 是唯一入口。
    pub fn begin(&mut self) {
        self.seq = self.seq.wrapping_add(1);
        self.measured = false;
        self.model_output = false;
    }

    /// 本轮收到过携带 `tiktokenPromptTokens` 的帧（**任一帧**即可 —— 分类器不按帧类型过滤）。
    pub fn mark_measured(&mut self) {
        self.measured = true;
    }

    /// 本轮见过模型侧产出（正文 / 思维链 / 工具调用 / 工具结果）。
    ///
    /// 这是「本轮有没有 model call」的**代理指标**：客户端看不到调用计数，只能看有没有产出。
    /// 名字按「代理」写，不要改成 `model_called` —— 那个更强的断言本 crate 无法验证。
    pub fn mark_model_output(&mut self) {
        self.model_output = true;
    }

    /// 回合结束记录（纯计算，便于单测）。
    ///
    /// `cid` = 终止帧回传的会话 id（`None` = 终止帧没带）；`snapshot_present` = 会话里是否
    /// 已有快照 —— 两者合起来才能分辨「首轮就没拿到」「曾有但这一轮缺」「cid 都没回」。
    #[must_use]
    pub fn turn_end_record(&self, cid: Option<&str>, snapshot_present: bool) -> Value {
        let mut fields = Map::new();
        fields.insert("turn".to_string(), Value::from(self.seq));
        fields.insert("measured".to_string(), Value::from(self.measured));
        fields.insert("model_output".to_string(), Value::from(self.model_output));
        fields.insert(
            "has_session_snapshot".to_string(),
            Value::from(snapshot_present),
        );
        fields.insert(
            "cid".to_string(),
            cid.map_or(Value::Null, |c| Value::String(c.to_string())),
        );
        Value::Object(fields)
    }

    /// 写盘（未设 `CM_TUI_SSE_DUMP` 时是空操作）。
    ///
    /// **每轮都写**而不是只写缺口：判断「缺口是不是与工具调用密集 / 压缩同轮」需要正常轮次做
    /// 对照，只留异常行会让文件失去参照。每轮一行，体量可忽略。
    pub fn record_turn_end(&self, cid: Option<&str>, snapshot_present: bool) {
        crate::serve::chat_classify::append_diag(
            "turn_end",
            self.turn_end_record(cid, snapshot_present),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_bumps_seq_and_resets_flags() {
        let mut diag = TurnUsageDiag::default();
        diag.begin();
        diag.mark_measured();
        diag.mark_model_output();
        let first = diag.turn_end_record(Some("cid-1"), true);
        assert_eq!(first["turn"], 1);
        assert_eq!(first["measured"], true);
        assert_eq!(first["model_output"], true);
        // 下一轮：序号 +1，两个标志都复位 —— 漏复位会把缺帧的轮次误记成已实测。
        diag.begin();
        let second = diag.turn_end_record(Some("cid-1"), true);
        assert_eq!(second["turn"], 2, "每次起回合序号 +1");
        assert_eq!(second["measured"], false);
        assert_eq!(second["model_output"], false);
    }

    #[test]
    fn turn_end_record_distinguishes_gap_causes() {
        let mut diag = TurnUsageDiag::default();
        diag.begin();
        // 有产出、无实测：本轮没收到快照，但确实跑过模型 —— 这才是要找的缺口。
        diag.mark_model_output();
        let v = diag.turn_end_record(Some("cid-1"), true);
        assert_eq!(v["measured"], false);
        assert_eq!(v["model_output"], true);
        assert_eq!(v["has_session_snapshot"], true);
        assert_eq!(v["cid"], "cid-1");
        // 终止帧没回 cid 是另一类原因（补拉被 `if let Some(cid)` 挡住），必须可区分。
        let v = diag.turn_end_record(None, false);
        assert!(v["cid"].is_null());
        assert_eq!(v["has_session_snapshot"], false);
    }
}
