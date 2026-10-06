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
├── crates/crabmate-client-api/ # Shared pure logic (URL / auth / secrets / approval / workspace / sessions / chat body / hash / health JSON; no IO)
├── crates/crabmate-tool-card/  # Tool-card compact/detail (in-repo path after W2; not git-pinned to Server)
├── crates/crabmate-connect/   # Connect-page logic (path dep in this repo; do not path back to Server)
├── crates/crabmate-tui/       # Binary crabmate-tui (chat / repl / full-screen tui; HTTP/SSE serve client in src/serve)
├── desktop-tauri/             # Desktop Linux (Tauri 2)
├── mobile-tauri/              # Android (Tauri 2)
├── web-host/                  # Binary crabmate-web (loopback static UI host)
├── frontend/                  # Business UI (Leptos CSR + WASM; contracts via crates.io crabmate)
├── e2e/                       # Playwright (browser UI; mock SSE in CI)
├── scripts/                   # check / connect sync / Victauri / Playwright
└── .github/workflows/         # CI (check + frontend tests + Playwright + desktop deb + Victauri nightly + dependency-security)
```

## Relationship to the Server repo

| Topic | Status |
|-------|--------|
| Shell + connect + business UI | Maintained **here** |
| Contract crate | crates.io `crabmate` `0.6.0` + `protocol` (see [contract_pin.md](docs/design/contract_pin.md)) |
| Server `serve` | Server repo; start locally or remotely — shell does not spawn it |
| Server `frontend/` / Playwright | UI and Playwright live **here**; after Server Phase C, Server has no `frontend/` sources |

## Quick start

Prerequisite for every target: a running **`crabmate serve`** (API-only by default). Official shell Origins are allowed by default on current Server (`tauri://localhost`, `http://tauri.localhost`) — no `CM_WEB_CORS_ALLOWED_ORIGINS` needed for Desktop/Android.

```bash
# Terminal A — Server (no --with-web needed for the shell)
crabmate serve --host 127.0.0.1 --port 8080
```

### Desktop

```bash
# Terminal B — this repo
make frontend           # sync UI into desktop-tauri/dist via prepare-sidecar
make desktop-dev
```

On the connect page, enter the server URL and optional Web API Bearer (**not** the model `API_KEY`). The shell loads **local** `index.html` and points API calls at `serve`. Workspace file upload / download / rename and chat image attachments are documented in [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md).

### Web UI in the system browser

Not Tauri: a tiny loopback static server opens the default browser. Still **not** `crabmate serve` — start API separately and allow the page Origin on CORS.

```bash
# Terminal A — API (allow the web-host Origin; Server ≥ v0.2.0 allows Tauri Origins only)
CM_WEB_CORS_ALLOWED_ORIGINS=http://127.0.0.1:4173 crabmate serve --host 127.0.0.1 --port 8080

# Terminal B — this repo
make web-release
sudo dpkg -i web-host/target/debian/crabmate-web_*.deb
crabmate-web --api-base http://127.0.0.1:8080
# or without installing:
#   cargo run --release --manifest-path web-host/Cargo.toml -- --root frontend/dist --api-base http://127.0.0.1:8080
```

Default listen is `127.0.0.1:4173`. `--no-open` skips `xdg-open`. Bearer: `--bearer` / `CM_WEB_API_BEARER_TOKEN` (plain browser stores it in `localStorage`). The `.deb` adds a **CrabMate Web** menu entry using the same icon as Desktop. A second launch on the same port reopens the browser instead of failing.

### Playwright / browser E2E

Playwright runs against the **client self-hosted** web UI: a pure-API `serve` plus `crabmate-web` (loopback static host, default `127.0.0.1:4173`). The script starts both and allows the web Origin via `CM_WEB_CORS_ALLOWED_ORIGINS`; no `serve --with-web` is needed (Server stays API-only).

```bash
make frontend
./scripts/e2e-playwright.sh
# or run a single spec: ./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

### Android

```bash
make apk
# or: ./mobile-tauri/scripts/build-apk.sh
# to build UI as well: CM_MOBILE_BUILD_FRONTEND=1 make apk
```

The Android shell starts with the in-app bottom status bar hidden; it can still be enabled from the side toolbar. Foreground keep-alive and approval notifications during `/chat/stream` are covered by [ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md).

## Remote terminal (crabmate-tui)

Start `crabmate serve`, then:

```bash
make tui
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 --bearer "$CM_WEB_API_BEARER_TOKEN" chat "hello"
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 repl   # interactive REPL
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 tui    # full-screen TUI
```

A bearer-mode `serve` without a server-side model `API_KEY` returns `LLM_API_KEY_REQUIRED`; pass a client-owned key with `--llm-api-key` (or `CM_API_KEY` / `CM_MODEL` / `CM_API_BASE`). Missing `--bearer` / `--llm-api-key` falls back to the Desktop shell's saved secrets in the same OS keyring (read-only; `--no-keyring` disables it).

Release package (binary only, no menu icon or config files):

```bash
make tui-release
sudo dpkg -i crates/crabmate-tui/target/debian/crabmate-tui_*.deb
```

No Rust toolchain? The same binary-only `.deb` is also attached to each `v*` GitHub Release (built by CI), ready to download and `dpkg -i`.

Keybindings, layout, and the settings panel: [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md).

## Personal cloud (remote API-only)

Expose only `api.…` → Caddy → loopback `serve` (no `--with-web`); the shell uses packaged UI against `https://api.…/` + Bearer. Steps: [`docs/design/personal_cloud_runbook.md`](docs/design/personal_cloud_runbook.md). VPS/systemd/Caddy: Server [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md).

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
| [docs/design/client_capability_matrix.md](./docs/design/client_capability_matrix.md) | Desktop / Android / Web / TUI capability matrix |
| [docs/design/coding_agent_client.md](./docs/design/coding_agent_client.md) | Coding-agent client plan (review / revert; Waves 1–3) |
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | Contract git tag / rev pinning |
| [frontend/README.md](./frontend/README.md) | UI build (trunk) |

Before commit: `pre-commit run --all-files` or `make check`. CI: `.github/workflows/ci.yml` (includes **frontend wasm**, **frontend/TUI unit tests**, **desktop / web / tui release .deb**); dependency audit: `.github/workflows/dependency-security.yml` (`make dependency-security`); Victauri shell E2E: nightly workflow or `./scripts/victauri-e2e.sh`.

## Conventions

- `crabmate-connect`: in-repo `path = "../../crates/crabmate-connect"`; shells must enable `features = ["tauri"]` (default features have no Tauri)
- `frontend` contracts: git tag / `rev`; do not `path` back to the Server tree
- Secret boundary matches Server ADR §2.3: cross-origin traffic accepts only Web Bearer + CORS

## License

Apache-2.0 (see [LICENSE](./LICENSE))
