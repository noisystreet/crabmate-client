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
2026-09-26：**设置页面布局 / 组件风格统一**——① 删除无入口的 `settings_modal` 弹窗表面（`settings_modal/` 三文件 + `settings_modal_dialog.rs` 及全部引用），设置只留在常规页与 IDE 页；② IDE 设置页删掉自持页头（`ide_settings_page/view_header.rs`），改用常规页的 `SettingsPageHeader`（经新 `SettingsPageHeaderSpec` 声明页头八项，两页各自持有 `data-testid`——两页同时挂载且 victauri `test_id` 取首个匹配，故不能共用）与 `SettingsNavItem` / `SettingsContentIntro`；③ 表单统一为一种结构「`<div class="settings-field">` + 真 `<label for>`」（MCP 超时 / MCP 行内 name / bearer 三处由 label 包裹组改为显式 id 关联，GitHub client-id 的无样式 `class="input"` 改 `settings-text-input`）；④ 块边界由三种收成一种：横向版心只放在内容列 `.settings-content`（`padding: 0 14px`，页头标题 / 块标题 / 字段 / 列表 / 反馈共一条对齐轴），块与字段都不再自持横向内边距（嵌套块如 MCP JSON 导入面板不再叠加出第二层缩进），唯一强调卡框 `.settings-block--emphasis`（由原 `--web-api-auth` 卡框规则改名、两处即时生效块共用；`--llm-cloud` 虚线修饰删除，`--api-base` 旧修饰位无规则、原在白名单，条目一并删除）。详见 P2 新增条目。
2026-09-30：**Android 系统栏图标随 Web 主题明暗切换**——修复浅色主题下顶栏系统图标不可见（白图标压近白顶栏），详见 P2「移动端边界与体验」新增条目。两个主题级 light 标志仍保持 `false`（镜像门禁要求），切换只在运行时经 `WindowInsetsControllerCompat` 覆盖。

> **已完成项已归档**：原 `- [x]` 条目移至 [`ui_issue_todo_done.md`](./ui_issue_todo_done.md)，本清单只保留未完成项。

## P0 · 明确缺陷

（无未完成项）

## P1 · 可访问性关键缺口

（无未完成项）

## P1 · 语义与反馈

（无未完成项）

## P1 · 对比度与焦点可见性

- [ ] 顶栏菜单条 `min-width:max-content`，窄屏可能与中间路径、右侧控件重叠（需实机验证）：`frontend/styles/shell-topbar.css`、`mobile.css`。

## P2 · 移动端边界与体验

- [ ] Android `adjustResize` 生效设备上 IME 可能双倍抬高 composer：`MainActivity.kt` 的 `--cm-ime-inset` 与 `--vv-keyboard-inset` 取 `max`，窗口已压缩时 `ime` 仍非零。

- [ ] 左侧 20px 点击盲区：`frontend/styles/shell-ds.css` `.nav-rail-edge-hit` 为 `pointer-events:auto` 且无点击处理（右侧感应条为 `none`，左右不对称）。

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

- [ ] 审批「允许始终」用 `btn-primary` 而「拒绝」用 `btn-danger`，持久授权的高影响操作视觉权重倒置：`frontend/src/app/approval_modal.rs`。

- [ ] 破坏性操作无确认：MCP 行删除（`settings_mcp_server_row_actions.rs`）、GitHub「断开」（`settings_github_block.rs`）、Web Bearer 空输入点保存=清除 token 无二次确认（`settings_sections.rs`）。

- [ ] MCP「应用导入」绕过保存全部直接写服务端且无确认：`frontend/src/app/settings_mcp_json_import.rs`（与「破坏性操作无确认」一并留待后续）。

## P2 · 样式与 token

- [ ] 启动 splash 硬编码深色 `#07090e`，浅色用户首帧深闪：`frontend/index.html`（同值亦见于 `desktop-tauri/splash.html`、`crates/crabmate-connect/assets/connect.html` 与 Android `values*/themes.xml`）。**部分修（2026-09-25）**：色值已收成单一来源并加门禁——Android 侧四个底色槽全部改引用镜像令牌 `@color/cm_bg`（`res/values/colors.xml` 的 `cm_*` 逐项等于 `tokens.css` 的 `--<token>`），启动页三处 `#07090e` 与 `--bg` 同值且「声明必须恰好命中一次」，由 `scripts/check-token-mirrors.sh` 冻结（见下条）。**未修（需独立行为改动）**：浅色偏好用户的首帧深闪——`data-theme` 由 WASM 水合后才写入，`index.html` 内联样式在 trunk 注入的 `<link>` 之前，故首绘恒深色；正解是在 `index.html` 加预绘脚本、从 localStorage 快照同步读出主题再写 `data-theme`，属独立后续项。

## P3 · 次要

- [ ] `.modal-backdrop` 遮罩层仍有 13 处重复模板，且「点击自身关闭」判定已分叉（裸 `on:click` vs. `mouse_event_target_is_current_target`），未纳入模态骨架收敛（本轮只收敛了面板本体 `FocusableModalPanel`）。

- [ ] `save_busy/load_busy` 期间 Ctrl+S 被静默吞掉：`frontend/src/ide_save.rs`。

- [ ] 同步期间关闭标签，快照索引写回可能命中错误标签：`frontend/src/ide_disk_sync.rs`。

- [ ] 空编辑器只有 aria-label，无可见占位文本：`frontend/src/app/ide_editor_pane.rs`。

- [ ] `ide_find` 每次按键对全文 lowercase 并分配，大文件下查找输入可能卡顿：`frontend/src/ide_find.rs`。

- [ ] composer `resize: vertical` 与 autosize 两个高度机制打架：`frontend/styles/layout-chat.css`、`frontend/src/app/chat/composer_input_stack.rs`。

- [ ] transcript 整区 `aria-live="polite"`，工具行频繁 status 更新持续触发读屏播报（权衡项）：`frontend/src/app/chat/tui_stream_view.rs`。

- [ ] 文件树不支持树内拖拽移动（能力矩阵未承诺，可选增强）：`frontend/src/workspace_file_drop.rs`。
