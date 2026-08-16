# 编程 Agent 客户端规划

> **状态**：Wave 1（对话 P2）已落地；Wave 2–3 未开工  
> **范围**：本仓 `frontend/`（对话、变更集、工作区/IDE 审查面）；官方壳只做连接与保活  
> **读者**：本仓贡献者；Wave 2 还原/结构化 changelog 需 Server 仓配合  
> **关联**：[`chat_ui_todo.md`](./chat_ui_todo.md)（Wave 1 勾选权威）、[`ui_issue_todo.md`](./ui_issue_todo.md)、[`tauri_gui_mvp_design.md`](./tauri_gui_mvp_design.md)、[`ADR-0003`](../adr/0003-chat-file-context-inject-cards.md)（从文件注入的上下文卡；非本规划 Wave 2）；路径 A：[client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)

本文是**产品与落地顺序**，不是 HTTP/SSE 契约。契约、错误码、changelog JSON、还原 API 的权威定义在 **Server** 仓；本仓只写 Client 需要什么、先做什么。

---

## 1. 目标与非目标

### 目标

把 Client 做成编程 Agent 的**审查与控制面**：提需求 → 看清改了什么 → 打开核对 → 接受或打回 → 再跑一轮。执行权威仍在已运行的 `crabmate serve`（路径 A 薄壳不变）。

对照开源头部：Cline 赢在 checkpoint + IDE diff；Aider 赢在 git 补丁循环。本仓两边都只有半截（有只读变更集，没有还原，也没有按文件打开）。

### 非目标

不要混进本规划的 PR：

- 完整 LSP / IntelliSense / Tab 幽灵文本（Continue 主场；薄壳税高）
- 调试器、内置终端复刻 VS Code
- 窄屏 / Android 上开放 IDE 布局（[`chat_ui_todo.md`](./chat_ui_todo.md) 已锁死；手机主路径是对话 + 审批 + 变更列表）
- KaTeX / Mermaid、Markdown 改整段虚拟 DOM（见 [`markdown_render_todo.md`](./markdown_render_todo.md)）
- 在 Tauri 里控制本机桌面或浏览器（若要做，权威在 Server 沙箱）
- Windows / macOS 官方壳（平台扩张，不提高编码闭环完整度）
- 在本仓发明并当作权威的还原/changelog HTTP 形状（须先在 Server 定契约再钉 pin）

---

## 2. 约束

1. **不** spawn / 内嵌 `serve`；**不** `path` 回 Server 源码树。
2. 工作区文件的真相在 Server 磁盘；Client 的 IDE / 文件树 / 变更集都是视图。
3. 官方桌面仅 Linux；Android 锁死 IDE 布局。Wave 2 的「打开文件」以宽屏 Desktop 为主；窄屏只保证变更**列表**可用。
4. 契约只经 git tag / rev 升级（见 [`contract_pin.md`](./contract_pin.md)）。Wave 2 还原未 pin 之前，Client **不得**假装有回滚按钮。
5. 小步 PR：一波里一项用户可见行为；不把 LSP、主题、设置文案塞进同一 PR。

---

## 3. 现状（代码事实）

| 面 | 现状 | 缺口 |
|----|------|------|
| 对话 | 流式、工具卡、审批、再生/分支、就地编辑、流式排队、查找气泡内高亮 | 无 |
| Ask / Plan / Act | composer 输入条旁 | Plan 产物仍不是一等公民（Wave 2.4） |
| 变更集 | `GET /workspace/changelog` → Markdown 弹窗（[`changelist_modal.rs`](../../frontend/src/app/changelist_modal.rs)） | 只读摘要；不能点文件进 IDE；不能还原 |
| 编辑器 | 宽屏轻量 CodeMirror；可写工作区文件 | 不是 LSP IDE；与变更集未打通 |
| Git | 克隆弹窗、GitHub Device Flow | 无 status / 暂存 / 提交 / 从审查面开 PR |
| 工具写盘 | `apply_patch` 等走 Server；UI 为工具卡 | 卡上无「打开此文件 / 看 hunk」 |

编码闭环完整度（作者判断，0–10；不是基准测试）：提需求 8、模式 8、看清改动 4、打开核对 3、打回还原 1、再跑一轮 7、提交 PR 3。

---

## 4. 落地顺序

勾选在实现 PR 里更新。Wave 1 的细项勾选以 [`chat_ui_todo.md`](./chat_ui_todo.md) **P2** 为准，此处只记依赖与验收口径。

### Wave 1 · 对话就是编码循环（仅 Client）

不依赖新 Server API。可与 [`ui_issue_todo.md`](./ui_issue_todo.md) P2 并行，但不要进同一 PR。

| # | 项 | 勾选权威 | 验收 |
|---|-----|----------|------|
| 1.1 | 就地编辑用户消息再发送（走现有 branch / regen） | `chat_ui_todo` P2 **已勾** | 改一句需求不必整段重打；历史分叉行为与现有菜单一致 |
| 1.2 | 流式进行中排队下一句（扩展 `ComposerStreamFollowUp`，不加第二套 Effect） | 同上 **已勾** | 工具/改文件进行中可再下一条；停流后按序发出；切走会话时排队正文写回该会话草稿 |
| 1.3 | Ask / Plan / Act 在 composer 附近可见 | 同上 **已勾** | 宽屏不必打开底栏才能切模式；窄屏不挤掉输入区 |
| 1.4 | 查找命中高亮落在气泡内 | 同上 **已勾** | wrap 描边 + `<mark>` |

