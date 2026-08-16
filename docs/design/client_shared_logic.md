# 多端 Client 共用逻辑抽取（规划）

> **状态**：S1–S4 **已落地**（`url` / `auth` / `secrets` / `approval` / `workspace` / `sessions` / `chat_body`）；S5 可选未开工  
> **范围**：`frontend`（WASM）、`crabmate-connect`（Desktop/Android 壳）、`crabmate-tui` / `crabmate-tui-core`（远程终端）之间的重复逻辑  
> **关联**：[remote_cli_tui.md](./remote_cli_tui.md)、[tauri_gui_mvp_design.md](./tauri_gui_mvp_design.md)、[contract_pin.md](./contract_pin.md)；Server [`client_shell_split.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)

---

## 1. 目标与非目标

### 目标

1. 把**无平台依赖**的契约对齐逻辑抽到单一 crate，供 WASM / tui-core /（可选）connect 共用，减少三份拷贝漂移。
2. 保持路径 A/B：**执行权威仍在 `serve`**；共享层只做 URL、鉴权形状、DTO 解析、决策枚举等。
3. 依赖方向安全：共享层**不得**引入 Tauri、`web-sys`、`reqwest`/`tokio`（WASM 友好；tui/connect 各自做 IO 适配）。

### 非目标

- **不**统一 SSE 全量 UI 分发（工具卡、时间线、澄清问卷）。
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

crabmate-tui ──► crabmate-tui-core ──reqwest──► serve (/health, /chat/stream, …)
                      │
frontend & tui-core ──┴── crabmate-sse-protocol 等线契约（Server git tag）
                         展示 crate：tool-card 已本仓 path（W2）；turn-layout 仍计划改本仓 path
                         见 display_crate_sink.md
```

| 已共享 | 谁用 | 覆盖 |
|--------|------|------|
| `crabmate-connect` | Desktop / Android | 探测、hash 交接、钥匙串 Bearer/LLM 槽、导航白名单 |
| `frontend/` | 两壳包内 UI | 业务 HTTP/SSE、设置、会话、审批 UI |
| `crabmate-tui-core` | 仅 `crabmate-tui` | 远程终端 HTTP/SSE 核心 |
| Server 契约 git tag | frontend（多 crate）；tui-core（sse-protocol） | SSE 分类等 |

设计 [`remote_cli_tui.md`](./remote_cli_tui.md) §3 已允许：connect 与 tui 重叠的「纯 HTTP 探测」可逐步上收；**不**阻塞终端分期。

---

## 3. 拟建布局

```text
crates/
  crabmate-client-api/   # S1–S4 已建：纯逻辑（url / auth / secrets / approval / workspace / sessions / chat_body）
  crabmate-tool-card/    # W2：工具卡 compact/detail（frontend path；不进 tui-core）
  crabmate-connect/      # 保持：Tauri commands + keyring IO + CORS + handoff（可选依赖 client-api）
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
| `health`（可选） | `/health` degraded 等 JSON 解析子集（不含壳 CORS） |

---

## 4. 高价值抽取对照（现状路径）

### 4.1 API 基址 + 路径拼接（优先）

| 端 | 路径 |
|----|------|
| frontend | `frontend/src/api/browser.rs` → `normalize_api_base_url` / `api_url` |
| tui-core | `crates/crabmate-tui-core/src/url.rs` → `normalize_api_base` / `api_url` |

**注意**：`crabmate-connect` `handoff.rs` 的 `normalize_base_url` 语义不同（可补 `http://`、拒 `0.0.0.0` 等）。共享层只收**严格绝对基址**子集；连接页输入规范化仍留 connect。

### 4.2 鉴权双头

| 端 | 路径 |
|----|------|
| connect | `crates/crabmate-connect/src/probe.rs` → `attach_bearer` |
| tui-core | `crates/crabmate-tui-core/src/client.rs` → `auth_headers` |
| frontend | `frontend/src/api/browser.rs` → `auth_headers` |

GitHub：`X-CrabMate-GitHub-Token` 目前主要在 frontend（+ 壳钥匙串槽）；tui 尚未接线——共享**头名常量**即可。

### 4.3 审批

| 端 | 路径 |
|----|------|
| tui-core | `approval.rs`、`client.rs` → `submit_chat_approval` |
| frontend | `sse_dispatch/types.rs`、`chat_stream/parser_v2.rs`、`api/http.rs` → `submit_chat_approval` |

共享：决策枚举、SSE `allowlistKey` 解析、body 形状。`approval_session_id` **生成器**可分端（`tui_…` vs `approval_…`），只共享合法字符约束若需要。

### 4.4 Workspace / Sessions

| 能力 | tui-core | frontend |
|------|----------|----------|
| workspace | `workspace.rs` | `http.rs` / `http_workspace_projects.rs` |
| sessions | `sessions.rs`（瘦 DTO） | `user_data.rs`（完整 `ChatSession` + PUT） |

共享：set 响应解析、list 行子集、`conversation_id_for_resume`（无本地 Web `id` 冒充）。file/dir/projects/clone、PUT 水合留 frontend。

### 4.5 Chat stream 核心 body

| 端 | 路径 |
|----|------|
| tui-core | `chat_stream.rs` → `chat_stream_body` |
| frontend | `chat_stream/http_request.rs` → `build_chat_stream_post_body` |

共享核心字段；图像 / resume / `client_llm` 注入 / 温度等仍留 WASM。

### 4.6 斜杠（低优先级）

| 端 | 路径 |
|----|------|
| tui | `crates/crabmate-tui/src/slash.rs` |
| frontend | `frontend/src/app/chat/composer_slash_control.rs` |

最多共享控制命令**名字表**（`help` / `workspace` / `cd`…）；handler 分端。

### 4.7 Health 探测子集

| 端 | 行为 |
|----|------|
| connect | `/health` → prefs → **壳 CORS**（`probe.rs`） |
| tui-core | 仅 `GET /health` |

共享：health GET 语义 / degraded 解析。CORS / Origin 常量属壳专用。

---

## 5. 明确不共享

| 类别 | 原因 | 代表 |
|------|------|------|
| Tauri 导航 / Origin 白名单 | WebView 安全模型 | `connect` `navigation.rs` / `allowed_origin.rs` |
| 壳 CORS 探测 | 仅包内 UI | `probe_shell_cors`、`SHELL_WEBVIEW_*` |
| Desktop 生命周期 | 托盘、单实例 | `desktop-tauri` |
| Android Keystore / 返回键 | Kotlin 桥 | `Secure*Store.kt`、`MainActivity.kt` |
| WASM `fetch` / AbortSignal / LS | 浏览器运行时 | `frontend/src/api/browser.rs`、`chat_stream/*` |
| 全量 SSE UI 分发 | 产品 UI | `sse_dispatch/*`、`parser_v2.rs` |
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
| **S5**（可选） | health 子集；斜杠名字表；frontend 其余纯逻辑继续上收 | WASM 体积与编译时间可接受 |

**建议开工顺序**：S0 → S1–S4（已完成）→ 可选 S5。

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
| 依赖 | frontend / tui-core → client-api；connect **可选**依赖 client-api 仅用 URL/auth 常量 |
| IO | 仍分端：`fetch` vs `reqwest` vs keyring |
| 下一步 | 可选 **S5**（health 子集 / 斜杠名字表）或功能并行 |

---

## 9. 非本规划事项

- Server D2 硬删同进程 `chat|repl|tui`（见 Server `client_shell_split.md` §2.5）。
- `crabmate-tui` 钥匙串 / GitHub 头 / `stream_resume`（终端功能，见 `remote_cli_tui.md`）。
- P4 全屏 ratatui / P5 发版说明。
