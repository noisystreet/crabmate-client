//! `/conv export` 的非 UI 部分：参数解析 / 正文渲染 / 落盘。
//!
//! 与 UI 分开的理由和 `tui_mode/export.rs` 那层一样：行模式 `repl` 与全屏 `tui` 共用
//! 同一套措辞与落盘规则，只有「结果往哪去」不同（stdout vs transcript）。纯逻辑 + IO
//! 都在这里，UI 层只做门控与报告，两边因此不会各自长出半套规则。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crabmate::cm_chat_export::DisplayExportMessage;
use crabmate_client_api::chat_export::{
    ExportFormat, export_filename_stem, export_json, export_markdown,
};

/// 一次导出请求（`/conv export [json|markdown|both] [--out <路径>] [--id <会话 id>]`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportCmd {
    pub format: ExportFormat,
    pub out: Option<String>,
    pub id: Option<String>,
}

/// 解析 `export` 之后的参数（**不含** `export` 本身）。
///
/// 位置参数只认一个格式词（与 Web `/export [json|markdown|both]` 同形），其余全走
/// `--out` / `--id`：路径与会话 id 都塞进位置参数时，一个带空格或恰好像 id 的路径就分不清了。
pub fn parse_args(raw: &str) -> Result<ExportCmd, String> {
    let mut format = None;
    let mut out = None;
    let mut id = None;
    let mut words = raw.split_whitespace();
    while let Some(word) = words.next() {
        match word {
            "--out" | "-o" => out = Some(flag_value(&mut words, "--out")?),
            "--id" => id = Some(flag_value(&mut words, "--id")?),
            other if other.starts_with('-') => {
                return Err(format!("未知参数 {other}；{}", ExportFormat::USAGE));
            }
            other => {
                if format.is_some() {
                    return Err(format!("多余的参数 {other}；{}", ExportFormat::USAGE));
                }
                format = Some(
                    ExportFormat::parse(other)
                        .ok_or_else(|| format!("不支持的格式 {other}；{}", ExportFormat::USAGE))?,
                );
            }
        }
    }
    Ok(ExportCmd {
        format: format.unwrap_or(ExportFormat::Json),
        out,
        id,
    })
}

fn flag_value<'a>(words: &mut impl Iterator<Item = &'a str>, flag: &str) -> Result<String, String> {
    words
        .next()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{flag} 缺参数值；{}", ExportFormat::USAGE))
}

/// 导出件的一段正文；`ext` 决定默认文件名的后缀。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub ext: &'static str,
    pub body: String,
}

impl Artifact {
    /// 报告行里给人的格式名（与 Web `/export` 的提示词一致：`JSON` / `Markdown`）。
    #[must_use]
    pub fn label(&self) -> &'static str {
        if self.ext == "json" {
            "JSON"
        } else {
            "Markdown"
        }
    }
}

/// 导出消息 → 要写出的正文。`both` 给两段，顺序固定 JSON 在前。
pub fn render_artifacts(
    format: ExportFormat,
    messages: &[DisplayExportMessage],
) -> Result<Vec<Artifact>, String> {
    let mut out = Vec::new();
    if format.wants_json() {
        out.push(Artifact {
            ext: "json",
            body: export_json(messages)?,
        });
    }
    if format.wants_markdown() {
        out.push(Artifact {
            ext: "md",
            body: export_markdown(messages),
        });
    }
    Ok(out)
}

/// 一次导出任务的结果（worker → UI）：正文已在 worker 里渲染好，写盘留在 UI 线程 ——
/// 只有那边知道这次导出是否还属于当前会话绑定。
#[derive(Debug)]
pub struct ExportDone {
    pub conversation_id: String,
    pub out: Option<String>,
    pub pages: u32,
    pub messages: usize,
    pub result: Result<Vec<Artifact>, String>,
}

