# UI 问题待办清单

由 UI 功能体检整理，按优先级排序；修复后逐项勾选（`- [x]`）或移除。
2026-09-13 复核：原报告 P0 与 P1 键盘/焦点均已修复（PR #135），剩余项已并入本清单。
2026-09-14：P1「语义与反馈」四项已修复并勾选；剩余为 P1 对比度、P2 与 P3。
2026-09-22 复核：P2「768px 断点重复硬编码」与 P3「900px 断点无登记」已由 `scripts/check-css-breakpoints.sh` 收口，P1 缺失类名 / P4 死 CSS 已由 `scripts/check-css-contract.sh` 一类门禁固化（两者均进 pre-commit 与 `scripts/check.sh`），相应条目已勾选；未定义 token 引用与 `var()` 浅色兜底已由 `scripts/check-css-tokens.sh` 收口（同进 pre-commit 与 `scripts/check.sh`）。
2026-09-24：P1「对比度与焦点可见性」前两条（light 主题主按钮白字、`--muted` 小字）与 P3「首屏主题快照硬编码 `light`」「`shell-ds.css` `--muted` 再稀释」已修（浅色覆层加深 + 首屏默认改 `dark`），详见条目内注记；同期修正条目里的主题路径笔误（真实路径为 `frontend/themes/*.css`）。
2026-09-25：新增两道 CSS 门禁并接入 pre-commit 与 `scripts/check.sh`——① `scripts/check-css-tokens.sh` 扩为三项（未定义引用 / `var()` 颜色兜底 / **主题覆盖完整性**：任一主题覆盖过的 token，其余主题必须尽量显式覆盖，主题身份差异登记 `THEME_IDENTITY_TOKENS`），补齐 `material.css` / `high-contrast.css` 缺失的 `--shadow-card` 与 `--terminal-panel-*`；② 新增 `scripts/check-css-literals.sh`（`frontend/styles` 内颜色与 `font-size` 字面量逐文件棘轮，预算表 `scripts/css_literals_budget.txt`，实际值必须等于预算值，见 P2「样式与 token」新增条目）。
2026-09-25（续）：**Phase 1a 字号统一**落地——`tokens.css` 排版区建立 13 档 px 字号 token（`--text-2xs…--text-7xl`），166 处 `font-size` 字面量与 16 处 `em` 全部收敛，`font-size` 预算清零；残留颜色 26 处转入 Phase 1b / 1c。
2026-09-25（续 2）：**Phase 1b 语义色归一**——GFM alert 五色（硬编码 Tailwind 色板）改为 `--info` / `--success` / `--accent` / `--warn` / `--error`，slash 菜单错误条底色改 `--error-bg`，并删除 `--danger` / `--danger-bg` / `--ok` 三个兼容别名（6 处消费点统一改指 `--error`）；`frontend/styles` 颜色字面量 26 → 20 处，`layout-chat.css` 预算 10 → 4（余 3 处 `#fff` inset 高光 + 1 处浮层阴影，转 Phase 1c）。
2026-09-25（续 3）：**Phase 1c 高光 / 前景 token**——新增 `--inset-highlight`（inset 亮线颜色，只在 `tokens.css` 定义，四主题同值故视觉零变化）与 `--on-accent`（实心强调色底上的前景，light `#fff` / material 与 high-contrast `#0a0a0a`），`components.css` 6 处与 `layout-chat.css` 3 处 `#fff` 字面量改走 token；material / high-contrast 的 `.btn-primary { color }` 覆写块随之删除，改由 `--on-accent` 驱动——high-contrast 的 `:disabled` 标签原为 `#737373`，与禁用底色 `color-mix(#f5f5f5 38%, #242424)` 算出的 `#737373` 同值（此前不可见），删除后回到基规则 `color-mix(var(--on-accent) 75%, transparent)` 的深字，可见性恢复。`frontend/styles` 颜色字面量 20 → 11 处（余 11 处均为遮罩与浮层阴影，转 Phase 1d；`tauri-shell.css` 的 2 处为固定红底白字，语义不随主题，长期保留预算）。
2026-09-25（续 4）：**Phase 1d 遮罩 / 浮层阴影 token**——新增 `--scrim-bg`（局部遮罩，`rgba(0,0,0,.5)`，抽屉 ×2 与面板遮罩 ×1 共用，两处 .48 归并到 .5）、`--shadow-menu`（浮层菜单，0 8px 24px / 12% 黑，菜单条 2 处 + slash 菜单 1 处共用）、`--shadow-media`（灯箱图片单层柔影）与 `--shadow-knob`（拨杆滑块贴合阴影）；灯箱整屏遮罩由固定 `color-mix(#000 62%)` 改用它本就该用的主题化 `--modal-backdrop-bg`（light 0.30 暖调 / 深色 0.54 / material 0.56 / high-contrast 0.72）。`frontend/styles` 颜色字面量 11 → 2 处，且这 2 处（`tauri-shell.css` 的 Windows 关闭键红底白字，固定色语义不随主题）为**有意长期保留**，预算表仅余该条；颜色字面量清理到此收尾。
2026-09-25（续 5）：**模态骨架收敛**——`frontend/src/app/focusable_menu.rs` 的 `FocusableModalPanel` 从 3 处扩到 12 处消费点，余下 9 处手抄模态根（IDE 新建文件 / 工作区克隆 / 审批 / 设置 / 工作区项目 / 会话管理 / 移动端变更清单 / 模型注册新增弹窗 / 工作区文件选择）全部迁入，`role` / `aria-modal` / 焦点陷阱 / Escape / 焦点归还只声明一次；壳新增 `dialog_role`（默认 `alertdialog`，普通弹窗传 `dialog`）、`labelledby`、`testid`（渲染 `data-testid`，原为会话管理弹窗的接口缺口）、`stop_pointerdown`、`on_escape` 五个 prop。并发一道 `scripts/check-modal-shell.sh` 门禁（见 P1 新增条目）。唯一语义变化：设置弹窗**新增** Escape 关闭（经既有脏表单守卫 `request_settings_modal_close`，未保存草稿仍先确认）；克隆弹窗在 Running 期间依旧吞掉 Escape。迁出过程中遇到两个 `E0525`（`Show` 的 children 必须是 `Fn`，而内联手抄面板会让闭包按值捕获 `Arc` 回调 / 表单信号并降级为 `FnOnce`），处置方式是每处面板抽成独立 `#[component]`。图片灯箱是唯一仍在壳外的模态根（`create_element` / `set_attribute` 命令式建 DOM，非 Leptos 视图），已就地豁免。
2026-09-25（续 6）：**Android 壳原生层对齐设计令牌**（承接 PR #169 的退出弹窗深色化）——退出弹窗配色从临时 `cm_dialog_*` 收成 `cm_*` 镜像令牌族；应用主题底色槽全改引令牌并显式声明系统栏图标明暗；通知渠道补描述 / 强调色 / 单色小图标；死模板布局与模板色清理；新增 `scripts/check-token-mirrors.sh` 把「Android res + 启动页 ↔ `tokens.css` 单一来源」冻结（进 pre-commit 与 `scripts/check.sh`，三不变量精确相等、无白名单）。详见 P2「样式与 token」新增条目；启动帧深闪仅完成单一来源，行为修复仍留待独立改动。
2026-09-25（续 7）：**镜像门禁补漏 + 通知渠道文案 i18n + 文档同步**——① `scripts/token_mirrors_check.py` 由三条不变量扩到**五条**：新增「`res/values*/themes.xml` 引用的每个 `@color/<name>` 必须在 `res/values*/colors.xml` 里有定义」（拼错此前只在 AAPT 构建期报错）与「手写 Kotlin（`java/edu/crabmate`，与 `scripts/ktlint-android.sh` 同范围、排除 Tauri 生成的 `generated/`）只许消费 `R.color.cm_*`」（此前可自带任意色值，绕过整条镜像链），两项均做双向验证；② 通知渠道名 / 描述补 `_en` 英文资源，`StreamKeepAliveService.ensureChannels()` 按**应用内语言**（`localeSlug`）二选一，与通知正文同一套约定（不新增 `values-en/`）；③ `mobile-tauri/README.md` 补注 Android 15+（targetSdk 35+）edge-to-edge 强制、`statusBarColor` / `navigationBarColor` 失效的机制变化并标注真机验证待做，`AGENTS.md` / `docs/TESTING.md` / `CHANGELOG.md` 同步为五条不变量。

