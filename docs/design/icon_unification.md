# UI 图标统一样式方案

状态：已实施（第 1–3 步落地；第 4 步因前置条件未实测，按本方案指示退回并从方案中移除 —— 见文末「实施记录」）
日期：2026-09-25
范围：`frontend/` 业务 UI（Web / Desktop / Android 共用）；不含 `crates/crabmate-connect/assets/connect.html`、`desktop-tauri/splash.html` 两个独立启动页，也不含第三方 vendor 资源

## 背景

`frontend/` 没有图标库、没有图标字体、没有 sprite，图标全靠三套机制**并存**手工维护：

1. **内联 SVG** —— 写在 Leptos view 里（28 处 / 10 个文件）。
2. **CSS `data-URI`** —— 把 SVG 编码进自定义属性，再由 `background` 消费（1 个 token，主题各一份）。
3. **文本字形** —— 用 Unicode 字符当图标（`×` `‹` `▾` `✓` `●` …）。

三套机制各自可用，但当主题从 1 套（深色默认）扩到 6 套（`:root` + `crabmate-light` / `material-dark` / `high-contrast-dark` / `shadcn-dark` / `shadcn-light`）后，成本开始显性化：data-URI 每加一个主题就要复制一份并重新编码色值；内联 SVG 的属性模板手抄 18 次，任一处漏改就与其余不一致；文本字形的字形表现依赖系统字体，跨平台（WebKitGTK / Android WebView）不稳定。本方案给出收敛路径。

## 目标与非目标

**目标**

- 内联 SVG 的属性模板单一来源，消除手抄不一致。
- 图标尺寸走 token，与既有字号 token 体系（`--text-*`）一致。
- data-URI 图标不再随主题复制 N 份。
- 明确文本字形图标的去留标准。

**非目标（已评估并否决）**

| 方案 | 否决理由 |
|---|---|
| 引入图标库（`lucide-leptos` / `icondata` 等） | 新增依赖 + 全量图标树；现用量仅 30 余个，引入即得不偿失 |
| 图标字体（self-hosted icon font） | 字体子集化维护成本高，且与已归档的「自托管字体」议题同源（本轮已撤回打包字体） |
| SVG sprite（`<use href="#id">`） | 需运行时注入 sprite 或构建期拼装，而本项目无构建期 CSS/JS 处理链（trunk 仅做 WASM + 资源拷贝） |

## 现状盘点

### 机制 1：内联 SVG —— 28 处 / 10 个文件