/// `~` / `~/…` → `$HOME/…`；`~user` 直接拒绝（要查 passwd / NSS，猜错会静默落进别人的目录）。
///
/// 刻意**不**复用 [`crate::workspace_path::resolve`]：那个路径属于 **serve 主机**
/// （还带 `CM_TUI_EXPAND_TILDE` 开关），而导出文件永远落在**本机**，必须展开。
pub fn expand_tilde(raw: &str) -> Result<String, String> {
    let path = raw.trim();
    if !path.starts_with('~') {
        return Ok(path.to_string());
    }
    let home = std::env::var("HOME").map_err(|_| "HOME 未设置，无法展开 ~".to_string())?;
    let home = home.trim_end_matches('/');
    if path == "~" {
        return Ok(home.to_string());
    }
    match path.strip_prefix("~/") {
        Some(rest) => Ok(format!("{home}/{rest}")),
        None => Err(format!("不支持 ~user 形式（{path}）；请写明确的路径")),
    }
}

/// 落盘目标：没给 `--out` → 当前目录的默认文件名；给了目录 → 目录 + 默认文件名；
/// 给了文件名 → 用它（`both` 时只换后缀，保住用户给的主干）。
pub fn resolve_targets(
    artifacts: &[Artifact],
    out: Option<&str>,
    stamp: &str,
) -> Result<Vec<PathBuf>, String> {
    let default_name = |ext: &str| format!("{stamp}.{ext}");
    let Some(raw) = out else {
        return Ok(artifacts
            .iter()
            .map(|a| PathBuf::from(default_name(a.ext)))
            .collect());
    };
    let base = PathBuf::from(expand_tilde(raw)?);
    if raw.ends_with('/') || base.is_dir() {
        return Ok(artifacts
            .iter()
            .map(|a| base.join(default_name(a.ext)))
            .collect());
    }
    if artifacts.len() == 1 {
        return Ok(vec![base]);
    }
    let stem = base.file_stem().map_or_else(
        || "chat_export".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let dir = base.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    Ok(artifacts
        .iter()
        .map(|a| dir.join(format!("{stem}.{}", a.ext)))
        .collect())
}

/// 写盘，返回实际写成的路径（顺序与 `artifacts` 一致）。
///
/// 两段里第二段失败时**第一段已经落盘**：错误信息只说失败的那一条，不假装整体回滚 ——
/// 回滚要删掉用户可能想要的半成品，代价比说清楚更大。
pub fn write_artifacts(
    artifacts: &[Artifact],
    targets: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::with_capacity(artifacts.len());
    for (artifact, path) in artifacts.iter().zip(targets) {
        std::fs::write(path, &artifact.body)
            .map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
        written.push(path.clone());
    }
    Ok(written)
}

/// 逐条报告行：`已导出 Markdown（12 页 / 348 条消息）→ /abs/x.md`。
#[must_use]
pub fn report_lines(
    artifacts: &[Artifact],
    written: &[PathBuf],
    pages: u32,
    messages: usize,
) -> Vec<String> {
    artifacts
        .iter()
        .zip(written)
        .map(|(artifact, path)| {
            format!(
                "已导出 {}（{pages} 页 / {messages} 条消息）→ {}",
                artifact.label(),
                absolute_display(path)
            )
        })
        .collect()
}

/// 把正文打到 stdout（行模式 `repl` 的默认出口）；多段之间空一行。
pub fn print_artifacts(artifacts: &[Artifact]) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    for (i, artifact) in artifacts.iter().enumerate() {
        if i > 0 {
            writeln!(stdout)?;
        }
        stdout.write_all(artifact.body.as_bytes())?;
        if !artifact.body.ends_with('\n') {
            writeln!(stdout)?;
        }
    }
    stdout.flush()
}

/// 当前时间的文件名主干（UTC；见 [`crabmate_client_api::chat_export::export_filename_stem`]）。
#[must_use]
pub fn now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    export_filename_stem(secs)
}

