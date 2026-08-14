# ADR-0002: Android 审批通知 + 前台保活

## Status

Accepted

## Context

Android 官方壳是薄 WebView：业务 UI 在包内 WASM，`/chat/stream` SSE 与 `POST /chat/approval` 都在 Chromium 里完成。工具审批（SSE `command_approval` → `pending_approval` 弹窗）**阻塞远程 `serve` 的当前轮**，直到用户 `deny` / `allow_once` / `allow_always`。

**现状（代码事实）**：

- 审批只存在于前台 UI：[`approval_modal.rs`](../../frontend/src/app/approval_modal.rs) 在 `on_approval` 时写入 `pending_approval`；提交走 [`submit_chat_approval`](../../frontend/src/api/http.rs)（`POST /chat/approval`）。无系统通知、无原生 HTTP 客户端。
- 切后台后的唯一恢复路径是 [`wire_stream_visibility_resume`](../../frontend/src/app/chat/stream_visibility_resume.rs)：`visibilitychange` hidden 时尽量 flush 会话；visible 时按 [`foreground_stream_action_after_hidden`](../../frontend/src/app/chat/foreground_stream_action.rs) 决定 `None` / `Resume` / `Hydrate`。`AbortController` 仍在则视为流活着，**不**强杀重挂。
- 该路径是「死了再救」。Android 在 Home / 锁屏 / 厂商杀进程后，WebView 常被 `pauseTimers` 或进程被回收，SSE 中断；用户不知道正在等审批，回 App 才 hydrate/resume，工具调用可能已超时或卡死。
- [`ADR-0001`](./0001-stream-session-switch-background-streaming.md) 解决的是**应用内**切会话时保留 Bound 重连句柄，不覆盖 **OS 把 Activity 送入后台**。

**约束**：

- 路径 A 薄壳：不 spawn `serve`、不在本仓改 Server SSE/审批契约、不做 FCM（需 Server 推送面）。
- 单流模型不变：同一时刻至多一个 `/chat/stream` attach；禁止原生再挂一条同 URL 的 SSE（会抢走或打乱现有 attach）。
- `targetSdk 36`：前台服务须声明类型；Android 12+ 禁止在已进入后台后启动 FGS；Android 13+ 通知需运行时 `POST_NOTIFICATIONS`；Android 15 `dataSync` FGS 约 6 小时上限。
- 安全：Web Bearer / 模型 key 仍只在 Keystore；通知与 Intent 不得携带密钥；审批决策仍须用户看见命令（现有弹窗）。

**要回答的问题**：在不改 Server、不复制 SSE 的前提下，如何降低「后台丢流 / 审批无人知」？

## Decision

采用「**发送时拉起 `dataSync` 前台服务 + SSE 仍由 WebView 消费 + 审批到达时升级系统通知**」。点通知只把已有 `MainActivity`（`singleTask`）拉回前台，用户在现有审批弹窗里 `POST /chat/approval`。

保留 [`visibilitychange` 软续传](../../frontend/src/app/chat/stream_visibility_resume.rs) 作为保活失败时的退路，**不删除**。

### 范围（v1 必做）

