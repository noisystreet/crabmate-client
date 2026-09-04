//! `crabmate-tui`：连接远程 `crabmate serve` 的终端客户端（P3：chat / repl / 斜杠）。

mod approval_tty;
mod slash;
mod tui_mode;
mod turn;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crabmate_client_api::secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT};
use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOutcome, ClientLlm,
    CommandApprovalRequest, ConnectionConfig, ServeClient, StreamResume, TermError,
};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

use crate::approval_tty::TtyApprovalGate;
use crate::slash::{handle_control_slash, is_control_slash};
use crate::turn::{TurnPrefs, run_turn};

#[derive(Debug, Parser)]
#[command(
    name = "crabmate-tui",
    about = "CrabMate remote terminal client (connects to crabmate serve via HTTP/SSE)",
    version
)]
struct Cli {
    /// Absolute serve root URL (e.g. http://127.0.0.1:8080).
    #[arg(long = "api-base", env = "CRABMATE_API_BASE", global = true)]
    api_base: Option<String>,

    /// Web API Bearer (`CM_WEB_API_BEARER_TOKEN` on serve). Not the model API_KEY.
    #[arg(long = "bearer", env = "CM_WEB_API_BEARER_TOKEN", global = true)]
    bearer: Option<String>,

    /// 随每轮 chat 发送的 `client_llm.api_key`（同 WASM UI「设置 → API 密钥」；
    /// 供 serve 无服务端模型 `API_KEY` 时使用，如 bearer 模式个人云）。
    #[arg(long = "llm-api-key", env = "CM_API_KEY", global = true)]
    llm_api_key: Option<String>,

    /// 模型名覆盖 → `client_llm.model`。
    #[arg(long = "llm-model", env = "CM_MODEL", global = true)]
    llm_model: Option<String>,

    /// 模型供应商 base URL 覆盖 → `client_llm.api_base`。
    #[arg(long = "llm-api-base", env = "CM_API_BASE", global = true)]
    llm_api_base: Option<String>,

    /// 不回退读取桌面壳已保存的钥匙串（默认：`--bearer` / `--llm-api-key` 缺省时回退）。
    #[arg(long = "no-keyring", global = true)]
    no_keyring: bool,

    /// Auto-approve non-allowlisted commands (`allow_once`). Dangerous.
    #[arg(long = "yes", global = true)]
    yes: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Probe `GET /health`.
    Connect,
    /// One-shot chat via `POST /chat/stream` (assistant text on stdout).
    Chat {
        /// User message. If omitted, read stdin.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        message: Vec<String>,
        /// Optional conversation id for continuation.
        #[arg(long = "conversation-id")]
        conversation_id: Option<String>,
        /// Skip `/health` probe before chat.
        #[arg(long = "no-probe")]
        no_probe: bool,
    },
    /// Interactive REPL (reedline); keeps `conversation_id` across turns.
    Repl {
        /// Optional conversation id to resume.
        #[arg(long = "conversation-id")]
        conversation_id: Option<String>,
        /// Skip `/health` probe before first turn.
        #[arg(long = "no-probe")]
        no_probe: bool,
    },
    /// Full-screen TUI (ratatui): status bar + streaming transcript + input bar.
    Tui {
        /// Skip `/health` probe before first turn.
        #[arg(long = "no-probe")]
        no_probe: bool,
    },
}

enum AnyApprovalGate {
    Auto(AutoAllowOnce),
    Tty(TtyApprovalGate),
}

/// 最近一次「serve 侧仍在跑但本端断开」的回合断点（供 `/resume` 续流）。
struct ResumePoint {
    message: String,
    conversation_id: Option<String>,
    job_id: u64,
    after_seq: u64,
}

/// repl 会话内可变的偏好：`client_llm` 覆盖（model/api_base/api_key）+ agent role + session mode。
/// chat 一次性用 `prefs()` 派生；空白值在发送层自动省略。
struct SessionPrefs {
    api_key: Option<String>,
    model: Option<String>,
    api_base: Option<String>,
    agent_role: Option<String>,
    session_mode: Option<String>,
}

