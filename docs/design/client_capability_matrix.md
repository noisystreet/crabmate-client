# Client surface capability matrix

> **Audience**: contributors aligning Desktop / Android / Web / TUI.  
> **Not**: HTTP/SSE contracts (Server) or crate-extract planning ([`client_shared_logic.md`](./client_shared_logic.md)).  
> **Maintenance**: a PR that changes a surface’s user-visible capability **must** update the matching cell here in the same change.

## Surfaces (columns)

| Column | What it is |
|--------|------------|
| **Desktop** | Linux Tauri shell + packaged `frontend/` WASM |
| **Android** | Tauri shell + **same** WASM; extra Kotlin / FGS |
| **Web** | Same WASM hosted by `crabmate-web` (or equivalent) in a normal browser |
| **TUI** | `crabmate-tui` / `crabmate-tui-core` (HTTP/SSE; not the WASM UI) |

Desktop, Android, and Web share one UI crate. Do not treat them as three independent products. TUI is a separate client. Execution stays on a running `crabmate serve`.

## Cell vocabulary

| Token | Meaning |
|-------|---------|
| **yes** | Shipped on this surface |
| **reduced** | Shipped with a documented downgrade (see Notes) |
| **no** | Intentional non-goal — do not “align” other surfaces to match |
| **planned** | Written in a design doc; not shipped |

Blank cells are forbidden.

## Matrix

### Connect and shell

| Capability | Desktop | Android | Web | TUI | Notes |
|------------|---------|---------|-----|-----|-------|
| Connect to remote `serve` (`/health`, Bearer) | yes | yes | yes | yes | WASM: connect page + hash handoff. TUI: `--api-base` / `--bearer`. |
| Packaged UI after connect | yes | yes | yes | no | TUI has no WebView. |
| Tray + single-instance | yes | no | no | no | Desktop lifecycle only. |
| Stream FGS + approval notification | no | yes | no | no | [ADR-0002](../adr/0002-android-approval-notification-foreground-keepalive.md). |
| Extra CORS Origin for browser | no | no | yes | no | Official shell Origins are default on Server; Web needs `CM_WEB_CORS_ALLOWED_ORIGINS`. |

### Auth and secrets

| Capability | Desktop | Android | Web | TUI | Notes |
|------------|---------|---------|-----|-----|-------|
| Web API Bearer (≠ model `API_KEY`) | yes | yes | yes | yes | Official shells: memory + keyring/Keystore, **no** plaintext `localStorage`. |
| Persist Bearer on device | yes | yes | reduced | reduced | Web: weak `localStorage` with an explicit warning. TUI: flags/env; missing `--bearer` falls back to the desktop shell keyring slot `com.crabmate.credentials` / `tauri_connect_web_api_bearer` (read-only; `--no-keyring` disables). |
| Model `client_llm` key on device | yes | yes | reduced | reduced | Chat sends `client_llm.{api_key,model,api_base}` over HTTPS; do not `PUT /user-data/secrets/client-llm` from UI. TUI: `--llm-api-key` / `--llm-model` / `--llm-api-base` (`CM_API_KEY` / `CM_MODEL` / `CM_API_BASE`); missing API key falls back to the shell `client_llm` keyring slot (read-only), no TUI-side keyring writes. |
| GitHub Device Flow | yes | yes | reduced | no | Native shells: Keystore/keyring slot + `X-CrabMate-GitHub-Token`. Browser: HttpOnly cookie path. |

### Chat and tools

