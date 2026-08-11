# crabmate-client

**English** | [简体中文](./README.zh-CN.md)

Official **Client** repository (path A): Desktop Linux / Android Tauri shells, shared `crabmate-connect`, and business UI in `frontend/`.  
Connects to a compatible **`crabmate serve`** (local or remote). Does **not** spawn or embed the Agent process.

> **Server / contract source of truth**: [noisystreet/CrabMate](https://github.com/noisystreet/CrabMate) (local checkout is often `../crabmate_agent`)  
> **Decision**: [client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)  
> **Contract pinning**: [client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)

## Layout

```text
.
├── crates/crabmate-connect/   # Connect-page logic (path dep in this repo; do not path back to Server)
├── crates/crabmate-tui-core/  # Remote terminal HTTP/SSE core
├── crates/crabmate-tui/       # Binary crabmate-tui (P3: chat / repl + slashes)
├── desktop-tauri/             # Desktop Linux (Tauri 2)
├── mobile-tauri/              # Android (Tauri 2)
├── frontend/                  # Business UI (Leptos CSR + WASM; contracts via git rev/tag)
├── e2e/                       # Playwright (browser UI; mock SSE in CI)
├── scripts/                   # check / connect sync / Victauri / Playwright
└── .github/workflows/         # CI (check + frontend + Playwright + desktop deb)
```

## Relationship to the Server repo

| Topic | Status |
|-------|--------|
| Shell + connect + business UI | Maintained **here** |
| Contract crates | Published from Server; UI pins git `rev` / `client-contract-vX.Y.Z` (see [contract_pin.md](docs/design/contract_pin.md)) |
| Server `serve` | Server repo; start locally or remotely — shell does not spawn it |
| Server `frontend/` / Playwright | UI and Playwright live **here**; after Server Phase C, Server has no `frontend/` sources |

## Makefile

```bash
make help
make frontend           # trunk build → frontend/dist
make frontend-check     # wasm32 cargo check
make check              # same as scripts/check.sh (includes frontend)
make test
make desktop-dev        # needs cargo-tauri ^2; run serve in another terminal
make desktop-release    # crabmate-desktop_*.deb (auto trunk --release UI; do not ship debug dist)
make desktop-bin-release
make apk                # Android; does not build frontend by default
make tui                # build crabmate-tui (remote terminal)
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

Design: [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md).

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
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | Contract git tag / rev pinning |
| [frontend/README.md](./frontend/README.md) | UI build (trunk) |

Before commit: `pre-commit run --all-files` or `make check`. CI: `.github/workflows/ci.yml` (includes **frontend wasm** and **desktop release .deb**).

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

## Conventions

- `crabmate-connect`: in-repo `path = "../../crates/crabmate-connect"`
- `frontend` contracts: git tag / `rev`; do not `path` back to the Server tree
- Secret boundary matches Server ADR §2.3: cross-origin traffic accepts only Web Bearer + CORS

## License

Apache-2.0 (see [LICENSE](./LICENSE))