impl SessionPrefs {
    fn from_options(
        api_key: Option<String>,
        model: Option<String>,
        api_base: Option<String>,
    ) -> Self {
        Self {
            api_key,
            model,
            api_base,
            agent_role: None,
            session_mode: None,
        }
    }

    /// 有任一非空字段时生成随本轮发送的 `client_llm`（空白值不发送）。
    fn client_llm(&self) -> Option<ClientLlm<'_>> {
        client_llm_overrides(
            self.api_key.as_deref(),
            self.model.as_deref(),
            self.api_base.as_deref(),
        )
    }

    /// 随本轮发送的完整偏好（client_llm + agent_role + session_mode）。
    fn prefs(&self) -> TurnPrefs<'_> {
        TurnPrefs {
            client_llm: self.client_llm(),
            agent_role: self.agent_role.as_deref(),
            session_mode: self.session_mode.as_deref(),
        }
    }
}

impl ApprovalGate for AnyApprovalGate {
    fn decide(&mut self, req: &CommandApprovalRequest) -> Result<ApprovalDecision, TermError> {
        match self {
            Self::Auto(g) => g.decide(req),
            Self::Tty(g) => g.decide(req),
        }
    }
}

fn make_gate(yes: bool) -> AnyApprovalGate {
    if yes {
        AnyApprovalGate::Auto(AutoAllowOnce)
    } else {
        AnyApprovalGate::Tty(TtyApprovalGate::new())
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if is_interrupted(&e) {
                return ExitCode::from(130);
            }
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let bearer = resolve_bearer(&cli);
    let client = build_client(&cli, bearer.as_deref())?;
    let Cli {
        llm_api_key,
        llm_model,
        llm_api_base,
        no_keyring,
        yes,
        command,
        ..
    } = cli;
    let llm_key = resolve_llm_key(llm_api_key.as_deref(), no_keyring);
    let mut overrides = SessionPrefs::from_options(llm_key, llm_model, llm_api_base);
    match command {
        Commands::Connect => run_connect(&client).await,
        Commands::Chat {
            message,
            conversation_id,
            no_probe,
        } => run_chat(&client, message, conversation_id, no_probe, yes, &overrides).await,
        Commands::Repl {
            conversation_id,
            no_probe,
        } => run_repl(&client, conversation_id, no_probe, yes, &mut overrides).await,
        Commands::Tui { no_probe } => run_tui_cmd(&client, no_probe, yes, &overrides).await,
    }
}

async fn run_tui_cmd(
    client: &ServeClient,
    no_probe: bool,
    yes: bool,
    overrides: &SessionPrefs,
) -> Result<()> {
    ensure_tty()?;
    maybe_probe(client, no_probe).await?;
    tui_mode::run_tui(client, overrides, yes).await
}

/// 三个 `client_llm` 覆盖中任一非空才构造；全空返回 `None`（不发送 `client_llm` 整块）。
fn client_llm_overrides<'a>(
    api_key: Option<&'a str>,
    model: Option<&'a str>,
    api_base: Option<&'a str>,
) -> Option<ClientLlm<'a>> {
    let llm = ClientLlm {
        api_key,
        model,
        api_base,
    };
    let any = [api_key, model, api_base]
        .into_iter()
        .any(|v| v.is_some_and(|s| !s.trim().is_empty()));
    any.then_some(llm)
}

/// `--bearer` / `CM_WEB_API_BEARER_TOKEN` 缺省（未提供）时回退读桌面壳钥匙串的 Web Bearer 槽。
fn resolve_bearer(cli: &Cli) -> Option<String> {
    let cli_value = cli.bearer.as_deref();
    let fell_back = !cli.no_keyring && cli_value.is_none();
    let value = with_shell_keyring_fallback(cli_value, cli.no_keyring, || {
        keyring_secret(WEB_API_BEARER_KEYRING_ACCOUNT)
    });
    if fell_back && value.is_some() {
        eprintln!(
            "[crabmate-tui] Web Bearer 未提供，已回退读取桌面壳钥匙串（--no-keyring 可关闭）"
        );
    }
    value
}

