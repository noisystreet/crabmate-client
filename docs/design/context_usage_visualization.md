# 上下文用量可视化页面方案（方案 B：全屏页 + 矩形图）

状态：Implemented（已实施；只读可视化，未改线协议）
日期：2026-10-11
范围：`frontend/` 业务 UI（Web / Desktop / Android 共用）；只读可视化，**不改线协议、不引依赖**
关联：[ADR-0005](../adr/0005-status-bar-context-watermark.md)（状态栏上下文水位语义）、[`prompt_tokens.rs`](../../crates/crabmate-client-api/src/prompt_tokens.rs)（用量 DTO）

## 背景

状态栏的「会话上下文」芯片（[`StatusBarContextChip`](../../frontend/src/app/status_context.rs#L321-L382)）目前是一个**纯展示**元素：根节点是 `<span class="status-chip status-chip-context">`，只有 `prop:title` tooltip，**没有任何点击事件或回调**。用户希望点击它跳转到一个「上下文窗口用量可视化页面」，像 GNOME 磁盘用量分析器（baobab）那样直观看到各部分占比。

同时，`frontend/` 现状有两个硬约束决定实现形态：

1. **无路由库**（无 `leptos_router`）：多视图是自研的「`RwSignal` + `<Show>` 条件渲染」；唯一的自研 hash 路由只服务设置页（`#/settings/<section>`）。
2. **无图表库**（无 plotters / chart.js / d3）：唯一可视化是 CSS `<div>` 进度条（[`StatusBarContextMeter`](../../frontend/src/app/status_context.rs#L281-L313) + [`status.css`](../../frontend/styles/status.css#L326-L361)）。

因此本方案的形态是「**新增一个全屏页（复用设置页外壳）+ 纯 CSS 矩形图**」，不引任何新依赖。

## 目标与非目标

**目标**

- 点击状态栏上下文芯片 → 打开一个**全屏**用量可视化页（方案 B）。
- 用**矩形图**（横向堆叠条 + 分段矩形色块图例）直观展示窗口内各部分的 token 占比。
- 在现有桶之外，**顺带展示**当前芯片忽略的 `tool_schema_tokens` / `reserved_output_tokens`（零契约成本的信息增量）。
- 复用现有数据链路（`status_bar_context_axis` + `context_axis_apply_watermark`），不新增 Server 字段。

**非目标（已评估）**

| 非目标 | 原因 |
|---|---|
| **环形图 / 饼图** | 用户明确指定用矩形图，不做环形 |
| **baobab 式逐层下钻（treemap 点进一层再点一层）** | 数据只到「聚合桶」级，**没有**逐消息 / 逐工具 / 逐附件 token 明细；且明细字段的契约权威在 Server，本仓不发明线协议字段 |
| **引图表库 / 路由库** | 与仓库「无依赖、CSS 自绘」的既有取向冲突，且现有桶数量少、矩形条足够 |
| **改 Server 契约加明细字段** | 超出本仓边界（AGENTS.md 明文：本仓不发在线协议字段） |
| **写入 / 修改任何用量数据** | 本页只读 |

## 数据契约盘点

### 可用字段（来自 `TiktokenPromptTokensSnapshot`）

| 字段 | 含义 | 当前芯片是否使用 |
|---|---|---|
| `used_input_tokens` | 已用（同口径，含每轮固定开销） | ✅ 分子 |
| `max_input_tokens` | 模型原始窗口（128k / 200k / 1M） | ✅ 分母 |
| `message_tokens` | 会话消息累计 | ✅（经反推参与拆分） |
| `attachment_tokens` | 附件注入 | ✅（经反推参与拆分） |
| `tool_schema_tokens` | 工具定义占用 | ❌ **本方案新增展示** |
| `reserved_output_tokens` | 为输出预留的窗口 | ❌ **本方案新增展示** |
| `provider_input_tokens` | 服务商上报的实际输入 | ❌（当前未用） |
| `counting_source` | 计数来源标记 | ❌（当前未用，可作脚注） |

### 客户端反推的两个派生量

- **固定开销** `F = used − (message_tokens + attachment_tokens)`（见 [`context_axis_split`](../../frontend/src/app/status_context.rs#L153-L160)）。服务端 `used = message + tool_schema + attachment + vendor_overhead`，但 DTO 不含 `vendor_overhead`，故只能反推。
- **「其余固定开销」** = `F − tool_schema_tokens`（当 `tool_schema_tokens` 可用时）。**这是本方案相对现状的细分增量**：把原来一个灰段 `F` 再拆成「工具 schema（显式字段）」+「网关 / 其余开销（反推）」。

> ⚠️ **待实现前确认**：`reserved_output_tokens` 与 `max_input_tokens` 的包含关系（是否已从 `raw_cap` 中扣除）需以 Server 口径为准；本仓不臆测。若已扣除，则「预留输出」段不在分母内，只能作为页面上的旁注指标，而非堆叠条的一段。

### 不可得数据（决定了下钻做不了）

- **逐消息 token**：`message_tokens` 是总和，无法回答「这条 user 消息占多少」。
- **逐工具调用 token**：只有 `tool_schema_tokens`（定义占用），没有「某次 tool call 结果多大」。
- **逐附件 / 逐文件 token**：`attachment_tokens` 也是总和。

故 baobab 的树状 / treemap 下钻在本仓**无法实现**；矩形图只能是**单层分桶**。

## 方案 B：全屏页容器与入口

### 1）入口改造 —— 让芯片可点击

把 [`StatusBarContextChip`](../../frontend/src/app/status_context.rs#L321-L382) 从纯展示改为可点击，**对齐状态栏既有可点击 chip 范式**（如 agent 角色菜单触发按钮），而非自创交互：

- 保持 `<span class="status-chip status-chip-context">` 的视觉不变；
- 增加 `on:click`（置位页面信号）、`role="button"`、`tabindex="0"`、`on:keydown`（Enter / Space 触发，关闭时 Esc 返回）；
- 保留原 tooltip，并追加「点击查看用量明细」文案（走 i18n）。

> 备选：把根节点从 `<span>` 换成 `<button>`。需先核对 `status.css` 里 `.status-chip` 系列的 `button` 默认样式重置，避免布局偏移；若成本高则用 `role="button"` 方案。

### 2）页面容器 —— 仿设置页全屏外壳

复用设置页的成熟范式，最大化降低新代码：

- **外壳**：`.settings-page*` 系列（[`modal.css`](../../frontend/styles/modal.css) 内），提供全屏铺满 + 头部（标题 + 返回/关闭）+ 内容滚动区。
- **模态语义**：如需 `role="dialog"` / `aria-modal`，**必须**经共享壳 [`FocusableModalPanel`](../../frontend/src/app/focusable_menu.rs#L60-L101) 发出（`check-modal-shell.sh` 门禁：`role="dialog"` 只允许在该文件出现）。
- **视图组件**：新增 `ContextUsagePageView`（单文件 [`frontend/src/app/context_usage_page.rs`](../../frontend/src/app/context_usage_page.rs)），结构对齐 [`SettingsPageView`](../../frontend/src/app/settings_page/view.rs#L29-L139)。
- **页面信号**：在 [`ModalSignals`](../../frontend/src/app/app_signals/modal.rs) 增设 `context_usage_page: RwSignal<bool>`（与既有 `settings_page` 同构），在 [`App`](../../frontend/src/app/mod.rs) 里与设置页一致**常驻挂载**，以 `class:settings-page-visible` 控制显隐（复用设置页范式，非 `<Show>` 条件挂载）。

> 与方案 A（侧栏 `SidePanelView` 变体）的取舍：侧栏空间窄、不适合放占比图与图例；本方案按用户指定采用全屏页（方案 B），侧栏方案仅作记录、不实施。

### 3）数据来源 —— 复用现有纯函数

页面不重算任何口径，直接调用现有实现，保证与状态栏芯片**同口径**：

- [`status_bar_context_axis`](../../frontend/src/app/status_context.rs#L179-L198) → 取 `ContextAxis { used, fixed, raw_cap, baseline_estimate }`；
- [`context_axis_apply_watermark`](../../frontend/src/app/status_context.rs#L227-L242) → 施加会话内滚动水位（单调不减）；
- 快照 [`TiktokenPromptTokensSnapshot`](../../crates/crabmate-client-api/src/prompt_tokens.rs) → 取 `tool_schema_tokens` / `reserved_output_tokens` 等附加桶。

**刷新时机（须在页面注明）**：`used` 只在回合末（`RUN_FINISHED`）或水合时刷新；流式进行中显示的是**上一回合末快照**。这是 ADR-0005 的已知时序特性，页面应显式提示，避免用户误以为数字卡住。

## 矩形图设计（核心）

### 主图：横向堆叠条（stacked bar）

一条 100% 宽、固定高度的矩形条，宽度轴 = `max_input_tokens`（`raw_cap`）。从左到右分段，每段宽 = `该桶 / raw_cap × 100%`：

```text
├─────────────────────────── 100% = max_input_tokens ───────────────────────────┤
[ 固定开销 F ][ 会话消息 ][ 附件 ][ 工具schema ][ 其余开销 ][ 预留输出 ][  剩余空间  ]
   （灰）      （brand）   （附色）  （附色）     （灰）      （斜纹）    （底衬）
```

分段与取数：

| 段 | 取数 | 说明 |
|---|---|---|
| 固定开销 F（含工具 schema 前的合并灰段） | `F`，或（当选 `tool_schema_tokens` 可用时）拆为下两段 | 每轮都付的底衬 |
| 会话消息 | `message_tokens` | 会话连续增长的内容 |
| 附件 | `attachment_tokens` | 注入附件 |
| 工具 schema | `tool_schema_tokens` | 仅当 `F` 可拆时单列 |
| 其余固定开销 | `F − tool_schema_tokens` | 反推的网关等开销 |
| 预留输出 | `reserved_output_tokens` | **仅当确认其在 `raw_cap` 内**时作为一段；否则降级为旁注 |
| 剩余空间 | `raw_cap − used`（再减去预留输出，若适用） | 底衬，非实占 |

约束：所有段宽之和不得超过 100%；任一段缺失（`None`）时该段宽度记 0 并在图例中标注「无数据」，**不隐藏图例**（隐藏会让用户误以为该桶为 0）。

### 图例：分段矩形色块（swatch 行）

主图下方一行或多行矩形色块，每个色块 = 一个桶，块内/块旁显示：**名称 · 绝对 token 数 · 百分比**。这与 baobab 的图例块同构，但**不可下钻**（点击无展开）。色块用 `prop:title` 提供完整 tooltip。

### 告警与水位标记

- **90% 告警线**：`used / raw_cap ≥ 90%` 时主图描红（复用 `--warn` / `--error`）。
- **滚动水位标记**：在条上叠加一条竖线标记「本会话峰值」（`watermark.used()`），与当前 `used` 对比，体现 ADR-0005 的滚动峰值语义。
- **基线估算态**：`baseline_estimate == true` 时整条改虚线描边（对齐 `status-context-meter-fill--estimate` 的既有表达），并提示「粗估，无服务端实测」。

### 无障碍

- 每个色块/段带可读文本（不单靠颜色区分）；
- 主图整体 `role="img"` + `aria-label` 汇总（如「已用 47.2%，窗口 128k」）；
- 键盘可关闭页面（Esc）。

## 实现接线（文件级改动清单）

| 文件 | 改动 |
|---|---|
| [`frontend/src/app/status_context.rs`](../../frontend/src/app/status_context.rs) | `StatusBarContextChip` 增点击入口（on:click / role / tabindex / on:keydown）；暴露只读取数 helper 供页面复用（避免口径分叉） |
| [`frontend/src/app/context_usage_page.rs`](../../frontend/src/app/context_usage_page.rs)（新增） | `ContextUsagePageView` 组件 + 矩形图子组件（纯 CSS） |
| [`frontend/src/app/app_signals/modal.rs`](../../frontend/src/app/app_signals/modal.rs) | 增 `context_usage_page: RwSignal<bool>` |
| [`frontend/src/app/mod.rs`](../../frontend/src/app/mod.rs) | 与 `SettingsPageView` 一致**常驻挂载**，以 `class:settings-page-visible` 控制显隐；并加入 `.shell-main` 隐藏联动 |
| [`frontend/styles/`](../../frontend/styles)（新增或追加） | 矩形图 / 色块 / 告警线样式；类名须有同 scope 规则 |
| `frontend/src/i18n/…` | 页面标题、段落名、提示文案（zh / en 双份） |
| [`scripts/css_literals_budget.txt`](../../scripts/css_literals_budget.txt) | 若新增 `frontend/styles/*.css` 文件或新增 `font-size` / 颜色 / `border-radius` 字面量，按棘轮同步预算 |
| [`scripts/css_tokens_allowlist.txt`](../../scripts/css_tokens_allowlist.txt) | 若用内联 `style="--context-viz-*-pct: …"` 注入段宽变量，须登记原因 |
| [`CHANGELOG.md`](../../CHANGELOG.md) | 用户可见新功能写入 `[Unreleased]` |
| [`AGENTS.md`](../../AGENTS.md) | 文档索引表登记本文件（设计文档路由） |

## 门禁清单

实施改动后必须全绿（均为 AGENTS.md 硬约束）：

| 门禁 | 关注点 |
|---|---|
| `bash scripts/check-css-contract.sh` | 新增类名必须在同 scope CSS 有规则，否则登 `css_contract_allowlist.txt` |
| `bash scripts/check-icons.sh` | **禁字面 `<svg>`**（除 `icon.rs`）→ 矩形图用 `<div>` + CSS，不要画 SVG |
| `bash scripts/check-css-literals.sh` | `font-size` / 颜色 / `border-radius` 字面量与 `css_literals_budget.txt` 棘轮一致 |
| `bash scripts/check-css-tokens.sh` | 内联注入的 CSS 变量登记 `css_tokens_allowlist.txt`；`var()` 回退不得含颜色字面量 |
| `bash scripts/check-css-breakpoints.sh` | 窄屏布局断点对齐 `MOBILE_LAYOUT_BREAKPOINT_PX = 768` |
| `bash scripts/check-modal-shell.sh` | `role="dialog"` / `aria-modal` 只能经 `focusable_menu.rs` 发出 |
| `bash scripts/lizard-rust.sh` | 任何函数 CCN > 10 即失败（取数与渲染须拆小函数） |
| `bash scripts/check-xml-comments.sh` | 注释内不得出现 `--`（本仓 `--token` 名易踩坑） |
| `bash scripts/check.sh` | 全量（含 frontend wasm32 clippy + ktlint） |

## 验证

- `make check`（或 `pre-commit run --all-files`）全绿。
- `make test-frontend`：新增纯函数（分段宽度计算）须有单测；取数口径复用现有 `status_context` 测试，不重复造。
- 手工：Desktop 与 Android 实机确认矩形图在窄屏（< 768px）不溢出、六套主题配色正常、Esc/返回可关闭、芯片点击可打开。
- **口径一致性核对**：页面显示的 `used / raw_cap` 与百分比必须与状态栏芯片**逐位一致**（同源纯函数）。

## 风险与未决

| 项 | 说明 | 处置 |
|---|---|---|
| 数据只到桶级 | 无法下钻，与 baobab 体验有本质差异 | 方案内明确为非目标；若产品确需下钻，须 Server 增字段（本仓不提） |
| `reserved_output_tokens` 与 `raw_cap` 关系未确认 | 影响「预留输出」是段还是旁注 | 实现前以 Server 口径确认，不臆测 |
| 刷新时机滞后 | 流式中显示上一回合末快照 | 页面注明；不为此改动水位逻辑（ADR-0005 明确不可删 `max`） |
| 芯片从 span 改可点击 | 可能影响状态栏布局 | 先核 `.status-chip` 的 button 重置；必要时用 `role="button"` |
| `tool_schema` 拆分的稳定性 | `F` 由反推得来，可能与真实分布有偏差 | 图例标注「反推」，与显式字段区分层级 |

## 附：与既有文档的关系

- 本方案是 ADR-0005「状态栏上下文水位」的**展示层延伸**：ADR-0005 定义「显示什么口径」，本方案定义「如何把一个口径展开成矩形图页面」。水位语义、同口径分子分母、回落时序**一律沿用 ADR-0005**，本方案不重新定义。
- 落地后在 [`CHANGELOG.md`](../../CHANGELOG.md) 记用户可见变更，并在 [`AGENTS.md`](../../AGENTS.md) 文档索引表登记本文件。
