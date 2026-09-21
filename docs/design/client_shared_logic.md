# 多端 Client 共用逻辑抽取（规划）

> **状态**：S1–S4 **已落地**；hash 交接键名、S5 health JSON 子集、**斜杠名字表**、**端点路径常量（`paths`）**、**通用 HTTP 错误文案（`messages`）** 均已落地；**S6（SSE / AG-UI 纯解析下沉）进行中**（S6a `prompt_tokens` 已落地，见 §4.11 / §6）  
> **范围**：`frontend`（WASM）、`crabmate-connect`（Desktop/Android 壳）、`crabmate-tui`（远程终端）之间的重复逻辑（原 `crabmate-tui-core` 已并入 `crabmate-tui` `src/serve/`，见 §2 注记）  
> **关联**：[remote_cli_tui.md](./remote_cli_tui.md)、[tauri_gui_mvp_design.md](./tauri_gui_mvp_design.md)、[contract_pin.md](./contract_pin.md)、产品面对照 [client_capability_matrix.md](./client_capability_matrix.md)；Server [`client_shell_split.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)

---

## 1. 目标与非目标

### 目标

1. 把**无平台依赖**的契约对齐逻辑抽到单一 crate，供 WASM / tui-core /（可选）connect 共用，减少三份拷贝漂移。
2. 保持路径 A/B：**执行权威仍在 `serve`**；共享层只做 URL、鉴权形状、DTO 解析、决策枚举等。
3. 依赖方向安全：共享层**不得**引入 Tauri、`web-sys`、`reqwest`/`tokio`（WASM 友好；tui/connect 各自做 IO 适配）。

### 非目标

- **不**统一 SSE 全量 UI 分发（工具卡、时间线、澄清问卷）——但 **AG-UI 纯解析与 DTO** 属共享范围（见 §4.11 / S6）；「解析」与「钩子实现」分界见该节。
- **不**统一 HTTP 客户端实现（WASM `fetch` vs `reqwest`）。
- **不**把钥匙串 / Keystore / `localStorage` 读写抽进共享层（只共享**槽名常量**）。
- **不**让 `frontend` 依赖 `crabmate-tui-core`，或让 `tui`/`frontend` 依赖 `crabmate-connect`。
- **不**阻塞功能交付：可与 `remote_cli_tui` P4/钥匙串等并行；小步 PR。

---

## 2. 现状端与已共享

```text
desktop/mobile ──path──► crabmate-connect ──reqwest──► serve (/health, prefs, …)
       │
       └──── WebView ──► frontend (WASM fetch) ──► serve (全 API / SSE)

crabmate-tui (serve/) ──reqwest──► serve (/health, /chat/stream, …)
        │
frontend & tui (serve/) ──┴── crabmate（protocol feature；crates.io 0.5.2）
                            展示 crate：tool-card 已本仓 path（W2）；turn-layout 仍计划改本仓 path
                            见 display_crate_sink.md
```

| 已共享 | 谁用 | 覆盖 |
|--------|------|------|
| `crabmate-connect` | Desktop / Android | 探测、hash 交接、钥匙串 Bearer/LLM 槽、导航白名单（导航钩子 / invoke 需 feature `tauri`） |
| `frontend/` | 两壳包内 UI | 业务 HTTP/SSE、设置、会话、审批 UI |
| `crabmate-tui` `serve/` 模块 | 仅 `crabmate-tui` | 远程终端 HTTP/SSE 核心（原 `crabmate-tui-core`） |
| Server 契约 git tag | frontend（多 crate）；tui `serve/`（protocol） | SSE 分类等 |

设计 [`remote_cli_tui.md`](./remote_cli_tui.md) §3 已允许：connect 与 tui 重叠的「纯 HTTP 探测」可逐步上收；**不**阻塞终端分期。

**2026-09**：`crabmate-tui-core` 已整体并入 `crabmate-tui`（`crates/crabmate-tui/src/serve/`，模块名 `serve`；依赖与 `Cargo.lock` 同步吸收）；下文 §1/§3/§6/§8 为历史规划叙述保留原文，涉及 tui-core 的现状路径均按 `serve/` 新位置理解。

**2026-09**：Markdown 渲染前的文本规范化（原 `frontend/src/markdown.rs::normalize_markdown_for_render` 及全部 CJK/围栏/标题/列表补丁）下沉为 `crabmate-client-api::markdown_normalize`（纯逻辑、零新依赖），`frontend` 改为消费共享实现（行为不变）；TUI `tui_mode/md.rs::assistant_styled_text` 已接入同一入口（先 normalize 再逐行轻渲染，渲染文本与 normalize 输出逐字一致）。

---

## 3. 拟建布局

```text
crates/
  crabmate-client-api/   # S1–S4 + handoff + health JSON：纯逻辑（无 IO）
  crabmate-tool-card/    # W2：工具卡 compact/detail（frontend path；不进 tui-core）
  crabmate-connect/      # 默认无 Tauri（probe / keyring）；壳 `features = ["tauri"]`；hash 拼装委托 client-api
  crabmate-tui-core/     # 变薄：reqwest ServeClient 调用 client-api
  crabmate-tui/          # CLI / TTY / slash 宿主
frontend/                # wasm fetch 适配器 + UI；S1–S4 已用 client-api 核心字段/解析
```

拟定模块（实现时可拆文件，名称可微调）：

| 模块 | 内容 |
|------|------|
| `url` | 严格绝对基址规范化 + path join（去尾 `/`、拒相对 URL） |
| `auth` | Bearer / `X-API-Key` 头名与值格式；可选 GitHub 头名常量 |
| `approval` | `deny` / `allow_once` / `allow_always`；`command_approval` data 解析；approval POST body 形状 |
| `workspace` | `POST /workspace` 响应 `ok`/`path`/`error` 解析；可选瘦 `WorkspaceInfo` |
| `sessions` | 瘦 list 行 + **仅** `server_conversation_id` 可作续聊 id |
| `chat_body` | `POST /chat/stream` **核心**字段（message / `client_sse_protocol` / conversation_id / approval_session_id） |
| `secrets` | `LlmSecretSlot` / Bearer 账户等**名字常量**（无 IO） |
| `handoff` | `#cm_api_base=` / `#cm_web_api_bearer=` 键名、RFC3986 分量编码、fragment 拼装（无 `history` / 无查询串） |
| `health` | `/health` degraded 检查摘要（不含壳 CORS） |
| `paths` | 跨端共用的端点路径常量与动态段构造器（≥2 端实际共用才收） |
| `slash` | 跨端共用的斜杠控制命令名字表（handler 分端） |
| `messages` | HTTP 错误体通用取错（`error` → `message` → `HTTP {status}`），display 层不属契约 |
| `prompt_tokens`（S6a） | tiktoken 快照 DTO + camelCase/snake_case 双键解析（无 IO） |
| `sse_dispatch`（S6b） | AG-UI 控制面 DTO + `SseControlSink` 钩子组签名 + `SseDispatch` 三态（钩子实现留端） |
| `ag_ui_parser`（S6c） | AG-UI 单行 JSON → 控制面分发（`parse_ag_ui_line`；仅 `serde_json` + `cm_sse_protocol`） |

---

## 4. 高价值抽取对照（现状路径）

**2026-09**：契约镜像瘦身落地——`crabmate-client-api` 不再本地镜像 Server 协议 DTO；`CommandApprovalData` / `ApprovalDecision` / `SessionListRow` / 审批 POST body / workspace 响应视图等改从契约 crate `crabmate`（0.5.2，`features=["protocol"]`）消费并按需 re-export（§4.3–4.5、§4.7 的「共享」类型即此来源）。client-api 仅保留产品逻辑：续聊 id 只认 `server_conversation_id`、degraded 文案、chat body 键集对照契约测试。SSE `command_approval` 数据形状不符契约（缺 `command`/`args`）时不弹审批：全屏 TUI 打印一条系统提示行（提醒回合可能无响应），Web 静默跳过。

**P3 收尾**：lib.rs 顶层 `pub use` 收窄为消费方实际使用的项（未收进顶层的项仍可经 `url::` / `workspace::` 等模块路径取用）；HTTP 状态 → 用户文案这类 display 层拼装移入 `messages` 模块（不属契约，Server 不依赖），后续新增用户文案优先落这里。

### 4.1 API 基址 + 路径拼接（优先）

| 端 | 路径 |
|----|------|
| frontend | `frontend/src/api/browser.rs` → `normalize_api_base_url` / `api_url` |
| tui (serve) | `crates/crabmate-tui/src/serve/url.rs` → `normalize_api_base` / `api_url` |

**注意**：`crabmate-connect` `handoff.rs` 的 `normalize_base_url` 语义不同（可补 `http://`、拒 `0.0.0.0` 等）。共享层只收**严格绝对基址**子集；连接页输入规范化仍留 connect。

### 4.2 鉴权双头

| 端 | 路径 |
|----|------|
| connect | `crates/crabmate-connect/src/probe.rs` → `attach_bearer` |
| tui (serve) | `crates/crabmate-tui/src/serve/client.rs` → `auth_headers` |
| frontend | `frontend/src/api/browser.rs` → `auth_headers` |

GitHub：`X-CrabMate-GitHub-Token` 目前主要在 frontend（+ 壳钥匙串槽）；tui 尚未接线——共享**头名常量**即可。

### 4.3 审批

| 端 | 路径 |
|----|------|
| tui (serve) | `serve/approval.rs`、`serve/client.rs` → `submit_chat_approval` |
| frontend | `sse_dispatch/types.rs`、`chat_stream/parser_v2.rs`、`api/http.rs` → `submit_chat_approval` |

共享：决策枚举、SSE `allowlistKey` 解析、body 形状。`approval_session_id` **生成器**可分端（`tui_…` vs `approval_…`），只共享合法字符约束若需要。

### 4.4 Workspace / Sessions

| 能力 | tui (serve) | frontend |
|------|----------|----------|
| workspace | `serve/workspace.rs` | `http.rs` / `http_workspace_projects.rs` |
| sessions | `serve/sessions.rs`（瘦 DTO） | `user_data.rs`（完整 `ChatSession` + PUT） |

共享：set 响应解析、list 行子集、`conversation_id_for_resume`（无本地 Web `id` 冒充）。file/dir/projects/clone、PUT 水合留 frontend。

### 4.5 Chat stream 核心 body

| 端 | 路径 |
|----|------|
| tui (serve) | `serve/chat_stream.rs` → `chat_stream_body` |
| frontend | `chat_stream/http_request.rs` → `build_chat_stream_post_body` |

共享核心字段；图像 / resume / `client_llm` 注入 / 温度等仍留 WASM。

### 4.6 斜杠名字表（已落地）

| 端 | 路径 |
|----|------|
| tui | `crates/crabmate-tui/src/slash.rs`、`tui_mode/controls.rs` |
| frontend | `frontend/src/app/chat/composer_slash_control.rs`、`composer_slash_menu.rs` |

共享：`crabmate_client_api::slash` 常量（`help` / `?` / `workspace` / `cd` / `model` / `conv` / `status` / `mode` / `role` / `quit` / `exit` / `q`）——只收 **≥2 端共用**的命令 head；handler 分端。单端命令（web 的 `agent` / `export` / `config` / `api-base` / `clear` / `api-key` / `skills` 等，tui-mode 的 `settings` / `find`）与 handler 一起留端，避免「拦截了但没 handler」的超集误伤。

### 4.7 Health 探测子集（已落地）

| 端 | 行为 |
|----|------|
| connect | `/health` → prefs → **壳 CORS**（`probe.rs`）；degraded 文案用 `health_degraded_note` |
| tui (serve) | `GET /health`；2xx 时同样解析 degraded 并打 stderr，不失败 |

共享：`crabmate_client_api::health_degraded_note`。CORS / Origin 常量仍属壳专用。

### 4.8 Hash 交接键名（已落地）

| 端 | 路径 |
|----|------|
| connect | `handoff.rs` → `build_*_handoff_url` |
| frontend | `api/connect_handoff.rs`（消费 + `replaceState`） |
| web-host | `root.rs` → `page_url` |
| Playwright | `e2e/fixtures/helpers.ts`（TS 不能依赖 crate；字面量须与常量一致） |

共享：`API_BASE_HASH_KEY` / `BEARER_HASH_KEY`、`percent_encode_unreserved`、`handoff_hash_fragment`。`history.replaceState`、hash 解析解码仍留 WASM（`urlencoding`）。

### 4.9 端点路径常量（已落地）

共享：`crabmate_client_api::paths`——只收 **≥2 端实际共用**的端点（`/health`、`/status?view=shell`、`/upload`、`/chat/stream`、`/chat/approval`、`/chat/branch`、`/workspace*`、`/user-data/{prefs,llm-overrides,workspaces/current/sessions}`、`/config/session/conversation-store`）；动态段用构造器（如 `chat_stream_cancel(job_id)`）。消费方：frontend `api/*`、tui `serve/`（`client.rs` / `sessions.rs` / `user_data.rs` / `workspace.rs` / `chat_stream.rs`）、connect `probe.rs`、tui CLI。单端端点（workspace `file*`、mcp-servers 等）与 `/uploads/` 静态资产不收，仍留各端字面量。

### 4.10 通用 HTTP 错误文案（已落地）

共享：`messages::http_error_text`（body `error` → `message`，trim 后非空才采用）+ `messages::http_error_message`（→ `HTTP {status}` 兜底）。消费方：frontend `http.rs`（`http_error_detail_from_body`）、`http_workspace_clone.rs`、`http_workspace_projects.rs`、`session_store.rs`（原 message 优先统一为 error 优先）。

留端（display 层差异）：code + `request_id` 拼装与 240 字符截断（frontend `http.rs`）、clone 的 `{code}: … (HTTP {status})` 包装、i18n 前缀与 401/403 特判文案（connect `probe.rs`、frontend `user_data.rs`）、tui `serve/error.rs` 的类型化 `thiserror` Display。`http_error_status_code` 的括号反解状态码模式暂保留（消除需错误携带结构化 status，改动面大，另行处理）。

### 4.11 SSE / AG-UI 纯解析（S6，进行中；S6a 已落地）

**现状重复面**（2026-09 实测）：

| 端 | 路径 | 行数 | 性质 |
|----|------|------|------|
| frontend | `frontend/src/sse_dispatch/types.rs` | 351 | 纯 DTO + `Option<&mut dyn FnMut(..)>` 钩子组；**零** `i18n` / `Locale` / `wasm-bindgen` |
| frontend | `frontend/src/api/chat_stream/parser_v2.rs` | 830（非测试约 490） | 纯 AG-UI JSON → 控制面分发；仅依赖 `serde_json` + `crabmate::cm_sse_protocol` + 上述 DTO |
| tui | `crates/crabmate-tui/src/serve/chat_classify.rs` | 244 | 同语义的**自建子集**（文件头自述"与 Web parser_v2 对齐的子集"） |
| frontend | `conversation_hydrate.rs` → `TiktokenPromptTokensSnapshot` | 21 | 纯 serde DTO（被 Sink 签名引用，须随迁） |
| frontend | `conversation_prompt_tokens_apply.rs` → `parse_tiktoken_prompt_tokens_value` / `tiktoken_from_ag_ui_object` | 23 | 纯函数（同文件 `apply_conversation_prompt_tokens_from_sse` 绑 leptos 信号，**留端**） |

TUI 侧判据：`serve/chat_stream.rs:5-8` 已 `use crabmate::cm_sse_protocol::…; use crabmate_client_api::{ChatStreamCoreFields, build_chat_stream_core_body, paths};` —— 依赖通道已通，收敛分类器**无需新增跨包依赖**。

**下沉目标**：`crabmate-client-api` 新增 `sse_dispatch`（DTO + 钩子组）、`ag_ui_parser`（`parse_ag_ui_line` / `SseDispatch` 三态）、`prompt_tokens`（tiktoken DTO + 解析）。

**必须留端**：

| 留端项 | 原因 |
|--------|------|
| 钩子**实现**（写 leptos `RwSignal` / TUI 终端态） | `types.rs` 只定义签名；实现属产品 UI |
| `ChatStreamCallbacks`（`Rc<dyn Fn>` 组合）+ `send_chat_stream` | 与 wasm `fetch` 同处 `chat_stream/mod.rs` |
| WASM `fetch` / `ReadableStream` 帧读取 | 浏览器运行时（`body_reader.rs`） |
| `Locale` 参数与错误文案 | i18n 不回退；`handle_sse_block` 3 处调用留端 |
| 工具**跨帧累积** | 两端 `TOOL_CALL_ARGS` / `TOOL_CALL_END` 均为显式空实现；实际累积在 `app/chat/composer_stream/callbacks/builders/tool_callbacks.rs`，写 `RwSignal<HashMap<…>>` |

**风险控制**：`sse_dispatch::` 现有 **25 个** frontend 消费文件、`TiktokenPromptTokensSnapshot` **10 个**（不含定义处）。为免大范围改 import，`frontend/src/sse_dispatch/mod.rs` 保留为 `pub use crabmate_client_api::sse_dispatch::*;` 转发壳，消费方路径 `crate::sse_dispatch::X` 不变。

**TUI 收敛方式**：`chat_classify.rs` 的自建 `classify_line`（244 行）改为「`SseControlSink` 收集器 → `LineAction`」适配器（约 60–80 行），未消费的子类仍回落 `Skip`（保持现状语义）。

---

## 5. 明确不共享

| 类别 | 原因 | 代表 |
|------|------|------|
| Tauri 导航 / Origin 白名单 | WebView 安全模型 | `connect` `navigation.rs` / `allowed_origin.rs` |
| 壳 CORS 探测 | 仅包内 UI | `probe_shell_cors`、`SHELL_WEBVIEW_*` |
| Desktop 生命周期 | 托盘、单实例 | `desktop-tauri` |
| Android Keystore / 返回键 | Kotlin 桥 | `Secure*Store.kt`、`MainActivity.kt` |
| WASM `fetch` / AbortSignal / LS | 浏览器运行时 | `frontend/src/api/browser.rs`、`chat_stream/mod.rs`、`body_reader.rs` |
| SSE **钩子实现**（非解析） | 产品 UI（写信号 / 终端态） | `app/chat/composer_stream/callbacks/**`、`sse_dispatch` 的钩子实现、`tui_mode/*` |
| TTY 审批 / reedline | 终端交互 | `approval_tty.rs`、`crabmate-tui` main |
| 工作区文件树 / clone SSE | 仅 Web | `http.rs` file/dir、`http_workspace_clone.rs` |

---

## 6. 分期

| 阶段 | 交付 | 验收 |
|------|------|------|
| **S0** | 本文档；命名与依赖边界共识 | 与 AGENTS / `remote_cli_tui` / connect 无冲突 |
| **S1** ✅ | `crabmate-client-api`：`url` + `auth` + `secrets` 常量；tui-core / connect / frontend 改依赖 | clippy；表征测试；`wasm32` check（frontend） |
| **S2** ✅ | `approval` 类型/解析；两端替换拷贝 | 审批决策串与 `allowlistKey` 单测对齐 |
| **S3** ✅ | `workspace` set 解析 + `sessions` 瘦模型 | tui `/workspace` `/conv list` 与 Web 字段一致 |
| **S4** ✅ | `chat_body` 核心字段 builder | `client_sse_protocol` 钉点不易漏 |
| **S5a** ✅ | hash 交接键名 + fragment 拼装；connect / frontend / `crabmate-web` 改依赖 | 键名与 `%2F` 编码单测对齐 |
| **S5 health** ✅ | `health_degraded_note`；connect / tui-core 改依赖 | degraded JSON 单测；CORS 仍留 connect |
| **S5 slash** ✅ | 斜杠名字表（`slash` 常量；单端命令与 handler 留端） | tui / web 控制命令匹配一致；WASM 体积与编译时间可接受 |
| **S5 paths/messages** ✅ | 端点路径常量（`paths`）+ 通用 HTTP 取错（`messages`） | 40 处路径替换零字面量漂移；错误文案取值顺序统一 error 优先 |
| **S6a** ✅ | `prompt_tokens`：`TiktokenPromptTokensSnapshot` + `parse_tiktoken_prompt_tokens_value` / `tiktoken_from_ag_ui_object` 下沉；frontend 侧 re-export | 既有 camelCase/snake_case 用例随迁通过；`apply_conversation_prompt_tokens_from_sse` 留端；frontend `wasm32` check |
| **S6b** 🅿️ | `sse_dispatch`：DTO + 4 组钩子 + `SseDispatch` 下沉；`frontend/src/sse_dispatch/mod.rs` 改 `pub use crabmate_client_api::sse_dispatch::*;` | 25 个消费方零 import 改动；`ToolJobState` 契约轮询样例单测随迁通过；`wasm32` check |
| **S6c** 🅿️ | `ag_ui_parser`：`parser_v2.rs` 非测试部分下沉；frontend `V2Parser` 退化为薄壳（仅实现本地 `SseParser`） | `golden_ag_ui_v2_parser_matches_expected` / `RUN_FINISHED` / `RUN_ERROR` / `tool_call_result` / `multi_line_tool_call_splits` 等单测随迁并通过 |
| **S6d** 🅿️ | TUI `serve/chat_classify.rs` 改为共享解析适配器（Sink 收集器 → `LineAction`，约 60–80 行） | TUI 单测；未消费子类仍回落 `Skip`；`scripts/lizard-rust.sh` CCN ≤10 / 行数 ≤920；`docs/design/shell_smoke_runbook.md` 手工 smoke |
| **S6e**（可选，缓做） | `sse_frame` 帧切分（`SseFrameKind` / `SseBufferProgress` / `process_sse_buffer_step` / `flush_sse_tail`）下沉 | 需先拆 `ChatStreamCallbacks` 出 `chat_stream/mod.rs` 并把 `Locale` 参数化；改动面大于 S6a–d，收益更低（`\n\n` 切分已部分委托 `cm_sse_protocol`） |

**建议开工顺序**：S0 → S1–S4 → S5a / S5 health → S5 slash / paths / messages（均已完成）→ **S6a → S6b → S6c → S6d**（S6e 视 S6a–d 收益再定）。

每步独立小 PR，均**不新增 crate**、不动 `scripts/rust-pkg-dirs.txt`；每步以 `make check`（含 frontend `wasm32` clippy）+ `make test` 收口。

---

## 7. 风险与缓解

| 风险 | 缓解 |
|------|------|
| WASM 依赖拖进原生 IO | client-api 禁止 `reqwest`/`tokio`/`tauri`；CI 对 client-api 做 `wasm32` check（若 frontend 依赖） |
| connect 与严格 URL 语义混淆 | 文档写清两段式：输入规范化（connect）→ 严格基址（client-api） |
| 大爆炸重构 | 每阶段一个小 PR；先 tui-core 迁入再改 frontend |
| 契约漂移 | 审批/会话字段以 Server OpenAPI / sse-protocol 为准；单测钉字符串 |

---

## 8. 已拍板（本规划）

| 项 | 决定 |
|----|------|
| 共享形态 | 新建 **`crabmate-client-api`**（纯逻辑），不是扩大 `connect` 或 `tui-core` |
| 依赖 | frontend / tui-core / connect / `crabmate-web` → client-api |
| IO | 仍分端：`fetch` vs `reqwest` vs keyring |
| 下一步 | 功能并行；剩余单端逻辑不强行上收 |
| SSE 解析边界（S6） | **纯解析 + DTO** 下沉 `crabmate-client-api`（`sse_dispatch` / `ag_ui_parser` / `prompt_tokens`）；**钩子实现 / wasm 帧读取 / i18n 文案 / 工具跨帧累积** 留端 |
| S6 不新增 crate | 全部落 `crabmate-client-api` 现有边界内；`crates/` 布局与 `rust-pkg-dirs.txt` 不变 |

---

## 9. 非本规划事项

- Server D2 硬删同进程 `chat|repl|tui`（见 Server `client_shell_split.md` §2.5）。
- `crabmate-tui` 钥匙串 / GitHub 头 / `stream_resume`（终端功能，见 `remote_cli_tui.md`）。
- P4 全屏 ratatui / P5 发版说明。
