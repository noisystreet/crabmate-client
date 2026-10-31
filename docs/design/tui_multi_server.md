# TUI 多 serve 连接与多 agent 并行（多 lane）

> **状态**：**W1 / W1b / W2 已落地**（`LaneApp` 抽取 `cc0c06f`、事件来源标签 `b73ed83`；每 lane 独立 worker 与独立 `overrides`、`--connect <name>=<url>` 多 serve 装配、左栏 lane 列表与切换 ——`Alt+←/→` 或点行，**单 lane 也显示**；lane 行带该 serve 当前的**工作区末段**，`POST /workspace` 的 409 `WORKSPACE_ALREADY_SET` 单独识别并提示「另起 lane」—— 见 §4.7）。**W3 部分落地**：多连接靠 `--connect`；运行期**只读**查询控制面已落地（`--ctl-socket <path>` 或默认 `$XDG_RUNTIME_DIR/crabmate/ctl.sock`，`/ctl` 列出实例，**只有 `observed == ready` 算可连** —— 看 `state` 会把死实例当活的）。运行期**加连接**也已落地（§4.8）：`/lane attach <name>` 连声明里**已就绪**的实例，`/lane new <name> [workspace]` 经控制面 `add` 写声明 → 轮询到 `observed == ready` → 直接连上，两者都不用重启进程。仍未做：`tui-connections.toml` 配置文件、启动逐 profile `/health` 探测、`/lane close`（软摘一条声明要接 Server 的 `disable` / `enable`；硬 `remove` 会丢掉手写 `env` / `config`，不做）。**W4 / W5 仍有缺口**：审批浮层随 `UiState` 下沉到每条 lane（不串台），后台 lane 弹审批会在当前 lane 留一行提示、`F4` 直接跳过去、`/lane cancel` 可取消它那一轮；仍无跨 lane 审批队列与全局待审批计数。状态栏无 `lanes N/M`（lane 行的 `·●▲×` 状态点已落地）。`MAX_LANES = 8` 已落地：启动超限直接报错，运行期 `/lane attach|new` 超限只在发起命令那条 lane 提示。tracking Issue [#327](https://github.com/noisystreet/crabmate-client/issues/327)。
> **范围**：**§1–§8 仅覆盖 `crates/crabmate-tui` 的全屏 `tui` 模式**；**§9 是 Desktop 复用评估**（结论性分析，不含实施承诺）。**不改** Server 契约（SSE / 审批 / 错误码 / 工作区语义），**不覆盖** Android / Web。
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

实现方式（**W1b 已落地**）：不拆枚举、不改 70 余处分派分支，只加一个自引用包装变体。

```rust
pub(super) enum UiEvent {
    // …既有变体全部保留（构造点与 lane 内的 match 一行不改）
    /// 带来源 lane：SSE 流增量与回合结果由某条 lane 产生。
    Lane(LaneId, Box<UiEvent>),
}

pub(super) struct LaneId(pub(super) u32);
```

- **裸变体 = 全局输入**：键盘 / 鼠标 / 粘贴 / 尺寸从不带标签（`spawn_key_reader` 直接发裸 `UiEvent`），由外层路由给当前 lane。
- **只有 SSE 流带标签**：`UiSink { lane, tx }` 的 `emit()` 统一包一层，`spawn_worker(lane, …)` 在装配时注入。
- **解包只在一处**：`TuiApp::on_msg` 按 `LaneId` 找 lane 后调 `LaneApp::on_msg`；找不到（该 lane 已关闭）就丢弃，绝不把增量写进别人的 transcript。
- **为什么不拆 `LaneEvent`**：`UiEvent` 有 25 个变体、70 余处构造与匹配点，拆分会让每个 struct 变体多补一层括号，diff 全是噪声；包装变体达到同样效果，且能随 W2 增量演进。
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
- 退出路径（已落地，issue #338）：`/quit` 是**进程级**语义 —— `TuiApp::exit_requested()` 扫**全部** lane，任一条提出就立刻退出，不再等用户切过去（只判当前 lane 会让后台 lane 的请求悬着，切回时突然掀掉整个 TUI）。摘单条连接用 `/lane detach`。左栏用 `LaneStatus::Quitting`（`×`）显示「已请求退出」，因为从 `/quit` 到真正退出之间可能隔着整轮生成。
- 退出落盘：`TuiApp::flush_all_before_exit()` **逐 lane 遍历**（`exit_flush`：补绑定 + 写草稿 + `clear_background`），**一份总预算**（`NORMAL_BUDGET` 1s / `FORCE_BUDGET` 400ms，按「任一 lane 仍在跑」选择）但**按剩余 lane 均分**：某台 serve 不可达时最多吃掉 1/N，不会让后面的 lane 颗粒无收、把草稿又丢掉一次。只落当前 lane 会让其它 lane 未发送的输入丢失（`Drafts` 是内存态）。

### 4.4 审批队列

现状单浮层在多 lane 下会串台：用户以为在回答 A，实际 `answer` 通道属于 B。

- **（未实施的设想）** 把审批收拢成跨 lane 的 `VecDeque<ApprovalRequest>`（元素带 `LaneId`）、浮层一次只问一个并在标题显示 `lane 名 + 排队数`（如 `proj-a · 还有 2 个待审批`）。现行实现是**每条 lane 各自一个浮层**：跨 lane 收拢会让 `answer` 通道（属于某条 lane 的 gate）画到别人的屏上，那正是串台。真正缺的是「一眼看见 + 一键跳过去」，已由 lane 行的 `▲`、后台提示与 `F4` 补上。
- 等待上限沿用 `APPROVAL_WAIT = 600s`（`approve.rs:28`），超时按 `Deny` 收尾并向对应 lane 发 `ApprovalExpired`（现有语义见 `approve.rs:53-63`）。**不要**照旧稿改成「排队时间从上限里扣」：gate 在各自 lane 的 worker 线程上**同步阻塞**，一条 lane 同时只有一个待审批，不存在跨 lane 排队，等待本来就从请求发出时刻起算；将来真做全局队列时再按「入队时刻 + 上限」判定。
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
- **无配置文件时**：回落现状（单一 `--api-base` 即 1 个 profile）。`--profile` 尚未实现；运行期加连接的入口是 `/lane attach|new`（走控制面 socket，见 §4.8）。
- **显式 `model` / `llm_api_base` 的价值**：`prefs.json` / `llm_overrides.json` 在多个 serve 实例共享同一 data 根时仍会互相覆盖（事实 3），profile 里钉死这两个字段可把「执行参数漂移」从并行路径上摘掉。

### 4.6 UI 与交互

- 左栏顶部增加 lane 区（或复用现有左栏会话列表上方的标题行区域），每项：`名称 + 状态点`（空闲 / 运行中 / 待审批 / 出错）。
- 切换：`Ctrl+N` 下一个 lane（或 `Alt+1..8` 直达），切换只改 `active`，**不打断**任何在途回合。
- 非活动 lane 的流**照常全量累积**（transcript 数据都在内存里），只是不参与绘制；重绘只画 `active`。
- 状态栏显示 `lane N/M` 与全局待审批计数，避免用户忘记还有别的 lane 在等审批。

**已落地的部分（与上面的设想有出入，以代码为准）**：

- lane 区落在左栏**下半**、**固定占左栏一半高度**（`layout::lane_pane_height`：有 lane 就切一半，会话列表至少留 1 行）。区内是 `tui_mode/lane_list.rs` 画的**一行一条的底色条**：标题 `连接 [n]` + 每条 lane 一行整行宽的实心底色（当前 lane 用 `selection` 底色 + `▎` 标记），**自上而下堆叠**，**每条恒占一行**（曾是「把可用行均分给各条、夹在 1..=2 行」；两行块在真终端上读起来像按钮而不像列表项，文字贴首行后第二行只剩一条没有信息的色带，还把切换 lane 的文字基线撑得跟上面的会话列表对不齐），条数少于可用行时下方留空、不拉伸。装不下时标题补 `↓n`，不静默丢弃。**单 lane 也显示**——隐藏时用户既看不出自己连的是哪台 serve，也看不出客户端支持多个连接。
- 版面口径只有一处：lane 区高度 `lane_pane_height`、可用条数 `lane_shown_len`、行 → 下标 `lane_index_at` 都在 `layout`，`render` 与 `TuiApp::lane_at_click` 共用；各算一遍的症状是「点第 2 行切到别的 lane」。
- 切换是 `Alt+←/→` 或点某一行，只改 `active`，不打断在途回合；单连接时 `switch_lane` 直接返回（no-op），点击落在唯一那行等于不换。
- **已落地（后续补齐）**：lane 行状态点（`·` 空闲 / `●` 运行中 / `▲` 待审批 / `×` 已请求退出，口径在 `lane_list::LaneStatus`）、后台审批提示文案，以及 `F4` 跳到下一条待审批的 lane（环形，从当前的下一条找起；无人等待时不动视图，只写一行说明）。
- **未落地**：`lanes N/M`、全局待审批计数、`Alt+1..8` 直达、`Ctrl+N`。

### 4.7 工作区：一实例一工作区，所以 lane 行要带工作区

**服务端的约束**（`POST /workspace` 的 409 语义；权威在 Server repo 的 `docs/design/ctl_workspace_unset.md` §2 ①，本仓不复制其正文）：实例已绑定某个路径后再提交**不同**路径返回 `409 WORKSPACE_ALREADY_SET`，不会静默换根 —— 一实例一工作区。

三条直接后果（都已落地）：

1. **lane 行显示该 serve 当前的工作区末段**（`连接 [n]` 区里 `名字 · 项目名`）。光有连接名，用户看不出两条 lane 是不是同一个工作区 —— 而在「一实例一工作区」下，那正是选错目标的常见原因。末段由**显示层**从 `UiState::workspace_path` 截取（完整路径在右栏标题里），lane 行是一行宽的窄条，整条路径会把名字挤没；剩余宽度不足 4 列时宁可不显示，也不挤出一个字母。`LaneEntry` 只带 `label` / `workspace` / `active`，仍不向渲染层暴露 lane 内部状态。
2. **409 不等于「切换失败」**。`crabmate-client-api` 的 `classify_workspace_set_failure(status, body)` 把「409 + **顶层** `code == WORKSPACE_ALREADY_SET`」单独判成 `WorkspaceSetErrorKind::AlreadySet`（其余仍走 `Failed` + 服务端原文）；`UiEvent::WorkspaceSet` 因此携带结构化的 `WorkspaceSetError` 而不是裸字符串。TUI 的提示是「该实例已绑定其它工作区（一实例一工作区）：<服务端原文>；换工作区请另起 lane 或另起 serve 实例」—— 措辞刻意不提「重试」，因为重试永远不会成功。**不从文案里抠当前绑定路径**：要根就另外 `GET /workspace`。
3. **不给 lane 加「工作区切换历史」**：那是把「一个 client 管多个工作区」塞回一个实例上，与 409 直接冲突。要并行不同工作区就**再加一条 lane**（§5.1 的默认拓扑本来就是「一 workspace 一 serve」）。

**未做**：待绑定态（`GET /workspace` 回 `path: ""`）在 lane 行的显式标记（例如「待绑定」而不是留白），以及 `/lane new` 之后立即探测该实例工作区的专门提示 —— 新增 lane 的首拉会带上 `GET /workspace`（`LaneApp::bootstrap`，见 §4.8），但「待绑定」仍只在右栏标题里体现。Desktop / Web 侧的同名问题（切会话自动重放旧路径撞 409）不在本文件的 §1–§8 范围内（Desktop 走 §9.4.1 的多窗口路线）。

### 4.8 运行期加连接（`/lane attach` / `/lane new`）

`--connect name=url` 只能在启动时装配。进程跑起来之后再加一条走**控制面**（`crabmatectl supervise --socket`）：声明的唯一写入者必须是 ctl —— 客户端直接写 `instances.toml` 会与 `--watch` 构成同一文件的两个写入者，而让客户端自己 spawn `crabmatectl` 等于把进程管理搬进 TUI。

| 命令 | 做什么 | 副作用 |
|------|--------|--------|
| `/lane`（空参）/ `/lane list` | 列实例（与 `/ctl` 同一实现） | 无（只读 socket） |
| `/lane attach <name>` | 在声明里找**已就绪**的实例，取 `base_url` 后直接连 | 无 |
| `/lane detach <name>` | 只摘掉**本进程**的连接（连不上就报现有名字；最后一条不许摘） | 无（不动声明与进程） |
| `/lane new <name> [workspace]` | `add` 写声明 → 轮询 `list` 到 `observed == ready` → 连 | 新增一条声明（ctl 负责起进程） |

- **就绪判据只有一处**：`crabmate-client-api::ctl::lane_readiness` —— `ready` **且带 `base_url`** 才算 `Ready`；`failed` / `stopped` / `held` 直接 `Failed`（带 Server 的 `last_error`，它才是唯一知道原因的地方）；`starting` / `stale` / 缺 `observed` / 认不出的观测态一律 `Pending`。放纯逻辑里既为可测，也避免 worker 的轮询循环与 `/ctl` 的显示口径漂移。
- **`add.workspace` 必填**：socket `add` 至今要求 `String`（放开省略的是 TOML 声明那条路）。所以 `/lane new` 的 workspace 取「显式参数 ＞ 当前 lane 的 serve 工作区」，两者都没有（本 lane 待绑定）就明说要路径。`created: false`（声明已存在）是幂等命中，不当失败。
- **轮询**：每 300 ms、上限 20 s（`up` 自己的 readiness 超时是 10 s，留一倍余量）。超时**不是**「起不来」的结论，文案让人「稍后 `/ctl` 复查、就绪后 `/lane attach`」。等待期间独占**发起命令那条 lane 的 worker**（该 lane 其它拉取排队），别的 lane 不受影响。
- **装配**：`LaneApp::assemble` + `LaneTemplate`（存**启动时**的 `overrides` / `yes` / `no_keyring` / bearer / `ctl_socket` / `tx`）。存启动初值而不是「当前 lane 的现值」：当前 lane 的 `/model` 覆盖只属于它。`LaneApp::client` 因此从 `&'a ServeClient` 改成 `Cow<'a, ServeClient>` —— 启动装配仍零拷贝借用，运行期造出来的客户端自己持有。
- **事件通道**：结果走 `UiEvent::CtlLaneReady { name, result }`，由 worker **裸发**（不包 `LaneId`）：目标 lane 此刻还不存在，按 id 分发只会被当成「已关闭的 lane」丢掉；由 `TuiApp` 拦截并装配 —— 单条 lane 无权改 `lanes`。
- **同名或同 URL 都不重连**：名字已在 `lanes` 里时只切过去并明说「已经是本进程的连接」；名字不同但 `base_url` 与某条 lane 相同（按 `same_api_base` 归一化比较：尾 `/`、大小写不算差异，`localhost` 与 `127.0.0.1` 仍是两个 Origin）也只切过去 —— 那是同一台 serve，重复连会各开一条 SSE、各读一份会话。静默再连会让左栏出现两条同名 lane，而 `/ctl` 里只有一条实例。
- **`/lane detach <name>` 只摘本进程的账**：声明与 serve 进程都不动（`/lane` 的语义是「本进程连不连」，与 attach 对称；要停实例用 `crabmatectl down`）。摘掉即 drop 该 `LaneApp`，它的 job channel 随之关闭，worker 线程与那个 runtime（含在途请求、SSE）自行收尾，不另发停止信号；`LaneId` 单调递增不复用，残留事件按 id 分发时匹配不到，直接丢弃（不会串台）。两种情形明说而不硬做：名字不存在（把现有名字列出来，省得再猜）与**最后一条**（界面至少要有一条 lane，退出用 `/quit`）。
- **主 lane 也按 URL 认领实例名**：启动 lane 的名字原本恒为 `local`。上面的同 URL 去重已经能拦住「同一台 serve 连两条」，但名字对不上时提示只能说「`local` 已经连着 …」，左栏的名字与 ctl 里的实例名仍是两套。启动时因此问一次控制面（`serve::ctl::primary_binding`）：`base_url` 与 `--api-base` 相同的实例，主 lane 直接用它的名字，`/lane attach server` 连同名/同 URL 两道闸都命中，只切过去并说「已经是本进程的连接」。比较只看 **URL**（`same_api_base`：去尾 `/`、不区分大小写；`localhost` 与 `127.0.0.1` 是两个 Origin，不合并），不看 `observed`（`stale` / `starting` 仍是那台 serve 的声明）。查不到（ctl 没起 / URL 未声明 / 回包读不懂）一律退回 `local`，**不拦启动** —— 这条查询只负责命名。
- **新 lane 的首拉**与启动同一套（`LaneApp::bootstrap`：会话 / 工作区根 / serve 状态 / user-data 快照）：只刷当前 lane 的话，切过去会看到空的会话列表。
- **未做**：`/lane close` —— Server 侧已补上 socket 的 `disable` / `enable`（`enabled = false` 是客户端软删的正式口径），但 TUI 还没接：`detach` 只断开本进程的连接，声明项仍在，要删声明/停实例目前只有 `crabmatectl`；per-profile bearer 槽（控制面里所有实例同机同用户，第一版共用主连接那份；某实例要求不同 Bearer 时症状是那条 lane 的拉取全错）、`MAX_LANES` 上限。

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

**实际进度（截至 `2026-10`，以此为准）**：W1 / W1b / W2 已落地，且 W2 顺带把 W5 的「lane 列表 + 切换」做了（列表在下半、单 lane 也显示、`Alt+←/→` 与点击切视图）；lane 行随后补上了各自的工作区末段与 409 `WORKSPACE_ALREADY_SET` 的分支提示（§4.7，属 workspace binding 接入而非新波次）；W3 落地了两种装配方式 —— 启动时 `--connect name=url`、运行期 `/lane attach|new`（§4.8，经控制面 socket，不重启进程），仍缺 `tui-connections.toml` 配置文件与启动逐 profile `/health` 探测；W4 的审批队列与 W5 的 `lanes N/M` / 全局待审批计数 / `Alt+1..8` 未开工；`MAX_LANES` 与「后台审批可发现性」（`F4`）已补齐。

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
| 7 | 退出落盘语义在强退 + 多 lane 下变复杂 | 保留「仍在跑 ⇒ 不等落定」；逐 lane 遍历 + 共用一份总预算（issue #338 已落地） |

---

## 8. 未核对项 / 待决

| 项 | 状态 |
|----|------|
| 同一 `conversation_id` 并发两轮的期望语义（排队 / 拒绝 / 未定义） | 已确认**无互斥**；语义需 Server 侧确认或实测。本方案不依赖它（每 lane 独立会话） |
| 工作区切换在途回合期间的行为 | Client 侧 `parse_workspace_set_ok_body` 支持 `error: "busy"`，但 Server 侧 grep 不到该字面量（疑为旧版兼容）；**不依赖**该保护 |
| 同 workspace 8 路并发写 `conversations.db` 是否触发 `database is locked` | **未实测**；本方案默认拓扑不触发（每 workspace 一库） |
| `SessionPrefs` 是否需要逐 lane（而非进程级） | 已按 §4.3 落地：`overrides` 随 `LaneApp` 逐 lane 持有（`/model` 只影响当前 lane） |
| lane 数上限（§4.3 `MAX_LANES = 8`） | **已实现**（`shell::MAX_LANES`）：启动走 `tui_mode::check_lane_capacity`，超限即报错退出（不静默只连前 N 条）；运行期 `/lane attach|new` 超限只在发起命令那条 lane 提示、不退出进程，且检查在两道去重**之后**（同名 / 同 URL 仍能切过去） |
| 是否需要 per-profile 钥匙串槽（多 bearer） | 第一版不做，需要时另开 Issue（牵动 Desktop / Android 槽名约定） |

---

## 9. Desktop 复用评估（§1–§8 只覆盖 TUI，本节回答「Desktop 能不能复用」）

### 9.1 结论

- **纯逻辑层可完整复用**：`crabmate-client-api` / `crabmate-tool-card` / `crabmate-connect` 的 probe 与 handoff 与 lane 数量无关。
- **TUI 的 lane 代码不能复用**：`LaneApp` / `UiEvent::Lane` / `TuiApp` 建立在 ratatui + 同步 `mpsc` + `current_thread` runtime 上，WASM 里没有这些；Leptos 是 signals + async。
- **但设计结论可以照搬**：LaneId 来源标签、per-lane 状态树、审批队列、「代数门控留在 lane 内」。
- Desktop 做多连接应当走 **多 WebView 窗口**，而不是在单窗口内做 lane（理由见 §9.4）。

### 9.2 Desktop 的连接模型（源码事实）

| 事实 | 位置 |
|------|------|
| Desktop 是**单 WebView 窗口**（label `"main"`），窗口辅助函数都依赖该 label | `desktop-tauri/src-tauri/src/main.rs:208-211` |
| 连接流程：connect 页探测 → 经 **URL hash** 交接 `api_base` + 可选 bearer → 载入包内业务 UI | `crates/crabmate-connect/src/lib.rs:3-5`、`build_handoff_url`（`:31-34`） |
| 前端 API 基址是 **thread_local 单值** `API_BASE_URL`，`api_base_url()` 全局读 | `frontend/src/api/browser.rs:31`、`:208-210` |
| 持久化是**固定 localStorage 键** `crabmate-api-base-url` | `frontend/src/api/browser.rs:26` |
| 前端另有 11 处 thread_local 全局状态（bearer、llm secrets、locale、滚动跟随、图片灯箱、github secrets…） | `frontend/src/**`（共 12 处 `thread_local!`） |
| 连接 bearer 是**单钥匙串槽** `tauri_connect_web_api_bearer`，连接成功即覆盖写 | `crates/crabmate-connect/src/lib.rs:5` |

即：**Desktop/Web 的「单连接」假设渗透在 12 个 thread_local 全局里**，比 TUI 更分散——TUI 只是 `TuiApp` 一个结构体持有 `client`，所以 TUI 能用「改结构归属」的手法低成本解耦，Desktop 不能照搬。

### 9.3 分层复用判断

| 层 | 判断 | 理由 |
|----|------|------|
| `crates/crabmate-client-api` | **完整复用** | 纯逻辑无 IO；TUI 已在用。URL 规范化、审批、workspace、sessions、chat body、hash handoff 都在这里 |
| `crates/crabmate-tool-card` | **完整复用** | 同上 |
| `crabmate-connect` 的 probe / handoff / normalize | **可复用** | `probe_server` / `build_handoff_url` / `normalize_base_url` 与 lane 数量无关 |
| `crabmate-connect` 的 keyring / 「当前连接」 | **需改** | 单槽 + 全局当前 URL；多连接要 per-profile 槽与 per-window 状态 |
| TUI 的 `LaneApp` / `UiEvent::Lane` / `TuiApp` | **不能复用** | ratatui + 同步 `mpsc` + `current_thread` runtime；WASM 无对应设施 |
| 设计模式（§4.2 的来源标签、代数门控留在 lane 内、§4.4 审批队列） | **思路可复用** | 不能共享代码，能共享结论 |

### 9.4 两条路

#### 9.4.1 路线 A：多 WebView 窗口（推荐）

每个窗口 = 独立 JS 堆 ⇒ **§9.2 那 12 处 thread_local 天然隔离**，一行都不用改。

必须解决的三点：

1. **localStorage 同 origin 共享**（硬性）：`crabmate-api-base-url` 是固定键，两个窗口会互踩 → 按 window label 分键（如 `crabmate-api-base-url:<label>`）。这是必改项，且**不能靠 WebView 隔离规避**（localStorage 属于 origin，不属于 WebView 实例）。
2. **钥匙串单槽**（大概率可省）：多窗口若共用同一 bearer（一人一机多项目通常同 key），不需要改；只有真要 per-profile bearer 才加槽，且那会牵动 Desktop / Android 的槽名约定。
3. **connect 流程假设单「当前连接」**（中等工作量）：`seed_connect_home` / `allowed_origin` / `navigation` 都以「这个窗口当前连谁」为前提，要改成 per-window 状态。

#### 9.4.2 路线 B：单窗口内多 lane（对齐 TUI）

要把 §9.2 的 12 处 thread_local 全部改成 per-lane context，再把会话 / 工作区 / 审批 / 设置状态树按 lane 切分。状态层是单连接长出来的，改造量显著大于 TUI 的 W1 + W1b，且**没有 W1b 那种「加个包装变体」的省力路径**（Leptos 的状态分散在各 `RwSignal`）。除非有「单窗口内必须并排看两路」的硬需求，不建议。

### 9.5 与 TUI 共享的前提（§3 对 Desktop 同样成立）

- 会话索引按 `ws_sha256` 分片、对话库在 `<workspace>/.crabmate/` ⇒ 不同 workspace 天然隔离
- `chat_queue_max_concurrent` 是进程级、默认 2 ⇒ 并行的自然单位是「不同 serve 进程」
- `allow_always` 每回合重建、不跨 client ⇒ 多窗口不存在跨端静默放行

因此 Desktop 多窗口也应遵循同一条使用约定：**一窗口 = 一 workspace = 一 serve**。

### 9.6 最小验证实验（改动量极小）

在 `desktop-tauri` 加一个「新建连接窗口」命令，起第二个 WebView 载 connect 页，让两个窗口各自连不同 `serve`：

- 预期立刻暴露 localStorage 键冲突（第二个窗口若沿用固定键，会覆盖第一个的 `api_base`）；
- 由此确认 §9.4.1 第 1 点是**唯一的前置硬障碍**，其余两点可延后。

### 9.7 非目标

- 不在本节承诺 Desktop 的实施排期；它需要独立 Issue（本 Issue #327 只覆盖 TUI）。
- 不做 Web 浏览器端的「多连接」（同一标签页只有一份 JS 堆，等同于路线 B，成本更高）。