/// `--llm-api-key` / `CM_API_KEY` 缺省（未提供）时回退读桌面壳钥匙串的 `client_llm` 密钥槽。
fn resolve_llm_key(llm_api_key: Option<&str>, no_keyring: bool) -> Option<String> {
    let fell_back = !no_keyring && llm_api_key.is_none();
    let value = with_shell_keyring_fallback(llm_api_key, no_keyring, || {
        keyring_secret(SecretSlot::ClientLlm.keyring_account())
    });
    if fell_back && value.is_some() {
        eprintln!(
            "[crabmate-tui] client_llm API key 未提供，已回退读取桌面壳钥匙串（--no-keyring 可关闭）"
        );
    }
    value
}

/// 显式值优先：`Some(非空)` 用显式值；`Some(空)` 表示"明确不带、不回退"；
/// 仅 `None`（未提供）且未禁用回退时才交给 `read`（钥匙串）补位。
fn with_shell_keyring_fallback(
    cli_value: Option<&str>,
    no_keyring: bool,
    read: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match cli_value {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None if no_keyring => None,
        None => read(),
    }
}

/// 读系统钥匙串条目（service 与桌面壳一致：`com.crabmate.credentials`）。
/// 无条目 / 钥匙串不可用 / 空值返回 `None`（调用方静默回退）。
fn keyring_secret(account: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    let raw = entry.get_password().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_client(cli: &Cli, bearer: Option<&str>) -> Result<ServeClient> {
    let api_base = cli
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .with_context(
            || "missing --api-base / CRABMATE_API_BASE (absolute http(s) URL to crabmate serve)",
        )?
        .to_string();
    let bearer = bearer
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    Ok(ServeClient::new(ConnectionConfig::new(api_base, bearer))?)
}

async fn run_connect(client: &ServeClient) -> Result<()> {
    client.probe_health().await?;
    println!("ok: {}", client.config().api_base);
    Ok(())
}

async fn run_chat(
    client: &ServeClient,
    message: Vec<String>,
    conversation_id: Option<String>,
    no_probe: bool,
    yes: bool,
    overrides: &SessionPrefs,
) -> Result<()> {
    maybe_probe(client, no_probe).await?;
    let text = resolve_message(message)?;
    if text.trim().is_empty() {
        bail!("empty message");
    }
    let mut gate = make_gate(yes);
    let prefs = overrides.prefs();
    let outcome = run_turn(
        client,
        &text,
        conversation_id.as_deref(),
        prefs,
        None,
        &mut gate,
    )
    .await?;
    // 单轮 chat：Ctrl+C 取消后保持 SIGINT 语义（130），而不是静默 0 退出。
    if outcome.cancelled_by_user {
        return Err(TermError::Interrupted.into());
    }
    finish_turn_stdout(&outcome);
    Ok(())
}

async fn run_repl(
    client: &ServeClient,
    conversation_id: Option<String>,
    no_probe: bool,
    yes: bool,
    overrides: &mut SessionPrefs,
) -> Result<()> {
    ensure_tty()?;
    maybe_probe(client, no_probe).await?;
    print_repl_banner(client, conversation_id.as_deref());
    let mut conversation_id = conversation_id;
    let mut resume: Option<ResumePoint> = None;
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("crabmate> ".to_string()),
        DefaultPromptSegment::Empty,
    );
    loop {
        match editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                if !dispatch_repl_line(
                    client,
                    &line,
                    &mut conversation_id,
                    &mut resume,
                    overrides,
                    yes,
                )
                .await?
                {
                    break;
                }
            }
            Ok(other) => {
                // Ctrl+C / Ctrl+D / future Signal variants → clean exit
                let _ = other;
                eprintln!();
                break;
            }
            Err(e) => bail!("reedline: {e}"),
        }
    }
    Ok(())
}

fn ensure_tty() -> Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(());
    }
    bail!("repl/tui requires interactive stdin/stdout TTY")
}

fn print_repl_banner(client: &ServeClient, conversation_id: Option<&str>) {
    eprintln!(
        "crabmate-tui repl → {}  (/help · Ctrl+C / Ctrl+D / /quit)",
        client.config().api_base
    );
    if let Some(cid) = conversation_id {
        eprintln!("conversation_id={cid}");
    }
}

