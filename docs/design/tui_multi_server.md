# TUI 多 serve 连接与多 agent 并行（多 lane）

> **状态**：**方案（未开工）**，tracking Issue [#327](https://github.com/noisystreet/crabmate-client/issues/327)。本文只写设计与分期，不含实现。
> **范围**：仅 `crates/crabmate-tui` 的全屏 `tui` 模式。**不改** Server 契约（SSE / 审批 / 错误码 / 工作区语义），**不覆盖** Desktop / Android / Web。
> **关联**：[`remote_cli_tui.md`](./remote_cli_tui.md)（TUI 母方案）、[`tui_settings_panel.md`](./tui_settings_panel.md)（设置面板，user-data 语义）、[`tui_usability.md`](./tui_usability.md)、[`client_shared_logic.md`](./client_shared_logic.md)（共享层边界）、[`contract_pin.md`](./contract_pin.md)（契约钉点）；Server `docs/design/user_data_dir.md`（user-data 权威）

---

## 1. 背景与现状

`crabmate-tui` 是**单连接 + 单状态 + 单 worker 串行**的结构。要同时驱动多个 agent，当前只能开多个终端窗口。

| # | 耦合点 | 位置 | 后果 |
|---|--------|------|------|
| 1 | CLI 只有一组全局参数（`--api-base` / `--bearer` / `--llm-*`，均 `global = true` 单值） | `src/main.rs:32-93` | 一次只能连一个 serve |
| 2 | `run_tui` 只收一个 `&ServeClient` | `src/main.rs:276` | 入口层无法表达多连接 |
| 3 | `TuiApp` 持单 `client` / `st` / `persisted` / `index` / `resume` / `sent_turn` / `cancel` | `src/tui_mode/mod.rs:86-120` | 状态无归属维度 |
| 4 | 会话域状态全在 `UiState`（`conversation_id` / `sessions` / 工作区树 / `running` / `cancel_sent`） | `src/tui_mode/state.rs:258-376` | 一份状态只能服务一条会话流 |
| 5 | `UiEvent` 各变体**不带来源标识**，`on_msg` 直接写 `self.st` | `src/tui_mode/worker.rs:32-162`、`mod.rs:620` | 无法区分事件属于哪一路 |
| 6 | 单个 worker 线程 + `current_thread` runtime `block_on` 串行消费 | `src/tui_mode/worker.rs:369-414` | **连同一 serve 的两轮都不能重叠**；回合期间刷新任务全部排队 |
| 7 | 单取消 `cancel: Option<StreamCancel>` + 单布尔 `st.running` | `mod.rs:101`、`state.rs:274` | 只能取消一路 |
| 8 | 单审批浮层 `approval: Option<ApprovalOverlay>` + 全局 `OverlayApprovalGate` | `state.rs:343`、`approve.rs:34-68` | 多路同时请求审批会互相覆盖 |

好消息：`ServeClient` 已是 `Clone` 且只含 `reqwest::Client` + `ConnectionConfig`（`src/serve/client.rs:40-44`），多实例天然成立，连接层无需改动。

---

## 2. 目标与非目标

### 2.1 目标

1. 一个 `crabmate-tui` 进程内并行运行多路 agent 会话（设计目标 **8 路**），每路可指向**不同** serve 实例。
2. 每路独立 transcript / 会话绑定 / 审批 / 取消 / 续流，**互不串台**。
3. 界面内可切换与总览各路状态（运行中 / 待审批 / 出错）。
4. 连接配置可持久化（多 profile），**密钥仍只进本机钥匙串**。
5. 单连接（1 路）行为保持与现状逐字等价。

### 2.2 非目标

- **不做**同一 workspace 内的多 agent 并行——物理工作树、git index、`conversations.db` 写冲突不在本文解决范围（见 §5.3）。
- **不做** Desktop / Android / Web 的多连接。
- **不改** Server 契约。
- **不做**跨 lane 编排（一个 agent 读另一个的输出、依赖调度等），仅并列运行。
- **不做** lane 级独立主题 / 独立字体等外观差异化。

---

## 3. 事实基础（Server 侧已核实）

以下结论来自对 Server 仓（`../server`）源码的只读核对，**是本文的设计前提**，不是契约变更。

| # | 事实 | 出处 |
|---|------|------|
| 1 | user-data 根可按实例隔离：`user_data_root()` 优先读 `CM_CRABMATE_USER_DATA_DIR`，否则 XDG `~/.local/share/crabmate` | `src/cm_internal/user_data/path.rs:7-8`、`docs/design/user_data_dir.md` §3 |
| 2 | 会话索引按 workspace 分片：`workspaces/<ws_sha256>/web_sessions.json`；对话正文在 `<workspace>/.crabmate/conversations.db` | `docs/design/user_data_dir.md` §3 |
| 3 | 全局共享（无 workspace 维度）：`prefs.json` / `llm_overrides.json` / `mcp_servers.json`；Server 注释明示共享同一 data 根时「**不可并行写**」 | `src/cm_internal/user_data/store.rs:449,462` |
| 4 | `allow_always` 的 allowlist 是**每个 stream job 新建的内存 `HashSet`**，不落盘 | `src/chat_job_queue/worker/stream_job_setup.rs:162`、`src/cm_tools/tool_runtime.rs:20` |
| 5 | 并发上限是**进程级单例队列**，默认 `chat_queue_max_concurrent = 2` | `src/web/app_state.rs:145`、`src/cm_config/finalize_parts/finalize_tail.rs:272`、`config/default_config.toml:69` |
| 6 | 全仓**无 per-conversation 互斥**，同一 `conversation_id` 并发两轮没有保护 | 全仓无 `conversation_lock` / `per_conversation` 类串行化，队列仅按 job 全局限流 |

由此推出两条关键设计约束：

- **事实 4** ⇒ 不存在「另一个 client 按了『始终允许』，我这边被静默放行」的跨端安全问题；也意味着「始终允许」只在本回合内有效（既有行为，非本方案引入）。
- **事实 5 + 6** ⇒ **并行的自然单位是「不同 workspace 的不同 serve 进程」**，而不是「同一 serve 的多个会话」。§5 给出三种拓扑的取舍。

---

## 4. 方案

### 4.1 Lane 化（第一步，无新功能的解耦）

引入 `LaneId` 与 `Lane`，把「属于一条连接/一条会话流」的状态从 `TuiApp` 与 `UiState` 中挪出来：

```rust
pub(crate) struct LaneId(u32);   // 稳定、单调递增；不回收，避免 ABA

pub(crate) struct Lane {
    pub id: LaneId,
    pub profile: Arc<ServerProfile>,   // 见 §4.5
    pub client: ServeClient,
    pub st: UiState,                   // transcript / 输入 / 会话 / 工作区树
    pub persisted: Option<PersistedSettings>,
    pub body: body_cache::BodyCache,   // transcript 物理行缓存（逐 lane 一份）
    pub index: session_index::UiIndexState,
    pub resume: Option<resume::ResumePoint>,
    pub sent_turn: Option<resume::TurnEcho>,
    pub cancel: Option<StreamCancel>,
    pub job_tx: Sender<WorkerJob>,
    pub drafts: drafts::Drafts,        // 可留在 TuiApp 按 lane 索引，见 §4.2 注
}

pub(crate) struct TuiApp<'a> {
    lanes: Vec<Lane>,
    active: LaneId,
    overrides: &'a mut SessionPrefs,   // 进程级：只作用于「当前 lane 的下一轮」
    // 其余进程级字段：spinner / last_size / exit / quit_pending / panel / bg
}
```

要点：

- **1 lane 行为不变**：W1 只做搬迁，不新增交互，既有测试全部保留（这是可回归的前提）。
- **`overrides` 留在 `TuiApp`**：它对应 CLI/env 与本进程 `/model` 等斜杠覆盖，语义是「本进程当前表达」，切换 lane 时不搬走；若后续需要逐 lane 覆盖，再下沉进 `Lane`（列为 §8 未决项）。
- **`drafts` 逐 lane**：会话级输入草稿本来就跟 `conversation_id` 走，跨 lane 复用会导致「A 的草稿出现在 B」——必须逐 lane（可由 `TuiApp` 以 `HashMap<LaneId, Drafts>` 持有，也可放进 `Lane`，二选一看实现期哪个更少改动）。

### 4.2 事件路由

```rust
pub(crate) enum UiEvent {
    // 全局事件：不归属任何 lane
    Key(KeyEvent), Paste(String), Mouse(MouseEvent), Resize,
    // lane 事件：一律带来源
    Lane { id: LaneId, ev: LaneEvent },
}

pub(crate) enum LaneEvent {
    Text(String), Thinking(String), System(String),
    ToolStart { .. }, ToolEnd { .. },
    TurnDone { .. }, Approval { .. }, ApprovalExpired { .. },
    Sessions(..), PromptTokens(..), ContextTrim(u32), UsageHydrated { .. },
    ConversationHistory { .. }, SessionIndex { .. }, Status { .. },
    PermissionWritten { .. }, Workspace { .. }, WorkspaceDir { .. },
    WsProjects(..), WsProjectSwitch(..), Skills(..),
}
```

- **单通道 + 带 tag**（推荐）：`UiEvent::Lane { id, ev }`，`on_msg` 按 `id` 找 lane 后分派。改动集中在 `worker.rs` 的 `UiSink`（多一个 `id` 字段）与 `mod.rs` 的 `on_msg` 分派表。
- **每 lane 通道 + 统一转发**：隔离更强，但 `run_loop` 的 `recv_timeout` / drain 逻辑要改成 select，且会改变既有节流语义（`event_loop.rs:186-219`）——不推荐作为第一版。
- **跨 lane 的「代数」门控必须留在 lane 内**：`ws_generation`、`permission_generation`、`UsageHydrated` / `Status` 的 `conversation_id` 校验，作用域都是「这一条会话流」，**不得**提升为全局代数（否则 A 的切会话会误丢 B 的回包）。

### 4.3 Worker 模型

每 lane 一个持久 worker 线程（各自 `current_thread` runtime），保留现有假设——`worker.rs:366-367` 注释说明「避免同一 `reqwest::Client` 跨多个临时 runtime 复用的连接池问题」：

```rust
fn spawn_lane_worker(id: LaneId, client: ServeClient, tx: Sender<UiEvent>) -> Sender<WorkerJob>
// 与现状 spawn_worker 的差别：
//   1) 所有 UiEvent 出站都包 Lane { id, .. }
//   2) job_rx drop（lane 关闭）后线程自行退出
```

约束：

- lane 数上限（建议 `MAX_LANES = 8`，与 §5 目标一致），启动时校验、运行期拒绝新增。
- 每 lane 一条 SSE 流；**同一 lane 内仍是串行**（一次只跑一轮），这与现有 `st.running` 语义一致。
- 退出路径：`flush_before_exit`（`mod.rs:910-912`）与 `exit_flush` 需要**逐 lane 遍历**，且要保留「有 lane 仍在跑 ⇒ 强制退出不等落定」的既有判定。

### 4.4 审批队列

现状单浮层在多 lane 下会串台：用户以为在回答 A，实际 `answer` 通道属于 B。

- `approval` 由 `Option<ApprovalOverlay>` 改为 `VecDeque<ApprovalRequest>`，元素带 `LaneId`。
- 浮层一次只问一个，标题显示 `lane 名 + 排队数`（如 `proj-a · 还有 2 个待审批`）。
- 等待上限沿用 `APPROVAL_WAIT = 600s`（`approve.rs:28`），但**排队时间要从等待上限里扣**：改为「入队时刻 + 上限」判定，超时按 `Deny` 收尾并向对应 lane 发 `ApprovalExpired`（现有语义见 `approve.rs:53-63`）。
- **默认拒绝方向不变**：超时 / UI 断开一律 `Deny`，多 lane 不改变这条保守策略。
- 非活动 lane 的审批请求**必须**仍然入队（否则用户切到该 lane 才发现回合卡住）。

### 4.5 连接 profiles

新增 `~/.config/crabmate/tui-connections.toml`（路径跟随 XDG，允许 `CM_TUI_CONNECTIONS` 覆盖）：

```toml
# 非机密。密钥一律走钥匙串 / env，禁止写在本文件。
[[server]]
name = "proj-a"
api_base = "http://127.0.0.1:8081"
model = "deepseek-chat"          # 可选：显式钉住，避免受共享 user-data 漂移影响
llm_api_base = "https://api.deepseek.com/v1"

[[server]]
name = "proj-b"
api_base = "http://127.0.0.1:8082"
```

- **凭据**：沿用既有 `--bearer` / `CM_WEB_API_BEARER_TOKEN` 与钥匙串槽 `WEB_API_BEARER_KEYRING_ACCOUNT`（`crates/crabmate-client-api/src/secrets.rs:4-8`）。多实例通常共用同一个 bearer，**因此第一版不扩展槽名**，避免牵动 Desktop / Android 的跨端槽名约定；只有实现期出现「多 bearer 真实需求」才另开 Issue 讨论 per-profile 槽。
- **无配置文件时**：回落现状（单一 `--api-base` 即 1 个 profile）。CLI 增加 `--profile <name>` 选择启动时激活哪个，`/lane new <name>` 在运行期追加。
- **显式 `model` / `llm_api_base` 的价值**：`prefs.json` / `llm_overrides.json` 在多个 serve 实例共享同一 data 根时仍会互相覆盖（事实 3），profile 里钉死这两个字段可把「执行参数漂移」从并行路径上摘掉。

### 4.6 UI 与交互

- 左栏顶部增加 lane 区（或复用现有左栏会话列表上方的标题行区域），每项：`名称 + 状态点`（空闲 / 运行中 / 待审批 / 出错）。
- 切换：`Ctrl+N` 下一个 lane（或 `Alt+1..8` 直达），切换只改 `active`，**不打断**任何在途回合。
- 非活动 lane 的流**照常全量累积**（transcript 数据都在内存里），只是不参与绘制；重绘只画 `active`。
- 状态栏显示 `lane N/M` 与全局待审批计数，避免用户忘记还有别的 lane 在等审批。

---

## 5. 并行拓扑

### 5.1 为什么默认拓扑是「N 个 serve」

`chat_queue_max_concurrent` 是**进程级**单例（事实 5），而 workspace 也是进程级单值。因此「每个 workspace 一条并行通道」在架构上就等于「每 workspace 一个 serve 进程」。

### 5.2 三种达到 8 路的配置

| 拓扑 | 配置 | 并行度 | 隔离 | 代价 |
|------|------|--------|------|------|
| **8 × serve（推荐）** | 8 进程，默认值不动 | 8 | 完整（workspace + data 根） | 进程数 × 8，`fastembed` 等资源 × 8 |
| 4 × serve | 每进程 `CM_CHAT_QUEUE_MAX_CONCURRENT=2` | 8 | 按 workspace 分组 | 中 |
| 1 × serve | `CM_CHAT_QUEUE_MAX_CONCURRENT=8` | 8 | **无**（同 workspace 并发） | 最小配置，物理冲突自担 |

可调参数（已核实）：

| 配置项 | 环境变量 | 默认 | 范围 | 出处 |
|--------|----------|------|------|------|
| 并发回合上限 | `CM_CHAT_QUEUE_MAX_CONCURRENT` | 2 | 1–256 | `finalize_tail.rs:272` |
| 排队上限 | `CM_CHAT_QUEUE_MAX_PENDING` | 32 | 1–8192 | `finalize_tail.rs:273` |
| 单轮只读工具并行 | `CM_PARALLEL_READONLY_TOOLS_MAX` | `max(concurrent, 3)` | 1–256 | `finalize_tail.rs:274-278` |

注意联动：把 `concurrent` 调到 8 会让**只读工具并行默认变成 8**（同一条推导规则），单轮内 8 个文件读取并发打同一块盘；单进程多路时应显式设小。

### 5.3 8 路真正先撞的墙（不在配置）

1. **模型 API 的 RPM / TPM 配额**——通常最先出现 429，实际并行度要按配额定。
2. **内存**：每路持有完整上下文；每个 serve 进程还各自加载 `cache/fastembed/`，8 进程就是 8 份，这是 8 × serve 最实在的成本。
3. **`conversations.db` 写冲突**（仅同 workspace 多路）：`Connection::open(path)`（`../server/src/conversation_store.rs:142`），全仓无 `journal_mode` / `busy_timeout` 的 PRAGMA，**可能**出现 `database is locked`（未实测，见 §8）。
4. **工具执行的实际资源**：并行编译 / 测试 / 大文件读写会互相拖慢，与协议无关。

---

## 6. 分期

| 波次 | 交付 | 验收 | PR 边界 |
|------|------|------|---------|
| **W1** | `Lane` / `LaneId` 抽取；`UiEvent::Lane { id, ev }`；`TuiApp { lanes, active }`（恒为 1 lane） | 现有 `tui` 行为与交互逐字不变；`make test` 全绿；`mod.rs` 行数**下降**（当前 918 行，见风险 1） | 纯重构，不加功能 |
| **W2** | 每 lane 独立 worker + 独立 `cancel` + 切换 lane（配置文件可只有 1 项） | 两路连同一 serve 的不同 conversation 可同时流式；切换不打断在途回合 | 含 §4.3 |
| **W3** | 多 profile 加载（`tui-connections.toml`）+ 启动时逐 profile `/health` 探测 + 失败提示不阻塞其它 lane | 3 路连 3 个不同 serve，各自独立 transcript；某台不可达时其余仍可用 | 含 §4.5 |
| **W4** | 审批队列（带 lane 标签 + 排队数 + 入队时刻计超时） | 两路同时要求审批，逐个弹出且来源标注正确；非活动 lane 超时按拒绝收尾并留痕 | 含 §4.4 |
| **W5** | lane 列表 UI + 状态点 + `lanes N/M` + 全局待审批计数 | 8 路拓扑压测可用；`/help` 与快捷键同步 | 含 §4.6 |

各波次遵循仓库既有小步纪律：**一项用户可见行为一个 PR**，每波更新 `docs/design/client_capability_matrix.md` 的 TUI 行与 `remote_cli_tui.md` 的状态行。

---

## 7. 风险

| # | 风险 | 缓解 |
|---|------|------|
| 1 | `src/tui_mode/mod.rs` 已 **918 行**，紧贴 `fn-nloc-ratchet` 的 920 行/文件上限 | W1 必须先拆模块（`TuiApp` 字段与 `on_msg` 分派各自成文件），否则首次提交即红 |
| 2 | 审批串台（最高用户可见风险） | §4.4：带 lane 标签 + 队列 + 浮层显示来源；超时仍 `Deny` |
| 3 | 事件路由改造波及面大（`UiEvent` 有 20+ 变体） | 先 `Lane { id, ev }` 包一层，不逐个改语义；`test_support` 里的构造点同步更新 |
| 4 | 跨 lane 代数门控被误提升为全局 | 明确写入 §4.2：门控留在 lane 内 |
| 5 | 多进程资源放大（`fastembed` 等） | 文档写清拓扑建议；不强行单进程 8 路 |
| 6 | 共享 data 根时 `prefs` / `llm_overrides` 互相覆盖 | profile 里显式钉 `model` / `llm_api_base`；文档约定「并行期间不保存设置面板」 |
| 7 | 退出落盘语义在强退 + 多 lane 下变复杂 | 保留「仍在跑 ⇒ 不等落定」，逐 lane 遍历，与现状同向 |

---

## 8. 未核对项 / 待决

| 项 | 状态 |
|----|------|
| 同一 `conversation_id` 并发两轮的期望语义（排队 / 拒绝 / 未定义） | 已确认**无互斥**；语义需 Server 侧确认或实测。本方案不依赖它（每 lane 独立会话） |
| 工作区切换在途回合期间的行为 | Client 侧 `parse_workspace_set_ok_body` 支持 `error: "busy"`，但 Server 侧 grep 不到该字面量（疑为旧版兼容）；**不依赖**该保护 |
| 同 workspace 8 路并发写 `conversations.db` 是否触发 `database is locked` | **未实测**；本方案默认拓扑不触发（每 workspace 一库） |
| `SessionPrefs` 是否需要逐 lane（而非进程级） | W2 实现期按实际需要决定 |
| 是否需要 per-profile 钥匙串槽（多 bearer） | 第一版不做，需要时另开 Issue（牵动 Desktop / Android 槽名约定） |
