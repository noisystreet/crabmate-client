# 远程 CLI / TUI（路径 B）

> **状态**：方案（已拍板走 **远程客户端**；**P3 已落地** `repl` 斜杠 `/help` `/workspace` `/conv`；**P4 已落地（M1–M4）**：全屏 `tui` 流式 transcript + 状态行 + 单行/多行输入 + Ctrl+C 取消 + 左栏会话与 `/status` 联动 + 审批浮层 + thinking 折叠 + 工具行摘要 + `/find` 搜索 + 翻页滚动 + 顶栏工作区 + 助手正文行内 Markdown 轻渲染 + 工作区目录树浏览（Ctrl+W））  
> **范围**：`crabmate-client` 新增终端面；`crabmate serve` 仍为执行权威  
> **关联**：Server [`client_shell_split.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)、[`命令行与路由.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/命令行与路由.md)、本仓 [`contract_pin.md`](./contract_pin.md) / [`personal_cloud_runbook.md`](./personal_cloud_runbook.md)

---

## 1. 目标与非目标

### 目标

1. 在 **Client 仓**提供官方终端入口：连接已运行的 `crabmate serve`，经 **HTTP + SSE** 对话 / 审批 / 工作区等。
2. 与 Desktop / Android / 浏览器共用同一契约钉点（`vX.Y.Z`）与鉴权模型（Web Bearer；GitHub 头/Cookie 按端）。
3. 发版与壳解耦：终端产物可独立 `.deb` / cargo 安装，**不**内嵌、**不** spawn `serve`。

### 非目标

- **不**把 Server 同进程 `repl`/`tui`（`run_agent_turn`）剪贴进本仓。
- **不**在本仓引入 Agent / 工具执行栈 / `path` 回主仓。
- **不**强制立刻删除 Server 的 `crabmate chat|repl|tui`（可并行一段时间，再 deprecate）。
- **不**在 P1 追求与现网 TUI 布局像素级一致。

---

## 2. 决策总览

| 项 | 决定 |
|----|------|
| 形态 | **远程客户端**（同 Tauri 壳）：只认 `serve` API |
| 二进制名 | **`crabmate-tui`**（避免与 Server 包名 `crabmate` / Desktop `crabmate-desktop` 冲突） |
| 子命令（终态） | `connect`（探测）· `chat`（单轮/管道）· `repl`（交互行编辑）· `tui`（全屏，M1–M4 已落地） |
| 契约 | crates.io `crabmate` `0.5.0` + `protocol`（与 `frontend` 同一钉点） |
| HTTP 客户端 | `reqwest` + rustls；SSE 解析对齐协议 crate |
| 连接配置 | 复用/对齐 connect 键：`api_base`、Web Bearer；密钥读取与壳**同源**（read-only 回退，见 §4），不写壳槽位 |
| GitHub | 原生端：钥匙串槽 `github` + `X-CrabMate-GitHub-Token`；无浏览器 Cookie 路径 |
| 审批 | `POST /chat/approval`；TTY 用菜单/读行；`--yes` 对齐 Server `chat --yes` 语义（仅远程放行决策，执行仍在 serve） |

```
crabmate-tui ──HTTP/SSE──► crabmate serve ──► Agent / tools
     ▲                         ▲
     │                         └── 同进程旧 repl/tui（过渡期仍可留在 Server）
     └── crabmate-client 仓发版
```

---

## 3. 仓内布局（拟定）

```text
crates/
  crabmate-connect/     # 已有：probe / Bearer / keyring（壳）
  crabmate-tui-core/    # 新建：API 客户端、SSE 消费、会话/审批、配置
  crabmate-tui/         # 新建：二进制 clap；chat / repl +（后）全屏 tui
```

- `tui-core`：**无** Tauri 依赖，可供测试与未来其它宿主复用。
- 可逐步把 `crabmate-connect` 里与「纯 HTTP 探测」重叠的逻辑抽到共用模块；**不**阻塞 P1（P1 可先在 `tui-core` 内复制最小 probe）。
- 多端（WASM / connect / tui）共用纯逻辑的边界与分期见 [`client_shared_logic.md`](./client_shared_logic.md)（拟建 `crabmate-client-api`）。

禁止：`path = "../crabmate_agent/..."`。

---

## 4. 能力映射（Server 同进程 → 远程）