| Capability | Desktop | Android | Web | TUI | Notes |
|------------|---------|---------|-----|-----|-------|
| Full-screen TUI (ratatui) | no | no | no | yes | `crabmate-tui tui`: top bar (workspace) + status bar + session sidebar + streaming transcript + composer. M1: stream + Ctrl+C cancel; M2: session sidebar with use/new + `/status` serve-default linkage; M3: approval overlay (Esc=deny), thinking fold (Ctrl+E), one-line tool summaries, `/find` search, paged scroll, multi-line input (Alt+Enter), a top bar showing the current workspace (via `GET /workspace`), and inline Markdown styling for assistant text (bold/italic/code/link/strike; code fences stay plain); M4: **workspace directory tree browse** in the left column (Ctrl+W; expand/collapse directories on demand via `GET /workspace[?path=]`). When the serve workspace is not set (empty root path) the sidebar shows an explicit "not set" hint and `p` opens the **project pool picker** (`GET /workspace/projects`; `POST /workspace/projects` switches and refreshes the tree — TUI only, no new serve contract). Sidebar hidden below 120 columns; switching sessions starts a fresh transcript (no history replay). |
| `POST /chat/stream` + command approval | yes | yes | yes | yes | Android: notification keep-alive. TUI: repl TTY prompt; full-screen approval overlay (Enter/once · a/always · Esc/n/deny) or `--yes`. |
| Stop in-flight turn (`POST …/cancel`) | yes | yes | yes | yes | WASM Stop POSTs cancel; if `job_id` is not in yet, SSE stays open until `x-stream-job-id`. TUI: first Ctrl+C cancels the turn (`x-stream-job-id` → `POST …/cancel`) and returns to the prompt; a second Ctrl+C force-quits. Full-screen `tui` uses the same semantics through an external-cancel channel (raw mode has no SIGINT); while an approval overlay is open the first Ctrl+C only denies that command (the turn keeps running) and a following Ctrl+C cancels it. `serve` without the cancel route degrades to a local interrupt. |
| Tool-card compact/detail | yes | yes | yes | no | TUI shows tool calls as one-line summaries updated with ✓/✗ + serve summary; `crabmate-tool-card` cards remain WASM-only. |
| Chat image attach / lightbox | yes | yes | yes | no | |
| Ask / Plan / Act in composer | yes | yes | yes | yes | TUI: `/mode ask|plan|act` sets `session_mode`; `/role <id>` sets `agent_role` for later turns. |
| Control slashes (not sent to the model) | yes | yes | yes | reduced | Shared names: `help` / `workspace` / `cd` / `status` / `model` / `mode` / `role` / `resume`. Web has more (`export`, …). TUI: `/conv` `/quit` and more. |
| Web session list + resume by `server_conversation_id` | yes | yes | yes | reduced | TUI: repl `/conv list` / `use`; full-screen `tui` shows the list in a sidebar (Tab → ↑/↓ select, Enter use, n new, r refresh). No full WASM session CRUD/export. |
| In-app stream resume after background | yes | yes | reduced | yes | TUI: repl records a dropped run's `x-stream-job-id` + `last_event_id` and `/resume` re-attaches with `stream_resume:{job_id,after_seq}`. Ctrl+C stops the run; if the cancel request is not acknowledged the job may stay alive and `/resume` still applies. Cleanly finished turns are not resumable. |
| Per-turn inject / trim transcript notes | yes | yes | yes | no | WASM: Server `timeline_log` `context_inject` / `context_trim`; **hidden by default**; Settings → Appearance. Export never includes them. TUI prints classified SSE as text (no toggle). |

### Workspace and IDE

| Capability | Desktop | Android | Web | TUI | Notes |
|------------|---------|---------|-----|-----|-------|
| `GET`/`POST /workspace` (set root) | yes | yes | yes | yes | TUI: `/workspace` `/cd`. |
| File tree + wide-layout IDE | yes | no | reduced | browse | Android / narrow: **locked off** ([`chat_ui_todo.md`](./chat_ui_todo.md), [`coding_agent_client.md`](./coding_agent_client.md)). Web: same WASM; IDE only when the viewport is wide. TUI: full-screen `tui` gets the directory tree in the left column (Ctrl+W; expand/collapse on demand) but no IDE tabs/editor/insert — see the Full-screen TUI row. |
| Save file to this device | yes | yes | yes | no | Desktop: native save dialog. Android: SAF `ACTION_CREATE_DOCUMENT` (not the share sheet; Xiaomi/MIUI often has no Files target). Web: browser download. Bytes via `GET /workspace/file/download` (PDF/binary). `GET /workspace/file/raw` is chat images only. Open IDE tab uses the buffer as UTF-8. Needs current `serve`. |
| Save folder to this device | yes | yes | yes | no | Right-click a folder (or empty tree area = workspace root) → zip via `GET /workspace/dir/archive` (16 MiB uncompressed / 256 files on Server). Same save dialogs as files. Needs Server **#898**. |
| Rename file in tree | yes | yes | yes | no | Inline rename → `POST /workspace/file/move`. Directories are not renamed. 409 prompts overwrite (warns if the destination IDE tab is dirty). Needs Server **#898**. |
| Changelog modal (read-only) | yes | reduced | yes | no | Android: list/summary only; do not unlock IDE to “match Desktop”. Restore/rollback: **planned**, blocked on Server. |
| Git clone UI | yes | yes | yes | no | |
| Drop local files onto tree (`PUT …/file/raw`) | yes | yes | yes | no | Needs a `serve` newer than crates.io **0.4.0**. |

### Coding-agent extras (not full IDE)

| Capability | Desktop | Android | Web | TUI | Notes |
|------------|---------|---------|-----|-----|-------|
| Open changelog/tool path in IDE | planned | no | planned | no | Wave 2; wide layout only. |
| Session restore / accept-reject hunks | planned | planned | planned | no | Server contract first; no fake buttons. |
| Git status / commit / open PR from review | planned | no | planned | no | Wave 3; Android stays list-oriented. |

## Out of this matrix

- Per-route API inventory (`GET /workspace/file`, …) — follow Server OpenAPI / pin.
- Shared crate modules — [`client_shared_logic.md`](./client_shared_logic.md).
- Manual Desktop/Android clicks — [`shell_smoke_runbook.md`](./shell_smoke_runbook.md).
- Server in-process `crabmate chat|repl|tui` — Server docs; official remote terminal is `crabmate-tui`.