1. **启动时机**：在 WASM `send_chat_stream` attach 开始时（含软续传 [`foreground_resume.rs`](../../frontend/src/app/chat/composer_stream/foreground_resume.rs)）经 `CrabMateMobile.startStreamKeepAlive()` 启动 FGS。必须在 Activity 仍前台时启动，禁止挂在 `visibilitychange` hidden 上。
2. **停止时机**：流结束 / `on_error` / 用户停止 / 断开回连接页 / 清 Bearer / 返回键确认退出应用 → `stopStreamKeepAlive()`。断开路径必须停服务，避免无凭证后仍挂「对话进行中」。**不要**在 `Activity.onDestroy` 里停（划掉 Recents 会走 onDestroy，与 `stopWithTask=false` 冲突）。
3. **WebView 定时器**：保活期间 `MainActivity` **不得** `WebView.pauseTimers()`；若 Tauri 基类在 `onPause` 暂停了，保活期间 `resumeTimers()`。不在 pause 时 abort 流或离开业务页。
4. **通知**：渠道 `crabmate.stream`。流进行中为 ongoing 常驻条；`on_approval` 调用 `notifyApproval(sessionId, command, args)`，同一 `notificationId` 升级为 heads-up（命令+args 截断，例如 80 字）。点按 `PendingIntent` 打开 `MainActivity`，不把 `approval_session_id` 当原生 POST 参数。用户划掉审批通知 ≠ deny。
5. **权限**：Manifest 声明 `POST_NOTIFICATIONS`、`FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_DATA_SYNC`；Service `foregroundServiceType="dataSync"`。首次发送弹出系统权限框期间**不**写状态栏错误（桥返回 `prompting`）；仅用户**明确拒绝**后写提示（`need_permission` 或权限回调）；授权后清除本条提示。`startForegroundService` 失败须捕获，不得让未处理异常打崩进程。
6. **桥与 Origin**：扩现有 [`CrabMateMobile`](../../mobile-tauri/src-tauri/gen/android/app/src/main/java/edu/crabmate/MainActivity.kt)（`startStreamKeepAlive` / `stopStreamKeepAlive` / `notifyApproval` / `clearApprovalNotification`）；仅包内 App Origin，URL 判定与 Keystore 桥相同。前端经 [`mobile_remote.rs`](../../frontend/src/mobile_remote.rs) 探测，桌面/浏览器无桥则 no-op。
7. **手写 Kotlin 边界**：新 `StreamKeepAliveService` 放在 `edu.crabmate`，与 `SecureBearerStore` 一样不进 Tauri `generated/`；ProGuard keep Service + 新桥方法。
8. **停止与生命周期**：流结束 / 停止 / 断开回连接页 / 返回键确认**退出应用**时停 FGS。`Activity.onDestroy`（含从最近任务划掉）**不停**服务，以便 `stopWithTask=false` + 点通知重建 Activity 后仍能 `resumeTimers`。

### 明确不做（v1）

- 通知操作按钮直接 `POST /chat/approval`（原生再实现一套鉴权/幂等/清 UI；「允许始终」尤其危险）。
- 原生 OkHttp/Rust 再消费 `/chat/stream` 或发明 job 状态轮询（后者属 Server 契约，权威在主仓）。
- FCM / 改 CORS / 改审批 API。
- 用 FGS 替代 `visibilitychange` 续传。

### 后续可选（非本 ADR 验收）

通知栏「允许一次 / 拒绝」：API base 连接时交给原生、Keystore 读 Bearer、body 用 `crabmate-client-api::ApprovalPostBody`，成功后 `evaluateJavascript` 清 `pending_approval`。须另开 ADR 或修订本文件。不提供「允许始终」通知按钮。

## Consequences

**正面**：

- 发送后切走时进程更不易被杀；保活成功时 `abort` 槽仍在，回前台走 `ForegroundStreamAction::None`，少一次 `stream_resume`。
- 审批到达且 JS 仍能跑时，用户能从状态栏回来处理，远程 `serve` 少卡在审批闸门上。
- 不改 Server、不双开 SSE、桌面行为不变。

**代价 / 约束**：

- 用户必须授予通知权限，否则 FGS/heads-up 在 Android 13+ 不可用。
- 厂商 ROM 仍可能杀 FGS；「忽略电池优化」只作设置深链/文档，不保证成功。
- `dataSync` 约 6 小时上限；超长 Agent 轮次可能被系统停服务，届时退回 visibility resume。
- FGS 保的是进程，**不保证** OEM 冻结 WebView 时 JS 仍处理 SSE。若 JS 冻结，审批通知会延迟到回前台事件刷出；此时保活仍可能减少必须 resume 的概率。
- 后续实现不得在 hidden 后启动 FGS，不得在通知/Intent 中放入 Bearer 或完整未截断密钥型 args。
- 实现后才是用户可见行为：落地时须同步 `README.md` + `README.zh-CN.md`、`mobile-tauri/README.md`、`docs/design/shell_smoke_runbook.md`、`CHANGELOG.md`（本 ADR 落地前只记设计条目）。

