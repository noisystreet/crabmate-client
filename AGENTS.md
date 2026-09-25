# AGENTS.md

## Project Identity

- Project: `crabmate-client`
- Purpose: Official Client (Desktop Linux / Android Tauri shells + `crabmate-connect` + business UI `frontend/`); connects to a running `crabmate serve`; does **not** maintain the Server
- Tech stack: Rust, Tauri 2, Leptos CSR (WASM)
- Target: Desktop Linux, Android; shells load packaged `frontend/dist`; API calls go to remote `serve` (Server defaults CORS for `tauri://localhost` + `http://tauri.localhost`; set `CM_WEB_CORS_ALLOWED_ORIGINS` only for extra browser Origins, including `http://127.0.0.1:4173` when using `make web-release`)

## Directory Overview

```text
.
├── crates/crabmate-client-api/ # shared pure logic (URL / auth / secrets / approval / workspace / sessions / chat body / hash handoff / health JSON; no IO)
├── crates/crabmate-tool-card/  # tool-card compact/detail (W2: in-repo path; not git-pinned to Server)
├── crates/crabmate-connect/
├── crates/crabmate-tui/        # binary crabmate-tui (remote terminal HTTP/SSE serve client in src/serve)
├── desktop-tauri/
├── mobile-tauri/
├── web-host/                # binary crabmate-web (loopback static UI host)
├── frontend/                # Business UI; contract crates.io crabmate 0.5.2 + protocol
├── e2e/                     # Playwright (browser UI)
├── scripts/                 # sync-connect, victauri-e2e, e2e-playwright, check.sh, check-boundaries.sh, check-css-breakpoints.sh, check-css-contract.sh, check-css-tokens.sh, set-version.sh; rust-pkg-dirs.txt = package list single source
├── docs/
│   ├── TESTING.md
│   └── design/
│       ├── tauri_gui_mvp_design.md
│       ├── contract_pin.md
│       ├── personal_cloud_runbook.md
│       ├── client_capability_matrix.md # Desktop / Android / Web / TUI capability cells
│       ├── remote_cli_tui.md   # remote crabmate-tui (HTTP/SSE; not in-process Agent)
│       ├── client_shared_logic.md  # extract shared pure logic across WASM / connect / tui
│       ├── coding_agent_client.md  # coding-agent review/revert plan (Waves 1–3)
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
- Contract crate: crates.io **`crabmate` `0.5.2`** with `default-features = false, features = ["protocol"]` (see `frontend/Cargo.toml`). Do not pin old package names (`crabmate-sse-protocol`, …) and do not enable `server`.
- Playwright E2E CI checkouts Server `serve` at git tag **`v0.5.2`** (same commit as the crates.io package; see `docs/design/contract_pin.md`)
- `crabmate-connect` is in-repo path only (`crates/crabmate-connect`); default features have **no** Tauri. Desktop/Android enable `features = ["tauri"]`
- `crabmate-client-api` is in-repo path only (`crates/crabmate-client-api`); no Tauri / `web-sys` / `reqwest` / `tokio`
- `crabmate-tool-card` is in-repo path only (`crates/crabmate-tool-card`); do not git-pin Server `crabmate-tool-card`
- Boundary rules above are **mechanically enforced** by `scripts/check-boundaries.sh` (pre-commit hook + `scripts/check.sh`): client-api dependency purity, `crabmate-connect` default features stay Tauri-free, exact contract-pin shape across all manifests, and version consistency across all Rust packages
- Package versions move in **lockstep** across all Rust packages; bump via `bash scripts/set-version.sh <semver>` (never hand-edit one Cargo.toml version)
- Package list **single source**: `scripts/rust-pkg-dirs.txt` — consumed by the `Makefile` `fmt` / `clippy` / `clean` loops, `scripts/check.sh`, and `scripts/check-boundaries.sh`; adding a package only touches this file (plus Makefile `test-*` grouping / CI cache if needed)
- Responsive breakpoint **single source**: `MOBILE_LAYOUT_BREAKPOINT_PX` in `frontend/src/app_prefs.rs`; `@media` cannot use `var()`, so CSS repeats the value under constraint — narrow `max-width: N` pairs with complementary `min-width: N + 1` (never cross-written) and other breakpoints are registered in `scripts/check-css-breakpoints.sh`; enforced by that hook + `scripts/check.sh`
- Class-name **contract gate**: every class name a consumer emits (this repo's Rust / HTML) must have a rule in the same scope's CSS (frontend / connect / splash; third-party vendor JS runtime classes are out of contract) — intentional hooks / layer names go into `scripts/css_contract_allowlist.txt` with a `# reason`, and a stale entry (rule added, consumer gone, scope typo) fails too; enforced by `scripts/check-css-contract.sh` (hook + `scripts/check.sh`)
- Design-token **gate**: inside `frontend/styles` + `frontend/themes`, every `var(--x)` must reference a token defined in that same range, and a `var()` fallback must not contain a color literal (the fallback never applies when the token exists and renders a fixed light color when it is missing) — runtime-injected properties (Rust `style.setProperty`, Android IME, inline `style="--x: ..."`) go into `scripts/css_tokens_allowlist.txt` with a `# reason`, and a stale entry (token now defined, reference gone) fails too; a theme file (`frontend/themes/*.css`, excluding `*.example.css`) must also explicitly cover every token any other theme covers (union rule — otherwise the missing theme silently inherits the default dark value), with deliberately per-theme tokens (font family, radius scale) listed under `THEME_IDENTITY_TOKENS` in `scripts/css_tokens_check.py`; enforced by `scripts/check-css-tokens.sh` (hook + `scripts/check.sh`)
- CSS literal **budget gate**: in `frontend/styles/*.css` (excluding `tokens.css`, whose job is to hold literals) each file's count of `font-size` literals and of color literals (`#hex` / `rgb()` / `hsl()` / `white` / `black`) must **equal** its per-file budget in `scripts/css_literals_budget.txt` — one more fails, and cleaning one up without lowering the budget fails too (the budget is ratcheted, not a floor); a stale entry (file gone / out of scope / both counts 0) fails as well; enforced by `scripts/check-css-literals.sh` (hook + `scripts/check.sh`)
- Icons **gate**: frontend icons must come from one template source and one size source — no literal `<svg` in `frontend/src/**/*.rs` except `frontend/src/icon.rs` (use the `Icon` component / `icon_*` helpers), and inside `frontend/styles/*.css` any `svg` type-selector rule's `width` / `height` (incl. `min-` / `max-`) must be `var(--icon-*)` (three tiers in `tokens.css`); opt out in place with `icon-gate: allow <reason>` on the rule's line (Rust) or in the rule's leading comment (CSS); known blind spot: it does **not** verify that a class name and the `<svg>` element are the same element, so `.container svg` still counts as evidence for `.container` — that regression class (e.g. the 2026-09-25 `workspace-tree-chevron` one) needs code review; enforced by `scripts/check-icons.sh` (hook + `scripts/check.sh`)
- XML comment **gate**: no `--` inside the body of any XML comment across the repo (pruned of build output / deps) — XML 1.0 §2.5 forbids it and aapt2 fails resource merging with `注释中不允许出现字符串 "--"`; the usual trap is writing a CSS custom property (`--bg`) or a CLI flag (`--release`) inside a comment — drop the dashed prefix or use plain words; `mobile-tauri/src-tauri/gen/android/**/res/**` is committed Tauri mobile resource source and **is** scanned; enforced by `scripts/check-xml-comments.sh` (hook + `scripts/check.sh`)
- Web Bearer ≠ model `API_KEY`（Web Bearer：官方壳仅内存 + 本机钥匙串/Android Keystore，**禁止**明文 `localStorage`；model keys 同样走钥匙串/Keystore；chat 经 HTTPS 发送 `client_llm.api_key` — do not `PUT /user-data/secrets/client-llm` from the UI；plain browser may keep weak localStorage with an explicit warning）
- **Split decision / contracts / SSE / CORS** are authoritative in the Server repo; this repo documents shell behavior and links out
- Scratch drafts go in **`agent_space/`** (gitignored); **do not** treat `agent_space/` as committed documentation
- **EN/ZH doc sync**: when bilingual pairs exist, **update both sides in the same change** (never one side only). Current pair: `README.md` ↔ `README.zh-CN.md`. Facts, commands, paths, and constraints must match; wording may be localized; content drift is forbidden. `AGENTS.md` is English-only (no `AGENTS.zh-CN.md`)