**Wave 1 已在 `chat_ui_todo` P2 勾完。** 聊天代码高亮仍是 `chat_ui_todo` P3，**不算**本规划 Wave 1。

### Wave 2 · 审查与可逆（编程 Agent 分水岭）

没有「打回 / 还原」，就只是能改仓库的聊天。

| # | 项 | 落点 | 依赖 |
|---|-----|------|------|
| 2.1 | 变更集：文件列表；点路径在 IDE 打开（宽屏） | Client；changelog **最好**改为结构化 JSON | 过渡期可解析现有 Markdown 中的路径，但不得把解析启发式写成契约 |
| 2.2 | 写盘工具卡：「打开此文件」/ 看 hunk | Client（路径已在工具参数里） | 无新 API 也可做最小「打开」 |
| 2.3 | 会话工作区还原（整次会话；可选按文件打回） | **必须先有 Server API** | 见 §5；未 pin 之前 Client 只做设计，不上按钮 |
| 2.4 | Plan 产物一等公民：展示计划，用户确认后再 Act | Client 展示；Plan 语义在 Server | 不新造 SSE 事件类型，除非 Server 已有 |

Android / 窄屏：2.1 至少是可滚动文件列表 + 摘要；**不要**为了 2.1 解锁 IDE 布局。

**建议 PR 切分**：2.2（可先于契约）→ 2.1（有 JSON 更好）→ 2.3（blocked on Server）→ 2.4。

### Wave 3 · 接到 git（后做）

建立在 Wave 2 结构化变更之上，不要先做一套 git GUI。

| # | 项 | 说明 |
|---|-----|------|
| 3.1 | 侧栏 git status（脏文件与变更集对齐或明确分工） | 权威仍在 Server 工作区 |
| 3.2 | 暂存 + 提交说明 | 提交动作是 Server 工具或专用 API，Client 只收集说明与确认 |
| 3.3 | 用已有 GitHub Device Flow 开 PR | 不新做 OAuth 网页 |

---

## 5. 本仓不写死的 Server 工作

下列内容**不得**在本文件写成最终路径/字段名。Server 定稿并打 `client-contract-v*` 后，本仓只链接并升级 pin。

Client **需要**的能力（验收语言，不是 schema）：

1. **结构化 changelog**：按会话列出相对路径、变更类型（增/改/删）、可选 unified diff 或可请求单文件 diff；带 revision，便于 UI 防过期提交。
2. **还原**：把某次会话（或某 revision）的工作区改动撤回到更早状态；失败要可展示的错误（冲突、未跟踪文件、非 git 工作区）。
3. **可选按文件打回**：与整次还原分开；没有按文件 API 时，Wave 2.3 可以先做「还原全部」+ 确认框。

开放决策（归 Server ADR，不在本仓拍板）：

- 还原实现：工作区快照 vs `git` reset/checkout vs 补丁反转
- changelog 是否与注入模型的 `session_workspace_changelist` 共用同一份结构化源
- 还原是否要求工作区必须是 git repo

---

## 6. 明确不做的替代方案

| 方案 | 为何不做 |
|------|----------|
| 先补齐 LSP 再谈审查 | 不解决「不敢让 Agent 大改」；体积与多语言服务器超出薄壳 |
| Client 本地做 checkpoint | 文件真相在 Server；本地快照会与远程工作区分叉 |
| 把 changelog Markdown 永远当唯一审查面 | 无法点文件、无法按文件操作、无法做可靠还原确认 |
| Android 上开放 IDE 以「对齐 Desktop」 | 与已锁定的非目标冲突；手机审查靠列表 |

---

## 7. 验证

- Wave 1：跟随 [`chat_ui_todo.md`](./chat_ui_todo.md)「验证」；手测就地编辑、排队、composer 旁模式。
- Wave 2.1–2.2：宽屏点变更/工具卡打开对应 tab；窄屏列表不打开 IDE。Playwright 能 mock changelog / 工具卡则补一条；不能替代 Desktop 手测。
- Wave 2.3：契约 pin 升级后的 mock + 一次真实 `serve` 还原手测（步骤进 [`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）。
- Wave 3：手测提交不把密钥写进 commit message UI 日志。

落地用户可见行为时：同步 `README.md` + `README.zh-CN.md`（若影响启动/能力描述）、`CHANGELOG.md` `[Unreleased]`、必要时 `shell_smoke_runbook.md`。

---

## 8. 文档维护

- 实现勾选：Wave 1 → [`chat_ui_todo.md`](./chat_ui_todo.md)（P2 已勾完）；Wave 2–3 → 本文表格。
- 状态行：Wave 1 已落地。还原 API 落地后把 §5 改成指向 Server 文档的链接，删掉猜测性字段。
- 还原机制一旦在 Server 选定，本仓**不**另写一份对立 ADR；只记 Client 按钮行为与失败展示。