fn absolute_display(path: &Path) -> String {
    std::fs::canonicalize(path).map_or_else(
        |_| path.display().to_string(),
        |abs| abs.display().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_format_and_flags() {
        assert_eq!(
            parse_args(""),
            Ok(ExportCmd {
                format: ExportFormat::Json,
                out: None,
                id: None
            })
        );
        assert_eq!(
            parse_args("md --out /tmp/a.md"),
            Ok(ExportCmd {
                format: ExportFormat::Markdown,
                out: Some("/tmp/a.md".to_string()),
                id: None
            })
        );
        assert_eq!(
            parse_args("--id c-1 both"),
            Ok(ExportCmd {
                format: ExportFormat::Both,
                out: None,
                id: Some("c-1".to_string())
            })
        );
        assert!(parse_args("markdown markdown").is_err(), "重复格式词要拒");
        assert!(parse_args("pdf").is_err(), "未知格式要拒");
        assert!(parse_args("--nope").is_err(), "未知 flag 要拒");
        assert!(parse_args("--out").is_err(), "缺值要拒");
    }

    #[test]
    fn tilde_expansion_only_takes_the_home_form() {
        assert_eq!(expand_tilde("/tmp/x").as_deref(), Ok("/tmp/x"));
        assert!(expand_tilde("~user/x").is_err(), "~user 不该猜");
        if let Ok(home) = std::env::var("HOME") {
            let home = home.trim_end_matches('/');
            assert_eq!(expand_tilde("~"), Ok(home.to_string()));
            assert_eq!(expand_tilde("~/a.json"), Ok(format!("{home}/a.json")));
        }
    }

    #[test]
    fn targets_follow_the_shape_of_each_format() {
        let both = vec![
            Artifact {
                ext: "json",
                body: "{}\n".to_string(),
            },
            Artifact {
                ext: "md",
                body: "# t\n".to_string(),
            },
        ];
        // 无 --out：当前目录的默认文件名
        let names = resolve_targets(&both, None, "chat_export_X").expect("targets");
        assert_eq!(
            names,
            vec![
                PathBuf::from("chat_export_X.json"),
                PathBuf::from("chat_export_X.md")
            ]
        );
        // 给目录：目录 + 默认文件名
        let dir = std::env::temp_dir();
        let in_dir = resolve_targets(&both, Some(dir.to_str().expect("utf8")), "s").expect("dir");
        assert_eq!(in_dir[0], dir.join("s.json"));
        // 给文件名 + both：只换后缀
        let named = resolve_targets(&both, Some("/tmp/report.txt"), "s").expect("named");
        assert_eq!(
            named,
            vec![
                PathBuf::from("/tmp/report.json"),
                PathBuf::from("/tmp/report.md")
            ]
        );
        // 给文件名 + 单段：原样用
        let one = resolve_targets(&both[1..], Some("/tmp/one.md"), "s").expect("one");
        assert_eq!(one, vec![PathBuf::from("/tmp/one.md")]);
    }

    #[test]
    fn writes_and_reports_the_absolute_path() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("crabmate_export_test_{}.md", std::process::id()));
        let artifacts = vec![Artifact {
            ext: "md",
            body: "# 导出\n".to_string(),
        }];
        let written =
            write_artifacts(&artifacts, std::slice::from_ref(&path)).expect("write lands");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "# 导出\n"
        );
        let lines = report_lines(&artifacts, &written, 2, 7);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Markdown"));
        assert!(lines[0].contains("2 页 / 7 条消息"));
        assert!(lines[0].contains("crabmate_export_test_"));
        let _ = std::fs::remove_file(&path);
    }

    /// 写不进去要给出路径与原因，而不是静默产出一个不存在的导出件。
    #[test]
    fn unwritable_target_reports_the_path() {
        let path = PathBuf::from("/proc/definitely/not/writable/x.md");
        let artifacts = vec![Artifact {
            ext: "md",
            body: "x".to_string(),
        }];
        let err = write_artifacts(&artifacts, &[path]).expect_err("must fail");
        assert!(err.contains("x.md"), "{err}");
    }
}