| 能力 | Server 现状 | 远程 `crabmate-tui` |
|------|-------------|----------------------|
| 一轮对话 | `run_agent_turn` | `POST /chat/stream`（主路径）或 `/chat` |
| 断线续传 | TUI SSE mirror | repl `/resume`：`stream_resume:{job_id,after_seq}`（job 取响应头 `x-stream-job-id`；cancel 已送达的回合不可续，cancel 未送达时 job 可能仍在跑、仍可续） |
| 审批 | 进程内 dialoguer | SSE 控制面 + `POST /chat/approval` |
| 回合取消 | 进程内中断 | Ctrl+C → `POST /chat/stream/{job_id}/cancel`（job 取响应头 `x-stream-job-id`；旧 serve 无路由时降级为本地中断） |
| 斜杠 /skills | 同进程 | 经 stream 内置命令或后续 REST |
| 工作区 | 本地 path | `POST /workspace`、`GET /workspace/...` |
| 会话列表/分支 | SQLite 同库 | `/user-data/.../sessions`、`POST /chat/branch` |
| 模型密钥 | 本机 keyring → 回合注入 | 每轮请求体 `client_llm.{api_key,model,api_base}`（同 WASM UI 设置子集）；CLI/env 提供，缺省时 read-only 回退壳钥匙串 |
| GitHub token | 现已请求作用域 | 头 `X-CrabMate-GitHub-Token` |

> **模型密钥 / Web Bearer（CLI/env + 壳钥匙串回退，P3 已落地）**：`crabmate-tui chat|repl` 支持 `--llm-api-key` / `--llm-model` / `--llm-api-base`（env 沿用 serve 侧模型 env 名：`CM_API_KEY` / `CM_MODEL` / `CM_API_BASE`），有任一非空时随 `POST /chat/stream` 发送 `client_llm.{api_key,model,api_base}`，语义同 WASM UI「设置 → API 密钥/模型」——供 bearer 鉴权且服务端未设 `API_KEY` 的 serve（如个人云）使用；密钥仅存进程内、不落盘。`--bearer` 仍是 Web Bearer（≠ 模型 API_KEY）。**`--bearer` / `--llm-api-key`（或对应 env）缺省时**，TUI read-only 回退读取桌面壳已写入同一系统钥匙串（service `com.crabmate.credentials`）的槽位：`tauri_connect_web_api_bearer` / `tauri_client_llm_api_key`，不写壳槽位；`--no-keyring` 关闭回退（无条目或钥匙串不可用时静默跳过）。

---

## 5. 分期

| 阶段 | 交付 | 验收 |
|------|------|------|
| **P0** | 本文档；命名与边界共识 | 与 AGENTS / 路径 A 无冲突 |
| **P1** | `crabmate-tui`：`chat` 单轮 + Bearer + `/chat/stream` 文本输出 | 对本地 `serve` 跑通一轮；CI 可选 smoke |
| **P2** | 交互 `repl`（reedline 或最小行编辑）+ 审批 TTY + 会话 id 续聊 | 非白名单命令可审批；Ctrl+C 干净退出 |
| **P3** | 工作区/会话斜杠子集与 WASM 设置对齐的常用操作 | `/workspace`、列会话等 |
| **P4** | 实验性 `tui`（ratatui）：流式 transcript + 状态行 + 左栏会话 + 多行底栏输入（**M1 已落地**）；左栏会话 + 状态行联动（**M2 已落地**）；审批浮层 / thinking 折叠 / 工具行摘要 / 滚动搜索 / 多行输入（**M3 已落地**）；工作区目录树浏览（**M4 已落地**） | TTY 全屏可用；非白名单命令浮层审批（Esc=拒绝）；长对话可滚动；多行可编辑发送；窄终端（<120 列）自动隐藏左栏；Ctrl+W 在左栏展开工作区目录树（↑↓ 选择 · Enter/→ 展开 · ← 收起/回父 · r 刷新 · w 回会话） |
| **P5** | 文档 / `.deb` 或 cargo 安装说明；Server 侧标注同进程 CLI「过渡 / deprecate 窗口」 | README 双语文案同步；Client `make tui-release` 产出仅二进制的 `crabmate-tui_*.deb`（无图标、无配置） |

**建议开工顺序**：P1 → P2；P4 可与 P3 并行但不要阻塞 P2。

---

## 6. 风险

| 风险 | 缓解 |
|------|------|
| 与 Server `crabmate` 二进制名冲突 | 固定 `crabmate-tui` |
| SSE/审批语义漂移 | 钉契约 tag；复用协议 crate；对照 WASM `chat_stream` |
| 用户期望「一条命令无 serve」 | README 写明须先 `crabmate serve`；个人云场景连远程 URL |
| 范围膨胀（复刻全部斜杠） | P1–P2 只保证对话+审批；其余按使用频率加 |
| 双实现维护（旧同进程 + 新远程） | Server 文档标明官方终端迁 Client；旧入口给迁移期 |

---

## 7. 已拍板

| 项 | 决定 |
|----|------|
| 策略 | **B：远程 CLI/TUI 进 Client 仓** |
| 同进程 Agent CLI | **不**迁入；Server 可暂留 |
| 执行权威 | 仅 `serve` |
| 下一步 | P4 已收尾（M4：工作区目录树浏览已落地）；文档/发版向 P5（README 双语、`.deb` 安装说明；Server 侧标注过渡窗口） |

---

## 8. 非本仓事项（Server）

- 文档：官方矩阵增加「Terminal remote client」；CLI/TUI 同进程改为「过渡」。
- 日后：`crabmate tui|repl` help 指向 `crabmate-tui`；再定删除时间表。