## P0 · 明确缺陷

- [x] `theme=system` 在 Linux 非 GNOME（KDE / XFCE / 无 gsettings）下固定浅色：`desktop-tauri/src-tauri/src/os_theme.rs` 仅探测 GNOME 的 gsettings，无 portal / KDE 回退，双通道（窗口 `.theme()` + 前端 `TAURI_OS_DARK_HINT`）一起落到 light。
- [x] IDE 语法高亮在 material / high-contrast（均为深色）下仍用浅色 token：`frontend/styles/ide-highlight.css`、`ide-codemirror.css` 仅覆盖 `[data-theme="dark"]`，且 `--ide-hl-*` 变量未定义、靠浅色字面量兜底。

## P1 · 可访问性关键缺口

- [x] 审批弹窗无焦点陷阱且无法 Esc 关闭：`frontend/src/app/approval_modal.rs`、`frontend/src/app/app_shell_effects/escape.rs` 未覆盖 `pending_approval`。
- [x] 部分对话框缺焦点陷阱：`ide_new_file_modal.rs`、`shell_confirm_dialog.rs`、`ide_confirm_dialog.rs`。
- [x] 键盘不可达：图片附件 `<label>`（`column.rs`）、右键/长按上下文菜单、顶部/底栏菜单、IDE 标签页（缺方向键）、工作区文件树文件行。
- [x] 语义缺口：聊天模式 `role="menuitem"` 孤儿节点；单选/当前会话缺 `aria-checked` / `aria-current`；未保存/置顶/星标状态对屏幕阅读器不可见。
- [x] 焦点归还闭环缺失：图片 lightbox 无 Tab 循环、关闭不还焦；hydrate / 待传 / uploads 图片键盘不可达；右键 / 下拉菜单（`FocusableRoleMenu` 全部调用点）不能 Esc 关闭、关闭不还焦；聊天 / IDE 查找栏与 IDE 跳转行栏不自动聚焦；`changelist_modal` 缺 Esc；`session_list` / `approval` / `settings` 模态关闭不还焦。