| 文件 | 处数 | 说明 |
|---|---|---|
| [workspace_shell.rs](file:///home/gzz/crabmate/client/frontend/src/workspace_shell.rs) | 10 | 全部经 `svg_common()` 生成 |
| [side_column_toolbar.rs](file:///home/gzz/crabmate/client/frontend/src/app/side_column_toolbar.rs) | 6 | 手写 |
| [chat/column.rs](file:///home/gzz/crabmate/client/frontend/src/app/chat/column.rs) | 3 | 手写 |
| [layout_mode_segment.rs](file:///home/gzz/crabmate/client/frontend/src/app/layout_mode_segment.rs) | 2 | 手写 |
| [settings_models_registry/preset_list.rs](file:///home/gzz/crabmate/client/frontend/src/app/settings_models_registry/preset_list.rs) | 2 | 手写 |
| [status_agent_role_menu.rs](file:///home/gzz/crabmate/client/frontend/src/app/status_agent_role_menu.rs) | 1 | 手写 |
| [settings_page/header.rs](file:///home/gzz/crabmate/client/frontend/src/app/settings_page/header.rs) | 1 | 手写，**无 `class`** |
| [ide_settings_page/view_header.rs](file:///home/gzz/crabmate/client/frontend/src/app/ide_settings_page/view_header.rs) | 1 | 手写，**无 `class`** |
| [status_session_mode_seg.rs](file:///home/gzz/crabmate/client/frontend/src/app/status_session_mode_seg.rs) | 1 | 手写 |
| [settings_models_registry/mod.rs](file:///home/gzz/crabmate/client/frontend/src/app/settings_models_registry/mod.rs) | 1 | 手写 |

`workspace_shell.rs` 已有局部工厂函数，但只服务自己的 10 个图标：

```rust
fn svg_common() -> (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str) {
    ("workspace-entry-icon workspace-entry-svg", "0 0 24 24", "none",
     "http://www.w3.org/2000/svg", "currentColor", "2")
}
```

其余 18 处手写，典型形态在所有文件里逐字重复：

```html
<svg class="shell-toolbar-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor"
     stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
```

### 机制 2：CSS `data-URI` —— 1 个 token，编码 6 份

唯一 token 为 `--status-agent-select-bg-image`（下拉三角 `M0 0h10L5 6z`），色值以 `fill='%23……'` 烧死在 URI 里：

| 定义处 | 烧入色值 |
|---|---|
| [tokens.css](file:///home/gzz/crabmate/client/frontend/styles/tokens.css#L148)（默认深色） | `#9099b4` |
| [crabmate-light.css](file:///home/gzz/crabmate/client/frontend/themes/crabmate-light.css#L74) | `#6b6357` |
| [material-dark.css](file:///home/gzz/crabmate/client/frontend/themes/material-dark.css#L70) | `#97979f` |
| [high-contrast-dark.css](file:///home/gzz/crabmate/client/frontend/themes/high-contrast-dark.css#L55) | `#c8c8c8` |
| [shadcn-dark.css](file:///home/gzz/crabmate/client/frontend/themes/shadcn-dark.css#L75) | `#a1a1aa` |
| [shadcn-light.css](file:///home/gzz/crabmate/client/frontend/themes/shadcn-light.css#L80) | `#696971` |

唯一消费点：[status.css:356](file:///home/gzz/crabmate/client/frontend/styles/status.css#L356)。**每新增一个主题就要复制一份。**

> 排除项：`base.css:63` 的 data-URI 是 `feTurbulence` 噪点纹理（256×256），不是图标，不纳入本方案。`frontend/vendor/ide-codemirror.js` 内的 1 处属第三方 vendor，不在契约与改造范围内。

### 机制 3：文本字形图标

实测命中位置（`frontend/src`）：

| 字形 | 用途 | 位置 |
|---|---|---|
| `×` | 关闭 | `ide_find_bar.rs:161,254`、`ide_tabs_bar.rs:283`、`chat/composer_pending_images.rs:36`、`chat/find_bar.rs:170`、`chat/chat_image_lightbox.rs:270`（**JS 侧** `set_text_content`）、`tauri_window_controls.rs:45`（`.tauri-win-ctrl-glyph`）——共 **7 处，跨 Rust 与 JS** |
| `‹` `›` | 左右/前进后退 | `sidebar_nav/mode_actions.rs:112`、`app/mod.rs:128`、`ide_find_bar.rs:96,105`、`ide_menu_bar/file_menu.rs:209`（`.ide-menu-submenu-chevron`） |
| `▾` | 展开 | `approval_bar.rs:40`、`workspace_tree.rs:637`、`layout-chat.css:371`（`content:`） |
| `▸` | 折叠 | `workspace_tree.rs:639` |
| `✓` | 勾选 | `ide_menu_bar/view_menu.rs:63,78` |
| `●` | 脏标记 | `ide_tabs_bar.rs:265`、`mobile_shell_header.rs:94` |
| `−` | 最小化 | `tauri_window_controls.rs:29`、`settings_sections.rs:693` |
| `…` | 更多 | `ide_layout.rs:112`、`chat/tui_stream_dom_sync.rs:747` |
| `◈` | 装饰 | `layout-chat.css:365`（`content:`） |
| `↑` `↓` | 查找上下条 | `chat/find_bar.rs:142,161` |
| `—` | 空值占位 | `status_bar.rs:304`、`i18n/settings_mcp.rs:216-217` |

关键观察：**28 处内联 SVG 里没有任何「关闭」图标**——所有关闭按钮都用文本 `×`。也就是说机制 1 与机制 3 之间没有可复用的替代关系，`×` 是「当前无 SVG 可换」的现状，而不是「有 SVG 却用文本」的退化。

## 实测不一致

以下五条 + 一条继承策略差异，是「手抄 N 次」的直接产物。

1. **属性模板手抄 18 次**，`svg_common()` 只覆盖 `workspace_shell.rs` 自己的 10 个图标。

2. **`aria-hidden="true"` 三层重复**：SVG 上有，外层 `<span aria-hidden="true">` 上还有（如 `side_column_toolbar.rs:87`、`layout_mode_segment` 外层、`status_agent_role_menu` 触发按钮）。冗余但无害，收敛时可一并理清。

3. **`xmlns` 有无不一**：`chat/column.rs:530`（`fill="currentColor"` 那处）与 `:569` 带 `xmlns`，`workspace_shell.rs` 带；其余手写多数**不带**。HTML5 解析器内联 SVG 可不写，但现状是无标准的随机分布。

4. **2 处 SVG 完全没有 `class`**（[settings_page/header.rs:59](file:///home/gzz/crabmate/client/frontend/src/app/settings_page/header.rs#L59)、[ide_settings_page/view_header.rs:24](file:///home/gzz/crabmate/client/frontend/src/app/ide_settings_page/view_header.rs#L24)），靠父选择器 `.settings-page-back svg`（`modal.css:902`）命中。这两处还是**同一个左箭头 chevron**（`<polyline points="15 18 9 12 15 6" />`）的重复实现。注意此形态处于 `check-css-contract.sh` 的灰度区：消费者没有类名，门禁自然不报警。

5. **全仓无 `--icon-*` token**（`grep --icon-` 仅命中本方案新建的门禁脚本注释，正式代码 0 处）。图标尺寸是字面量且大量重复：

   - `1.125rem`（= 15.75px，根字号 14px）出现在 **6 个类**：`sidebar.css:312`、`mobile.css:349`、`shell-ds.css:218`、`shell-ds.css:571`、`shell-ds.css:1057`、`shell-ds.css:1185`。
   - `0.625rem`（= 8.75px）出现在 **3 个类**：`status.css:134`、`status.css:237`、`shell-ds.css:584`。
   - `1.25rem` 容器：`sidebar.css:300`、`sidebar.css:440`。

   注意 `frontend/styles/*.css` 存在**字面量预算棘轮**（`scripts/css_literals_budget.txt`，`scripts/check-css-literals.sh`）：把上面的尺寸改成 token 后，须同步下调对应文件的计数预算——该棘轮只管 `font-size` / 颜色 / `border-radius` 三类，`width` / `height` 尺寸字面量不在其中，故本项**不触发**该门禁，但仍应把 `1.125rem` 收进 token 以免再度分叉。

6. **`stroke` / `stroke-width` 挂载位置两种策略**：`workspace_shell.rs` 放在**子 `path`** 上，其余 18 处放在 `<svg>` 元素上。两者渲染结果等价（`stroke` 可继承），但混用会让「改一处模板」的假设失效。

## 方案

分四步，按依赖顺序推进；每步独立可交付、可回滚。

### 第 1 步：共享 `Icon` 组件收敛属性模板

在 `frontend/src/` 新增图标组件（放哪个模块待定，倾向与既有 `components` 组织一致），把「`viewBox` / `fill` / `stroke` / `stroke-width` / `stroke-linecap` / `stroke-linejoin` / `aria-hidden` + 尺寸类」收敛为唯一来源，只让调用方提供 `d` / `points` / 子节点与类名。

- 覆盖范围：先迁 18 处手写（含 2 处无 `class` 的重复 chevron，合并为同一图标）。
- `workspace_shell.rs` 的 `svg_common()` 逐步并入该组件，或保留但其产物与组件同模板。
- `stroke` 挂载位置统一到 `<svg>` 元素（与多数现状一致，改动面更小）。
- 门禁联动：迁移后消费者类名会变化，须同步跑 `scripts/check-css-contract.sh`（新类名要有同 scope CSS 规则，否则进 `scripts/css_contract_allowlist.txt`）。

### 第 2 步：建立 `--icon-*` 尺寸 token

在 `tokens.css` 排版区附近建立尺寸 token（与 `--text-*` 同区，语义相邻）：

- `--icon-sm` = `0.625rem`（8.75px，现状 3 个类）
- `--icon-md` = `0.875rem`（12.25px，用于 `chevron` 一类的中间档；**新增档位需先确认无现成值可复用**）
- `--icon-lg` = `1.125rem`（15.75px，现状 6 个类）

把现状字面量改为 `var(--icon-*)`。因六套主题同值，视觉应零变化——**须逐主题肉眼比对确认**。

### 第 3 步：决定文本字形（机制 3）的去留

按用途分两类处理：

- **可换 SVG 的**：`×`（7 处，含 JS 侧那处）、`‹` `›`、`▾` `▸`、`✓`、`−`、`…`、`↑` `↓` 均可用一个 24×24 路径替换，纳入第 1 步组件后即统一。
- **应保留字形的**：`●`（脏标记，本质是小圆点，用 CSS `border-radius` 画更合适，非图标）、`—`（空值占位，是排版符号不是图标）、`◈`（CSS `content` 装饰）、`▾` 在 CSS `content` 里的那处（伪元素无法放 SVG，须保留或改 `background`）。

判据：**能进 DOM 且需要随主题变色的 → 换 SVG；作为排版符号或伪元素内容的 → 保留**。

风险：`×` 的原生 DOM 那处（`chat_image_lightbox.rs:270`）——「在 WASM 外的 JS 片段里」的判断**有误**，实测是 WASM 内的 `web_sys` 建 DOM（`doc.create_element("button")` + `set_text_content(Some("×"))`），与 Leptos 同 crate，`Icon` 组件可达。但替换需在命令式子树里挂载 Leptos 视图，`UnmountHandle` 的类型参数是 `icon_x` 的 `impl IntoView::State`、**无法命名**从而存不进 `LightboxBind`，只能 `.forget()`（每次开灯箱泄漏一个 Owner）或用 `create_element_ns` 手写第二份模板；而该按钮已有 `aria-label`（字形仅装饰）、颜色经 `color: var(--text)` 已随主题变化，故**保留文本**。完整权衡见「实施记录 · 第 3 步」。

### 第 4 步：CSS mask 替代 data-URI

用 `mask-image` + `background-color: currentColor` 替代「色值烧进 URI」：

```css
.icon-chevron-down {
  background-color: currentColor;
  -webkit-mask-image: var(--chevron-down-svg);
  mask-image: var(--chevron-down-svg);
}
```

这样 data-URI 只需定义 **1 份**（不再随主题复制），颜色由 `currentColor` 跟随主题。

**前置条件（本轮完成 WebKitGTK 侧版本核对；实机渲染转手工，见「实施记录 · 第 4 步」）**：

- `mask-image` 在 **WebKitGTK**（Desktop Linux 壳）与 **Android WebView** 上需确认支持与 `-webkit-` 前缀行为；
- `mask` 的 `size` / `position` / `no-repeat` 简写兼容性（建议一律写长名属性 + `-webkit-` / 无前缀成对声明，绕开简写差异）；
- 若任一平台不支持，则退回「保持每主题一份 data-URI」，并把第 4 步从方案中移除，仅保留第 1–3 步。

## 验证与门禁

复用现有 CSS 门禁，并新增一道图标门禁：

- `bash scripts/check-css-contract.sh` —— 改了类名就跑（第 1、3 步主要风险点）。
- `bash scripts/check-css-tokens.sh` —— 新增 `--icon-*` 后确认六套主题覆盖完整（union 规则要求所有主题覆盖同一 token 集）。
- `bash scripts/check-css-literals.sh` —— `width` / `height` 尺寸字面量不在其预算内，但改动若顺带动了 `font-size` / 颜色 / `border-radius` 须同步 `scripts/css_literals_budget.txt`。
- `bash scripts/check-css-breakpoints.sh` —— 图标若在响应式块内有尺寸覆写，须与 `MOBILE_LAYOUT_BREAKPOINT_PX` 一致。
- `bash scripts/check-icons.sh` —— 图标门禁（本轮新增，见下）。
- `bash scripts/check.sh` —— 全量。

**图标门禁（已落地）**：`scripts/check-icons.sh` + `scripts/icons_check.py`（与既有 `check-css-*.sh` 同范式，已接入 `scripts/check.sh` 与 `.pre-commit-config.yaml`），两条规则：

1. `frontend/src/**/*.rs`（除 `icon.rs`）不得出现字面 `<svg` —— 内联 SVG 一律走共享组件，顺带堵住「在 HTML 字符串里内联一份 `<svg>`」这条捷径；
2. `frontend/styles/*.css` 里 `svg` 类型选择器规则的 `width` / `height`（含 `min-` / `max-`）必须是 `var(--icon-*)` —— 尺寸只有一个来源。

有意例外在规则所在行写 `icon-gate: allow <理由>`（Rust 该行 / CSS 该规则的前置注释）。

**已知盲区**：本门禁**不**校验「类名与元素是否同体」。若把容器类名直接挂到 `<svg>` 上，`.container svg` 这类**永不匹配的后代选择器**仍会为 `.container` 提供「类名已覆盖」的假证据（`check-css-contract.sh` 只查类名是否出现过），本门禁也看不到 —— 2026-09-25 的 `workspace-tree-chevron` 回归即属此类，需靠 code review 兜住。

**必做实测项**（非门禁能覆盖）：

1. 第 2 步后逐主题比对图标尺寸，确认零视觉变化。
2. 第 3 步后逐个替换点确认键盘可达性（关闭按钮多为 `<button>`，换 SVG 后 `aria-label` 须保留）。
3. 第 4 步在 WebKitGTK + Android WebView 实机确认 mask 渲染（**本轮未做，已转手工**；自动化探测因缺 `python3-gi-cairo` 放弃，版本核对结论见「实施记录 · 第 4 步」）。

## 附：与既有文档的关系

- 本方案是 `docs/design/ui_issue_todo.md` **Phase 1a–1d**（字号 / 语义色 / 高光前景 / 遮罩阴影 token 归一）的延续——那四期把「颜色与字号」字面量清到 0，本方案处理剩下的一类字面量：**图标**。
- 「P2 · 样式与 token」中尚未勾选的「启动 splash 硬编码深色 `#07090e`」与本方案无关（那是启动底色，非图标），不要在本方案内顺带修改。
- 落地后在 `ui_issue_todo.md` 新增一条 P2 条目追踪，或在本文档顶部勾选状态；**单一信息源**：清单指本文档，本文档不复制清单内容。

## 实施记录（2026-09-25）

已按上节要求勾选本文档顶部状态；`ui_issue_todo.md` 不再新增条目（单一信息源指向本文件）。

### 第 1 步：共享 `Icon` 组件 —— 完成

新增 [frontend/src/icon.rs](file:///home/gzz/crabmate/client/frontend/src/icon.rs)：`Icon` 组件（`viewBox` / `fill` / `stroke` / `stroke-width` / 线帽端点 / `aria-hidden` 唯一来源，`IconStyle::Stroke` / `Fill` 二态）+ 11 个图形 helper（`icon_chevron_left` / `_right` / `_down`、`icon_x`、`icon_check`、`icon_minus`、`icon_plus`、`icon_maximize`、`icon_arrow_up` / `_down`、`icon_search`）。

- 迁移后全仓图标渲染点 **52 处**：29 处经 helper 调用 + 23 处直接 `<Icon>`（其中 `workspace_shell.rs` 的 10 个文件类型图标整组并入）。`git diff` 侧的证据：删除 28 行手写 `<svg>` 开标签、25 行文本字形。
- 其中 2 处**无 `class`** 的设置页返回 chevron 合并为 `icon_chevron_left`。
- 文档「机制 3」清单未列而实测存在的 `□`（窗口最大化）与 `+`（字号步进 / 新建对话）按同组一致性一并换 SVG。
- 审查阶段补漏：`⌕`（[mode_actions.rs:70](file:///home/gzz/crabmate/client/frontend/src/app/sidebar_nav/mode_actions.rs#L70)，侧栏搜索面板开关）同为「文本字形图标」且不在文档清单内，换成第 11 个 helper `icon_search`。
- `workspace_shell.rs` 的 `svg_common()` **删除**，其 10 个文件类型图标全部并入 `<Icon class="workspace-entry-icon workspace-entry-svg">`（第 1 步文档允许的「逐步并入该组件」路径，`stroke` / `stroke-width` 因此从子元素移到 `<svg>`）。
- `stroke` / `stroke-width` 挂载位置统一到 `<svg>`（文档第 136 行）。
- 门禁联动结论：消费者类名沿用原有字面量；`Icon` 内部 `class=class` 是变量，不被 `check-css-contract.sh` 的 `RE_ATTR` 采集；无 class 处写 `icon_x("")`（函数调用），故 `css_contract_allowlist.txt` **无需新增条目**。

### 第 2 步：`--icon-*` 尺寸 token —— 完成

[tokens.css](file:///home/gzz/crabmate/client/frontend/styles/tokens.css#L46-L52) 排版区新增三档（与文档规划一致，**未新增第四档**）：`--icon-sm` 0.625rem / `--icon-md` 0.875rem / `--icon-lg` 1.125rem，与 `--text-*` 同锚点（`1rem = --text-lg = 14px`）。

- 档位映射：被替换前文本字形的渲染量级 —— 9–11px 字号 → `--icon-sm`；12–14px → `--icon-md`；16–18px → `--icon-lg`。
- **文档第 185 行的主题覆盖预期修正**：`check-css-tokens.sh` 的「主题必须覆盖同一 token 集」union 规则**只扫 `frontend/themes/*.css`**。`--icon-*` 与 `--text-*` 同层定义在 `tokens.css`，六套主题自动继承，**不需要**逐主题复制，也不会触发主题覆盖缺口（实测缺口 0）。
- **文档第 122 行的预算预期确认**：尺寸字面量不在 `css_literals_budget.txt` 之内，改 token 不触发该门禁；实测 `frontend/styles` 内 `font-size` 字面量仍为 0。
- 消费点尺寸一律 `width/height: var(--icon-*)`；无 class 的图标（`icon_x("")` 等）走**容器选择器**（如 `.tauri-win-ctrl-glyph svg`）定尺寸，避免出现无尺寸规则的裸 `<svg>`。`.settings-font-size-stepper-btn` 属白名单语义标识（无自身规则），故用容器选择器 `.settings-font-size-stepper svg`，避免白名单条目变 stale。
- **审查阶段补收**：`modal.css` 里 3 处遗留尺寸字面量按同一档位映射收进 token —— `.settings-model-registry-add svg`（14px → `--icon-md`，渲染 −1.75px）、`.settings-model-registry-edit svg`（16px → `--icon-lg`，−0.25px）、`.settings-page-back svg`（16px → `--icon-lg`，−0.25px）。**至此 `frontend/styles` 内不存在图标尺寸字面量**（原 16px 不落在三档上，按「12–14px → `--icon-md`、16–18px → `--icon-lg`」就近映射，偏差 ≤0.25px）。

### 第 3 步：文本字形去留 —— 完成

按判据「**能进 DOM 且需要随主题变色的 → 换 SVG；作为排版符号或伪元素内容的 → 保留**」：

- **换 SVG**：`×`（Rust 侧 6 处）、`‹` `›`、`▾` `▸`、`✓`、`−`、`↑` `↓`（以上均在文档清单内）+ 清单外的 `□`（窗口最大化）、`+`（字号步进 / 新建对话）、`⌕`（侧栏搜索开关）。
- **保留字形**（含 3 处对文档清单的修正 + 审查阶段补登记的同类项）：
  1. `…` @ `ide_layout.rs:112` —— 文档记为「更多」图标有误，实测是加载省略号且为 `role="status"` 的**唯一可访问名**，换 SVG 会令读屏失去内容 → 保留。
  2. `…` @ `chat/tui_stream_dom_sync.rs:747` —— 文档清单此处有误，实测是 `#[test]` 内的测试桩数据 → 保留。
  3. `×` @ `chat/chat_image_lightbox.rs:270` —— 命令式 `web_sys` 建 DOM。原文档（第 158 行）记为「WASM 外的 JS 片段、可能访问不到组件」**有误**：该函数是 WASM 内的 Rust，与 Leptos 同 crate、组件可达。但实测替换代价高于收益，故仍保留文本，理由见下方「第 3 步补：原生 DOM 路径复评」。
  4. `●`（脏标记，本质小圆点）、`—`（空值占位，排版符号）、CSS `content` 的 `◈`（`layout-chat.css:365`）与 `▾`（`layout-chat.css:371`，伪元素放不下 SVG）。
  5. 文档盘点未列、审查阶段一并登记为「保留」的同类项：`▸` @ [chat/tui_tool_process.rs:373](file:///home/gzz/crabmate/client/frontend/src/app/chat/tui_tool_process.rs#L373)（**HTML 字符串**经 `set_inner_html` 注入，无元素句柄；`layout-chat.css` 靠 `transform: rotate(90deg)` 表达展开态，复评见下）、` ✓` @ [settings_mcp_status.rs:304](file:///home/gzz/crabmate/client/frontend/src/app/settings_mcp_status.rs#L304)（保存成功反馈文案的后缀排版符号）、`⚙️` 等工具卡 emoji @ [i18n/tool_cards.rs:196](file:///home/gzz/crabmate/client/frontend/src/i18n/tool_cards.rs#L196)（工具种类的彩色 emoji 体系，属另一议题）。

### 第 3 步补：原生 DOM 路径的 `×` / `▸` 复评 —— 结论为「保留文本」

第 3 步把两处判为「保留」时给的**理由是错的**（「在 WASM 外 / 访问不到 Leptos」）。本轮复评按实际代码重新定论，结论不变但依据更换：

- **`×` @ [chat_image_lightbox.rs:270](file:///home/gzz/crabmate/client/frontend/src/app/chat/chat_image_lightbox.rs#L270)**（`btn.set_text_content(Some("×"))`）：`build_overlay` 是 WASM 内的 Rust 函数（`doc.create_element("button")` + `set_text_content`），**组件可达**。可用且仅有三条替换路径，各有一处硬伤：
  1. `leptos::mount::mount_to(btn_html_element, || icon_x(""))` —— 复用组件、零重复，但返回的 `UnmountHandle` 类型参数是 `icon_x` 的 `impl IntoView::State`，**无法命名**，因而存不进 `LightboxBind` 以随灯箱关闭而释放；只能 `.forget()`，即每次打开灯箱永久泄漏一个 reactive `Owner`。
  2. `create_element_ns(svg_ns, "svg")` 手搭 —— 又把属性模板抄了第二份（门禁也看不到，因为它不是字面 `<svg`），正是本方案要消除的东西。
  3. `set_inner_html("<svg …>")` 字符串常量 —— 同上，第二份模板来源。
  另加两条削弱替换收益的事实：该按钮已有 `aria-label`（字形纯装饰，无信息量），且 `shell-ds.css:951` 的 `color: var(--text)` 使字形**已经随主题变色**——判据「需要随主题变色」由文本颜色即已满足。故**保留文本**，不为一个装饰字形引入生命周期 hack 或第二份模板。
- **`▸` @ [tui_tool_process.rs:373](file:///home/gzz/crabmate/client/frontend/src/app/chat/tui_tool_process.rs#L373)**（`html.push_str("<span …>▸</span>")`）：该处是**纯字符串拼接**，产物经 `set_inner_html` 注入且随流式同步反复重建——函数内没有元素句柄可挂载，只能走上面第 2 / 3 条（第二份模板）。且本门禁规则 1 会直接拦下内联 `<svg` 字面量，使这条捷径在评审时显性化。故**保留文本**（展开/收起由 `layout-chat.css` 的 `transform: rotate(90deg)` 表达）。

> 小结：这两处与 `i18n/tool_cards.rs` 的 emoji 同属「Leptos 组件树之外」的渲染路径；共享 `Icon` 组件的适用边界就是**组件树内**。本方案不为此扩张组件的适用面。

### 第 4 步：CSS mask —— 未实施（本轮仅闭合 WebKitGTK 侧版本核对；实机渲染转手工）

前置条件核对进展：

- **WebKitGTK 侧（Desktop Linux 壳）—— 版本门槛已闭合**：本机 `libwebkit2gtk-4.1 = 2.52.6`（Debian 13 trixie，`pkg-config webkit2gtk-4.1` 同为 2.52.6）。无前缀 `mask` / `mask-image` 与 `mask-size` / `mask-position` / `mask-repeat` / `mask-mode` / `mask-composite` 长名属性自 **Safari 15.4**（2022-03，与 WebKitGTK 2.36 同期）起支持，2.52.6 远高于该门槛；`-webkit-mask-image` 更早在 Safari 4 / 早期 WebKitGTK 即可用。故文档第 177 行「若任一平台不支持则退回」的退回条件在 WebKitGTK 侧**不成立**。
- **Android WebView 侧 —— 结论有分叉，仍未闭合**：Android System WebView 跟随 Chromium，无前缀 `mask-image` 需 **Chrome / WebView 120+**（2023-12），`-webkit-mask-image` 自 Chrome 4 起可用（长名属性仅子集）。本项目 `minSdk = 24`，WebView 版本随 Play 商店自更新，**构建期无法断言**——只能实机抽查，或直接接受「`-webkit-` + 无前缀成对声明」的写法兜住旧版。
- **实机渲染确认：本轮不做，转手工**。曾尝试自动化（WebKitGTK 经 `WebKit2.WebView.get_snapshot` 采像素）未成：本机缺 `python3-gi-cairo`，`gi.require_foreign("cairo")` 报 `No module named 'gi._gi_cairo'`，临时探测脚本已删除、不入库。若推进第 4 步，须由人工在 desktop 壳与 Android 实机确认渲染与简写行为（见「必做实测项」第 3 条）。

结论：**仍按文档指示退回，保留现状「每主题一份 data-URI」**（`--status-agent-select-bg-image` 共 6 份），第 4 步从本轮方案移除。WebKitGTK 侧门槛已闭合；Android 侧版本与两端实机渲染留待有实机条件时再议。

### 验证

- `bash scripts/check.sh` 全绿：`check-no-main-path` / `check-boundaries` / `check-css-breakpoints` / `check-css-contract` / `check-css-tokens` / `check-css-literals` / **`check-icons`（本轮新增）** / `check-xml-comments` / `cargo fmt` / `cargo clippy`（含 frontend wasm32）/ `lizard`（CCN>10 = 0）/ `ktlint-android`。
- `make test-frontend`：681 passed / 0 failed。
- **图标门禁本轮落地**（原「先在 pre-commit 观察，稳定后再固化」的建议被推翻：观察期内的 `workspace-tree-chevron` 回归证明回归空间是真实的，且有门禁也未必够——见下条盲区）：新增 `scripts/check-icons.sh` + `scripts/icons_check.py`，两条规则「`icon.rs` 之外无字面 `<svg`」与「`svg` 选择器规则的尺寸须为 `var(--icon-*)`」，已接入 `scripts/check.sh` 与 `.pre-commit-config.yaml`，并在 `AGENTS.md` 登记。规则 1 还顺带堵住了「在 HTML 字符串 / `set_inner_html` 里内联一份 `<svg>`」这条绕过共享组件的捷径。落地时按当前代码实测零违规（`<svg` 字面量仅存在于 `icon.rs`）。
- **审查阶段发现并修复的回归**：`workspace_tree.rs` 的树节点折叠箭头换 SVG 时丢掉了外层 `<span class="workspace-tree-chevron">`，类名落到 `<svg>` 本体 —— `sidebar.css` / `mobile.css` 里 `.workspace-tree-chevron svg` 这类**后代**选择器永不匹配（`--icon-sm` 丢失），而 `.workspace-tree-chevron` 自身的盒尺寸（桌面 20px / 窄屏 44px 触控区）直接压到 svg 上。修法是恢复 wrapper `<span>`（与 `.ide-menu-check` 同构，CSS 零改动）。
- **门禁盲区（本轮未闭合，已登记进 `check-icons.sh` 文档串与 `AGENTS.md`）**：`check-css-contract.sh` 只校验「消费者类名是否在 CSS 中出现过」，`.foo svg` 这类**永不匹配的后代选择器**照样为 `.foo` 提供证据；新增的 `check-icons.sh` 也只看「`svg` 规则尺寸是否为 token」，同样不校验「类名与元素是否同体」。因此上述 `workspace-tree-chevron` 这类回归仍无法被机械拦下，**须靠 code review 兜住**——这也是本轮在文档与脚本里都显式写明盲区的原因。

### 遗留（非本方案范围）

- **工具卡 emoji**（`i18n/tool_cards.rs` 按工具种类给彩色 emoji）与**原生 DOM 字符串路径的图标**（`▸` @ `tui_tool_process.rs`、`×` @ `chat_image_lightbox.rs`、` ✓` @ `settings_mcp_status.rs`）：前者是独立视觉体系；后者在 Leptos 组件树之外，替换要么需要生命周期 hack、要么再造一份属性模板（见「第 3 步补」），均按判据保留，清单见第 3 步第 3、5 条。
- **图标尺寸字面量已清零**：`frontend/styles` 内不再有图标尺寸的 px / rem 字面量（原 `.settings-page-back svg` 的 16px 已在审查阶段收进 `--icon-lg`）；第 2 步补收范围见上。
