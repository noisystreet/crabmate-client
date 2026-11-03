//! 全屏 `tui` 的 `/conv export`：门控 + 报告。
//!
//! 单独成文件而不是写进 `mod.rs`：那里贴着 920 行门禁（同 `history.rs` / `help.rs` 的理由）。
//!
//! 导出是**只读**的：不写会话索引、不动 transcript、不改绑定。唯一的副作用是写用户要的
//! 文件，而写之前要先确认这份回包**仍属于当前会话** —— 拉取期间用户可能已经切走，把上一个
//! 会话的内容写成本次请求的文件名，比不导出更糟。

use crabmate_client_api::chat_export::ExportFormat;

use super::LaneApp;
use super::state::LineKind;
use super::worker::WorkerJob;
use crate::conv_export::{self, ExportCmd, ExportDone};

/// `/conv export`：解析结果 → 门控 → 发 worker job。
pub(super) fn run_command(app: &mut LaneApp<'_>, cmd: Result<ExportCmd, String>) {
    let cmd = match cmd {
        Ok(cmd) => cmd,
        Err(usage) => {
            app.st.push_line(LineKind::System, &usage);
            return;
        }
    };
    let bound = app.st.conversation_id.clone();
    let conversation_id = match gate(
        app.st.running,
        app.export_pending.is_some(),
        cmd.id.as_deref(),
        bound.as_deref(),
    ) {
        Ok(cid) => cid,
        Err(reason) => {
            app.st.push_line(LineKind::System, reason);
            return;
        }
    };
    let sent = app
        .job_tx
        .send(WorkerJob::ExportConversation {
            conversation_id: conversation_id.clone(),
            format: cmd.format,
            out_path: cmd.out.clone(),
        })
        .is_ok();
    if !sent {
        app.st
            .push_line(LineKind::System, "导出失败：worker 已退出");
        return;
    }
    app.export_pending = Some(conversation_id);
    app.st.push_line(
        LineKind::System,
        &format!("正在导出整段会话（{}）…", label(cmd.format)),
    );
}

/// 三道门 + 解析出这次要导的会话 id。`Err` 是直接写进 transcript 的一句话。
///
/// 每种拒绝都**说清楚**（照 `/older` 的 `OlderGate` 口径）：静默什么都不做，用户只会以为
/// 命令没被识别。
fn gate(
    running: bool,
    busy: bool,
    id: Option<&str>,
    bound: Option<&str>,
) -> Result<String, &'static str> {
    if running {
        return Err("回合进行中，导出会读到半截会话：等这一轮结束再导");
    }
    if busy {
        return Err("已有一次导出在进行：等它结束");
    }
    pick_conversation(id, bound)
}

/// 显式 `--id` 优先于当前绑定（导另一个会话正是 `--id` 的用途）；空白的那个视为没给。
fn pick_conversation(id: Option<&str>, bound: Option<&str>) -> Result<String, &'static str> {
    let candidate = id.and_then(non_blank).or_else(|| bound.and_then(non_blank));
    candidate
        .map(str::to_string)
        .ok_or("当前会话未绑定：先 /conv use <id>，或给 --id")
}

fn non_blank(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 回包：过期就丢（连文件都不写），否则落盘并报路径。
pub(super) fn on_result(app: &mut LaneApp<'_>, done: ExportDone) {
    let pending = app.export_pending.take();
    if pending.as_deref() != Some(done.conversation_id.as_str()) {
        app.st
            .push_line(LineKind::System, "导出结果已过期（期间换了会话），未写盘");
        return;
    }
    let artifacts = match done.result {
        Ok(artifacts) => artifacts,
        Err(e) => {
            app.st
                .push_line(LineKind::System, &format!("导出失败：{e}"));
            return;
        }
    };
    let stamp = conv_export::now_stamp();
    match conv_export::resolve_targets(&artifacts, done.out.as_deref(), &stamp)
        .and_then(|targets| conv_export::write_artifacts(&artifacts, &targets))
    {
        Ok(written) => {
            for line in conv_export::report_lines(&artifacts, &written, done.pages, done.messages) {
                app.st.push_line(LineKind::System, &line);
            }
        }
        Err(e) => app
            .st
            .push_line(LineKind::System, &format!("导出失败：{e}")),
    }
}

/// 报告里的格式名（`both` 说「JSON + Markdown」，与 Web `/export` 的提示同口径）。
fn label(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Json => "JSON",
        ExportFormat::Markdown => "Markdown",
        ExportFormat::Both => "JSON + Markdown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_names_each_refusal() {
        assert!(gate(true, false, Some("c1"), None).is_err(), "回合在跑要拒");
        assert!(gate(false, true, Some("c1"), None).is_err(), "在途导出要拒");
        let err = gate(false, false, None, None).expect_err("未绑定要拒");
        assert!(err.contains("--id"), "要告诉用户怎么补：{err}");
    }

    #[test]
    fn explicit_id_wins_over_the_binding() {
        assert_eq!(
            gate(false, false, Some("other"), Some("bound")),
            Ok("other".to_string())
        );
        assert_eq!(
            gate(false, false, None, Some("bound")),
            Ok("bound".to_string())
        );
        // 空白 `--id` 等于没给：不能拿空串去请求（会变成会话不存在）。
        assert_eq!(
            gate(false, false, Some("   "), Some("bound")),
            Ok("bound".to_string())
        );
    }

    #[test]
    fn labels_match_the_web_usage_words() {
        assert_eq!(label(ExportFormat::Json), "JSON");
        assert_eq!(label(ExportFormat::Markdown), "Markdown");
        assert_eq!(label(ExportFormat::Both), "JSON + Markdown");
    }
}