## Alternatives Considered

| Option | Pros | Cons |
|--------|------|------|
| **A. 仅保留 visibility resume（现状）** | 无新权限、无 FGS 政策 | 后台丢流/审批无人知；手机主路径不可用 |
| **B（本决策）. Attach 时 FGS + 通知升级 + 点按回弹窗** | 不改契约；复用现有审批 POST；与 0001 续传互补 | 通知权限；OEM 杀进程；WebView 仍可能被冻 |
| **C. 原生再挂 `/chat/stream`** | JS 冻住也能看到事件 | 双消费者破坏单流；重复鉴权/序号；违背薄壳 |
| **D. FCM / Server 推审批** | 进程被杀仍能提醒 | 须改 Server 与推送面；超出本仓；个人云运维重 |
| **E. v1 就在通知里 POST 审批** | 少一次点进 App | 双提交与 UI 不同步风险；通知栏不够审命令；「允许始终」不可接受 |
| **F. WorkManager / 后台 Job 轮询 job 状态** | 看起来更「后台友好」 | 无此 Server API；有了也不是 SSE 审批闸门的替代 |

## 实施计划

### 阶段 A — 前台保活

- 新增 `StreamKeepAliveService`（`startForeground` + `FOREGROUND_SERVICE_TYPE_DATA_SYNC`）；Manifest 权限与 `android:foregroundServiceType="dataSync"`。
- `MobileBridge.startStreamKeepAlive` / `stopStreamKeepAlive`；`onTaskRemoved` 在流未结束时不停服务。
- `MainActivity`：保活期间禁止 `pauseTimers`。
- WASM：`send_chat_stream` 前 start；ended/error/abort/disconnect 时 stop。
- 验收：发送后 Home 30–60s 再回 App，流仍在且 `abort` 非空，不走整段 `stream_resume`。

### 阶段 B — 审批通知

- `on_approval`（[`callbacks/assemble.rs`](../../frontend/src/app/chat/composer_stream/callbacks/assemble.rs)）在 `replace_with_pending_approval` 之后 `notifyApproval`。
- 弹窗提交成功或 `pending_approval` 清空 → `clearApprovalNotification`。
- 验收：后台等到 `command_approval`，通知出现；点开后现有弹窗可允许/拒绝，SSE 继续。

### 阶段 C — 权限 UX

- 首次 attach 前请求 `POST_NOTIFICATIONS`；拒绝时状态栏文案（i18n zh/en）。
- ktlint 覆盖新手写 Kotlin。

### 阶段 D — 不在本 ADR v1

- 通知 action 原生 POST：另议。

### 回归

- `make check`（含 `ktlint-android`）。
- 真机冒烟补进 `docs/design/shell_smoke_runbook.md`（实现时）：发送 → Home → 审批通知 → 点回决策。Playwright 不能替代 Android FGS。

## 关联

- 互补：[ADR-0001](./0001-stream-session-switch-background-streaming.md)（应用内切会话保留 Bound；本决策管 OS 后台）
- 壳模型：[`docs/design/tauri_gui_mvp_design.md`](../design/tauri_gui_mvp_design.md)
- 主要落地（实现时）：`mobile-tauri/src-tauri/gen/android/app/src/main/java/edu/crabmate/MainActivity.kt`、新增 `StreamKeepAliveService.kt`、`AndroidManifest.xml`、`frontend/src/mobile_remote.rs`、`frontend/src/app/chat/composer_stream/mod.rs`、`foreground_resume.rs`、`callbacks/assemble.rs`、`frontend/src/app/approval_modal.rs`