- [x] 模态根 12 处各写一遍 `role="dialog"` / `aria-modal` / 焦点陷阱 / Escape / 焦点归还，语义已静默分叉（`stop_pointerdown` 有无、`prevent_default` 有无、Escape 是否经脏守卫）。已修（2026-09-25）：余下 9 处手抄模态根迁入 `FocusableModalPanel`，壳补齐 `dialog_role` / `labelledby` / `testid` / `stop_pointerdown` / `on_escape`；并由新增的 `scripts/check-modal-shell.sh` 固化——`frontend/src/**/*.rs` 除 `app/focusable_menu.rs` 外不得出现 `role="dialog"` / `"alertdialog"` / `set_attribute("role", …)` / `aria-modal`，有意例外就地写 `modal-gate: allow <理由>`（当前仅图片灯箱一条）。门禁已进 pre-commit 与 `scripts/check.sh`。

## P1 · 语义与反馈

- [x] slash 浮层打开且无匹配项时 Enter 被吞（`prevent_default` 后 accept 空转），以 `/` 开头的正文无法键盘发送且无反馈：`frontend/src/app/chat/composer_slash_menu.rs`。已修：无选中项时 Enter 不消费，交还 composer 走发送路径（Tab 仍吞掉防焦点跳出）。
- [x] IDE 标签纯切换（内容已在缓冲、不丢失）也弹「放弃未保存更改」确认，确认框语义与行为不符，且每次带脏切换都被强制拦截：`frontend/src/ide_tabs.rs`（`try_switch_tab`）。已修：`switch_to` 先把编辑器内容 persist 回原标签，纯切换无损，移除确认（打开新文件路径同型确认一并移除；关闭标签的确认保留——persist 后标签被删，内容确会丢失）。
- [x] MCP 配置加载失败被静默吞掉（`if let Ok` 忽略 Err，仅 probing 复位、无错误反馈）：`frontend/src/app/settings_mcp_status.rs`（`spawn_reload_mcp`）。已修：Err 写入设置页 `feedback` 通道展示。
- [x] `ide_confirm_user` 并发第二个确认请求仍使第一个等待方静默返回 `false`（id 机制已防误取他人结果，但请求不排队）：`frontend/src/ide_confirm.rs`。已修：`pending` 改 FIFO 请求队列，UI 只消费队首，应答按队首 `id` 回写，等待方按自身 `id` 匹配结果。

