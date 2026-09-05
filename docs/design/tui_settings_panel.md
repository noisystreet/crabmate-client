# TUI 设置面板（对齐 Desktop 设置）

> **状态**：方案（Proposed，未进入实现）。本文只规划，落地按 §8「分期」逐波提交。
> **范围**：`crabmate-tui`（`tui` 全屏为主，`repl` 仅入口引导）；设置**选项字段与语义尽量对齐 Desktop**（`frontend/` Settings），不追求 UI 像素一致。
> **关联**：[`remote_cli_tui.md`](./remote_cli_tui.md)（TUI 母方案）、[`client_capability_matrix.md`](./client_capability_matrix.md)（能力单元格，改动须同 PR 更新）、[`contract_pin.md`](./contract_pin.md)（契约钉点）、Server `docs/`（SSE / user-data / 配置权威）

---

## 1. 背景与现状

TUI 的「设置」能力目前分散在三个互不相通的载体里：

| 载体 | 能设什么 | 生命周期 |
|------|---------|---------|
| CLI / env（`main.rs` `Cli`：`--api-base` `--bearer` `--llm-api-key` `--llm-model` `--llm-api-base` `--no-keyring` `--yes`） | 连接地址、凭据、模型/密钥覆盖、自动审批 | 启动定死，进程内不可改 |
| 会话覆盖（`tui_mode/controls.rs` `/model` `/mode` `/role` → `main.rs` `SessionPrefs`；全屏状态行带 `*` 标记） | model / session_mode / agent_role | 进程级，重启即忘 |
| serve 默认偏好（`GET /status?view=shell` → `serve_defaults.rs`） | 只读展示 model / role / mode 默认 | 随 `/status` 拉取 |

Desktop（`frontend/` Settings）的设置不是这样：**字段实体与持久化由 serve 的 user-data 目录承担**（`/user-data/llm-overrides`、`/user-data/prefs`、`/user-data/mcp-servers`），密钥放本机钥匙串/Keystore（官方壳禁止明文 `localStorage`），每轮 `POST /chat/stream` 再按需携带 `client_llm.*`。连同一台 serve 的多个官方壳由此**设置同源、跨端共享**。

本方案的落点：给 `crabmate-tui tui` 加一个设置面板，让 TUI 的设置项**字段名、校验、持久化去向、随轮发送语义与 Desktop 对齐**——同一 serve 下 TUI 改完，Desktop/Web 设置页可见，反之亦然。

---

## 2. 目标与非目标

### 目标

1. 全屏 `tui` 新增 `/settings`（+ F2）设置面板：分区浏览 + 字段编辑 + 显式保存/放弃，取代「斜杠改 override」作为会话偏好主入口（斜杠保留兼容）。
2. 设置项清单与 Desktop Settings 对齐（§4 逐字段表），持久化尽量落到与 Desktop 相同的 serve user-data 与钥匙串槽位，而不是发明一套 TUI 私有格式。
3. 面板里能看到三层有效值，避免“看不见生效值”的困惑：本进程 override（`*`）＞ serve user-data 已存值 ＞ `/status?view=shell` server 默认（空 = 跟随 server）。

### 非目标

- **不**做 Desktop 的像素/导航样式复刻；终端内用现有 ratatui 组件体系。
- **不**实现 Desktop 中终端环境无意义的设置：CSS 主题/字体/背景光晕、IDE 编辑器（行号/换行/Tab/字体）、聊天字体排版、per-turn inject 旁注开关等——矩阵中 TUI 格已是 `no` 的项**不要反向“对齐”**（`client_capability_matrix.md` cell vocabulary）。
- **不**管理 `saved_models` 注册表与 `executor_llm`（Desktop 主/执行器拆分）；TUI 只对齐“当前主模型”的扁平字段，写回时对不管理的字段做**合并保真**（§6）。
- **不**在 TUI 里放明文配置文件存密钥；不把密钥 PUT 给 serve（同 AGENTS：官方壳密钥只走本机钥匙串）。
- 本次只写文档，不建新 ADR（无跨端策略冲突；§9 若实现期出现「改写 serve user-data 是否越界」的争议再单开 ADR-0004）。

---

## 3. 对齐原则

