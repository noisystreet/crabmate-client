# Changelog

User- and maintainer-facing changes for **crabmate-client** (official Client shells + business UI).

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is intended to follow [Semantic Versioning](https://semver.org/).  
Server-side changes (contracts / SSE / CORS, etc.) live in [CrabMate](https://github.com/noisystreet/CrabMate) and are not duplicated here.

On release: move `[Unreleased]` entries into a new version section and tag (e.g. `v0.1.1`). Write maintainer-readable summaries; do not list every internal commit.

## [Unreleased]

### Added

- Root `CHANGELOG.md` (Keep a Changelog; English)
- Default English [README.md](./README.md); Chinese [README.zh-CN.md](./README.zh-CN.md); [AGENTS.md](./AGENTS.md) English-only

### Changed

- Pin Server contract crates and Playwright serve checkout to product tag **`v0.1.0`** (was `client-contract-v0.1.0` / unpinned default branch)
- Post-connect first paint: LLM hydrate runs in parallel with session list; hydrate path no longer repeats `GET /user-data/prefs`; `initialized` still waits for prefs to land, avoiding a read-only TTL race on the first chat
- CodeMirror: load `ide-codemirror.js` dynamically when entering the IDE so it no longer blocks chat cold start
- Desktop: maximize on page-load **Started** when navigating to the `serve` UI; no need to wait for WASM Finished
- Sync read-only TTL settings signals when prefs land; after a CM script load failure, leave and re-enter to retry
- Frontend: split high-CCN helpers/components (workspace HTTP, SSE info dispatch, theme sync, IDE tabs, composer/toolbar/settings)
- Desktop: extract main-window navigation / page-load handlers from `finish_create_main_window`
- Lizard gate: per-module **count of functions with CCN > 10** (was per-module max CCN); caps in `scripts/lizard_module_ccn_caps.toml`; ratchet requires measured count to match the cap (shrink → lower the cap / `--write-caps`)
- SSE: document that AG-UI `STATE_SNAPSHOT` is intentionally ignored (`on_state_snapshot: None`); conversation sync stays on `GET /conversation/messages`
- Session hydrate / load-older: surface `GET /conversation/messages` failures on the status error banner instead of failing silently (`CONVERSATION_NOT_FOUND` on auto-hydrate stays soft so mock/ephemeral ids do not block Ready)

## [0.1.0] - 2026-08-08

First packable milestone of the path A official Client repo (relative to the Server shell split; corresponds to merged PRs #1–#5).

### Added

- Repo layout: `desktop-tauri` / `mobile-tauri` / `crates/crabmate-connect` / `frontend` / `e2e`
- Business UI migrated here; contracts pinned to Server tag **`client-contract-v0.1.0`**
- Playwright E2E migrated here and driven by CI
- Makefile, `pre-commit`, complexity gates, desktop `.deb` packaging CI
- Shell docs: `docs/design/tauri_gui_mvp_design.md`, `shell_smoke_runbook.md`, `contract_pin.md`, `docs/TESTING.md`

### Changed

- Desktop package name **`crabmate-desktop`** (can coexist with Server `crabmate` `.deb`); no longer embeds a `serve` sidecar
- `make desktop-release` / `beforeBuildCommand`: force `trunk build --release` and reject oversized debug WASM via size gate
- Single-window boot: remove separate splash; connect page fills the main work area (centered card)
- Wide viewports default-expand the workspace side panel (mobile/narrow still collapsed by default)
- Session main window defaults to maximized after connect

### Fixed

- Session hydrate: do not invent tool stub cards when a matching tool result exists; merge matches on `tool_call_id` to avoid sandwiching dual tool cards
- Real-LLM E2E: prefer keyring `client_llm`, with `CM_WEB_API_BEARER_TOKEN`