## P1 · 对比度与焦点可见性

- [x] light 主题主按钮白字 ≈3.2–3.9:1，低于 AA 小字（12px/500 需 4.5:1）：`frontend/styles/components.css`、`frontend/themes/light.css`。已修（2026-09-24）：`.btn-primary { color: #fff }` 为三主题共用，故只改 light 覆层——`--btn-primary-bg` 渐变改为 `color-mix(--accent 68%, #0a0c10)` → `color-mix(--accent 60%, #0a0c10)`（最亮顶部 ≈6.3:1），`--btn-primary-border` 同步压深；`:hover` 的 `brightness(1.07)` 提亮后仍 ≈5.7:1。
- [x] light 主题 `--muted` 小字 ≈3.4:1，仍用于 10–11px 大写标签：`frontend/themes/light.css`、`components.css`、`status.css`、`modal.css`、`shell-topbar.css`。已修（2026-09-24）：`--muted` `#8a8278` → `#6f675c`，在 `--bg` / `--surface-hover` / `--surface` 上分别为 4.99 / 4.90 / 5.57:1（`--status-agent-select-bg-image` 的 SVG fill 同步）；`components.css` 中 muted 半透明只出现在 `:disabled`（WCAG 对比度豁免），未改；`status.css` / `shell-topbar.css` 的 10–11px 标签本就用纯 `var(--muted)`，随 token 加深即达标。
- [x] light 主题（出厂默认）五个语义色当文字色用时**全部低于 AA**，且 `--info` 与 `--accent` 同值、`--surface` 纯白压在暖底上冷暖冲突：`frontend/themes/light.css`。已修（2026-09-25，**取代上两条记录的数值**）：`--accent` `#7a8c7a`(3.58:1) → `#55705b`、`--info` → 独立取青 `#3a6e83`、`--success` → `#4a7454`、`--warn` → `#836628`、`--error` → `#a0524f`，在 `--bg` / `--surface-hover` / `--surface` 三层底色上最差 4.54:1（小字 AA 需 4.5:1）；`--surface` `#ffffff` → `#fffdfa`、`--bg` → `#f4f1ea`、`--surface-hover` → `#f0ece2`、`--border` → `#e2ddd1`、`--border-subtle` → `#ebe7dc`、`--text` → `#1c1913`、`--muted` → `#6b6357`(5.02–5.83:1，chevron SVG 同步)、`--modal-backdrop-bg` 随 `--text`；`--btn-primary-bg` 由 `--accent 68%/60%` 改 `92%/84%` 混 `#0a0c10`（旧比例在新 accent 下会压成近黑，白字 ≈8.7:1 但丢失主色身份），现顶部 ≈6.1:1、hover ≈5.5:1。仍需实算复核：`--nav-rail-bg` 仍为 `bg-elevated 90%`，侧栏略亮于页面（未在本次范围内）。
- [ ] `.ide-editor-textarea:focus` `outline: none` 且无 `:focus-visible` 替代，键盘焦点只剩 caret：`frontend/styles/ide-layout.css`。
- [ ] lightbox 操作 / 关闭按钮无 `:hover` 与 `:focus-visible`：`frontend/styles/shell-ds.css`。
- [ ] 顶栏菜单条 `min-width:max-content`，窄屏可能与中间路径、右侧控件重叠（需实机验证）：`frontend/styles/shell-topbar.css`、`mobile.css`。