1. **语义对齐，不是 UI 对齐**：Desktop 每个字段对应「serve user-data 的哪个键 / 哪个钥匙串槽 / 随轮请求体哪个字段」，TUI 沿用同一套，用户迁移零成本。
2. **同源共享**：连同一 serve 时，非机密字段以 serve user-data 为准（设备无关）；密钥以本机钥匙串为准（设备相关）。TUI 写 serve 前先 GET 合并，避免覆盖 Desktop 独有字段。
3. **空 = 跟随 server**：与 Desktop 一致——override 字段为空就不写/不发送，回退 `/status` 默认（`settings_llm_open.rs` 的灌草稿语义同款）。
4. **三层显示，一层保存**：面板展示「override / user-data / server 默认」合成值；保存动作按字段归属写对应载体。
5. **密钥纪律不降级**：`api_key` 只进钥匙串（`com.crabmate.credentials`，`crabmate_client_api::SecretSlot`），不落盘明文、不进 serve user-data。

---

## 4. Desktop 分区 → TUI 对齐映射

### 4.1 分区级结论

Desktop 设置共 8 个分区（`settings_page`；`settings_modal` 旧弹窗已无开启点，只作参考）。逐区对齐结论：

| Desktop 分区 | TUI 对齐结论 | 依据 / 理由 |
|---|---|---|
| **Connection**（Web Bearer + serve API 基址） | **reduced**：面板内**只读展示**，编辑仍走 CLI/env/钥匙串回退 | 两者在启动期定死（`ServeClient` 由 `ConnectionConfig` 建一次），会话中改需重建 client+worker，不划算；语义与来源提示写进面板 |
| **Appearance**（locale/theme/bg_decor/show_turn_context_inject） | **no** | CSS/DOM/`matchMedia` 渲染，终端无对应；`show_turn_context_inject` 矩阵 TUI 格已是 `no`（L63） |
| **Llm（模型配置）** | **reduced→主模型扁平字段**：对齐 `client_llm.{api_base,model,temperature,llm_context_tokens,llm_thinking_mode}` + 密钥 | Desktop 的“注册表 + 主/执行器套用”面向多预设管理；TUI 只管理当前主模型这一份（§4.2 / §4.3） |
| **Tools**（readonly tool TTL 缓存开关） | **reduced**：对齐同名字段（§4.2） | 开关 = serve prefs 布尔 + 随轮发送 `readonly_tool_ttl_cache_secs`，语义一致即可用 |
| **Session**（SQLite 会话存储 + 字体） | **reduced**：SQLite 开关可读/切（serve 两布尔 + POST）；字体 **no** | SQLite 开关本来就是 serve 级（`/status` 驱动），字体属 CSS 渲染 |
| **Shortcuts**（只读快捷键说明） | **yes**：设置面板内一个「快捷键」信息分区 | 复用现有 `/help` 文案，集中展示（含本面板自身按键） |
| **Github** | **no** | 矩阵 L48 TUI=no（Device Flow 需外开浏览器/原生钥匙串/Cookie，终端都没有） |
| **Mcp** | **reduced（后置）**：全局开关 `global_enabled` + `tool_timeout_secs` 读/改；服务器行管理 **no** | MCP 实体在 serve user-data、stdio 在 serve 机拉起；Web UI 本身也只回 `has_*` 摘要、逐字段编辑走 JSON 导入（`user_data.rs` DTO），终端面板做行编辑收益低 |
| IDE 编辑器设置页（独立入口） | **no** | 内嵌 CodeMirror 才有；远程终端无 WebView |

### 4.2 TUI 设置面板的字段清单（对齐列）

按「模型 / 会话 / 工具 / 连接（信息）」分组。校验区间、空值语义、键名对照 Desktop 同名字段（引用前端证据见后）。

**模型（写 `/user-data/llm-overrides` 的 `client_llm.*`，合并保真）**

| 字段 | Desktop 对照键 | 控件 | 空值 / 默认 | 校验（对齐 `settings_commit.rs` / 草稿默认） |
|---|---|---|---|---|
| 模型名 | `client_llm.model` | 文本 | 空 = 跟随 server | 自由文本 |
| 网关 / API Base | `client_llm.api_base` | 预设下拉（server/ollama/deepseek/minimax/zhipu/moonshot/custom）+ 自定义 URL 文本 | `server` = 空（沿用服务端）；其余填 URL | 需 `http(s)://` 前缀（对齐 `client_llm_presets.rs` 与前端 URL 校验） |
| 温度 | `client_llm.temperature`（Desktop 随轮单独 `chat_temperature_override`） | 数值文本 | 空 = 跟随 server | `0.0..=2.0` |
| 上下文 tokens | `client_llm.llm_context_tokens` | 数值文本 | 空 = 跟随 server | 正整数 ≤ 10_000_000（`settings_commit.rs` 同区间） |
| 思考模式 | `client_llm.llm_thinking_mode` | 枚举 | `server` | `server / on / off` |
| API 密钥 | 钥匙串 `SecretSlot::ClientLlm`（不是 serve 字段） | password 编辑 + 清除 | 未设 = 跟随 server 服务端 `API_KEY` | 仅本机，不回显明文；有「已设」标记 |

