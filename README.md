# crabmate-client

**English** | [简体中文](./README.zh-CN.md)

<p align="center">
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/ci.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/code-complexity.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/code-complexity.yml/badge.svg?branch=main" alt="code-complexity" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/dependency-security.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/dependency-security.yml/badge.svg?branch=main" alt="Dependency security" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/e2e-playwright.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/e2e-playwright.yml/badge.svg?branch=main" alt="E2E Playwright" /></a>
  <br />
  <a href="https://github.com/noisystreet/crabmate-client/stargazers"><img src="https://img.shields.io/github/stars/noisystreet/crabmate-client?style=flat&logo=github" alt="GitHub stars" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/commits/main"><img src="https://img.shields.io/github/last-commit/noisystreet/crabmate-client?logo=github" alt="Last commit" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/issues"><img src="https://img.shields.io/github/issues/noisystreet/crabmate-client" alt="Issues" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/pulls"><img src="https://img.shields.io/github/issues-pr/noisystreet/crabmate-client" alt="Pull requests" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/blob/main/LICENSE"><img src="https://img.shields.io/github/license/noisystreet/crabmate-client" alt="License" /></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust 1.85+" /></a>
</p>

Official **Client** repository (path A): Desktop Linux / Android Tauri shells, shared `crabmate-connect`, and business UI in `frontend/`.  
Connects to a compatible **`crabmate serve`** (local or remote). Does **not** spawn or embed the Agent process.