## P2 · 移动端边界与体验

- [ ] Android `adjustResize` 生效设备上 IME 可能双倍抬高 composer：`MainActivity.kt` 的 `--cm-ime-inset` 与 `--vv-keyboard-inset` 取 `max`，窗口已压缩时 `ime` 仍非零。
- [ ] 左侧 20px 点击盲区：`frontend/styles/shell-ds.css` `.nav-rail-edge-hit` 为 `pointer-events:auto` 且无点击处理（右侧感应条为 `none`，左右不对称）。
- [x] 768px 断点在 Rust（`app_prefs.rs`）与多份 CSS 重复硬编码，无单一来源/校验，改漏会脱节。已修：权威值仍是 `MOBILE_LAYOUT_BREAKPOINT_PX`，新增 `scripts/check-css-breakpoints.sh` 强制窄屏 `max-width: N` 与互补 `min-width: N+1` 成对（禁交叉书写）、二级断点须登记、e2e 移动视口须落窄屏侧。
- [x] 未定义 token 硬编码 fallback（`--shell-border` / `--surface-1` / `--accent-muted` / `--accent-warn` 等），切主题时这些位置颜色不变。已修：未定义 token 引用统一改指已有 token（不加别名），并剥离全部 `var()` 内浅色兜底字面量——token 缺失时立刻暴露而非静默渲染固定浅色；新增 `scripts/check-css-tokens.sh` 门禁（未定义引用、`var()` 颜色兜底、白名单失效三项必须为零，运行时注入属性登记 `scripts/css_tokens_allowlist.txt`），已进 pre-commit 与 `scripts/check.sh`。
- [ ] 纯浏览器宽屏触控平板（>768px 非壳）软键盘不抬高 composer。

## P2 · IDE 与工作区

- [ ] 标签 `For` 的 key 含 `idx`，任意关闭/钉住都会令其后所有标签 DOM 重建、键盘焦点丢失，`prop:id` 随之漂移：`frontend/src/app/ide_tabs_bar.rs`。
- [ ] 标签栏仅 `overflow-x:auto`，多标签溢出不自动滚到活动标签：`frontend/styles/ide-layout.css`。
- [ ] 磁盘同步对多个脏标签逐个弹确认，无法一次性「全部取消」：`frontend/src/ide_disk_sync.rs`。
- [ ] 语法高亮语言表缺 `.css/.html/.kt/.java` 等常见后缀（文件树图标分类已含）：`frontend/src/ide_syntax_highlight.rs`。
- [ ] 跳转行输入非法（非数字/超界）静默 no-op：`frontend/src/app/ide_find_bar.rs`。
- [ ] 手动「刷新列表」清空 `subtree_expanded`，已展开目录全部折叠：`frontend/src/workspace_shell.rs`。
- [ ] 嵌套空目录展开后无空态提示（根级有，子目录没有）：`frontend/src/workspace_tree.rs`。

## P2 · 空态与确认