**会话（写 `/user-data/prefs`，合并保真）**

| 字段 | Desktop 对照键 | 控件 | 空值 / 默认 | 说明 |
|---|---|---|---|---|
| Agent role | `prefs.cm_role`（`/status` `default_agent_role_id`） | 文本 | 空 = serve 默认 | 可用 role 列表以 `/status` 为准，无独立枚举端点则文本输入 |
| 会话模式 | `prefs.session_mode` | 枚举 | 空 = serve 默认 | `ask / plan / act`（与 `/mode` 校验一致） |
| 只读工具缓存 | `prefs.disable_readonly_tool_ttl_cache`（取反） | 开关 | 开 = 跟随 server | 关时随轮发送 `readonly_tool_ttl_cache_secs: 0` |

**连接（只读信息，编辑引导 CLI/env）**

| 项 | 展示 |
|---|---|
| serve API 基址 | 当前生效 `api_base`（来源标注：flag/env/默认） |
| Web Bearer | 已提供 / 来自钥匙串回退 / 未提供（`--no-keyring` 时提示）——**只读**，注明「Web Bearer ≠ 模型 API_KEY」 |

**本地（非 Desktop 对齐项，TUI 自有，进面板「本地行为」小分区）**

| 项 | 说明 |
|---|---|
| 自动审批（`--yes`） | 会话内切换等价物；仅影响放行决策，执行仍在 serve |
| thinking 默认折叠 | 读当前全局态，可一键切默认（进程级） |

### 4.3 明确不做的字段（避免误配）

- `saved_models[]` 注册表增删改、`executor_llm.*`、`execution_mode`：TUI 写 `llm-overrides` 时**原样合并保留**，不展示、不编辑。
- 每台 MCP 服务器的 command/args/env/cwd/url/Bearer、GitHub 全部字段、字体/主题/背景、IDE 全字段、`show_turn_context_inject`。
- 会话「切换会话存储 SQLite」归 §7 后置（需确认 POST 端点契约）而非 W1/W2。

---

## 5. 交互与入口设计（草案）

### 5.1 入口

- `tui` 全屏：输入 `/settings`（加入 `controls.rs` `Control` 枚举解析）；同时绑全局键 **F2**（不占用已用 Ctrl+* 组合）。Esc 逐层关闭（编辑态→面板→输入态）。
- `repl`：不加面板；`/model /mode /role` 已覆盖。仅 `/help` 文案里提示“设置面板见 `crabmate-tui tui` /settings”。

### 5.2 面板形态

复用现有「审批浮层」同款绘制管线（`Clear` + 独立色块 + 边框块，`render.rs`），做成**居中或右侧覆盖的大浮层**（宽取 `min(终端宽-4, 96)`、高取 `终端高-4` 上限；终端过窄时降级为全宽行列表）：

```
┌─ 设置（对齐 Desktop）───────────────────────────────┐
│ 分区         字段 / 当前生效值              override │
│ ┌────────┐  模型                                  │
│ │连接     │  [模型名]      deepseek-chat*        │
│ │模型(高亮)│  网关          server / …(预设)        │
│ │会话     │  温度 / 上下文 / 思考 / API密钥(已设)    │
│ │工具     │  会话                                │
│ │本地行为 │  role / mode                          │
│ │快捷键   │  工具                                │
│ └────────┘  本地行为                              │
│  当前生效标记： * 本进程override · 无标记=serve已存  │
│ [↑↓] 选择 [Enter] 编辑 [Tab] 切换分区              │
│ [S] 保存全部 [Esc] 放弃（有改动先确认） [F2] 关闭    │
└───────────────────────────────────────────────────┘
```