> **Server / contract source of truth**: [noisystreet/CrabMate](https://github.com/noisystreet/CrabMate) (local checkout is often `../crabmate_agent`)  
> **Decision**: [client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)  
> **Contract pinning**: [client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)

## Layout

```text
.
├── crates/crabmate-client-api/ # Shared pure logic (URL / auth / secrets / approval / workspace / sessions / chat body; no IO)
├── crates/crabmate-tool-card/  # Tool-card compact/detail (in-repo path after W2; not git-pinned to Server)
├── crates/crabmate-connect/   # Connect-page logic (path dep in this repo; do not path back to Server)
├── crates/crabmate-tui-core/  # Remote terminal HTTP/SSE core
├── crates/crabmate-tui/       # Binary crabmate-tui (P3: chat / repl + slashes)
├── crates/crabmate-web-host/  # Binary crabmate-web (loopback static UI host)
├── desktop-tauri/             # Desktop Linux (Tauri 2)
├── mobile-tauri/              # Android (Tauri 2)
├── frontend/                  # Business UI (Leptos CSR + WASM; contracts via crates.io crabmate)
├── e2e/                       # Playwright (browser UI; mock SSE in CI)
├── scripts/                   # check / connect sync / Victauri / Playwright
└── .github/workflows/         # CI (check + frontend tests + Playwright + desktop deb + Victauri nightly + dependency-security)
```

## Relationship to the Server repo

| Topic | Status |
|-------|--------|
| Shell + connect + business UI | Maintained **here** |
| Contract crate | crates.io `crabmate` `0.4.0` + `protocol` (see [contract_pin.md](docs/design/contract_pin.md)) |
| Server `serve` | Server repo; start locally or remotely — shell does not spawn it |
| Server `frontend/` / Playwright | UI and Playwright live **here**; after Server Phase C, Server has no `frontend/` sources |

## Makefile

```bash
make help
make frontend           # trunk build → frontend/dist
make frontend-check     # wasm32 cargo check
make check              # same as scripts/check.sh (includes frontend)
make dependency-security # cargo audit + cargo deny (all workspaces; not in pre-commit)
make test
make desktop-dev        # needs cargo-tauri ^2; run serve in another terminal
make desktop-release    # crabmate-desktop_*.deb (auto trunk --release UI; do not ship debug dist)
make desktop-bin-release
make web-release        # crabmate-web_*.deb (trunk --release + loopback static host; system browser)
make apk                # Android; does not build frontend by default
make tui                # build crabmate-tui (remote terminal)
make tui-release        # crabmate-tui_*.deb (binary only; no icon, no config)
make clean
```

## Remote terminal (P3)

Start `crabmate serve`, then:

```bash
make tui
./crates/crabmate-tui/target/debug/crabmate-tui \
  --api-base http://127.0.0.1:8080 \
  --bearer "$CM_WEB_API_BEARER_TOKEN" \
  chat "hello"

# Interactive REPL (conversation id across turns; TTY approval or --yes for allow_once)
./crates/crabmate-tui/target/debug/crabmate-tui \
  --api-base http://127.0.0.1:8080 \
  repl
# In repl: /help · /workspace [path] · /conv list|new|use <id>
```

Piping the message into `chat` (no argv) consumes stdin, so a later approval prompt cannot read a decision — use **`--yes`**, or pass the message as an argument:

```bash
echo "hello" | crabmate-tui --api-base http://127.0.0.1:8080 --yes chat
crabmate-tui --api-base http://127.0.0.1:8080 chat "hello"
```

Design: [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md). Release package (binary only, no menu icon or config files):

```bash
make tui-release
sudo dpkg -i crates/crabmate-tui/target/debian/crabmate-tui_*.deb
crabmate-tui --api-base http://127.0.0.1:8080 repl
```

## Docs

| Doc | Contents |
|-----|----------|
| [AGENTS.md](./AGENTS.md) | Agent constraints, commands, doc-sync rules (English only) |
| [CHANGELOG.md](./CHANGELOG.md) | User/maintainer-facing changes (Keep a Changelog; English) |
| [README.zh-CN.md](./README.zh-CN.md) | Chinese README |
| [docs/TESTING.md](./docs/TESTING.md) | pre-commit / Victauri / CI |
| [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md) | Shell architecture (path A) |
| [docs/design/shell_smoke_runbook.md](./docs/design/shell_smoke_runbook.md) | Desktop/Android manual smoke |
| [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md) | Remote terminal crabmate-tui |
| [docs/design/client_shared_logic.md](./docs/design/client_shared_logic.md) | Shared pure logic extract (WASM / connect / tui) |
| [docs/design/coding_agent_client.md](./docs/design/coding_agent_client.md) | Coding-agent client plan (review / revert; Waves 1–3) |
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | Contract git tag / rev pinning |
| [frontend/README.md](./frontend/README.md) | UI build (trunk) |

Before commit: `pre-commit run --all-files` or `make check`. CI: `.github/workflows/ci.yml` (includes **frontend wasm**, **frontend/TUI unit tests**, **desktop / web / tui release .deb**); dependency audit: `.github/workflows/dependency-security.yml` (`make dependency-security`); Victauri shell E2E: nightly workflow or `./scripts/victauri-e2e.sh`.

## Quick start (Desktop)

Prerequisite: **`crabmate serve`** (API-only by default) already running. Official shell Origins are allowed by default on current Server (`tauri://localhost`, `http://tauri.localhost`)—no `CM_WEB_CORS_ALLOWED_ORIGINS` needed for Desktop/Android.

```bash
# Terminal A — Server (no --with-web needed for the shell)
crabmate serve --host 127.0.0.1 --port 8080

# Terminal B — this repo
make frontend           # sync UI into desktop-tauri/dist via prepare-sidecar
make desktop-dev
```

On the connect page, enter the server URL and optional Web API Bearer (**not** the model `API_KEY`). The shell loads **local** `index.html` and points API calls at `serve`.

## Quick start (web UI in the system browser)

Not Tauri: a tiny loopback static server opens the default browser. Still **not** `crabmate serve` — start API separately and allow the page Origin on CORS.

```bash
# Terminal A — API
crabmate serve --host 127.0.0.1 --port 8080
# allow the web-host Origin (Server ≥ v0.2.0 already allows Tauri Origins only):
#   CM_WEB_CORS_ALLOWED_ORIGINS=http://127.0.0.1:4173 crabmate serve …

# Terminal B — this repo
make web-release
sudo dpkg -i crates/crabmate-web-host/target/debian/crabmate-web_*.deb
crabmate-web --api-base http://127.0.0.1:8080
# or without installing:
#   cargo run --release --manifest-path crates/crabmate-web-host/Cargo.toml -- --root frontend/dist --api-base http://127.0.0.1:8080
```

Default listen is `127.0.0.1:4173`. `--no-open` skips `xdg-open`. Bearer: `--bearer` / `CM_WEB_API_BEARER_TOKEN` (plain browser stores it in `localStorage`). The `.deb` adds a **CrabMate Web** menu entry using the same icon as Desktop. A second launch on the same port reopens the browser instead of failing.

## Personal cloud (remote API-only)

Expose only `api.…` → Caddy → loopback `serve` (no `--with-web`); the shell uses packaged UI against `https://api.…/` + Bearer. Steps: [`docs/design/personal_cloud_runbook.md`](docs/design/personal_cloud_runbook.md). VPS/systemd/Caddy: Server [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md).

## Quick start (business UI via browser + serve)

For Playwright / same-origin browser testing only (not the Desktop/Android path):

```bash
make frontend
CM_WEB_STATIC_DIR="$PWD/frontend/dist" crabmate serve --with-web --host 127.0.0.1 --port 8080
```

## Quick start (Android)

```bash
make apk
# or: ./mobile-tauri/scripts/build-apk.sh
# to build UI as well: CM_MOBILE_BUILD_FRONTEND=1 make apk
```

The Android shell starts with the in-app bottom status bar hidden; it can still be enabled from the side toolbar.

During an in-flight `/chat/stream`, the shell starts a foreground service (notification **Chat in progress**) so the WebView is less likely to be killed after Home or lock. When the server asks for command approval, that notification upgrades to **Command approval needed** (truncated command text). Tap it to return to the in-app approval dialog. Android 13+ will ask for notification permission on the first send; if you deny it, keep-alive alerts are unavailable (status-bar hint). OEM battery savers may still kill the process.

See [ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md).

## Conventions

- `crabmate-connect`: in-repo `path = "../../crates/crabmate-connect"`
- `frontend` contracts: git tag / `rev`; do not `path` back to the Server tree
- Secret boundary matches Server ADR §2.3: cross-origin traffic accepts only Web Bearer + CORS

## License

Apache-2.0 (see [LICENSE](./LICENSE))