/// 返回 `false` 表示用户请求退出。
async fn dispatch_repl_line(
    client: &ServeClient,
    line: &str,
    conversation_id: &mut Option<String>,
    resume: &mut Option<ResumePoint>,
    overrides: &mut SessionPrefs,
    yes: bool,
) -> Result<bool> {
    let text = line.trim();
    if text.is_empty() {
        return Ok(true);
    }
    // 仅形如 `/resume` `/model` 等控制输入才拦截；普通消息即使首词为 model/resume 也要发给模型。
    match repl_control_head(text) {
        Some("resume") => {
            return resume_turn(client, conversation_id, resume, overrides, yes).await;
        }
        Some("model") => {
            handle_model_slash(text, overrides);
            return Ok(true);
        }
        Some("mode") => {
            handle_mode_slash(text, overrides);
            return Ok(true);
        }
        Some("role") => {
            handle_role_slash(text, overrides);
            return Ok(true);
        }
        _ => {}
    }
    if is_control_slash(text) {
        return match handle_control_slash(client, text, conversation_id).await {
            Ok(keep) => Ok(keep),
            Err(e) => {
                eprintln!("error: {e:#}");
                Ok(true)
            }
        };
    }
    let mut gate = make_gate(yes);
    let prefs = overrides.prefs();
    match run_turn(
        client,
        text,
        conversation_id.as_deref(),
        prefs,
        None,
        &mut gate,
    )
    .await
    {
        Ok(outcome) => {
            after_turn_outcome(&outcome, text, conversation_id, resume);
            finish_turn_stdout(&outcome);
            if let Some(cid) = outcome.conversation_id {
                *conversation_id = Some(cid);
            }
            Ok(true)
        }
        Err(e) if is_interrupted(&e) => Err(e),
        Err(e) => {
            eprintln!("error: {e:#}");
            if capture_resume_point(resume, text, conversation_id.as_deref(), &e) {
                eprintln!("hint: /resume 可续传该回合（serve 端 job 仍在跑时）");
            }
            Ok(true)
        }
    }
}

/// 回合正常返回后的断点处置：Ctrl+C **cancel 未送达**（job 可能仍在跑）时把断点更新为
/// 本回合，允许 `/resume`；其余（正常完成 / cancel 已送达 / 无 job）清空断点。
fn after_turn_outcome(
    outcome: &ChatStreamOutcome,
    text: &str,
    conversation_id: &Option<String>,
    resume: &mut Option<ResumePoint>,
) {
    if cancel_failed_keeps_resume(outcome.cancelled_by_user, outcome.cancel_acknowledged)
        && let Some(job_id) = outcome.job_id
    {
        *resume = Some(ResumePoint {
            message: text.to_string(),
            conversation_id: conversation_id.as_deref().map(str::to_string),
            job_id,
            after_seq: outcome.last_event_id,
        });
        eprintln!("hint: cancel 未送达，job 可能仍在 serve 上跑；可 /resume 续传");
        return;
    }
    *resume = None;
}

/// Ctrl+C 取消请求未送达（cancel 失败）时保留续流点。
fn cancel_failed_keeps_resume(cancelled_by_user: bool, cancel_acknowledged: bool) -> bool {
    cancelled_by_user && !cancel_acknowledged
}