- 左列分区导航（仿 Desktop NavRail），右列当前分区字段行；每行展示合成生效值与 override 来源。
- 行级 Enter → 该字段编辑行（复用现有输入缓冲编辑原语：文本直输、数值过滤、枚举 ←→ 循环、开关回车翻转）。
- 底部操作条固定：`S` 保存全部（按字段归属分写 PUT / 钥匙串）、`Esc` 放弃。**有脏字段时 Esc 先弹确认**（对齐 Desktop 的 close-guard 语义，避免“改了以为存了”）。
- 回合进行中打开：只读展示，保存禁用并提示（与审批浮层优先级一致的防叠原则）。

### 5.3 三层显示规则

| 情形 | 面板行显示 |
|---|---|
| `SessionPrefs` override 非空（含 `*`，来自斜杠/CLI） | override 值 + `*` |
| 无 override，serve user-data 有值 | user-data 值 |
| 都无 | `(跟随 server)`——附 `/status?view=shell` 取到的默认值（可读） |

保存后：override 清空（值已落到 user-data 层），行回到「user-data 值」显示；仅**本地行为**类字段留在进程内。

---

## 6. 数据与持久化语义

### 6.1 三层来源

1. **本进程 override**：`SessionPrefs`（现由 CLI/env/斜杠写入）。
2. **serve user-data**（设备无关，Desktop/Web/TUI 共享）：
   - `client_llm.{api_base,model,temperature,llm_context_tokens,llm_thinking_mode}`、`executor_llm.*`、`saved_models[]`、`execution_mode` → `PUT /user-data/llm-overrides`（前端 `api/user_data.rs` L219-226 已在用）。
   - `cm_role`、`session_mode`、`disable_readonly_tool_ttl_cache` → `PUT /user-data/prefs`（前端 `user_data.rs` L210-217；防抖 PUT 在 `user_prefs_sync.rs`）。
3. **/status server 默认**：`GET /status?view=shell`（`serve_defaults.rs` 已解析 model/role/mode）。

### 6.2 合并保真（关键）

两个 PUT 端点都是**全量 DTO**。TUI 保存前必须先 GET 现值，**只改自己管理的键，其余原样回写**：

- `llm-overrides`：改 `client_llm.*`；`executor_llm` / `saved_models` / `execution_mode` 原样保留 → Desktop 的模型注册表不会被 TUI 覆盖。
- `prefs`：改 `cm_role` / `session_mode` / `disable_readonly_tool_ttl_cache`；`locale` / `theme` / 布局 / IDE 字段等原样保留。
- 保存窗口冲突：Desktop 同时开着保存时存在最后写入者覆盖窗口；缓解 = 保存前逐字段重读再 PUT（先做简单版：整表单保存前读一次合并）。

### 6.3 密钥

`client_llm.api_key` 不进 serve：写本机钥匙串 `SecretSlot::ClientLlm.keyring_account()`（与现状回退读取**同一**槽位 → 天然读写闭环），复用已有 `keyring` 依赖（`crabmate-tui/Cargo.toml` 已带 `keyring 4.1.5`）。无钥匙串可用时退回“仅本会话内存”，并在面板提示不持久。

> 现状 `client_capability_matrix.md` L47 写 TUI “no TUI-side keyring writes”——本方案**改变这一格**：实现 PR 须把 L46/L47 两格 Notes 同步改为“面板可写（会话设置/保存到同一槽位）”，行内降级仍标 `reduced`（TUI 无系统 GUI 钥匙串弹窗等体验）。**（已随实现落地：auth Notes 已改为“面板可写/清同一 `tauri_client_llm_api_key` 槽位，`--no-keyring` 同时禁读禁写”。）**

### 6.4 随轮发送

保存到 user-data 后，新一轮回合仍按 Desktop 同规则随 body 发送（空值不发送）：`client_llm.{api_key?,api_base,model,llm_thinking_mode?}`、顶层 `temperature?`、`readonly_tool_ttl_cache_secs?`。**已实现部分**：`crabmate-tui-core` 请求体装配已加顶层 `temperature`（f64，仅 0.0..=2.0）与 `client_llm.llm_thinking_mode`（仅 on/off）；`llm_context_tokens` / `readonly_tool_ttl_cache_secs` 待 W2 剩余项，字段名与校验区间以契约 `crabmate 0.5.0` 与前端 `http_request.rs` L77-86 为准。

---

## 7. 分期（逐波小步 PR）

