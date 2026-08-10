# Changelog

User- and maintainer-facing changes for **crabmate-client** (official Client shells + business UI).

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is intended to follow [Semantic Versioning](https://semver.org/).  
Server-side changes (contracts / SSE / CORS, etc.) live in [CrabMate](https://github.com/noisystreet/CrabMate) and are not duplicated here.

On release: move `[Unreleased]` entries into a new version section and tag (e.g. `v0.1.1`). Write maintainer-readable summaries; do not list every internal commit.

## [Unreleased]

### Added

- Design: multi-client shared pure-logic extract plan (`docs/design/client_shared_logic.md`)
- Remote terminal **`crabmate-tui`** (P3): `connect` / `chat` / `repl` with TTY or `--yes` approval; control slashes `/help`, `/workspace` (`/cd`), `/conv` (show/list/new/use) against serve HTTP APIs (design in `docs/design/remote_cli_tui.md`)
- Root `CHANGELOG.md` (Keep a Changelog; English)
- Default English [README.md](./README.md); Chinese [README.zh-CN.md](./README.zh-CN.md); [AGENTS.md](./AGENTS.md) English-only
- pre-commit / `scripts/check.sh`: **ktlint** for hand-maintained Android Kotlin (`edu/crabmate`, excludes Tauri `generated/`); `bash scripts/ktlint-android.sh` (`--format` to fix)
- Personal cloud shell runbook: connect packaged UI to remote API-only `serve` (`docs/design/personal_cloud_runbook.md`)

### Changed

- Model `API_KEY` (`client_llm` / `executor_llm`): persist in the **device keyring** (Desktop) / **Android Keystore** (not plaintext `localStorage` in shells, not serve `PUT /user-data/secrets/*`); chat sends the key in `client_llm.api_key` / `executor_llm.api_key`. Writes await confirmation (failures surface in Settings); legacy localStorage is migrated only after durable success. Plain browser falls back to weak localStorage with an explicit save warning. Saved-model preset keys stay in the same secure store and are stripped from `llm-overrides`. Re-enter keys once if they previously lived only in the server keyring.
- Docs/Victauri: shell path no longer requires `CM_WEB_CORS_ALLOWED_ORIGINS` when using Server **`v0.2.0+`** (defaults official shell Origins)
- **Phase 2 runtime**: Desktop/Android shells load **packaged** business UI after connect; hash handoff sets **API base** (`cm_api_base`) + Bearer. `serve` stays API-only. Victauri E2E no longer requires `--with-web`.
- Android `MainActivity`: treat only `connect.html` as connect home; back/disconnect from packaged UI offers return-to-connect; Keystore Bearer bridge limited to connect page
- Align Playwright/browser docs with Server **API-only-by-default**: browser/E2E paths that still host SPA use **`--with-web`**
- Pin Server contract crates and Playwright serve checkout to **`client-contract-v0.1.1`** (was product tag `v0.2.0`)
- Android: keep the system Back handler above Tauri `AppPlugin` (re-install after WebView ready, on resume, and briefly thereafter) and trim WebView history on remote so Back cannot `goBack()` to a bare connect page and auto-login; returning to connect still uses `?manual=1`
- Android: wire WebView navigation allowlist (`AllowedServeOrigin` via `crabmate-shell-navigation`); parse connect-page origin by host (not substring); keep `MobileBridge` in ProGuard
- Connect: reject cleartext HTTP to public hosts (LAN/loopback/CGNAT `100.64/10`/`.local`/`.internal` still allowed; public must use HTTPS); probe follows redirects only when the host is unchanged
- Android: persist connect-page Web API Bearer via platform **AndroidKeyStore AES-GCM** (`CrabMateMobile` / `SecureBearerStore`, app-origin only) instead of plaintext `localStorage` (no `security-crypto`/Tink)
- Android: navigation allowlist must not call `Webview::url()` inside `on_navigation` (MainPipe re-entrancy → `GetUrl` unwrap panic / startup abort on wryCreate)
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
- Real-LLM E2E: prefer client keyring/`__CRABMATE_E2E_CLIENT_LLM_KEY`, with `CM_WEB_API_BEARER_TOKEN`
