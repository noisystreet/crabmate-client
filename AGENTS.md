# AGENTS.md

## Project Identity

- Project: `crabmate-client`
- Purpose: Official Client (Desktop Linux / Android Tauri shells + `crabmate-connect` + business UI `frontend/`); connects to a running `crabmate serve`; does **not** maintain the Server
- Tech stack: Rust, Tauri 2, Leptos CSR (WASM)
- Target: Desktop Linux, Android; shells load packaged `frontend/dist`; API calls go to remote `serve` (Server defaults CORS for `tauri://localhost` + `http://tauri.localhost`; set `CM_WEB_CORS_ALLOWED_ORIGINS` only for extra browser Origins)

## Directory Overview

```text
.
├── crates/crabmate-client-api/ # shared pure logic (URL / auth / secrets / approval / workspace / sessions / chat body)
├── crates/crabmate-connect/
├── crates/crabmate-tui-core/   # remote terminal HTTP/SSE core
├── crates/crabmate-tui/        # binary crabmate-tui
├── desktop-tauri/
├── mobile-tauri/
├── frontend/                # Business UI; contract git rev/tag
├── e2e/                     # Playwright (browser UI)
├── scripts/                 # sync-connect, victauri-e2e, e2e-playwright, check.sh
├── docs/
│   ├── TESTING.md
│   └── design/
│       ├── tauri_gui_mvp_design.md
│       ├── contract_pin.md
│       ├── personal_cloud_runbook.md
│       ├── remote_cli_tui.md   # remote crabmate-tui (HTTP/SSE; not in-process Agent)
│       ├── client_shared_logic.md  # extract shared pure logic across WASM / connect / tui
│       └── shell_smoke_runbook.md
├── CHANGELOG.md             # Release notes (English)
├── README.md                # English (default)
├── README.zh-CN.md          # Simplified Chinese
├── AGENTS.md                # Agent / contributor constraints (English only)
└── .cursor/rules/
```

## Hard Constraints

- **Forbidden**: `path = "../crabmate_agent/..."` or any Cargo path dependency back into the Server monorepo tree
- **Forbidden**: shell spawning / bundling a `crabmate serve` sidecar
- Contract crates only via git **tag** (`vX.Y.Z` product release, or `client-contract-vX.Y.Z`) or `rev` pinned to Server (see `frontend/Cargo.toml`; current pin: **`client-contract-v0.1.1`**)
- Playwright E2E CI checkouts Server `serve` at the same contract pin (**`client-contract-v0.1.1`**; see `docs/design/contract_pin.md`)
- `crabmate-connect` is in-repo path only (`crates/crabmate-connect`)
- `crabmate-client-api` is in-repo path only (`crates/crabmate-client-api`); no Tauri / `web-sys` / `reqwest` / `tokio`
- Web Bearer ≠ model `API_KEY`（Web Bearer：官方壳仅内存 + 本机钥匙串/Android Keystore，**禁止**明文 `localStorage`；model keys 同样走钥匙串/Keystore；chat 经 HTTPS 发送 `client_llm.api_key` — do not `PUT /user-data/secrets/client-llm` from the UI；plain browser may keep weak localStorage with an explicit warning）
- **Split decision / contracts / SSE / CORS** are authoritative in the Server repo; this repo documents shell behavior and links out
- Scratch drafts go in **`agent_space/`** (gitignored); **do not** treat `agent_space/` as committed documentation
- **EN/ZH doc sync**: when bilingual pairs exist, **update both sides in the same change** (never one side only). Current pair: `README.md` ↔ `README.zh-CN.md`. Facts, commands, paths, and constraints must match; wording may be localized; content drift is forbidden. `AGENTS.md` is English-only (no `AGENTS.zh-CN.md`)

## Required Commands

```bash
make help
make frontend                # trunk → frontend/dist
make check                   # or pre-commit run --all-files (includes frontend wasm + ktlint)
make ktlint-android          # hand-maintained Android Kotlin only (`edu/crabmate`)
make test
make desktop-dev
make desktop-release         # .deb (auto trunk --release UI)
bash scripts/check-no-main-path.sh
bash scripts/lizard-rust.sh              # CCN>10 count must equal module cap; if lower, tighten cap / --write-caps
bash scripts/ktlint-android.sh           # needs java; `--format` to fix
./scripts/victauri-e2e.sh all   # needs a usable crabmate serve binary; not in default CI
./scripts/e2e-playwright.sh     # Playwright; needs frontend/dist + serve
```

## Documentation Rules

When updating docs:

1. **Bilingual pairs**: changing `README.md` requires `README.zh-CN.md` (and vice versa); any future `*.zh-CN.md` pairs follow the same rule. `AGENTS.md` stays English-only
2. **Same PR / same intent**: do not leave “translate later” half-updated docs
3. **CHANGELOG**: user-facing changes go under `[Unreleased]` in English (no separate `CHANGELOG.zh-CN.md`)

| Change type | Update where |
|-------------|--------------|
| User-visible shell behavior / startup | **`README.md` + `README.zh-CN.md`** (must sync), plus `desktop-tauri/` / `mobile-tauri/` READMEs |
| Agent / contributor constraints | **`AGENTS.md`** (English only) |
| UI build / trunk | `frontend/README.md` |
| Shell architecture / lifecycle / connect model | `docs/design/tauri_gui_mvp_design.md` |
| Shared pure logic extract (WASM / connect / tui) | `docs/design/client_shared_logic.md` |
| Manual shell smoke steps | `docs/design/shell_smoke_runbook.md` |
| Personal cloud (shell → remote API-only) | `docs/design/personal_cloud_runbook.md`; VPS/Caddy authority is Server |
| Victauri / pre-commit / CI commands | `docs/TESTING.md` |
| User/maintainer release notes | `CHANGELOG.md` (English; Keep a Changelog; move Unreleased into a version section on release) |
| Contract pin tag / rev | `docs/design/contract_pin.md`; policy authority is Server |
| SSE / CORS / Bearer / API base / contract semver | **Server** `docs/`; this repo only links |
| Path A progress checkboxes | Server `docs/design/client_shell_split_todo.md` |

## Coding Rules

- Follow existing shell code style
- Commit subjects: Conventional Commits + bilingual Chinese/English subject (see `.cursor/rules/conventional-commits.mdc`)
- **Forbidden**: `git commit --no-verify`

## Server doc index (read-only links)

- [client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)
- [client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)
- [client_turn_smoke_runbook.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_turn_smoke_runbook.md) (host + contracts; shell steps follow this repo’s `shell_smoke_runbook`)
- [SSE协议.md](https://github.com/noisystreet/CrabMate/blob/main/docs/SSE协议.md)
- [配置说明.md](https://github.com/noisystreet/CrabMate/blob/main/docs/配置说明.md)
- [client_ui_runtime_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_ui_runtime_split.md)
- [个人VPS部署指南.md](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md)
