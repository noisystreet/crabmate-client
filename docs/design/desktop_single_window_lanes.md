# 单窗口多 lane（路线 B 第一阶段）：形态、容器与推进顺序

- 状态：**Proposed**（设计草案，尚未实施）
- 跟踪 Issue：[#561](https://github.com/noisystreet/crabmate-client/issues/561)（形态与实现）、
  [#558](https://github.com/noisystreet/crabmate-client/issues/558)（W1 剩余输入项）
- 前序结论：[`tui_multi_server.md`](tui_multi_server.md) §9.4.2（路线 B 的成本与拦路条件）、
  [`desktop_multi_window.md`](desktop_multi_window.md)（路线 A：一连接一窗口，**并存不替换**）
- 覆盖范围：**标签式互斥切换**。分栏（并排）明确不做，见 §12。

## 0. 对齐口径

用户要求「外观、逻辑与 TUI 尽量一致」。本设计把一致性拆成四种，**每条都要能落到代码上**：

| 类别 | 含义 | 处理 |
|---|---|---|
| 已经一致 | 共享层已提供同一份类型 / 同一份纯函数 | 直接用，**不要重做**（§2.1-A） |
| 必须补上（外观） | TUI 有明确口径、桌面还没有 | 照口径补（§2.1-B） |
| 必须补上（逻辑） | 语义上必须对齐，否则会串台或漏提示 | 照口径补（§2.1-C） |
| 有意不同 | 形态差异导致照抄就错 | 写清为什么不同（§2.1-D） |

**「尽量一致」不等于「一律照抄」**：§2.1-D 那几条若照搬，会把 TUI 为「单进程多 lane」设计的
妥协搬到「多条 HTTP 流」的形态上，结果比不一致更糟。

## 1. 结论先行

1. 本阶段做「一个 Desktop 窗口内持有多条 lane，左栏点一行 = 切到那条 lane」，与 TUI 的 lane 列表同形态。
2. **分层已就位、承载物还不存在**：`WindowSignals` / `LaneSignals` / `AppSignals::new_lane` 与
   `decide_attach` 已提交（PR #563），但没有任何运行时结构持有 N 条 lane，`ACTIVE` 仍是每页单槽。
3. 下一刀是 **lane 容器**，且必须先做成 **N=1 等价重构**（单连接零行为变化、818 条 frontend 测试全绿）。
4. 外观与逻辑的**地基已经在共享层**：`ConnectionSnapshot` 就是「一条连接的一行」的公共形状，
   `·●▲×` 四字形两端逐字相同。所以本阶段的外观工作不是「设计一套新列表」，而是
   **把已有的连接行改成消费 N 行**。
5. 「后台 lane 的流式不中断」不靠条件渲染，靠「N 棵子树同时挂载 + CSS 隐藏」（§5）。
6. 左栏数据源是必决议项（§6.5）：现在的名单一行 = 一个 window label。
7. 路线 B 打破「一窗口 = 一 workspace = 一 serve」这条**界面**约定；「一条 lane = 一个 workspace
   = 一台 serve」仍成立（`tui_multi_server.md` §9.5）。

## 2. 起点：已经有什么、还缺什么

核对基准：分支 `refactor/561-window-lane-split`（PR #563，基于 `main` @ `a8d00af`）。

| 能力 | 落点 | 状态 |
|---|---|---|
| `api_base` 从窗口级聚合拆到 `ConnSignals` | `frontend/src/app/app_signals/connection.rs` | ✅ 已合并（#562） |
| `WindowSignals` / `LaneSignals` / `AppSignals::new_lane(&window)` | `app_signals/{window_signals,lane_signals,mod}.rs` | ✅ 已提交（PR #563），**无运行时持有者** |
| `serve_identity`（`same_api_base` / `is_absolute_api_base`） | `crates/crabmate-client-api/src/serve_identity.rs` | ✅ 已提交；TUI 改为 delegate |
| `lane_attach::decide_attach` + `MAX_LANES = 8` | `crates/crabmate-client-api/src/lane_attach.rs` | ✅ 已提交；TUI `attach_lane` 已换用 |
| `ScopeHolder`（值离开 `thread_local`，`ACTIVE` 只存指针） | `frontend/src/api/connection_scope.rs` | ✅ 已合并（#559 / #560） |
| 存储域 per-holder（`read/write/clear_*` 仍取 `ACTIVE`） | `api/browser.rs` / `api/storage_scope.rs` | ❌ 未做（#558 项 3b，**接第二条 lane 前必须做**） |
| lane 容器（N 份 lane 级状态 + 当前索引） | —— | ❌ 未做 |
| 每 lane 一棵组件子树 | `frontend/src/app/mod.rs` 单根 `App` | ❌ 未做 |
| 状态 / 未读按「前台 lane」判定 | `app_shell_effects/connection_report.rs` | ❌ 未做（现为窗口级 DOM `focus` / `blur`） |
| 壳侧注册表按 label 记「一窗口一连接」 | `desktop-tauri/src-tauri/src/desktop_connections.rs` | ✅ 现状（路线 A 需要，路线 B 不动它，见 §6.5） |

一句话：**类型先于实例**。`LaneSignals` 文件头已写明它只是「8 个字段的构造清单」，
不是「每 lane 一份的载体」——不要写成 `vec![app.lane]`，那不成立。

## 2.1 与 TUI 的对齐清单

### A. 已经一致（不要重做）

| 项 | TUI | Desktop | 结论 |
|---|---|---|---|
| 状态字形 | `LaneStatus::mark()`：`·` `●` `▲` `×` | `nav_rail_connections::status_glyph`：同四字 | **逐字相同**；新增 lane 行直接调用它，不要再写一份 `match` |
| 行的数据形状 | `LaneEntry { label, active, status, unread, context, busy_secs, pulse }` | `ConnectionSnapshot { key, display_name, authority, active, status, unread, context, busy_secs }` | 共享层注释明写「这是 TUI 的 `LaneEntry` 与 Desktop 连接列表的公共形状，单连接就是长度为 1 的切片」。**lane 行必须复用 `ConnectionSnapshot`**（§6.2） |
| 状态判定 | 自有 `LaneStatus` 快照 | `connection_status_from_turn(turn_busy, pending_approval)` | 同一口径，Desktop 用共享函数；TUI 的 `LaneStatus` 是它在终端侧的等价枚举 |
| 未读判定 | 外壳置位 / 切过去清零 | `connection_unread_after(focused, turn_busy, prev)` | 复用，别另写（§8.2 只改输入） |
| 耗时文本 | `busy_duration_text` | 同一个函数（`45s` / `2m05s` / `1h02m`） | 复用 |
| 忙碌判据 | 渲染侧看 status | `ConnectionSnapshot::is_busy()`（`Running` 或 `WaitingApproval`） | 复用；别各处写 `matches!`，漏 `WaitingApproval` 的症状是「那条看起来空闲」 |
| 挂载判定 | `attach_lane` → `decide_attach` | 同一份 | 已完成（PR #563） |
| 上限 | `shell::MAX_LANES`（= 共享常量 8） | 同一常量 | 复用；不要在 Web 另立一个数 |
| 当前 lane 下标 | `TuiApp.active: usize` | 建议 `RwSignal<usize>` | 同形状；单 lane 时恒 0 |
| 面板位置 | 左栏下半（§4.6） | 同一位置（`desktop_multi_window.md` §13 已声明「同一位置、同一条版面契约」） | 不改版面位置 |

### B. 必须补上（外观）

| 项 | TUI 口径（原文位置） | Desktop 要做的 |
|---|---|---|
| 标题行 | `连接 [n/8] ▲k`，装不下补 `↓m`（`lane_list::lane_title`） | 左栏「连接」标题改成同一格式：`n` 为已连条数，`▲k` 为全局待审批数，溢出补 `↓m` |
| 行尾读数顺序 | `62% 12s •`，**从右往左**按重要性排；宽度不够**从左边丢**（先丢 ctx，再丢耗时，未读点留到最后）（`lane_list::lane_tail`） | 行 meta 的拼接顺序与丢弃顺序照此实现 |
| 名字优先 | 名字至少留 4 列（`LANE_LABEL_MIN`），尾部读数最多占半行 | 窄宽度先保名字，再谈读数 |
| 整行底色条 | 一行恒定高、补满宽度、行间不留空行、左右各内缩 1 列；当前 lane 用 `selection`，其它用抬升一档的 `overlay_bg`；等审批的**非当前** lane 用告警前景 | active 行用 `--surface-hover`（或既有 selection 类），其余用条带底色；行距与左侧会话列表同一行距（TUI 原注释：两行块读起来像按钮） |
| 生成中不换字形 | `●` 只做亮度呼吸，字形不变、行宽恒定 | 用 CSS 过渡做呼吸，**不得**改字号 / 加粗 / 换符号（会挤动折行点） |
| 符号宽度稳定 | 只用宽度确定是 1 的字符（`⏸` 在部分终端算双列会撑破行） | 不放 emoji（渲染宽度不稳），沿用四字形 |
| 焦点用底色不用标记列 | 当前 lane 不靠额外标记列（竖条多占一列还会让文字横移一格） | 不要再加一个「当前」图标列 |
| 空 / 截断 | 条数少于可用行 → 下方留空、不拉伸 | 不撑高 |

### C. 必须补上（逻辑）

| 项 | TUI 做法 | Desktop 缺口与对策 |
|---|---|---|
| 「当前 lane」只有一个写入口 | `active` 只在 `TuiApp` 里改；未读清零跟着同一处走（注释：不逐条写在切换路径上 —— `Alt+←/→`、点击、`F4`、`/lane attach` 各有出口，漏一处就漏一次清零） | 加 `set_active(i)` 单一入口，未读清零、`ACTIVE` holder 切换、忙碌重算都挂这里（§8.2） |
| 未读 = 「后台 lane 有新输出且没看」 | `mark_background_unread(id, ev)` 置位；切过去清零 | 现式为 `connection_unread_after(窗口focused, busy, prev)`；多 lane 后把「窗口失焦」换成「窗口失焦 **或** 该 lane 不是 active」（§8.2） |
| 后台 lane 的审批要「说一声」 | 浮层只画当前 lane，所以 `announce_background_approval` 在**当前 lane** 留一句提示 + 左栏 `▲`（issue #337） | 左栏 `▲` 已有；系统通知已有（`desktop_multi_window.md` §5）；**缺「在当前 lane 留一句提示」这第二保险** —— 要么补（状态栏错误条 / 聊天区一行），要么明确「只靠通知」并说明理由（§8.4） |
| 切换键位 | `Alt+→` / `Ctrl+N` 下一条、`Alt+←` 上一条、`Alt+1..8` 直达、`F4` 跳到下一条等审批；单 lane 时全部 no-op | **完全没有**，要补（§6.6） |
| 直达越界只提示 | `GotoLane(n)` 越界不切视图、只留一句提示 | 同上，别静默 |
| lane 关闭的「最后一条」特例 | `client_capability_matrix.md` 记：只有「最后一条 lane」的后果要明说，其余直接断 | Desktop 关闭 lane 前要判「是否最后一条」并改写提示（§8.3） |

### D. 有意不同（照抄会错）

| 项 | TUI 为什么那样 | Desktop 为什么不能照抄 |
|---|---|---|
| 只渲染当前 lane | ratatui 每帧只画当前 lane，其它 lane 的状态在内存结构里，不存在「卸载」 | Leptos 卸载会停 Effect、重建会重跑副作用 → **必须**同时挂载 + 隐藏（§5） |
| 共用一条事件通道 | 所有 lane 共用一个 `rx`（worker 单消费者，避免抢消息），靠 `LaneId` 分发 | Desktop 每 lane 一条独立 HTTP 流，回调闭包各自持有句柄 → 天然按 lane 归属，**不需要**共享通道，也**不能**引入「全局分发器」 |
| 会话刷新合并 | 回合结束各 lane 置意图位，外壳按窗口合并后发一次（issue #349，防 N 倍扇出） | 每 lane 连的是**不同** serve，会话列表互不相干 → **不能合并**（合并会让 A 台的刷新打到 B 台）。同 serve 两条 lane 已被 `decide_attach` 的同 base 去重挡住，所以 TUI 那个扇出问题在 Desktop **不存在** |
| 同 serve 的 refresh 回包 fanout | `fanout_shared_refresh` 把回包复制给同 serve 的其它 lane（否则被跳过的 lane 会话列表永不更新） | 同上：一台 serve 只有一条 lane，没有「其它 lane」可 fanout |
| 待就绪 lane 不可点 | `◌` / `✗` 排在真连接之后，`lane_index_at` 只映射前 `lanes.len()` 行 —— 点一行却切到别的 lane 更糟 | Desktop 的「可连实例」分组（§14）**是可点的**（点了就连上去）。这是**产品口径不同**，不要为了对齐把桌面改成不可点；但**行索引 ↔ lane 索引的单一来源**这条要照抄（§6.4） |
| `pulse` 相位 | 终端靠字形亮度呼吸，相位进 `LaneEntry` | 共享层刻意不带 `pulse`（注释：终端靠呼吸字形、Web 靠 CSS transition，进共享层只会互相牵制）→ Web 用 CSS，不进 `ConnectionSnapshot` |
| 关闭 lane 的视觉 | `/lane detach` 摘掉后那一行消失 | 同形：关闭 lane = 断开这条 HTTP 流，窗口仍在 |

## 3. 决策表

| 决策 | 结论 | 依据 |
|---|---|---|
| 形态 | **标签（互斥切换）** | issue 已勾选；对齐 TUI（一屏只显示一条 lane） |
| 窄屏 / Android | **只有宽屏 Desktop 开多 lane**；Android / Web 保持单连接 | 与 issue 非目标一致；容器在 `tauri_shell_available()` 为假时退化为 N=1 |
| 上限 | **复用 `MAX_LANES = 8`** | 已是共享常量；`decide_attach` 的名额检查在两道去重之后（TUI 实测顺序） |
| 同 serve 去重 | **一台 serve 只占一条 lane**（`SwitchToSameBase`）；壳注册表仍按窗口记 | `decide_attach` 已表达；壳侧不需要知道 lane |
| lane 名 | **必填**，空名回落 `authority` | 照 TUI（`/lane attach name=url` 要求给名）；`ConnectionSnapshot::new` 已有同一回落规则 |
| 左栏数据源 | **前端自渲染本窗口的 lane 列表**（§6.5 方案 B），列表项复用 `ConnectionSnapshot` | 不用扩壳 IPC 契约；与其他窗口列表并存但不合并 |
| 首刀形状 | **N=1 等价重构**，先零行为变化 | 闭包捕获约束已实测（§4.2） |

除「形态」外都需 owner 确认后才开工，确认后勾进 #561。

## 4. lane 容器

### 4.1 结构草案

骨架，不是最终 API。

```rust
/// 一条 lane：lane 级信号 + 它自己的连接作用域 + 它的存储域后缀。
struct LaneCtx {
    signals: AppSignals,   // AppSignals::new_lane(&window)
    holder: ScopeHolder,   // 这条 lane 的 api_base / bearer / 水合标记
    key: String,           // 存储域后缀，见 §7；单 lane 时等于壳注入值
    lane_id: u32,          // 单调递增、不复用（照 TUI 的 next_lane_id，避免 ABA）
}

/// 窗口级：一份，跨 lane 不变。
struct WindowCtx {
    window: WindowSignals,        // shell_ui / sidebar / modal / ide_* / resize
    lanes: Vec<LaneCtx>,
    active: RwSignal<usize>,      // 单 lane 时恒 0
    focused: RwSignal<bool>,      // 窗口级 DOM focus / blur（已有）
}
```

`active` 用 `usize`（TUI 同形）。`lane_id` 不复用的理由与 TUI 相同：关掉第 2 条再新建时若复用编号，
「上一轮那个 lane 的迟到回调」会写进新 lane 的 transcript —— 串台比丢一条更糟。

### 4.2 不要用 `Deref` 复用 284 处 `app.chat`

PR #563 的实测结论（已写进 `lane_signals.rs` 文件头）：`Deref<Target = LaneSignals>` 会让
`move ||` 闭包捕获**整个** `AppSignals`，把 `Rc<RefCell<…>>` 与 `Arc<Mutex<AbortController>>`
拖进 Leptos 的 `Send + Sync` 要求：

```text
error[E0277]: `Rc<RefCell<Option<(f64, f64)>>>` cannot be sent between threads safely
   --> src/app/mod.rs:212   <Show when=move || app_ctx.signals.chat_composer...
```

视图一律取**具体句柄**。这与 TUI 的同一条约束是同一个道理、两种表述：TUI 的 `overrides`
**自有而非借用**，注释写「`&mut` 借用会让第二条 lane 无法构造」；Web 侧对应的是「别让闭包
捕获整份聚合」。

### 4.3 `ACTIVE` 的定位

保留，语义收窄为「**当前前台 lane 的默认 holder**」（#558 既定口径）：

- 切 lane 时把目标 lane 的 `holder` 提升为 `ACTIVE`（与 `set_active` 同一处，§8.2）；
- 后台 lane 的 SSE 回调、per-lane 审批轮询、任何可能被多条 lane 并发触发的 fetch **必须**显式带
  `holder`——那正是「多 lane 串台」的唯一成因；
- 单 lane 下两者同一份，零行为变化。

已知一义两用：`bearer_settled` 既是「已水合」也是「被显式设过值（含清空）」。多 lane 且要区分
「B 没水合过」与「B 被清空过」时需拆两个标记。

### 4.4 接线拆分

`bootstrap_app_shell()` 现在一次做完：建 `AppSignals` → `run_shell_wiring_in_order` → 组 `AppShellCtx`。
多 lane 后按 **Effect 读哪一级的 signal** 拆两层：

| 层 | 内容 | 次数 |
|---|---|---|
| 窗口级 | locale / 主题 / 面板宽度 / IDE 布局 / 全局 Escape / 视口断点 | 1 |
| lane 级 | 会话水合、SSE 接线、工作区域、`/status` 与任务、`LlmSettings`、上下文用量 | 每 lane 1 |

`provide_context(ChatShellLeptosContext)` 目前挂在 `App` 顶层，要移进每条 lane 的视图作用域。
**实施时需验证**它仍满足 `provide_context` 的约束（现在能通过说明类型满足；移位置不改类型）。

## 5. 保活：N 棵子树同时挂载 + 隐藏

TUI 的等价做法是「每帧只画当前 lane」：其它 lane 的状态活在 `LaneApp` 里，渲染不碰它们。
Web 侧没有「帧」，只有「挂载 / 卸载」，所以要先说清卸载会怎样：

- 流式状态**在 lane 级信号里**（`ChatStreamTransport` 的 `attach_generation`、`AbortController`、
  `stream_overlay_*`、`tool_output_chunks`）；SSE 回调闭包**持有这些句柄**。
- 因此组件卸载**不会** abort 请求，也不会丢已收数据 —— 数据在 signal 里，不在组件里。
- 但卸载会停掉该子树里的 `Effect`（滚动跟随、`visibilitychange` 恢复、DOM 同步），重建时重跑；
  重跑带来额外请求与可见跳变，这正是 issue 所说「不丢帧、不重放」的风险面。

**决定：同时挂载 + `display: none`**，不用条件渲染。这样「后台 lane 继续收流、继续长高度」
与 TUI 的「后台 lane 状态照常推进，只是不画」在语义上等价。

| 项 | 做法 | 备注 |
|---|---|---|
| 隐藏 | 非前台 lane 的子树 `display: none` | 不进布局、不参与滚动锚定 |
| 代价 1：N 套 Effect | 后台 lane 降频：只保留「SSE 回调 + 审批状态」必跑；定时类按**前台间隔 ×10** 放慢，切回前台**立即补拉一次** | 力度已定 10×；分类规则与逐项清单见 §11 |
| 代价 2：N 份 DOM | 由 `MAX_LANES` 封顶 | 8 条的 transcript 同时挂载是性能假设，**未测** |
| 门禁 | 隐藏用既有工具类；新增类名须同步同 scope 的 CSS 规则 | `scripts/check-css-contract.sh` |

**未核对**：本条只能由「两条 lane 各跑一轮流式、来回切换」的桌面手测确认（issue 验收第 2 条）。

#### 5.1 预建 8 条的成本实验（2026-11-21，已跑）

临时把容器改成「启动即建 8 条 `LaneRuntime`（各含完整 wiring，都指向同一条 serve）」，
用 `scripts/smoke-desktop-shell.sh` 在 xvfb 里跑真实 WebKitGTK：

| 项 | 结果 |
|---|---|
| 能否启动 | ✅ 两次冒烟均通过（进程存活 / 日志无 error·panic / WebKit 子进程起来） |
| `debug_assert!(holder.is_base_settled())` | ✅ 未触发（8 条 lane 的 holder 都在 `new_lane` 前显式 `set_api_base`） |
| 8 套接线互相踩 | ✅ 未见（`register_connection` 等无 panic） |
| **serve 侧请求量** | ❌ **没测到**：serve 无 access log；冒烟判据是「WebKit 子进程出现即通过」，在页面真正发请求前就结束，`ss` 采样到 0 条到 18080 的连接 |
| 静态量级（推断） | lane 级 wiring 约 **29 个 `Effect::new`**（`app_shell_effects` 22 + `wire_chat_session_lifecycle` 2 + `status_tasks_wiring` 2 + `github_wiring` 3）→ 8 条 ≈ **232 个常驻 Effect**，且每条都会各发一轮首帧请求（`/status`、`/sessions`、`/user-data`） |

**结论（对 §5 的意义）**：

1. **A 路径（预建 N 棵子树）在技术上可行** —— 启动不崩，这是最大的不确定项，已排除。
2. 但**预建 8 条 = 8 倍冷启动请求**（8 × 首帧），这正是降频 10× 要治的病；而降频只作用于「后台轮询」，治不了「启动时 8 条各发一次首帧」。
3. 因此「预建」必须换成「**懒建**」：只在用户真正新建 lane 时才 `add_lane`。而懒建要求视图树能**动态增删** —— 这正是 `Send + Sync` 阻塞所在（见 PR 描述「已知限制」）。

一句话：**实验排除了「8 条预建会崩」，但把矛盾从「资源够不够」推到了「视图能不能动态重建」**。后者是设计选择，不是性能问题。

#### 5.2 已定：懒建，形状对齐 TUI

**决定（2026-11-21）**：不预建，**用户点「连接」时才建那条 lane**；形状对齐 TUI 的
「一条 lane 一棵子树 + 切换只换显示」。

一个决定方案形状的技术事实（实测，两个方向都验过）：

| 形态 | 能否捕获含 `Rc` 的值 | 证据 |
|---|---|---|
| `on:click=move \|_\| …` 等**事件回调** | ✅ **可以** | `settings_sections.rs` 的 `save_api_base_click(scope.clone(), …)`，`ScopeHolder` 含 `Rc`，编译通过 |
| `<Show when=move \|\| …>` / `key=` 等**响应式闭包** | ❌ **不行** | 把 `app_ctx.signals().chat` 写进闭包后报 9 处 `Rc<(dyn Fn)> cannot be sent between threads safely` |

**推论**：「新建 lane」这件事**只能在事件回调里做**，而视图挂载要绕开响应式闭包 →
必须用 **`leptos::mount::mount_to(host, || app_body(rt))`** 手工挂到占位容器上，并保留
`UnmountHandle` 供关 lane 时卸载。这也是懒建唯一可行的形状。

三步（每步都可独立验证）：

| 步 | 内容 | 验证 |
|---|---|---|
| **S1**（已完成） | `App` 只剩「取当前 lane → 交给 `app_body(rt: Rc<LaneRuntime>)`」；14 个 `*_signals()` / `*_view_input()` 搬到 `LaneRuntime`（`app_signals/lane_view_inputs.rs`），视图与 `provide_context` 进 `app/app_body.rs` | N=1 等价：826 测试 + wasm32 clippy + 桌面冒烟（xvfb + 真实 WebKitGTK）**均已通过** |
| **S2**（已完成，挂载后隐藏） | `LaneSet::add_lane(scope)` + 窗口级动作注册表 `app/lane_actions.rs`；两处「连接」入口改调它，`App` 在 `.lane-layer` 容器上 `mount_to(app_body(rt))` | 冒烟能起 + 手测「点两次得到两条 lane」 |
| **S3a**（已完成） | 切换只换 class（`lane-slot--active`，不重建子树）；键位 §6.6 全表 | 冒烟 + Playwright 布局用例；**切 lane 的行为需图形会话手测** |
| **S3b**（已完成） | 左栏「本窗口 lane 列表」（§6.5 方案 B）+ 点击切换；§6.5 的单窗口降级写进能力矩阵（注 3） | 单 lane 时整组不渲染；条数与前台高亮跟 `labels` / `active` 走 |

> **#569 手测结果（2026-11-22）**：连两台不同 serve 后点另一条 lane，主区内容不变。两个独立成因：
>
> 1. **数据面** —— 请求基址只经 `ACTIVE` 指针取（`api_url` / `prepare_api_auth` 的零参口径），
>    后台那条的请求打到前台那台。**已修**：`api_url_on` / `prepare_api_auth_on` / `auth_headers_for`
>    加每条 lane 自己的 `ScopeHolder`，`/status`、`/tasks` 与首屏会话桶已改用它。
> 2. **整屏交互** —— 一开始记成「`set_active` 写 `LanePhase` 时 panic，`active` 从未写入」，**这条
>    判断是错的**：`reactive_graph` 的 effect 走 `Executor::spawn_local` + 通道异步执行，订阅者
>    panic 拦不住同一函数里后面的 `active.set(index)`。实测到的是一条更早、更凶的 panic：
>    `connection_report` 第一帧读 `connections_registered()` 时，那份「全局」signal 被建在 memo 的
>    computation 作用域里（`with_cleanup` 在每次 run 后清掉本次创建的节点）→ 建完即 dispose →
>    之后每次读都 panic；wasm32 上 panic 是 trap 而不是 unwind（`pageerror: unreachable`），
>    Rust 栈不展开、`Drop` 全跳过，`js-sys` 任务里的 `RefCell` 借用永久不还
>    （第二条 panic：`RefCell already borrowed`），`spawn_local` 那条链一起死 —— 界面「页面还画着，
>    切 lane / 点会话都没反应」。
>    **已修**：三个应用级单例（连接名单 / 已登记标记 / 可连实例）改在 `bootstrap_app_shell` 里建在
>    `App` 的 owner 上（`connection_feed::init_app_signal_singletons`）。
>    `e2e/specs/mock-lane-switch.spec.ts` 的第二个用例已从 `test.fixme` 恢复为 `test` 并通过。

**上限**：`MAX_LANES = 8`（`crabmate_client_api::lane_attach`，与 TUI 同源，也与 `Alt+1..8` 对齐）。
满了之后新建动作直接拒绝 —— 不建 slot、不接线；每条 lane 都带一整套 Effect 与后台轮询，
没有上限就是「点几次堆几条」。

与 TUI 的对应：

| TUI | Web（懒建方案） |
|---|---|
| `TuiApp.lanes: Vec<LaneApp>` | `LaneSet.lanes: Rc<RefCell<Vec<Rc<LaneRuntime>>>>` |
| `active: usize` | `active: RwSignal<usize>` |
| `/lane attach` → 造 `LaneApp` 并 push | `add_lane` → 造 `LaneRuntime` + `mount_to` 挂一棵树 |
| 每帧按 `active` 取 `lane_mut()` | 每棵子树各自绑定建出来那一刻的句柄，切换只改 `display` |
| `FocusChanged` → `LanePhase` 转移 | 同一份 `lane_phase`（已在 `client-api`）+ 写回 `signals.phase` |

**S1 已处理**：`provide_context(ChatShellLeptosContext)` 已随视图一起搬进 `app_body`，按 owner
生效 —— `mount_to` 为新子树建 owner，ctx 就属于那条 lane。N=1 时 owner 仍是 `App` 的，行为与
改造前一致。

**S2 还要回来改两处**（`app_body` 头部注释里也记了）：

- `wire_layout_toggle_hotkey` 现在**每条 lane 各注册一次**全局监听，挂到第二条 lane 后会一次
  按键 toggle 两次 —— 热键是窗口级的（toggle 的是 `shell_ui.editor_layout_mode`），届时应上移回 `App`。
- `IdeSession` 暂按 lane 建（它绑着这条 lane 的 chat 信号）；若确认编辑器状态该是窗口级
  （切 lane 不该丢编辑器标签），再上移。

## 6. 左栏「连接」区

### 6.1 现状（已核对）

| 项 | 现状 |
|---|---|
| 数据面 | `connection_feed::connection_feed()`：应用级 signal，来自壳 `list_connections` + `connections_changed` |
| 行粒度 | **一行 = 一个 window label**（`ConnectionSnapshot.key` 恒等于 label） |
| 行渲染 | 已有 `status_glyph(status)` + `row_meta(&snapshot)` + `connection_display_name(&snapshot)` |
| 点击 | `tauri_focus_connection(key)`；单连接时进「设置 → 连接」 |
| 状态 / 未读 | 本窗口上报 `report_connection_status` / `_unread` / `_metrics`，先登记再上报 |
| 高亮 | 按 `authority` 与自己的 `api_base` 比对（广播的 `active` 恒 `false`） |

### 6.2 列表项形状：直接复用 `ConnectionSnapshot`

多 lane 的「一条 lane 的一行」**就是** `ConnectionSnapshot`，不要新造 `LaneRow`：

- 共享层已经把它定义成「TUI `LaneEntry` 与 Desktop 连接列表的公共形状」；
- 字段齐：`display_name` / `authority` / `active` / `status` / `unread` / `context` / `busy_secs`；
- `pulse` 刻意不在里面（§2.1-D）—— 呼吸效果归 CSS。

单 lane 时就是长度为 1 的切片，与今天的渲染输入完全一致 → N=1 等价重构天然成立。

### 6.3 外观口径

照 §2.1-B 的表实现，优先级从高到低：

1. **标题**：`连接 [n/8] ▲k`（溢出补 `↓m`）。容量与待审批数合并进标题 —— TUI 已把原先状态栈里的
   `lanes` 段撤掉，理由写在 `lane_title` 注释里：两项的操作对象都在左栏这块，留在别处只是第二份读数。
2. **行**：整行底色条、一行恒定高、行距与会话列表一致；当前 lane 用 selection 底色，其余抬升一档；
   等审批的**非当前** lane 用告警前景（颜色之外还有 `▲` 兜底，无色模式仍可读）。
3. **行尾读数**：`62% 12s •`，宽度不够从左边丢，未读点最后丢。
4. **动画**：`●` 只做亮度呼吸，不换字形、不改行宽。

### 6.4 命中：行索引 ↔ lane 索引单一来源

TUI 把这件事写成一个函数对（`layout::lane_shown_len` / `lane_index_at`），注释写明理由：
「一行对应哪条 lane 与鼠标命中必须是同一份算法，否则会出现点第 2 行切到第 3 条」。

Desktop 同样要求：**列表渲染顺序**与**点击命中换算**共用一个函数；且只映射前 `lanes.len()` 行
（待就绪 / 可连实例分组不参与映射）。桌面的可连实例分组可点，但它走的是另一条路径（连接动作），
不参与「切 lane」的索引换算 —— 这两件事不能共用一个下标。

### 6.5 数据源：三个方案

| 方案 | 做法 | 代价 / 风险 |
|---|---|---|
| A 扩壳 IPC | `register_connection` 带 lane 列表，`report_connection_*` 带 lane 序号，壳按 `(label, lane)` 存 | 动 `desktop-tauri` + `crabmate-connect` 命令面；路线 A 与 B 共用一条通道，契约变大 |
| B 前端自渲染 | 左栏只画**本窗口**的 N 条 lane（数据在 lane 容器里）；别的窗口仍一行一条来自注册表 | 前端为主，壳不动；跨窗口看不到对方的 lane |
| C 混合拼接 | 本窗口 N 条前端画，其他窗口一行一条，两段拼一张表 | 行语义、排序、去重口径都要另外定 |

**推荐 B**：

- lane 列表是**纯前端状态**，与 issue「lane 级状态留在前端」一致；
- 不扩壳契约，Android / Web 零改动；
- 路线 A 的名单能力原样保留，两种入口共存；
- **降级必须写明**：别的窗口有哪几条 lane，本窗口看不到；要切过去得先切窗口。这条要进
  [`client_capability_matrix.md`](client_capability_matrix.md) 的 Desktop 单元格。

按 B 实现后，点击分三种（顺序照 `decide_attach`）：点本窗口某条 lane → 切 active；
点可连实例 / 新地址 → `decide_attach` 决定 `Append` / `SwitchToSame*`；点别的窗口那行 → 仍 `focus_connection`。

### 6.6 键位：与 TUI 同表

| 键 | 行为 | 单 lane 时 | 备注 |
|---|---|---|---|
| `Alt+→` / `Ctrl+N` | 下一条 lane（循环） | no-op | TUI 两键**同一实现**：给同一结果，才不必记两套语义 |
| `Alt+←` | 上一条 lane | no-op | |
| `Alt+1..8` | 直达第 n 条（1-based，序号与列表自上而下一致） | no-op | 越界**不切**、只提示 |
| `F4` | 跳到下一条**在等审批**的 lane（循环，没有就留原地并提示） | 无审批时提示 | 后台审批的唯一线索，别省 |

实现注意：TUI 这些键是**精确匹配修饰符**（不放宽成 `contains`），且都在「进入 lane 之前」拦截 ——
不能让切换键漏进输入框。桌面同理：全局监听，输入框获得焦点时也不放行给输入框。
`Alt+数字` 在部分 Linux 桌面环境被窗口管理器占用（TUI 不受影响），这一点要进冒烟检查项。

**落地（2026-11-21，S3a）**：`app/lane_hotkeys.rs`。按键 → 意图是**纯函数** `intent_for`
（修饰符用 `KeyMods` 判定，精确匹配，四组用例钉住 `Alt+9` / `Alt+Shift+→` / 输入法组合都不算），
`web_sys::Event` 那层只做提取。一处偏离上表：**只有真的换了前台才 `preventDefault`** —— 越界与
单 lane 时没有动作，就不该顺手把按键的默认行为吞掉。

### 6.7 入口口径（已定：不保留「开新窗口」）

**Desktop 的「连接」动作一律解释为「在**当前窗口**新建一条 lane」**，不再开新 WebView。
现有两处入口都改成同一个动作：

| 位置 | 现状 | 改动 |
|---|---|---|
| `settings_sections.rs` 的 `SettingsConnectionNewWindowBlock` | `tauri_open_connect_window(None)` 建窗 | ✅ 改调 `lane_actions::new_lane(None)` |
| 左栏「可连实例」行的「连接」 | 建窗并交接基址 | ✅ 改调 `lane_actions::new_lane(Some(base))`，基址作为那条 lane 的起点 |

**落地（2026-11-21，S2）**：两处入口都只改 `on:click` 一行，动作经窗口级注册表
`app/lane_actions.rs` 转发 —— 每窗口唯一，同 `confirm_dialog::register_shell_confirm` 的形状；
两个入口都藏在 `SettingsPageViewInput` / `CtlRow` 深处，穿参要改 6 处签名。前端那两个 wrapper
（`tauri_open_connect_window` / `tauri_open_connection`）连同它们的 `invoke` 声明已删除，壳侧
命令与注册保留（调试期退路）。

**为什么现在就定**：它决定了第 2 步（`add_lane`）的唯一调用者，也决定了 §5 的视图渲染
必须支持「lane 数会变」—— 如果留着开新窗口，「新建 lane」就没有必然的入口。

壳侧与遗留能力：

- `desktop_connections::open_connect_window` / `open_connection` **暂不删除**：同一
  workspace 开多个窗口在调试期仍是唯一手段，删掉等于把一条已经验过的退路先砍了。
  但它们**不再由 UI 调用**，所以改完要检查它们是否仍被 `invoke_handler` 注册（保留注册、
  仅去掉前端调用点）。
- `focus_connection`（点别的窗口那一行切过去）**保留**：历史多窗口场景仍要能切回去，
  且它与「切 lane」是两条不同的路径（§6.5 的点击三分支）。
- [`desktop_multi_window.md`](desktop_multi_window.md) 的路线 A 降级为「**历史能力**」：
  代码与文档保留，但不再有产生新窗口的用户入口。

**仍未定**：那两个建窗命令最终是删还是留（建议先留，等 §5 落地并验证「一个窗口能装下
多 workspace」之后再评估）；以及去掉入口后 capability matrix 里「多窗口」那一格怎么写。

## 7. 存储与凭据：域要跟 lane 走

[`desktop_multi_window.md`](desktop_multi_window.md) §4 已预告：「若改走单窗口多 lane，存储域必须跟着
**连接作用域**走而不是跟着窗口走，那时 `key` 恒等于 label 这条等式本身也要拆」。

- 键形态不变：`<基础键>#<key>`，连接级再加 `@<fp>`（`api/storage_scope.rs` 只拼不拆）。
- **改的是 `key` 的来源**：从「壳注入的 `__CM_CONN_KEY__`」改成「`ScopeHolder` 上的连接身份」。
  单 lane 时两者相等（注入值 = label），零行为变化。
- lane 后缀不要用 `#` 或 `@`：`SCOPE_SEP` / `SCOPE_FP_SEP` 已占；建议 `-l<n>`（label 字符集允许 `-`）。
- 需要补的正是 #558 项 3b：`read_scoped_local_storage_raw` / `write_api_base_local_storage`
  / `clear_local_storage_web_api_bearer` 补 `*_on(holder, …)`。键拼接是纯逻辑可单测。
- **别把签名读成「全隔离」**：只覆盖 base URL 与 Web API Bearer；GitHub token 与模型密钥是账号级。

**落地（2026-11-21，S2）**：`-l<n>` 已在 `LaneSet::lane_storage_label` 实现（第一条新 lane 是
`<注入 key>-l1`；没有注入 key 时不分域，与主 lane 同口径），读写侧的 holder 维度
（`read_scoped_local_storage_raw_on` / `write_api_base_local_storage_on` /
`clear_local_storage_web_api_bearer_on`）此前已随 #558 项 3b 落地 —— 所以新 lane 不会与主 lane
抢同一批键。

## 8. 状态、未读与退出

### 8.1 状态：字形已一致，耗时来源要改

- 字形与枚举不用动（§2.1-A/B）。
- **`busy_secs` 的来源必须改**：现在由壳按 label 记 `started_at` 并跑 1s ticker
  （`desktop_connections::Entry` / `spawn_busy_ticker`），因为它要服务「别的窗口读这条连接的读数」。
  一个窗口 N 条 lane 时壳只有一行，秒数会混。
  多 lane 下由**前端按 lane 自己的开始时刻**算（TUI 同形：`turn_started` 在 lane 内），
  壳侧 ticker 只保留给「别的窗口读我这一行」的单 lane 路径。
  单 lane 时两者同源（同一个开始时刻），仍可零行为变化。

### 8.2 未读：`active` 也算一种「失焦」

现在：`connection_unread_after(窗口focused, busy, prev)`。多 lane 后：

```text
可见 = 窗口 focused && 该 lane 是 active
next_unread = connection_unread_after(可见, busy, prev)
```

语义与 TUI 等价（TUI：后台 lane 有新输出 → 置位；切过去 → 清零）。

**单一入口**：`set_active(i)` 负责四件事 —— 改 `active`、把目标 lane 的 holder 提为 `ACTIVE`、
清零目标 lane 未读、触发受影响两条 lane 的状态重算。TUI 的原注释写得很直白：不逐条写在切换路径上
（`Alt+←/→`、点击、`F4`、`/lane attach` 各有出口），漏一处就漏一次清零。

### 8.3 `Quitting`：桌面不产生，但要说明关闭语义

共享枚举里 `Quitting`（`×`）的注释明确写「Desktop 的多窗口不产生这个状态：一个窗口要走就真的走了」。
多 lane 后「关闭一条 lane 时回合怎么办」**已定案**：关闭 = **复用既有的用户中止链路**
（`stream_user_abort` 的 `apply_user_abort_of_inflight_stream` + `POST /chat/stream/{job_id}/cancel`），
不引入第三种关闭语义。

理由是通用性而不是省事：让「关闭」与「用户按停止」走同一条链，桌面就不需要实现
`Quitting`（`ConnectionStatus` 里那句「Desktop 的多窗口不产生这个状态」自然延续），
`status_glyph` 的 `×` 分支保留但走不到；而 Android / Web 单连接、多窗口关窗口、多 lane
关一条，全都成了同一件事：**中止 + 销毁**。

阶段由 `crabmate_client_api::lane_phase::LanePhase` 表达：`CloseRequested` → `Closing`
（收尾中，`keeps_background_work()` 已为 false）→ `CloseSettled` → `Closed`
（终态，吸收一切迟到事件）。

被否的替代方案是「请求退出 → 等本轮结束」：`Quitting` 的 `×` 在后台 lane 没有承载物
（左栏只画一行），等于实现一个看不见的状态。

选「立即 abort」时，还要照 TUI 的「最后一条 lane」特例：`client_capability_matrix.md` 记着
「只有最后一条 lane 的后果要明说」—— 桌面这里对应「关掉最后一条 lane = 回到未连接 / 进设置页」，
提示文案要写清，别留一个空壳。

### 8.4 审批可见性

- 判定改为 `窗口 focused ∧ 该 lane 是 active`；
- 前台 lane 弹 `ApprovalModal`（既有行为）；
- 后台 lane **不弹**，靠两处表达：左栏 `▲` + 系统通知（口径见
  [`0002`](../adr/0002-android-approval-notification-foreground-keepalive.md) 与 `desktop_multi_window.md` §5）；
- TUI 还有**第三处**：`announce_background_approval` 在当前 lane 留一句提示（issue #337，
  因为后台浮层画不到、而通知可能被系统静音）。桌面**暂不补这一处**（#561 决策）：只靠
  左栏 `▲` + 系统通知。**遗留风险写明**：系统静音 / 通知被拒时，后台审批在当前视图内
  没有任何提示，左栏 `▲` 是唯一兜底 —— 这一条要进 `client_capability_matrix.md` 的
  Desktop 单元格，不能只留在设计文档里；
  将来若要补，位置是「事件 → 多 sink」（§11 已决项 4），**不要**在
  `connection_report` 里再手写第三处（那会让「审批变待审批」这件事没有统一广播点）；
- 未读口径继续复用 `connection_unread_after`；
- 不引入审批队列（#460 仍暂缓）。

## 9. 推进顺序

| 阶段 | 内容 | 验收 | 阻塞 |
|---|---|---|---|
| 0 | 合并 PR #563 | CI 全绿 | 无 |
| 1 | 本文 + `desktop_multi_window.md` §4/§13 交叉引用 + `AGENTS.md` 文档表 | 文档链接与行长门禁 | 无 |
| 2 | #558 项 3b（存储域 per-holder）→ 1b（holder 挂 `AppSignals`）→ 7（thinking fp per-scope） | 单测覆盖键拼接；单连接手测零变化 | 无（可并行于 1） |
| 3 | lane 容器 + 接线拆分 + `set_active` 单一入口，**N=1 等价重构** | 818 frontend 测试全绿；单连接手测零变化 | 2 完成 |
| 4 | 接第二条 lane（`decide_attach` 驱动）+ 左栏切 lane（方案 B）+ **外观口径落地**（标题 / 行 / 读数优先级 / 命中） | 两条 lane 会话 / 工作区 / 草稿互不串；外观与 TUI 逐项对照 | 3 完成 |
| 5 | 键位（`Alt+1..8` / `Alt+←→` / `Ctrl+N` / `F4`）+ 保活与后台降频 + 审批按前台 lane | issue 验收第 2、4 条（手测） | 4 完成 |
| 6 | `ACTIVE` 收窄门禁（#558 项 5/6） | 门禁脚本进 `check.sh` | 4 完成（否则一加就红） |
| 7 | 收尾：capability matrix、`shell_smoke_runbook.md`、CHANGELOG | — | 全部完成 |

**阶段 3 的约束（实测，别重走）**：容器（`LaneSet` + `current` / `set_active`）在没有
第二条 lane 之前**没有任何消费者** —— `clippy -D warnings` 会把它们判成 `dead_code`，
而补 `#[expect(dead_code)]` 正是 #558 当初把 1b 标为「故意没做」要避开的形态。所以
阶段 3 必须**与「第二条 lane 从哪来」同批**落地（左栏点击 → 新建 lane 的入口，也就是
阶段 4 的前半），不能先落一个空容器再等消费者。

可独立先做的只有「**切前台的唯一写入口**」：`connection_scope::activate(holder)`
（已被 `install_new_holder` 消费，单 lane 时等于没动）。它把「谁当前台」收成一条写入
路径 —— 将来 `set_active` 直接调它，不必再碰 `ACTIVE` 这个 `thread_local`，否则切换
会散成两处（容器改下标一处、指针另一处），漏一处就是「下标说 A、请求发到 B」。

CHANGELOG 从阶段 4 起才需要条目（用户可见行为从这里开始）；阶段 2、3 是纯内部重构。

## 10. 风险清单

- **闭包捕获**：把 lane 级字段包成一层结构的尝试都会被 `Send + Sync` 打回（§4.2）。
- **窗口级字段声明两处**：`WindowSignals` 与 `AppSignals` 各声明一遍，加第 7 个窗口级字段要改两处，
  漏改**不报错**（`window_signals.rs` 文件头已写）。
- **`initialized` 归属未定**：`new_lane` 里它是新建的（每 lane 一份）。若语义是「启动水合完成」，
  后台 lane 要各自水合；若是「壳起来了」，应是窗口级。**实施前必须定**（§11）。
- **照抄 TUI 会错的三处**：会话刷新合并 / 同 serve refresh fanout / 只渲染当前 lane（§2.1-D）。
  这三条都是 TUI 为单进程多 lane 做的妥协，搬进「每 lane 一条 HTTP 流」会引入新 bug。
- **耗时读数串台**：若沿用壳的 label 级 ticker，N 条 lane 会显示同一个秒数（§8.1）。
- **`Alt+数字` 被窗口管理器截走**：Linux 桌面常见，需在冒烟步骤里显式验证（§6.6）。
- **门禁**：新代码守 CCN ≤ 10、函数 nloc ≤ 200、Rust 文件 ≤ 920 行；容器与接线拆分容易把
  `app/mod.rs` 与 `app_shell_wiring` 顶穿，优先新开文件。
- **门禁顺序**：#558 项 6 的门禁必须等 3b/4 迁完再落地，否则 CI 立刻红。

## 11. 开放问题

### 已决（2026-11-21 实测）

1. **`AppSignals::initialized` → lane 级（保持现状，不改归属）**。
   全仓唯一把它置 `true` 的地方是 `chat/session_storage.rs` 的会话水合完成回调；
   它门控的是 composer 发送按钮、workspace / `/status` / tasks 的首次拉取与 GitHub
   刷新 —— 全是「这个壳的**会话域**就绪了吗」。窗口级消费者（`user_prefs_sync`）借用
   第一条 lane 的就绪信号即可，不必为它新增一个窗口级标志。
2. **`connection_feed()` 不再收 `api_base`**（已改）。注册那一刻读前台默认
   （`api_base_url()`，会顺手完成基址水合），语义变成「壳里这一行代表本窗口**当前
   前台**那条 lane」。原注释写着「同一窗口内 `api_base` 不变」——多 lane 下这条前提
   不成立，去掉入参顺带消掉了「谁先调用谁决定基址」这个隐含时序依赖。切 lane 后由
   容器重新登记（§6.5），壳侧 `register_connection` 是幂等覆盖。
3. **关闭一条 lane 的语义**（原问题 4）→ 复用用户中止链路，不产生 `Quitting`；
   阶段用 `crabmate_client_api::lane_phase::LanePhase` 表达（`CloseRequested` →
   `Closing` → `CloseSettled` → `Closed`）。详见 §8.3。
4. **三个开放问题共享同一张表**：降频 / 关闭 / 审批可见性都是 `LanePhase` 上的查询
   （`!is_foreground()` / `CloseRequested` / `can_present_modal()`），所以那张表已
   先落在 `crates/crabmate-client-api/src/lane_phase.rs`（纯逻辑 + 9 条单测，TUI 可
   按 `active` 映射）。它不需要第二条 lane 就有消费者，因此不落入「空容器 =
   dead_code」那个坑。

5. **后台降频力度 = 前台间隔 ×10**，切回前台**立即补拉一次**。分类**规则**已定（事件
   驱动常驻、定时类受 `is_foreground()` 门控），逐项清单要按 `wire_*` 填 —— 清单属
   实施细节，不再是待决问题。
6. **后台审批暂不补第三处提示**：只靠左栏 `▲` + 系统通知。这是 owner 的价值判断
   （通用性分析只回答了「若加，加在哪」）；遗留风险与将来的落点写在 §8.4。

### 仍需答复

（设计层面已无阻塞项。）剩余是**实施清单与实测**：定时类 Effect 的逐项分类、10× 之后
的实际新鲜度、「第二条 lane 的入口」形态（§6.5 方案 B —— 落地前需 owner 确认，它决定
阶段 3 的容器什么时候有消费者）。

## 12. 非目标

- **不做分栏（并排显示）**：需要单独设计布局、断点单源约束与窄屏退化。
- **不砍掉路线 A**：多窗口的代码与文档保留，但**入口已移除**（§6.7）—— 「连接」一律在
  当前窗口新建 lane；`focus_connection` 仍保留，历史多窗口场景还能切回去。
- 不改 Android / Web 的单连接语义，不做「浏览器标签页多连接」。
- 不改 TUI 的 lane 实现，不把 `LaneApp` 搬进 WASM；下沉的只是判定（已完成）。
- 不引入审批队列（#460 仍暂缓）。
- 不重新设计 lane 列表视觉：沿用 `ConnectionSnapshot` + 既有字形 + 既有 token。