- [ ] 空态缺失：会话列表标题过滤无结果（`sidebar_nav/session_rail.rs`）、「管理会话」无会话（`session_list_modal.rs`）、任务列表空 `<ul>`（`side_column.rs`）、MCP 服务器列表（`settings_mcp_block.rs`）、模型预设列表（`settings_models_registry/preset_list.rs`）。
- [ ] 审批「允许始终」用 `btn-primary` 而「拒绝」用 `btn-danger`，持久授权的高影响操作视觉权重倒置：`frontend/src/app/approval_modal.rs`。
- [ ] 模型预设新增弹窗温度/上下文 token 无范围校验，与「保存全部」的 `validate_temperature_override` 不一致：`frontend/src/app/settings_models_registry/submit.rs`。
- [ ] 破坏性操作无确认：MCP 行删除（`settings_mcp_server_row_actions.rs`）、GitHub「断开」（`settings_github_block.rs`）、Web Bearer 空输入点保存=清除 token 无二次确认（`settings_sections.rs`）。
- [ ] 保存语义混合（预设开关/删除、Bearer、API base、MCP 导入立即落盘 vs 主题/语言/LLM 需「保存全部」）；MCP「应用导入」绕过保存全部直接写服务端且无确认：`frontend/src/app/settings_mcp_json_import.rs`。

## P2 · 样式与 token

