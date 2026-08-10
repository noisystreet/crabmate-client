//! `crabmate-tui`：连接远程 `crabmate serve` 的终端客户端（P2：`chat` + `repl` + 审批）。

mod approval_tty;
mod turn;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crabmate_tui_core::{
    ApprovalDecision, ApprovalGate, AutoAllowOnce, ChatStreamOutcome, CommandApprovalRequest,
    ConnectionConfig, ServeClient, TermError,
};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

use crate::approval_tty::TtyApprovalGate;
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
    let client = build_client(&cli)?;
    match cli.command {
        Commands::Connect => run_connect(&client).await,
        Commands::Chat {
            message,
            conversation_id,
            no_probe,
        } => run_chat(&client, message, conversation_id, no_probe, cli.yes).await,
        Commands::Repl {
            conversation_id,
            no_probe,
        } => run_repl(&client, conversation_id, no_probe, cli.yes).await,
    }
}

fn build_client(cli: &Cli) -> Result<ServeClient> {
    let api_base = cli
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .with_context(
            || "missing --api-base / CRABMATE_API_BASE (absolute http(s) URL to crabmate serve)",
        )?
        .to_string();
    let bearer = cli.bearer.clone().unwrap_or_default();
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
) -> Result<()> {
    maybe_probe(client, no_probe).await?;
    let text = resolve_message(message)?;
    if text.trim().is_empty() {
        bail!("empty message");
    }
    let mut gate = make_gate(yes);
    let outcome = run_turn(client, &text, conversation_id.as_deref(), &mut gate).await?;
    finish_turn_stdout(&outcome);
    Ok(())
}

async fn run_repl(
    client: &ServeClient,
    conversation_id: Option<String>,
    no_probe: bool,
    yes: bool,
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
                if !dispatch_repl_line(client, &line, &mut conversation_id, yes).await? {
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
        "crabmate-tui repl → {}  (Ctrl+C / Ctrl+D / /quit to exit)",
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
    yes: bool,
) -> Result<bool> {
    let text = line.trim();
    if text.is_empty() {
        return Ok(true);
    }
    if matches!(text, "/quit" | "/exit" | "/q") {
        return Ok(false);
    }
    let mut gate = make_gate(yes);
    match run_turn(client, text, conversation_id.as_deref(), &mut gate).await {
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