/// `/resume`：用记录的断点重发 `stream_resume:{job_id,after_seq}` 续流。
async fn resume_turn(
    client: &ServeClient,
    conversation_id: &mut Option<String>,
    resume: &mut Option<ResumePoint>,
    overrides: &SessionPrefs,
    yes: bool,
) -> Result<bool> {
    let Some(point) = resume.take() else {
        eprintln!("nothing to resume (上次回合已正常跑完，或从未产生可续断点)");
        return Ok(true);
    };
    let mut gate = make_gate(yes);
    let prefs = overrides.prefs();
    let stream_resume = StreamResume {
        job_id: point.job_id,
        after_seq: point.after_seq,
    };
    match run_turn(
        client,
        &point.message,
        point.conversation_id.as_deref(),
        prefs,
        Some(stream_resume),
        &mut gate,
    )
    .await
    {
        Ok(outcome) => {
            after_turn_outcome(&outcome, &point.message, conversation_id, resume);
            finish_turn_stdout(&outcome);
            if let Some(cid) = outcome.conversation_id {
                *conversation_id = Some(cid);
            }
            Ok(true)
        }
        Err(e) if is_interrupted(&e) => Err(e),
        Err(e) => {
            eprintln!("error: {e:#}");
            if capture_resume_point(resume, &point.message, point.conversation_id.as_deref(), &e) {
                eprintln!("hint: /resume 可再试");
            }
            Ok(true)
        }
    }
}

/// 错误为 `TermError::InterruptedStream`（网络中断但 serve 侧 job 仍在跑）时记录续流点。
fn capture_resume_point(
    resume: &mut Option<ResumePoint>,
    message: &str,
    conversation_id: Option<&str>,
    err: &anyhow::Error,
) -> bool {
    let Some(TermError::InterruptedStream {
        job_id, after_seq, ..
    }) = err.root_cause().downcast_ref::<TermError>()
    else {
        return false;
    };
    *resume = Some(ResumePoint {
        message: message.to_string(),
        conversation_id: conversation_id.map(str::to_string),
        job_id: *job_id,
        after_seq: *after_seq,
    });
    true
}

/// 识别 main 层拦截的 repl 控制斜杠（仅 `/` 开头才返回命令名；普通消息不拦截）。
fn repl_control_head(text: &str) -> Option<&'static str> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?;
    match rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "resume" => Some("resume"),
        "model" => Some("model"),
        "mode" => Some("mode"),
        "role" => Some("role"),
        _ => None,
    }
}

/// 取斜杠后的参数（trim；无参为空串）。
fn slash_arg(text: &str) -> String {
    text.split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn clear_word(arg: &str) -> bool {
    matches!(arg, "off" | "none" | "clear")
}

/// `/model [name]`：查看或设置 repl 会话内的 `client_llm.model` 覆盖（`off`/`none`/`clear` 清除）。
fn handle_model_slash(text: &str, overrides: &mut SessionPrefs) {
    let arg = slash_arg(text);
    if arg.is_empty() {
        match overrides
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(model) => println!("model override: {model}"),
            None => println!("model override: (none — 使用 serve 默认模型；/model <name> 设置)"),
        }
        return;
    }
    if clear_word(&arg) {
        overrides.model = None;
        println!("model override cleared");
        return;
    }
    overrides.model = Some(arg);
    println!(
        "model override: {}",
        overrides.model.as_deref().unwrap_or_default()
    );
}

/// `/mode ask|plan|act`：查看或设置本轮 `session_mode`（`off` 清除；无效值给出提示）。
fn handle_mode_slash(text: &str, overrides: &mut SessionPrefs) {
    let arg = slash_arg(text);
    if arg.is_empty() {
        match overrides.session_mode.as_deref() {
            Some(m) if !m.trim().is_empty() => println!("session mode: {m}"),
            _ => println!("session mode: (serve 默认；/mode ask|plan|act 设置)"),
        }
        return;
    }
    if clear_word(&arg) {
        overrides.session_mode = None;
        println!("session mode cleared");
        return;
    }
    if !matches!(arg.as_str(), "ask" | "plan" | "act") {
        eprintln!("invalid session mode '{arg}'; 可选 ask / plan / act（off 清除）");
        return;
    }
    overrides.session_mode = Some(arg);
    println!(
        "session mode: {}",
        overrides.session_mode.as_deref().unwrap_or_default()
    );
}

/// `/role <id>`：查看或设置本轮 `agent_role`（`off` 清除）。可用 role 见 `/status`。
fn handle_role_slash(text: &str, overrides: &mut SessionPrefs) {
    let arg = slash_arg(text);
    if arg.is_empty() {
        match overrides.agent_role.as_deref() {
            Some(r) if !r.trim().is_empty() => println!("agent role: {r}"),
            _ => println!("agent role: (serve 默认；/role <id> 设置，可用列表见 /status)"),
        }
        return;
    }
    if clear_word(&arg) {
        overrides.agent_role = None;
        println!("agent role cleared");
        return;
    }
    overrides.agent_role = Some(arg);
    println!(
        "agent role: {}",
        overrides.agent_role.as_deref().unwrap_or_default()
    );
}

