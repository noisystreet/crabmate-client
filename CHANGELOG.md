# Changelog

User- and maintainer-facing changes for **crabmate-client** (official Client shells + business UI).

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is intended to follow [Semantic Versioning](https://semver.org/).  
Server-side changes (contracts / SSE / CORS, etc.) live in [CrabMate](https://github.com/noisystreet/CrabMate) and are not duplicated here.

On release: move `[Unreleased]` entries into a new version section and tag (e.g. `v0.1.1`). Write maintainer-readable summaries; do not list every internal commit.

## [Unreleased]

### Changed

- Pin line contract to crates.io **`crabmate` 0.4.0** (`features = ["protocol"]`). Playwright / Victauri `serve` checkout uses git tag **`v0.4.0`**.
- Vendor `crabmate-tool-card` as `crates/crabmate-tool-card` (in-repo path). Frontend no longer git-depends on Server `crabmate-types`; LLM gateway presets live in `frontend/src/client_llm_presets.rs`.

### Security

- Chat markdown: remote images get `referrerpolicy=no-referrer`; `javascript:` / `data:` href/src stay stripped; anchors that lose `href` unwrap to text; links use ammonia `target=_blank` instead of a post-pass string replace
- Desktop / Android / connect lockfiles: bump transitive `plist` 1.8.0 → 1.10.0 so `quick-xml` is 0.41.0 (RUSTSEC-2026-0194 / RUSTSEC-2026-0195)

### Added

- Design: chat cards for **file-backed** context injection ([ADR-0003](docs/adr/0003-chat-file-context-inject-cards.md); Proposed — compact path list, not full injected user bubbles; L6/auto skills wait on Server metadata; slash skill `path` is Client-only)
- Workspace file tree: right-click a file → **Save to this device** (Desktop native save dialog; browser/Android download). Uses the open IDE buffer if that path is already open; otherwise `GET /workspace/file`. Hidden for folders. Text files only (same limit as the IDE reader).
- Design: coding-agent client plan (`docs/design/coding_agent_client.md`): review/revert loop over a full IDE; Wave 1 is `chat_ui_todo.md` P2; Waves 2–3 need Server contracts for restore / structured changelog
- Chat P2: in-place edit user messages (context menu → branch/regen); queue the next composer send while a turn is streaming; find hits highlight inside bubbles (`<mark>`); Ask/Plan/Act moved from the status bar onto the composer action row
- Design: chat UI follow-ups (`docs/design/chat_ui_todo.md`); P1 keyboard/ARIA and P2 chat loop are done; remaining is P3 chat code highlighting
- Loopback web UI host (`make web-release` / `crabmate-web`): static `frontend/dist` on `127.0.0.1:4173`, system browser; not Tauri and not `crabmate serve`. The `.deb` installs a menu entry with the same CrabMate icon as Desktop (`Icon=crabmate-web`). A second launch on the same port opens the existing instance. Logs redact `--bearer`. CI `build-web-deb` packs a stub UI (`CM_WEB_SKIP_FRONTEND=1`) and checks package layout
- Remote terminal `.deb` (`make tui-release` / `crabmate-tui`): binary only at `/usr/bin/crabmate-tui`; no menu icon, no config files, not `crabmate serve`. CI `build-tui-deb` packs and checks layout
- Chat markdown: closed fences show a language label and Copy button; streaming active lines render paired `*em*` / `_em_` and complete `[text](url)` (http(s) only); `~~~` fences buffer like backticks; `>` quotes freeze as their own block; GFM `[!NOTE]`/`[!TIP]`/… alerts keep `markdown-alert-*` classes
- Chat: bare `http://` / `https://` URLs in markdown (and streaming active lines) render as links (case-insensitive scheme; CJK paths and IPv6 literals kept); skipped inside code/fences and when markdown is off; `javascript:` is not autolinked
- CI: `dependency-security` workflow (`cargo audit` + `cargo deny check licenses bans sources` on each Cargo workspace; policy in `deny.toml`; not in pre-commit). Local: `make dependency-security`
- Android: in-flight `/chat/stream` starts a `dataSync` foreground service (channel `crabmate.stream`) so the WebView is less likely to freeze after Home/lock; `command_approval` upgrades that notification (truncated command). Tap opens the existing approval modal (`POST /chat/approval` still in-UI). Android 13+ requests `POST_NOTIFICATIONS` on first send; denial is shown on the status bar. Disconnect / Stop / stream end / return-to-connect stop the service ([ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md))
- Connect page: remember recent server URLs (up to 8) under `localStorage` key `crabmate.connect.recentUrls` with a clickable 「最近连接」 list and a clear action; a previously stored single server URL is seeded into the list on first load so switching servers no longer loses the prior address
- Shared pure client logic crate `crabmate-client-api` **S3–S4**: workspace set JSON parse, thin session list rows + resume `conversation_id`, and `POST /chat/stream` core body builder (`message` / `client_sse_protocol` / ids); wired into `tui-core` and `frontend`
- Shared pure client logic crate `crabmate-client-api` **S2**: `ApprovalDecision` / `CommandApprovalRequest` / `allowlistKey` parse / `ApprovalPostBody`; wired into `crabmate-tui-core` and `frontend` SSE approval path
- Shared pure client logic crate `crabmate-client-api` (S1): strict API base URL normalize/join, Web API auth header shapes, and secret-slot / keyring account name constants; wired into `crabmate-tui-core`, `crabmate-connect`, and `frontend` (`docs/design/client_shared_logic.md`)
- CI: run `make test-frontend` and `make test-tui` in default `check` job; desktop unit tests use `cargo test --bins` (excludes Victauri integration binaries that fake-pass without `VICTAURI_E2E`)
- CI: nightly Victauri shell E2E workflow (`victauri-e2e-nightly.yml`; mock suites via `victauri-e2e.sh`; failure log artifacts)
- Design: Android approval notification + foreground keep-alive ([ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md); Accepted — `dataSync` FGS at `/chat/stream` attach, heads-up on `command_approval`, tap opens existing modal; no FCM / native SSE)
- Design: multi-client shared pure-logic extract plan (`docs/design/client_shared_logic.md`)
- Remote terminal **`crabmate-tui`** (P3): `connect` / `chat` / `repl` with TTY or `--yes` approval; control slashes `/help`, `/workspace` (`/cd`), `/conv` (show/list/new/use) against serve HTTP APIs (design in `docs/design/remote_cli_tui.md`)
- Root `CHANGELOG.md` (Keep a Changelog; English)
- Default English [README.md](./README.md); Chinese [README.zh-CN.md](./README.zh-CN.md); [AGENTS.md](./AGENTS.md) English-only
- pre-commit / `scripts/check.sh`: **ktlint** for hand-maintained Android Kotlin (`edu/crabmate`, excludes Tauri `generated/`); `bash scripts/ktlint-android.sh` (`--format` to fix)
- Personal cloud shell runbook: connect packaged UI to remote API-only `serve` (`docs/design/personal_cloud_runbook.md`)

### Fixed

- Chat P2 follow-up: in-place edit save is refused while a turn is in flight or another follow-up is queued; switching sessions parks a queued composer line back onto that session’s draft; transcript sync no longer re-runs on every edit keystroke, and find `<mark>` wrapping does not walk the full transcript on each stream token
- Shell a11y (P1): approval modal traps Tab and Escape submits `deny`; confirm / new-file dialogs trap focus (Escape closes even from the path field); image attach is a real button; context menus take focus and arrow keys; session/file/message/IDE-tab Shift+F10; IDE tabs Left/Right; workspace file rows are keyboard-activatable; Ask/Plan/Act and side-view items are `menuitemradio`; current session `aria-current`; pin/star/unsaved names are announced
- IDE: entering the editor no longer keeps the stale “rebuild the frontend” banner. CodeMirror load status is an explicit state machine; the warning is reactive and only shows after that load fails. A failed `<script>` is removed so leaving and re-entering IDE can retry (Tauri packaged UI with hash handoff included)
- Chat markdown: message `#` / `##` render as `h3` / `h4` so they do not steal the page outline; heading `{#id}` is not turned into a DOM `id`. Normalize splits glued `~~~` fences, `。>` / `！>` / `？>` quotes (not `：>`), and CJK list markers missing a space (`-项` / `1.项`; not `-rf` / `1.0`)
- Chat markdown: GFM task-list checkboxes survive sanitizer as read-only (`disabled` + `pointer-events: none`); fenced `language-*` classes are kept on `pre`/`code`; closed transcript blocks reuse `.msg-md-prose` so headings, quotes, `hr`, and code chrome match the changelist modal
- Linux Desktop: `theme=system` follows xdg-desktop-portal, then GNOME gsettings, `GTK_THEME` / gtk settings.ini, then KDE `kdeglobals` (watches portal + gsettings). Non-GNOME sessions no longer always resolve to light
- IDE: define `--ide-hl-*` on dark / light / material / high-contrast so syntax colors follow the preset (high-contrast stays grayscale; material uses dark pastels)
- Session hydration: when `GET /conversation/messages` returns non-empty `messages` but the client parses zero rows, show a status-bar parse error with retry instead of silently keeping stale local timeline
- Mobile: left session-drawer backdrop only covers the dimmed strip to the right of the rail (same as the right side-column backdrop), so dismiss clicks are not intercepted by the open list
- GitHub Device Flow (Android): opening the GitHub App for authorization backgrounds the WebView and can abort `GET /github/oauth/device/status` with `TypeError: Failed to fetch`; polling retries that class of network errors and transient HTTP 5xx/429/408 until the device code expires, instead of aborting the flow
- Frontend: same-workspace partition GET no longer overwrites in-memory sessions when the load is empty/stale vs the active id or a stream is busy (protects E2E `seedSession` and mid-turn commentary); init still records a partition when workspace fetch fails; skipped loads do not clear the session PUT gate or mark the bucket as matched; E2E `seedSession` seeds via API request then a single navigation (avoids SPA debounce clobber)
- GitHub Device Flow (Desktop/Android): protected API / chat stream awaits Keystore-or-keyring hydrate of the `github` slot before attaching `X-CrabMate-GitHub-Token` (fixes cold-start race); Android secure-slot load/write retries align with WebView URL-cache sampling so restarts keep Device Flow tokens
- Frontend: workspace-bound sessions — switching workspace loads that bucket only (empty bucket gets a default empty session; no longer copies the previous workspace’s list); clone flushes the old bucket then opens an empty session; avoid patching the prior active session’s `workspace_root` before the partition reload; block debounced `PUT …/sessions` until the new bucket is committed (prevents cross-bucket overwrite on slow Android/remote links); flush failure aborts switch/clone; empty-session handoff no longer mutates the old in-memory list on timeout; async session PUTs carry a persist epoch so post-switch keepalive cannot write a pre-switch snapshot; overlapping switches are refused while blocked
- GitHub Settings: Device Flow poll tasks are generation-gated and cancelled on Settings unmount / reconnect / disconnect, so stale `spawn_local` polls no longer update UI or apply tokens after the user left or started a new flow; success persist rolls back if the generation became stale mid-write, and disconnect skips logout/local clear once superseded
- Chat (Android/Desktop WebView): when returning from background (`visibilitychange`), soft-resume `/chat/stream` only if a job id remains but the abort slot is empty (avoid tearing down a live attach/scratch); otherwise hydrate conversation messages when the turn looked busy while hidden
- GitHub Device Flow (Desktop/Android): persist user token only after durable keyring/Keystore write **and** read-back; Settings reconcile re-hydrates from the secure slot when memory is empty (avoids false “disconnected” without wiping a good in-memory token on flaky reads)
- Frontend: `GET /user-data/prefs` failure no longer marks prefs as hydrated and debounced PUT no longer overwrites server theme, sidebar width, or recent workspaces with local defaults; load/save errors surface in the status bar with retry
- Security: official Desktop/Android shells no longer write Web API Bearer to plaintext `localStorage` (`crabmate-api-bearer-token`); in-shell UI keeps it in memory and hydrates from device keyring / Android Keystore; disconnect / return-to-connect and Settings clear also wipe the native slot (reconnect requires re-entering Bearer); plain browsers keep weak localStorage with an explicit save warning; protected API fetches await secure-store hydrate before sending auth headers
- Frontend: mobile/narrow platform entry layout (side panel collapse, Android default hidden status bar) no longer writes back to `/user-data/prefs` and overwrites desktop-saved layout preferences
- GitHub Settings: validate OAuth Client IDs against the Server contract and report local-storage save/clear failures instead of showing false success; align Android docs with client-local Client ID and token storage
- Chat: markdown tables render with light-gray cell borders (chat transcript lacked table border styles; `--border-subtle` was nearly invisible on dark surfaces)
- Chat: long unbroken strings / code blocks no longer overflow message bubbles on narrow (mobile) widths
- Shell: clicking outside an open **Project** / Edit / View topbar menu dismisses it (portaled backdrop escapes topbar `backdrop-filter`; `pointerdown` also closes when clicking other topbar chrome)
- Desktop: when the connected API base is not loopback, **Choose workspace** opens the server project-pool / path modal instead of the local OS folder picker (local `127.0.0.1` / `localhost` still uses the native dialog)
- Android: model API key Keystore bridge no longer calls `WebView.getUrl()` from the JS binder thread (use UI-thread URL cache refreshed on resume/navigation sampling); deny bridge access until the cache is ready (frontend retries); serialize Keystore encrypt/decrypt with retries so saves stick across app restarts

### Changed

- Pin Server contract crates and Playwright / Victauri serve checkout to product tag **`v0.3.0`** (was `client-contract-v0.1.1`)
- Settings: drop extra hint copy (Appearance / MCP / Session / GitHub / IDE / session and workspace modals); move Web API Bearer and API base into a **Connection** section
- Connect page: drop the extra lead/hint copy; keep title, server URL, Web API secret, and Connect
- Chat **`run_command` tool card**: show `command` + `args` on the compact row (parse SSE `arguments` / `arguments_preview` when the summary is only `tool: run_command`); running detail includes `$ …`.
- Android: start with the in-app bottom status bar hidden even when another client saved it as visible; users can still enable it from the side toolbar
- Android: Keystore Web API Bearer bridge (`getSecureBearer` / `setSecureBearer`) allowed on packaged App Origin (connect page **and** business UI) so UI can hydrate/clear without plaintext `localStorage`
- Settings: merge primary + executor into one **Models** section; configure endpoints only via the **+** model dialog (inline primary form removed); old `#/settings/executor-llm` opens Models
- Shell: in workspace **project-pool** mode, the topbar workspace title shows only the project directory name (full path remains in the hover tooltip)
- Model `API_KEY` (`client_llm` / `executor_llm`): persist in the **device keyring** (Desktop) / **Android Keystore** (not plaintext `localStorage` in shells, not serve `PUT /user-data/secrets/*`); chat sends the key in `client_llm.api_key` / `executor_llm.api_key`. Writes await confirmation (failures surface in Settings); legacy localStorage is migrated only after durable success. Plain browser falls back to weak localStorage with an explicit save warning. Saved-model preset keys stay in the same secure store and are stripped from `llm-overrides`. Re-enter keys once if they previously lived only in the server keyring.
- Docs/Victauri: shell path no longer requires `CM_WEB_CORS_ALLOWED_ORIGINS` when using Server **`v0.2.0+`** (defaults official shell Origins)
- **Phase 2 runtime**: Desktop/Android shells load **packaged** business UI after connect; hash handoff sets **API base** (`cm_api_base`) + Bearer. `serve` stays API-only. Victauri E2E no longer requires `--with-web`.
- Android `MainActivity`: treat only `connect.html` as connect home; back/disconnect from packaged UI offers return-to-connect; Keystore Bearer bridge available on App Origin (connect + packaged UI)
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