| Wave | 交付 | 验收 |
|---|---|---|
| **W1 面板骨架 + 模型/会话扁平字段** | `/settings` + F2 浮层；分区导航；三层显示；字段编辑：模型名、网关/api_base（预设表随 §6.4 对齐）、role、mode；`S` 保存（`llm-overrides` / `prefs` 合并 PUT）；`Esc`/脏确认；回合中只读；F2 加入 `/help` 与「快捷键」分区；斜杠 `/model /mode /role` 保留 | 连本地 serve 冒烟：面板改 model/role/mode → 保存 → `/status` 与 Desktop 设置页可见 → 下一轮请求体带 `client_llm.model`；重启 TUI 后值仍在（user-data 层） |
| **W2 完整 LLM 字段 + 密钥 + 工具开关** | 温度/思考模式字段与 API 密钥写钥匙串槽 **已随面板一并落地**（校验区间对齐；密钥重启后从同一槽位回读；矩阵 auth Notes 已同步）；剩：上下文 tokens、只读工具缓存开关、网关预设下拉（与 `client_llm_presets.rs` 同名表；跨仓同步纪律见 §8） | 校验区间对齐；密钥保存后重启可回读；关缓存开关后请求体出现 `readonly_tool_ttl_cache_secs: 0` |
| **W3 后置项（契约核对后）** | MCP 全局开关 + `tool_timeout_secs`（`/user-data/mcp-servers` 系端点）；Session SQLite 会话存储开关（`/status` 布尔 + POST） | 开关读/写与 Desktop 设置页同源一致；不实现服务器行管理 |

各 Wave 内**一项用户可见行为一个 PR**（引用 `coding_agent_client.md` 约束第 5 条的小步纪律）。

---

## 8. 落地纪律（文档同步，同一 PR）

| 变更 | 更新位置 |
|---|---|
| 新增面板 / W1-W3 能力落地 | `client_capability_matrix.md`：新增「Settings panel（对齐 Desktop）」行或扩展现有行；改 L46/L47 钥匙串 Notes；单元格不得留空 |
| TUI 母方案进度 | `remote_cli_tui.md`：阶段表 P4/P5 行注明「设置面板 Wx 已落地」或追加行 |
| 用户可见 shell 行为 / 启动项变化 | `README.md` ↔ `README.zh-CN.md` **双语同改**（若新增默认持久化行为/新命令） |
| 发布记录 | `CHANGELOG.md` `[Unreleased]`（英文） |
| 网关预设常量跨仓重复 | `frontend/src/client_llm_presets.rs` 与 TUI 同名表需在 W2 PR 内加同步注释（两侧引用同一 doc），避免漂移 |
| 契约字段扩展（温度/上下文/思考/ttl） | 属本仓 `tui-core` 请求体装配，不改钉点 tag；字段名以契约 crate + 前端 `http_request.rs` 为准 |

---

## 9. 风险与开放问题

| 项 | 说明 / 缓解 |
|---|---|
| `/user-data/prefs`、`/user-data/llm-overrides` 的 **GET 回灌**端点在契约里的形态 | 前端已 PUT；GET 路径（`user_prefs_sync.rs` 加载）实现期核对；个人云 API-only 场景需确认可用 |
| 保存覆盖窗口（Desktop/TUI 同时保存） | 先读后写合并；文档注明单写者优先，不引入锁 |
| 钥匙串写路径改变矩阵格 | 实现 PR 同改矩阵 Notes（§6.3）；`--no-keyring` 语义保持不变（显式不读不写） |
| 面板打开时机与现有浮层（审批）冲突 | 审批浮层打开时禁开面板（已有 overlay 优先级先例）；回合运行中只读 |
| temperature/context/thinking 是否被契约 0.5.0 serve 接受 | 前端已在发同名键；实现期以契约 crate 为准，不被接受就回退「存 user-data 但不随轮发」，字段仍对齐 |
| 可用 role/model 清单无枚举端点 | W1 先用文本 + `/status` 默认值提示；若后续 serve 提供列表再加下拉 |

---

## 10. 验证

- 单测：字段校验（温度区间/上下文上限/思考枚举，对齐 Desktop）、合并 PUT 构造（不丢 `saved_models`/非管理 prefs 键）、三层显示合成、`/settings` 解析与脏确认逻辑。沿用现有 CCN/行数门禁（大逻辑拆 `settings_panel.rs`，避免 `mod.rs` 超限）。
- 冒烟（`docs/design/shell_smoke_runbook.md` 同风格）：连本地 serve → F2 开面板 → 改模型保存 → `/status` 变化 + 请求体含 `client_llm`；改 role/mode 同验；重启 TUI 仍在；Desktop 设置页可见同值；密钥保存后 `--no-keyring` 不读。
