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
make clean
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
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | Contract git tag / rev pinning |
| [frontend/README.md](./frontend/README.md) | UI build (trunk) |

Before commit: `pre-commit run --all-files` or `make check`. CI: `.github/workflows/ci.yml` (includes **frontend wasm** and **desktop release .deb**).

## Quick start (business UI + serve)

```bash
make frontend
# Other terminal: Server checkout or installed crabmate
CM_WEB_STATIC_DIR="$PWD/frontend/dist" crabmate serve --host 127.0.0.1 --port 8080
```

## Quick start (Desktop)

Prerequisite: **`crabmate serve`** already running locally or remotely (default `http://127.0.0.1:8080/`).

```bash
make frontend           # optional: sync UI into desktop-tauri/dist
# or: export CRABMATE_FRONTEND_DIST=$PWD/frontend/dist

make desktop-dev
# or: cd desktop-tauri/src-tauri && cargo tauri dev
```

On the connect page, enter the server URL and optional Web API Bearer (**not** the model `API_KEY`).

## Quick start (Android)

```bash
make apk
# or: ./mobile-tauri/scripts/build-apk.sh
# to build UI as well: CM_MOBILE_BUILD_FRONTEND=1 make apk
```

## Conventions

- `crabmate-connect`: in-repo `path = "../../crates/crabmate-connect"`
- `frontend` contracts: git tag / `rev`; do not `path` back to the Server tree
- Secret boundary matches Server ADR §2.3: cross-origin traffic accepts only Web Bearer + CORS

## License

Apache-2.0 (see [LICENSE](./LICENSE))