async fn maybe_probe(client: &ServeClient, no_probe: bool) -> Result<()> {
    if no_probe {
        return Ok(());
    }
    client
        .probe_health()
        .await
        .context("serve health probe failed")
}

fn finish_turn_stdout(outcome: &ChatStreamOutcome) {
    let _ = writeln!(io::stdout());
    if let Some(cid) = outcome.conversation_id.as_deref() {
        let _ = writeln!(io::stderr(), "conversation_id={cid}");
    }
}

fn resolve_message(parts: Vec<String>) -> Result<String> {
    if !parts.is_empty() {
        return Ok(parts.join(" "));
    }
    std::io::read_to_string(io::stdin()).context("read message from stdin")
}

fn is_interrupted(err: &anyhow::Error) -> bool {
    err.root_cause()
        .downcast_ref::<TermError>()
        .is_some_and(|t| matches!(t, TermError::Interrupted))
}

#[cfg(test)]
mod tests {
    use super::{
        ResumePoint, SessionPrefs, TermError, after_turn_outcome, cancel_failed_keeps_resume,
        capture_resume_point, handle_mode_slash, handle_model_slash, handle_role_slash,
        repl_control_head, with_shell_keyring_fallback,
    };
    use anyhow::anyhow;
    use crabmate_tui_core::ChatStreamOutcome;

    #[test]
    fn explicit_value_wins_over_keyring() {
        let v =
            with_shell_keyring_fallback(Some(" tok "), false, || panic!("must not read keyring"));
        assert_eq!(v.as_deref(), Some("tok"));
    }

    #[test]
    fn explicit_empty_disables_keyring_fallback() {
        let v = with_shell_keyring_fallback(Some("   "), false, || panic!("must not read keyring"));
        assert!(v.is_none());
    }

    #[test]
    fn missing_value_falls_back_to_keyring() {
        let v = with_shell_keyring_fallback(None, false, || Some("keyring-secret".into()));
        assert_eq!(v.as_deref(), Some("keyring-secret"));
    }

    #[test]
    fn no_keyring_skips_fallback() {
        let v = with_shell_keyring_fallback(None, true, || panic!("must not read keyring"));
        assert!(v.is_none());
    }

    #[test]
    fn missing_entry_yields_none() {
        let v = with_shell_keyring_fallback(None, false, || None);
        assert!(v.is_none());
    }

    #[test]
    fn interrupted_stream_error_records_resume_point() {
        let err = anyhow!(TermError::InterruptedStream {
            job_id: 11,
            after_seq: 5,
            cause: "connection reset".into(),
        });
        let mut resume = None;
        assert!(capture_resume_point(&mut resume, "hi", Some("c1"), &err));
        let p = resume.expect("point");
        assert_eq!(p.job_id, 11);
        assert_eq!(p.after_seq, 5);
        assert_eq!(p.message, "hi");
        assert_eq!(p.conversation_id.as_deref(), Some("c1"));
    }

    #[test]
    fn plain_stream_error_does_not_record_resume_point() {
        let err = anyhow!(TermError::Stream("boom".into()));
        let mut resume = None;
        assert!(!capture_resume_point(&mut resume, "hi", None, &err));
        assert!(resume.is_none());
    }

    #[test]
    fn cancel_failure_keeps_resume_flag() {
        assert!(cancel_failed_keeps_resume(true, false));
        assert!(!cancel_failed_keeps_resume(true, true));
        assert!(!cancel_failed_keeps_resume(false, false));
        assert!(!cancel_failed_keeps_resume(false, true));
    }

