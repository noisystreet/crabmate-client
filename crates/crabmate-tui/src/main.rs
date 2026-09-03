//! `crabmate-tui`：连接远程 `crabmate serve` 的终端客户端（P3：chat / repl / 斜杠）。

mod approval_tty;
mod slash;
mod turn;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crabmate_client_api::secrets::{KEYRING_SERVICE, SecretSlot, WEB_API_BEARER_KEYRING_ACCOUNT};
use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOutcome, ClientLlm,
    CommandApprovalRequest, ConnectionConfig, ServeClient, TermError,
};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

use crate::approval_tty::TtyApprovalGate;
use crate::slash::{handle_control_slash, is_control_slash};
use crate::turn::run_turn;

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
}

enum AnyApprovalGate {
    Auto(AutoAllowOnce),
    Tty(TtyApprovalGate),
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
    let llm = client_llm_overrides(
        llm_key.as_deref(),
        llm_model.as_deref(),
        llm_api_base.as_deref(),
    );
    match command {
        Commands::Connect => run_connect(&client).await,
        Commands::Chat {
            message,
            conversation_id,
            no_probe,
        } => run_chat(&client, message, conversation_id, no_probe, yes, llm).await,
        Commands::Repl {
            conversation_id,
            no_probe,
        } => run_repl(&client, conversation_id, no_probe, yes, llm).await,
    }
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
    llm: Option<ClientLlm<'_>>,
) -> Result<()> {
    maybe_probe(client, no_probe).await?;
    let text = resolve_message(message)?;
    if text.trim().is_empty() {
        bail!("empty message");
    }
    let mut gate = make_gate(yes);
    let outcome = run_turn(client, &text, conversation_id.as_deref(), llm, &mut gate).await?;
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
    llm: Option<ClientLlm<'_>>,
) -> Result<()> {
    ensure_repl_tty()?;
    maybe_probe(client, no_probe).await?;
    print_repl_banner(client, conversation_id.as_deref());
    let mut conversation_id = conversation_id;
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic("crabmate> ".to_string()),
        DefaultPromptSegment::Empty,
    );
    loop {
        match editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                if !dispatch_repl_line(client, &line, &mut conversation_id, llm, yes).await? {
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

fn ensure_repl_tty() -> Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(());
    }
    bail!("repl requires interactive stdin/stdout TTY")
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
    llm: Option<ClientLlm<'_>>,
    yes: bool,
) -> Result<bool> {
    let text = line.trim();
    if text.is_empty() {
        return Ok(true);
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
    match run_turn(client, text, conversation_id.as_deref(), llm, &mut gate).await {
        Ok(outcome) => {
            finish_turn_stdout(&outcome);
            if let Some(cid) = outcome.conversation_id {
                *conversation_id = Some(cid);
            }
            Ok(true)
        }
        Err(e) if is_interrupted(&e) => Err(e),
        Err(e) => {
            eprintln!("error: {e:#}");
            Ok(true)
        }
    }
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
    use super::with_shell_keyring_fallback;

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
}
