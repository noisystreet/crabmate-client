//! 全屏 `tui` 的 `/conv export`：门控 + 报告。
//!
//! 单独成文件而不是写进 `mod.rs`：那里贴着 920 行门禁（同 `history.rs` / `help.rs` 的理由）。
//!
//! 导出是**只读**的：不写会话索引、不动 transcript、不改绑定。唯一的副作用是写用户要的
//! 文件，而**回包无条件落盘**：导出件属于「请求时那个会话」（正文与文件名自带归属），
//! 中途 `/conv use` 切走只是换了视图、不是撤销请求 —— 把已经翻完的活丢掉比写出来更糟。
//! 一次只允许一个导出在途（`export_in_flight`）：两个回包会往同一个默认文件名上抢。

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
        app.export_in_flight,
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
            conversation_id,
            format: cmd.format,
            out_path: cmd.out.clone(),
        })
        .is_ok();
    if !sent {
        app.st
            .push_line(LineKind::System, "导出失败：worker 已退出");
        return;
    }
    app.export_in_flight = true;
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

/// 回包：先清在途标记再落盘并报路径（无条件写，理由见模块注释）。
///
/// 期间切走过要**点名归属**：报告落进的是当前视图，不写清楚「这是哪个会话的导出件」，
/// 用户会以为眼前这段对话刚被导出。
pub(super) fn on_result(app: &mut LaneApp<'_>, done: ExportDone) {
    app.export_in_flight = false;
    let switched = app.st.conversation_id.as_deref() != Some(done.conversation_id.as_str());
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
            if switched {
                app.st.push_line(
                    LineKind::System,
                    &format!(
                        "（这次导出的是会话 {}，不是你正在看的那个）",
                        done.conversation_id
                    ),
                );
            }
        }
        Err(e) => app
            .st
            .push_line(LineKind::System, &format!("导出失败：{e}")),
    }
}

/// Ctrl+C 的接管：导出没有取消通道（它不像回合那样有 `cancel` 令牌），所以**先挡住**
/// 「空闲 lane 直接退出」那条分支 —— 屏幕上根本看不出还有活在做，一下 Ctrl+C 退出就会
/// 把已经翻了几十页的结果一起丢掉。返回 `true` = 这次 Ctrl+C 被这里吃掉。
pub(super) fn on_ctrl_c(app: &mut LaneApp<'_>) -> bool {
    if !app.export_in_flight {
        return false;
    }
    app.st.push_line(
        LineKind::System,
        "导出进行中：它不可中断（翻完全部页后自动写盘），稍候",
    );
    true
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
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;

    use super::super::test_support::{test_app, test_client, test_prefs};
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("crabmate_export_{tag}_{}.md", std::process::id()))
    }

    fn done(out: &Path, conversation_id: &str) -> ExportDone {
        ExportDone {
            conversation_id: conversation_id.to_string(),
            out: Some(out.display().to_string()),
            pages: 3,
            messages: 9,
            result: Ok(vec![conv_export::Artifact {
                ext: "md",
                body: "# 导出\n".to_string(),
            }]),
        }
    }

    fn transcript(app: &LaneApp<'_>) -> String {
        app.st
            .lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

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

    /// 回包**无条件落盘**：期间 `/conv use` 切走也要写（导出件属于请求时那个会话），
    /// 只在报告里点名归属。旧实现比的是「回包 id vs 请求 id」—— 两者恒等，
    /// 那句「切走就不写盘」的注释与 CHANGELOG 从来不是真的。
    #[test]
    fn a_finished_export_lands_even_after_switching_away() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        let path = temp_path("switch");
        app.st.conversation_id = Some("c2".to_string());
        app.export_in_flight = true;

        on_result(&mut app, done(&path, "c1"));

        assert!(!app.export_in_flight, "回包要清在途标记，否则再也导不了");
        assert_eq!(
            std::fs::read_to_string(&path).expect("导出件应已写盘"),
            "# 导出\n"
        );
        let text = transcript(&app);
        assert!(text.contains("已导出 Markdown"), "{text}");
        assert!(text.contains("会话 c1"), "切走后要点名归属：{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// 没切走就不加那句归属提示：正常报告不该多一行噪音。
    #[test]
    fn the_ownership_note_only_shows_up_after_a_switch() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        let path = temp_path("same");
        app.st.conversation_id = Some("c1".to_string());

        on_result(&mut app, done(&path, "c1"));

        let text = transcript(&app);
        assert!(text.contains("已导出 Markdown"), "{text}");
        assert!(!text.contains("不是你正在看的那个"), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// 失败也要清在途标记：否则一次网络错误就永久锁死 `/conv export`。
    #[test]
    fn a_failed_export_clears_the_in_flight_flag() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);
        let path = temp_path("fail");
        app.export_in_flight = true;

        on_result(
            &mut app,
            ExportDone {
                conversation_id: "c1".to_string(),
                out: Some(path.display().to_string()),
                pages: 0,
                messages: 0,
                result: Err("serve 断了".to_string()),
            },
        );

        assert!(!app.export_in_flight);
        assert!(!path.exists(), "失败不该留下空文件");
        assert!(transcript(&app).contains("导出失败：serve 断了"));
    }

    /// 导出在途时 Ctrl+C 只出一句提示，不退程序（否则白导一次）。
    #[test]
    fn ctrl_c_does_not_quit_while_an_export_is_running() {
        let client = test_client();
        let mut prefs = test_prefs();
        let (job_tx, _job_rx) = mpsc::channel();
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut app = test_app(&client, &mut prefs, ui_rx, ui_tx, job_tx);

        assert!(!on_ctrl_c(&mut app), "空闲时不该接管 Ctrl+C");

        app.export_in_flight = true;
        assert!(on_ctrl_c(&mut app), "导出在途时 Ctrl+C 被这里吃掉");
        assert!(!app.exit, "不能顺手退出");
        assert!(transcript(&app).contains("不可中断"));
    }
}
