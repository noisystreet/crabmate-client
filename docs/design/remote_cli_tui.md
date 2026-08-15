# 远程 CLI / TUI（路径 B）

> **状态**：方案（已拍板走 **远程客户端**；**P3 已落地** `repl` 斜杠 `/help` `/workspace` `/conv`）  
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
| 子命令（终态） | `connect`（探测）· `chat`（单轮/管道）· `repl`（交互行编辑）· `tui`（全屏，后置） |
| 契约 | git tag 钉 `crabmate-sse-protocol` / `crabmate-api-contract` 等（同 `frontend`） |
| HTTP 客户端 | `reqwest` + rustls；SSE 解析对齐协议 crate |
| 连接配置 | 复用/对齐 connect 键：`api_base`、Web Bearer（钥匙串账户可与壳同源或 `tui_*` 前缀，实现时定一） |
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
| 断线续传 | TUI SSE mirror | `Last-Event-ID` + `stream_resume` |
| 审批 | 进程内 dialoguer | SSE 控制面 + `POST /chat/approval` |
| 斜杠 /skills | 同进程 | 经 stream 内置命令或后续 REST |
| 工作区 | 本地 path | `POST /workspace`、`GET /workspace/...` |
| 会话列表/分支 | SQLite 同库 | `/user-data/.../sessions`、`POST /chat/branch` |
| 模型密钥 | 本机 keyring → 回合注入 | 本机 keyring → 请求体 `client_llm.api_key`（同 WASM UI） |
| GitHub token | 现已请求作用域 | 头 `X-CrabMate-GitHub-Token` |

---

## 5. 分期

| 阶段 | 交付 | 验收 |
|------|------|------|
| **P0** | 本文档；命名与边界共识 | 与 AGENTS / 路径 A 无冲突 |
| **P1** | `crabmate-tui`：`chat` 单轮 + Bearer + `/chat/stream` 文本输出 | 对本地 `serve` 跑通一轮；CI 可选 smoke |
| **P2** | 交互 `repl`（reedline 或最小行编辑）+ 审批 TTY + 会话 id 续聊 | 非白名单命令可审批；Ctrl+C 干净退出 |
| **P3** | 工作区/会话斜杠子集与 WASM 设置对齐的常用操作 | `/workspace`、列会话等 |
| **P4** | 实验性 `tui`（ratatui）：流式中区 + 底栏输入；布局可简化 | TTY 全屏可用；不要求三栏齐全 |
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
| 下一步 | 实现 **P4**（实验性全屏 `tui`）或发版向 **P5** |

---

## 8. 非本仓事项（Server）

- 文档：官方矩阵增加「Terminal remote client」；CLI/TUI 同进程改为「过渡」。
- 日后：`crabmate tui|repl` help 指向 `crabmate-tui`；再定删除时间表。