## Required Commands

```bash
make help
make frontend                # trunk → frontend/dist
make check                   # or pre-commit run --all-files (includes frontend wasm32 clippy + ktlint)
make dependency-security     # cargo audit + cargo deny (all workspaces; not in pre-commit)
make ktlint-android          # hand-maintained Android Kotlin only (`edu/crabmate`)
make test
make desktop-dev
make desktop-release         # .deb (auto trunk --release UI)
make web-release             # crabmate-web .deb (trunk --release + loopback static host)
make tui-release             # crabmate-tui .deb (binary only; no icon, no config)
bash scripts/check-no-main-path.sh
bash scripts/check-boundaries.sh         # mechanical boundary rules (client-api purity / connect default features / contract pin shape / version sync)
bash scripts/check-css-breakpoints.sh    # responsive breakpoints must match MOBILE_LAYOUT_BREAKPOINT_PX (single source)
bash scripts/check-css-contract.sh       # consumer class names need CSS rules in the same scope, or an allowlist entry
bash scripts/check-css-tokens.sh         # var(--x) must be defined in styles/themes; no color literal in var() fallback; themes must cover the same tokens
bash scripts/check-css-literals.sh       # per-file font-size / color literal budget in frontend/styles must match the ratchet
bash scripts/check-icons.sh              # no literal <svg> outside icon.rs; svg rule sizes must be var(--icon-*)
bash scripts/check-xml-comments.sh       # no `--` inside XML comment bodies (XML 1.0 §2.5; aapt2 fails resource merging)
bash scripts/set-version.sh 0.5.1        # bump all Rust package versions (+ Cargo.locks) in one step
bash scripts/lizard-rust.sh              # any function with CCN>10 fails (global zero rule; no per-module caps)
bash scripts/dependency-security.sh      # cargo audit + cargo deny; not in pre-commit / check.sh
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
| Android stream FGS / approval notifications | `docs/adr/0002-android-approval-notification-foreground-keepalive.md` (Accepted) |
| Chat cards for file-backed context inject / skill path | `docs/adr/0003-chat-file-context-inject-cards.md` (Proposed — no fake cards without Server metadata; slash skill path is Client-only) |
| Shared pure logic extract (WASM / connect / tui) | `docs/design/client_shared_logic.md` |
| Multi-client repo maintenance (single-workspace trade-off, boundary guards, version bump) | `docs/adr/0004-multi-client-repo-maintenance.md` (Proposed) |
| Desktop / Android / Web / TUI capability alignment | `docs/design/client_capability_matrix.md` (update the cell in the same PR as the capability) |
| Chat UI follow-ups (composer / transcript / a11y) | `docs/design/chat_ui_todo.md` |
| Coding-agent client (review / revert loop; not full IDE) | `docs/design/coding_agent_client.md` (Wave 1 checkboxes stay in `chat_ui_todo.md`; restore/changelog JSON authority is Server) |
| Manual shell smoke steps | `docs/design/shell_smoke_runbook.md` |
| Personal cloud (shell → remote API-only) | `docs/design/personal_cloud_runbook.md`; VPS/Caddy authority is Server |
| Victauri / pre-commit / CI commands | `docs/TESTING.md` |
| Dependency audit (`cargo audit` / `cargo deny`) | `deny.toml`; CI `.github/workflows/dependency-security.yml`; not in pre-commit |
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
