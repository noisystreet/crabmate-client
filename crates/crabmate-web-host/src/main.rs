//! `crabmate-web`：在回环上托管 `frontend/dist`，并用系统浏览器打开。
//!
//! **不是** `crabmate serve`：不提供聊天 API。API 仍指向外部 `serve`
//!（`--api-base` / `CRABMATE_API_BASE`）。浏览器 Origin 须加入 serve 的
//! `CM_WEB_CORS_ALLOWED_ORIGINS`。
//!
//! 服务层用 `std::net`（见 [`crate::static_files`]）：每个连接单请求 + `Connection: close`，
//! 避免浏览器 reload 复用 keep-alive 连接时的挂起竞态。

mod open;
mod root;
mod static_files;

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::root::{page_url, page_url_for_log, resolve_web_root};
use crate::static_files::{require_index, serve_connection};

#[derive(Debug, Parser)]
#[command(
    name = "crabmate-web",
    about = "Host CrabMate frontend on loopback and open the system browser (not crabmate serve)",
    version
)]
struct Cli {
    /// Listen address (loopback recommended).
    #[arg(long, env = "CRABMATE_WEB_LISTEN", default_value = "127.0.0.1:4173")]
    listen: SocketAddr,
    /// Directory containing Trunk `index.html` (overrides search).
    #[arg(long, env = "CRABMATE_WEB_ROOT")]
    root: Option<PathBuf>,
    /// Remote `crabmate serve` root; injected via `#cm_api_base=` like the Tauri connect page.
    #[arg(long = "api-base", env = "CRABMATE_API_BASE")]
    api_base: Option<String>,
    /// Web API Bearer (`CM_WEB_API_BEARER_TOKEN` on serve). Not the model API_KEY.
    #[arg(long, env = "CM_WEB_API_BEARER_TOKEN")]
    bearer: Option<String>,
    /// Do not invoke `xdg-open`.
    #[arg(long = "no-open")]
    no_open: bool,
}

enum Bind {
    Listening(TcpListener),
    AlreadyRunning,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = resolve_web_root(cli.root.as_deref()).with_context(|| {
        "frontend dist not found; pass --root, set CRABMATE_WEB_ROOT, install the crabmate-web .deb, or run make frontend-release"
    })?;
    require_index(&root)?;
    warn_if_public_bind(cli.listen);

    let open_url = page_url(cli.listen, cli.api_base.as_deref(), cli.bearer.as_deref());
    let log_url = page_url_for_log(cli.listen, cli.api_base.as_deref(), cli.bearer.as_deref());
    match bind_listener(cli.listen)? {
        Bind::AlreadyRunning => reopen_existing(cli.listen, &open_url, &log_url, cli.no_open),
        Bind::Listening(listener) => serve_forever(
            listener,
            &root,
            cli.listen,
            &open_url,
            &log_url,
            cli.no_open,
        ),
    }
}

fn bind_listener(listen: SocketAddr) -> Result<Bind> {
    match TcpListener::bind(listen) {
        Ok(listener) => Ok(Bind::Listening(listener)),
        Err(e) if e.kind() == ErrorKind::AddrInUse => Ok(Bind::AlreadyRunning),
        Err(e) => bail!("bind {listen}: {e}"),
    }
}

fn reopen_existing(listen: SocketAddr, open_url: &str, log_url: &str, no_open: bool) -> Result<()> {
    eprintln!("note: {listen} already in use; opening the existing instance");
    eprintln!("open {log_url}");
    maybe_open_browser(open_url, no_open);
    Ok(())
}

fn serve_forever(
    listener: TcpListener,
    root: &Path,
    listen: SocketAddr,
    open_url: &str,
    log_url: &str,
    no_open: bool,
) -> Result<()> {
    eprintln!(
        "crabmate-web serving {} on http://{listen}/",
        root.display()
    );
    eprintln!("open {log_url}");
    eprintln!(
        "CORS: allow this page Origin on serve (e.g. CM_WEB_CORS_ALLOWED_ORIGINS=http://127.0.0.1:{})",
        listen.port()
    );
    maybe_open_browser(open_url, no_open);
    let root = root.to_path_buf();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // 每连接单请求（`Connection: close`），stuck 连接有 10s 读写超时兜底。
                let root = root.clone();
                thread::spawn(move || {
                    if let Err(e) = serve_connection(stream, &root) {
                        eprintln!("warn: {e:#}");
                    }
                });
            }
            Err(e) => eprintln!("warn: accept: {e}"),
        }
    }
    bail!("server loop ended")
}

fn maybe_open_browser(url: &str, no_open: bool) {
    if no_open {
        return;
    }
    if let Err(e) = open::open_browser(url) {
        eprintln!("note: xdg-open failed ({e}); open the URL above manually");
    }
}

fn warn_if_public_bind(listen: SocketAddr) {
    if listen.ip().is_loopback() {
        return;
    }
    eprintln!("warning: listening on {listen} (not loopback); this exposes the UI on the network");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_bind_reports_already_running() {
        let held = TcpListener::bind("127.0.0.1:0").expect("hold port");
        let addr = held.local_addr().expect("local addr");
        match bind_listener(addr).expect("bind_listener") {
            Bind::AlreadyRunning => {}
            Bind::Listening(_) => panic!("expected AlreadyRunning"),
        }
    }
}
