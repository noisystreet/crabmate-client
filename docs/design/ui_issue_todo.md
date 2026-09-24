# UI 问题待办清单

由 UI 功能体检整理，按优先级排序；修复后逐项勾选（`- [x]`）或移除。
2026-09-13 复核：原报告 P0 与 P1 键盘/焦点均已修复（PR #135），剩余项已并入本清单。
2026-09-14：P1「语义与反馈」四项已修复并勾选；剩余为 P1 对比度、P2 与 P3。
2026-09-22 复核：P2「768px 断点重复硬编码」与 P3「900px 断点无登记」已由 `scripts/check-css-breakpoints.sh` 收口，P1 缺失类名 / P4 死 CSS 已由 `scripts/check-css-contract.sh` 一类门禁固化（两者均进 pre-commit 与 `scripts/check.sh`），相应条目已勾选；未定义 token 引用与 `var()` 浅色兜底已由 `scripts/check-css-tokens.sh` 收口（同进 pre-commit 与 `scripts/check.sh`）。
2026-09-24：P1「对比度与焦点可见性」前两条（light 主题主按钮白字、`--muted` 小字）与 P3「首屏主题快照硬编码 `light`」「`shell-ds.css` `--muted` 再稀释」已修（浅色覆层加深 + 首屏默认改 `dark`），详见条目内注记；同期修正条目里的主题路径笔误（真实路径为 `frontend/themes/*.css`）。

## P0 · 明确缺陷

- [x] `theme=system` 在 Linux 非 GNOME（KDE / XFCE / 无 gsettings）下固定浅色：`desktop-tauri/src-tauri/src/os_theme.rs` 仅探测 GNOME 的 gsettings，无 portal / KDE 回退，双通道（窗口 `.theme()` + 前端 `TAURI_OS_DARK_HINT`）一起落到 light。
- [x] IDE 语法高亮在 material / high-contrast（均为深色）下仍用浅色 token：`frontend/styles/ide-highlight.css`、`ide-codemirror.css` 仅覆盖 `[data-theme="dark"]`，且 `--ide-hl-*` 变量未定义、靠浅色字面量兜底。

## P1 · 可访问性关键缺口

- [x] 审批弹窗无焦点陷阱且无法 Esc 关闭：`frontend/src/app/approval_modal.rs`、`frontend/src/app/app_shell_effects/escape.rs` 未覆盖 `pending_approval`。
- [x] 部分对话框缺焦点陷阱：`ide_new_file_modal.rs`、`shell_confirm_dialog.rs`、`ide_confirm_dialog.rs`。
- [x] 键盘不可达：图片附件 `<label>`（`column.rs`）、右键/长按上下文菜单、顶部/底栏菜单、IDE 标签页（缺方向键）、工作区文件树文件行。
- [x] 语义缺口：聊天模式 `role="menuitem"` 孤儿节点；单选/当前会话缺 `aria-checked` / `aria-current`；未保存/置顶/星标状态对屏幕阅读器不可见。
- [x] 焦点归还闭环缺失：图片 lightbox 无 Tab 循环、关闭不还焦；hydrate / 待传 / uploads 图片键盘不可达；右键 / 下拉菜单（`FocusableRoleMenu` 全部调用点）不能 Esc 关闭、关闭不还焦；聊天 / IDE 查找栏与 IDE 跳转行栏不自动聚焦；`changelist_modal` 缺 Esc；`session_list` / `approval` / `settings` 模态关闭不还焦。

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
- [ ] 启动 splash 硬编码深色 `#07090e`，浅色用户首帧深闪：`frontend/index.html`（同值亦见于 `desktop-tauri/splash.html`、`crates/crabmate-connect/assets/connect.html` 与 Android `values*/themes.xml`）。

## P3 · 次要

- [ ] `prefers-reduced-motion` 漏 2 处无限动画：会话流式徽章脉冲、克隆进度条。
- [x] 首屏主题快照硬编码 `light`，深色用户有短暂浅色闪烁。已修（2026-09-24）：`frontend/src/app/shell_prefs_storage.rs` 的 `read_shell_ui_initial_snapshot()` 默认改 `dark`，与 splash（`index.html` 内联深色）、桌面窗口底色（`BOOT_SHELL_BG`）及 `tokens.css` 的 `:root` 默认深色对齐（偏好要等 `GET /user-data/prefs` 才到，改 `system` 此时拿不到 OS 明暗会退回 light，故不用）；首帧值不会写回服务端覆盖用户偏好——`UserPrefsSyncPhase` 在偏好加载完成前禁止 PUT。
- [x] 对比度风险点 `frontend/styles/shell-ds.css:303`（`--muted` 再稀释），需实测验证。已修（2026-09-24）：实为 `.nav-rail-search-label` / `.nav-rail-scroll-label`（10px 大写，`color-mix(--muted 88%, transparent)`，行号已漂移至 285 / 384）——`--muted` 加深后 88% 半透明仍只 ≈3.9:1，两处改为纯 `var(--muted)`；同类小号文字稀释一并去半透明：`layout-chat.css` `.chat-tui-role` / `.chat-tui-think-summary`、`modal.css` `.settings-mcp-tool-openai`（装饰符 `▾`、`::placeholder`、`:disabled` 保持原样）。
- [ ] 死代码：`approval_bar.rs` 的 `ApprovalBar`（已被 approval_modal 替代、全仓无引用）。
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
