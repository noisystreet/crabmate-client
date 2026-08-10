//! `crabmate-tui`：连接远程 `crabmate serve` 的终端客户端（P1：`chat`）。

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crabmate_tui_core::{ConnectionConfig, ServeClient, run_chat_stream};

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
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
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
        } => run_chat(&client, message, conversation_id, no_probe).await,
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
) -> Result<()> {
    if !no_probe {
        client
            .probe_health()
            .await
            .context("serve health probe failed")?;
    }
    let text = resolve_message(message)?;
    if text.trim().is_empty() {
        bail!("empty message");
    }
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let outcome = run_chat_stream(
        client,
        &text,
        conversation_id.as_deref(),
        &mut stdout,
        &mut stderr,
    )
    .await?;
    let _ = writeln!(stdout);
    if let Some(cid) = outcome.conversation_id {
        let _ = writeln!(stderr, "conversation_id={cid}");
    }
    Ok(())
}

fn resolve_message(parts: Vec<String>) -> Result<String> {
    if !parts.is_empty() {
        return Ok(parts.join(" "));
    }
    std::io::read_to_string(io::stdin()).context("read message from stdin")
}