- [x] 未定义 token：`--text-muted` / `--surface-muted` / `--surface-2` / `--panel` / `--fg` / `--warning`；`status.css` 的 `color-mix(… var(--panel) …)` 因变量失效整句作废。已修（2026-09-22）：`--panel` 那条 `color-mix` 随 P4 死 CSS 清理移除；其余五个的引用统一改指已有 token（`--fg` 仅保留在 `splash.html` / `connect.html` 两个独立页自持定义），`frontend/styles` + `frontend/themes` 内已无未定义引用、无 `var()` 浅色兜底，由 `scripts/check-css-tokens.sh` 固化。
- [ ] API 层窄路径硬编码中文错误串：`frontend/src/api/http.rs`、`github_secrets_local.rs`、`llm_secrets_local.rs`、`web_api_bearer_local.rs`、`user_data.rs`。
- [x] `frontend/styles` 消费侧 `font-size` 字面量 166 处未 token 化（含 16 处 `em`）。已修（2026-09-25，**Phase 1a**）：新增 13 档字号 token（`--text-2xs…--text-7xl`，单位恒为 px，定义于 `tokens.css` 排版区；`--text-lg` = 14px 即根字号，`base.css` 的 `html, body { font-size: var(--text-lg) }` 是其唯一消费点，故 1rem = 14px），166 处按渲染 px 就近并档（±0.5px 内，平局取较大档，故 10.5px → `--text-sm`）；`em` 全部消灭——11 处按父级固定字号并入档，聊天列内 5 处（内联 code / skill chip / file-ref）改 `calc(var(--crabmate-chat-font-size, 14px) * <ratio>)` 以保留用户字号缩放。逐文件预算 `font-size` 全部下调为 0（5 个双零条目按规则删除），由 `scripts/check-css-literals.sh` 固化；四道 CSS 门禁全绿。残留颜色 26 处仍见下条。
- [x] `frontend/styles` 消费侧颜色字面量已从 26 处清到 2 处（逐文件预算见 `scripts/css_literals_budget.txt`，现值为棘轮上限，新增即 fail）。已修（2026-09-25，**Phase 1b**）：GFM alert 五色 `#3b82f6 / #22c55e / #a855f7 / #f59e0b / #ef4444` 改 `--info / --success / --accent / --warn / --error`（`color-mix(… 70%, var(--border))` 保饱和），`layout-chat.css` 内联错误底 `rgba(207,34,46,.08)` 改 `var(--error-bg)`，并删除 `--danger / --danger-bg / --ok` 兼容别名（`tokens.css` + 三主题，6 处消费点改指 `--error`）。已修（2026-09-25，**Phase 1c**）：新增 `--inset-highlight`（inset 亮线，仅 `tokens.css` 定义、四主题同值）与 `--on-accent`（实心强调色底前景，light `#fff` / material 与 high-contrast `#0a0a0a`），收掉 `components.css` 6 处与 `layout-chat.css` 3 处 `#fff`；material / high-contrast 的 `.btn-primary { color }` 覆写块删除改用 `--on-accent`，其中 high-contrast 禁用态标签原 `#737373` 与其底色同值（不可见），删除后回到基规则深字。剩余：`layout-chat.css` 1 / `mobile.css` 1 / `modal.css` 1 / `shell-topbar.css` 2 / `shell-ds.css` 3 / `sidebar.css` 1（遮罩与浮层阴影，待阴影 token，转 Phase 1d）、`tauri-shell.css` 2（壳层关闭键红底白字，固定色语义不随主题，可长期留预算）。已修（2026-09-25，**Phase 1d**）：新增 `--scrim-bg`（局部遮罩）、`--shadow-menu`（浮层菜单阴影）、`--shadow-media`（灯箱图片柔影）、`--shadow-knob`（拨杆滑块阴影），收掉上述 9 处；灯箱整屏遮罩改走既有主题化 `--modal-backdrop-bg`（light 0.30 暖调 / 深色 0.54 / material 0.56 / high-contrast 0.72，light 下由固定 62% 黑变浅为有意统一）。至此 `frontend/styles` 颜色字面量 26 → 2 处，仅余 `tauri-shell.css` 的壳层关闭键红底白字（固定色语义不随主题，有意长期保留），预算表仅余该条。
- [ ] 启动 splash 硬编码深色 `#07090e`，浅色用户首帧深闪：`frontend/index.html`（同值亦见于 `desktop-tauri/splash.html`、`crates/crabmate-connect/assets/connect.html` 与 Android `values*/themes.xml`）。**部分修（2026-09-25）**：色值已收成单一来源并加门禁——Android 侧四个底色槽全部改引用镜像令牌 `@color/cm_bg`（`res/values/colors.xml` 的 `cm_*` 逐项等于 `tokens.css` 的 `--<token>`），启动页三处 `#07090e` 与 `--bg` 同值且「声明必须恰好命中一次」，由 `scripts/check-token-mirrors.sh` 冻结（见下条）。**未修（需独立行为改动）**：浅色偏好用户的首帧深闪——`data-theme` 由 WASM 水合后才写入，`index.html` 内联样式在 trunk 注入的 `<link>` 之前，故首绘恒深色；正解是在 `index.html` 加预绘脚本、从 localStorage 快照同步读出主题再写 `data-theme`，属独立后续项。
- [x] Android 壳原生层（WebView 之外）未跟随设计令牌：主题底色槽自持字面量、系统栏图标明暗未声明、通知图标 / 颜色用系统默认、模板残留布局与模板色未清。已修（2026-09-25）：`Theme.crabmate_mobile`（`values/` 与 `values-night/` 各自自包含——同名 style 是整体替换而非逐项合并）四个底色槽全改 `@color/cm_bg`，并显式声明 `android:windowLightStatusBar` / `android:windowLightNavigationBar = false`（底色恒深色，DayNight 浅色变体默认给深色图标→在深色状态栏上不可见；此前靠继承，换 parent 会静默回归，`windowLightNavigationBar` 加 `tools:targetApi="27"` 抑制 lint）；`StreamKeepAliveService` 两条通知渠道补 `description`、加 `setColor(getColor(R.color.cm_accent))`，小图标由框架 `stat_sys_warning` / `stat_notify_sync` 换成自绘单色 vector `ic_stat_stream`（三条递减横杠）/ `ic_stat_approval`（警示三角，惊叹号用 `evenOdd` 挖空——小图标只取 alpha，不能复用 launcher 位图否则渲染成白块）；`MainActivity` 的 WebView 预绘底色由 `parseColor("#07090E")` 改 `getColor(R.color.cm_bg)`，弹窗按钮色改指 `cm_error` / `cm_muted` / `cm_accent`；删除死文件 `res/layout/activity_main.xml`（全仓零引用）与未引用模板色（`purple_*` / `teal_*`）。新增 `scripts/check-token-mirrors.sh`（pre-commit + `scripts/check.sh`）冻结三条不变量（后续「续 7」扩到五条）：`colors.xml` 只放 `cm_*` 镜像且逐项等于 `tokens.css`；`values*/themes.xml` 无颜色字面量且应用主题四项底色 + 两项系统栏标志齐备；启动页三处首绘底色等于 `--bg` 且声明恰好命中一次。三条均精确相等、无白名单。