    #[test]
    fn after_cancel_failure_records_current_run() {
        let outcome = ChatStreamOutcome {
            cancelled_by_user: true,
            cancel_acknowledged: false,
            job_id: Some(5),
            last_event_id: 2,
            ..ChatStreamOutcome::default()
        };
        let mut resume = None;
        let conv = Some("c9".to_string());
        after_turn_outcome(&outcome, "hello", &conv, &mut resume);
        let p = resume.expect("resume point kept on cancel failure");
        assert_eq!(p.job_id, 5);
        assert_eq!(p.after_seq, 2);
        assert_eq!(p.message, "hello");
        assert_eq!(p.conversation_id.as_deref(), Some("c9"));
    }

    #[test]
    fn after_clean_or_acked_finish_clears_resume() {
        for (cancelled, acked) in [(false, false), (true, true)] {
            let outcome = ChatStreamOutcome {
                cancelled_by_user: cancelled,
                cancel_acknowledged: acked,
                job_id: Some(5),
                ..ChatStreamOutcome::default()
            };
            let mut resume = Some(ResumePoint {
                message: "old".into(),
                conversation_id: None,
                job_id: 1,
                after_seq: 0,
            });
            after_turn_outcome(&outcome, "hi", &None, &mut resume);
            assert!(resume.is_none(), "case {cancelled}/{acked} should clear");
        }
    }

    #[test]
    fn llm_overrides_empty_yields_no_client_llm() {
        let o = SessionPrefs::from_options(None, None, None);
        assert!(o.client_llm().is_none());
    }

    #[test]
    fn model_slash_sets_and_clears_override() {
        let mut o = SessionPrefs::from_options(None, None, None);
        handle_model_slash("/model deepseek-chat", &mut o);
        assert_eq!(o.model.as_deref(), Some("deepseek-chat"));
        assert!(o.client_llm().is_some());
        handle_model_slash("/model off", &mut o);
        assert!(o.model.is_none());
        assert!(o.client_llm().is_none());
    }

    #[test]
    fn plain_messages_are_not_swallowed_as_control() {
        assert_eq!(repl_control_head("model gpt-5 please"), None);
        assert_eq!(repl_control_head("resume where we left"), None);
        assert_eq!(repl_control_head("  model  "), None);
        assert_eq!(repl_control_head("mode act now"), None);
        assert_eq!(repl_control_head("role coder please"), None);
        assert_eq!(repl_control_head("/model gpt-5"), Some("model"));
        assert_eq!(repl_control_head("/mode plan"), Some("mode"));
        assert_eq!(repl_control_head("/ROLE coder"), Some("role"));
        assert_eq!(repl_control_head("/RESUME"), Some("resume"));
        assert_eq!(repl_control_head("/models"), None);
        assert_eq!(repl_control_head("/status"), None);
    }

    #[test]
    fn mode_slash_validates_sets_and_clears() {
        let mut p = SessionPrefs::from_options(None, None, None);
        handle_mode_slash("/mode act", &mut p);
        assert_eq!(p.session_mode.as_deref(), Some("act"));
        handle_mode_slash("/mode plan", &mut p);
        assert_eq!(p.session_mode.as_deref(), Some("plan"));
        handle_mode_slash("/mode bogus", &mut p);
        assert_eq!(
            p.session_mode.as_deref(),
            Some("plan"),
            "invalid mode ignored"
        );
        handle_mode_slash("/mode off", &mut p);
        assert!(p.session_mode.is_none());
    }

    #[test]
    fn role_slash_sets_and_clears() {
        let mut p = SessionPrefs::from_options(None, None, None);
        handle_role_slash("/role coder", &mut p);
        assert_eq!(p.agent_role.as_deref(), Some("coder"));
        handle_role_slash("/role clear", &mut p);
        assert!(p.agent_role.is_none());
    }

    #[test]
    fn prefs_carry_role_and_mode() {
        let mut p = SessionPrefs::from_options(Some("k".into()), None, None);
        handle_role_slash("/role coder", &mut p);
        handle_mode_slash("/mode ask", &mut p);
        let t = p.prefs();
        assert_eq!(t.agent_role, Some("coder"));
        assert_eq!(t.session_mode, Some("ask"));
        assert!(t.client_llm.is_some());
    }
}