## P3 · 次要

- [ ] `prefers-reduced-motion` 漏 2 处无限动画：会话流式徽章脉冲、克隆进度条。
- [x] 首屏主题快照硬编码 `light`，深色用户有短暂浅色闪烁。已修（2026-09-24）：`frontend/src/app/shell_prefs_storage.rs` 的 `read_shell_ui_initial_snapshot()` 默认改 `dark`，与 splash（`index.html` 内联深色）、桌面窗口底色（`BOOT_SHELL_BG`）及 `tokens.css` 的 `:root` 默认深色对齐（偏好要等 `GET /user-data/prefs` 才到，改 `system` 此时拿不到 OS 明暗会退回 light，故不用）；首帧值不会写回服务端覆盖用户偏好——`UserPrefsSyncPhase` 在偏好加载完成前禁止 PUT。
- [x] 对比度风险点 `frontend/styles/shell-ds.css:303`（`--muted` 再稀释），需实测验证。已修（2026-09-24）：实为 `.nav-rail-search-label` / `.nav-rail-scroll-label`（10px 大写，`color-mix(--muted 88%, transparent)`，行号已漂移至 285 / 384）——`--muted` 加深后 88% 半透明仍只 ≈3.9:1，两处改为纯 `var(--muted)`；同类小号文字稀释一并去半透明：`layout-chat.css` `.chat-tui-role` / `.chat-tui-think-summary`、`modal.css` `.settings-mcp-tool-openai`（装饰符 `▾`、`::placeholder`、`:disabled` 保持原样）。
- [ ] 死代码：`approval_bar.rs` 的 `ApprovalBar`（已被 approval_modal 替代、全仓无引用）。
- [ ] `.modal-backdrop` 遮罩层仍有 13 处重复模板，且「点击自身关闭」判定已分叉（裸 `on:click` vs. `mouse_event_target_is_current_target`），未纳入模态骨架收敛（本轮只收敛了面板本体 `FocusableModalPanel`）。
- [ ] `save_busy/load_busy` 期间 Ctrl+S 被静默吞掉：`frontend/src/ide_save.rs`。
- [ ] 同步期间关闭标签，快照索引写回可能命中错误标签：`frontend/src/ide_disk_sync.rs`。
- [ ] 空编辑器只有 aria-label，无可见占位文本：`frontend/src/app/ide_editor_pane.rs`。
- [ ] `ide_find` 每次按键对全文 lowercase 并分配，大文件下查找输入可能卡顿：`frontend/src/ide_find.rs`。
- [ ] composer `resize: vertical` 与 autosize 两个高度机制打架：`frontend/styles/layout-chat.css`、`frontend/src/app/chat/composer_input_stack.rs`。
- [ ] transcript 整区 `aria-live="polite"`，工具行频繁 status 更新持续触发读屏播报（权衡项）：`frontend/src/app/chat/tui_stream_view.rs`。
- [ ] `cm-boot-spin` 无限旋转未纳入 `prefers-reduced-motion`：`frontend/index.html`。
- [x] 设置页 900px 断点与主 768px 体系并存且无注释：`frontend/styles/modal.css`。已修：900px 作为与主断点语义独立的二级断点，在 `scripts/check-css-breakpoints.sh` 的 `EXTRA_BREAKPOINTS` 显式登记并写明用途（设置弹窗 `.settings-layout` 双列转单列）。
- [ ] MCP 超时输入非法字符静默保留旧值：`frontend/src/app/settings_mcp_block_toolbar.rs`。
- [ ] MCP 远端 bearer placeholder 硬编码 `••••••••`：`frontend/src/app/settings_mcp_server_row.rs`。
- [ ] 文件树不支持树内拖拽移动（能力矩阵未承诺，可选增强）：`frontend/src/workspace_file_drop.rs`。
