# UI 图标统一样式方案

状态：已实施（第 1–3 步落地；第 4 步因前置条件未实测，按本方案指示退回并从方案中移除 —— 见文末「实施记录」）
日期：2026-09-25
范围：`frontend/` 业务 UI（Web / Desktop / Android 共用）；不含 `crates/crabmate-connect/assets/connect.html`、`desktop-tauri/splash.html` 两个独立启动页，也不含第三方 vendor 资源

## 背景

`frontend/` 没有图标库、没有图标字体、没有 sprite，图标全靠三套机制**并存**手工维护：

1. **内联 SVG** —— 写在 Leptos view 里（28 处 / 10 个文件）。
2. **CSS `data-URI`** —— 把 SVG 编码进自定义属性，再由 `background` 消费（1 个 token，主题各一份）。
3. **文本字形** —— 用 Unicode 字符当图标（`×` `‹` `▾` `✓` `●` …）。

三套机制各自可用，但当主题从 1 套（深色默认）扩到 6 套（`:root` + `light` / `material` / `high-contrast` / `shadcn` / `shadcn-light`）后，成本开始显性化：data-URI 每加一个主题就要复制一份并重新编码色值；内联 SVG 的属性模板手抄 18 次，任一处漏改就与其余不一致；文本字形的字形表现依赖系统字体，跨平台（WebKitGTK / Android WebView）不稳定。本方案给出收敛路径。

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
| [light.css](file:///home/gzz/crabmate/client/frontend/themes/light.css#L74) | `#6b6357` |
| [material.css](file:///home/gzz/crabmate/client/frontend/themes/material.css#L70) | `#97979f` |
| [high-contrast.css](file:///home/gzz/crabmate/client/frontend/themes/high-contrast.css#L55) | `#c8c8c8` |
| [shadcn.css](file:///home/gzz/crabmate/client/frontend/themes/shadcn.css#L75) | `#a1a1aa` |
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

   注意 `frontend/styles/*.css` 存在**字面量预算棘轮**（`scripts/css_literals_budget.txt`，`scripts/check-css-literals.sh`）：把上面的尺寸改成 token 后，须同步下调对应文件的 `font-size` / 颜色计数预算之外的尺寸计数——该棘轮只管 `font-size` 与颜色两类，尺寸字面量不在其中，故本项**不触发**该门禁，但仍应把 `1.125rem` 收进 token 以免再度分叉。

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

把现状字面量改为 `var(--icon-*)`。因四主题同值，视觉应零变化——**须逐主题肉眼比对确认**。

### 第 3 步：决定文本字形（机制 3）的去留

按用途分两类处理：

- **可换 SVG 的**：`×`（7 处，含 JS 侧那处）、`‹` `›`、`▾` `▸`、`✓`、`−`、`…`、`↑` `↓` 均可用一个 24×24 路径替换，纳入第 1 步组件后即统一。
- **应保留字形的**：`●`（脏标记，本质是小圆点，用 CSS `border-radius` 画更合适，非图标）、`—`（空值占位，是排版符号不是图标）、`◈`（CSS `content` 装饰）、`▾` 在 CSS `content` 里的那处（伪元素无法放 SVG，须保留或改 `background`）。

判据：**能进 DOM 且需要随主题变色的 → 换 SVG；作为排版符号或伪元素内容的 → 保留**。

风险：`×` 的 JS 侧那处（`chat_image_lightbox.rs:270`）在 WASM 外的 JS 片段里，替换时需确认该处能否访问 Leptos 组件（不能的话保持文本，或改为内联 SVG 字符串）。

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

**前置条件（待实测，不可断言支持）**：

- `mask-image` 在 **WebKitGTK**（Desktop Linux 壳）与 **Android WebView** 上需确认支持与 `-webkit-` 前缀行为；
- `mask` 的 `size` / `position` / `no-repeat` 简写兼容性；
- 若任一平台不支持，则退回「保持每主题一份 data-URI」，并把第 4 步从方案中移除，仅保留第 1–3 步。

## 验证与门禁

复用现有五道 CSS 门禁，不新增体系：

- `bash scripts/check-css-contract.sh` —— 改了类名就跑（第 1、3 步主要风险点）。
- `bash scripts/check-css-tokens.sh` —— 新增 `--icon-*` 后确认四主题覆盖完整（union 规则要求所有主题覆盖同一 token 集）。
- `bash scripts/check-css-literals.sh` —— 尺寸字面量不在其预算内，但改动若顺带动了 `font-size` / 颜色须同步 `scripts/css_literals_budget.txt`。
- `bash scripts/check-css-breakpoints.sh` —— 图标若在响应式块内有尺寸覆写，须与 `MOBILE_LAYOUT_BREAKPOINT_PX` 一致。
- `bash scripts/check.sh` —— 全量。

可选：新增 `check-icons.sh`（与既有 `check-css-*.sh` 同范式，`scripts/check-icons.sh` + `scripts/icons_check.py`）强制「内联 SVG 一律走共享组件」与「无裸 `1.125rem` 图标尺寸」。是否值得加，取决于第 1、2 步落地后是否仍有回归空间——**建议先在 pre-commit 观察，稳定后再固化为门禁**。

**必做实测项**（非门禁能覆盖）：

1. 第 2 步后逐主题比对图标尺寸，确认零视觉变化。
2. 第 3 步后逐个替换点确认键盘可达性（关闭按钮多为 `<button>`，换 SVG 后 `aria-label` 须保留）。
3. 第 4 步在 WebKitGTK + Android WebView 实机确认 mask 渲染。

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
  3. `×` @ `chat/chat_image_lightbox.rs:270` —— WASM 外原生 DOM 构建，无法访问 Leptos 组件（文档第 158 行已允许保持文本）→ 保留。
  4. `●`（脏标记，本质小圆点）、`—`（空值占位，排版符号）、CSS `content` 的 `◈`（`layout-chat.css:365`）与 `▾`（`layout-chat.css:371`，伪元素放不下 SVG）。
  5. 文档盘点未列、审查阶段一并登记为「保留」的同类项：`▸` @ [chat/tui_tool_process.rs:373](file:///home/gzz/crabmate/client/frontend/src/app/chat/tui_tool_process.rs#L373)（**HTML 字符串**写进原生 DOM，与第 3 条同因；`layout-chat.css` 靠 `transform: rotate(90deg)` 表达展开态）、` ✓` @ [settings_mcp_status.rs:304](file:///home/gzz/crabmate/client/frontend/src/app/settings_mcp_status.rs#L304)（保存成功反馈文案的后缀排版符号）、`⚙️` 等工具卡 emoji @ [i18n/tool_cards.rs:196](file:///home/gzz/crabmate/client/frontend/src/i18n/tool_cards.rs#L196)（工具种类的彩色 emoji 体系，属另一议题）。

### 第 4 步：CSS mask —— 未实施（按方案指示退回）

前置条件（`mask-image` / `-webkit-mask-*` 在 **WebKitGTK** 与 **Android WebView** 的实机支持与简写行为）在本机无法验证。依文档第 178 行「若任一平台不支持，则退回」的指示：**保留现状「每主题一份 data-URI」**（`--status-agent-select-bg-image` 共 6 份），第 4 步从本轮方案移除，待有实机验证条件后再议。

### 验证

- `bash scripts/check.sh` 全绿：`check-no-main-path` / `check-boundaries` / `check-css-breakpoints` / `check-css-contract` / `check-css-tokens` / `check-css-literals` / `check-xml-comments` / `cargo fmt` / `cargo clippy`（含 frontend wasm32）/ `lizard`（CCN>10 = 0）/ `ktlint-android`。
- `make test-frontend`：681 passed / 0 failed。
- 门禁脚本未新增（文档第 190 行的可选 `check-icons.sh`）：按「先在 pre-commit 观察，稳定后再固化」的建议，本轮不引入。
- **审查阶段发现并修复的回归**：`workspace_tree.rs` 的树节点折叠箭头换 SVG 时丢掉了外层 `<span class="workspace-tree-chevron">`，类名落到 `<svg>` 本体 —— `sidebar.css` / `mobile.css` 里 `.workspace-tree-chevron svg` 这类**后代**选择器永不匹配（`--icon-sm` 丢失），而 `.workspace-tree-chevron` 自身的盒尺寸（桌面 20px / 窄屏 44px 触控区）直接压到 svg 上。修法是恢复 wrapper `<span>`（与 `.ide-menu-check` 同构，CSS 零改动）。
- **门禁盲区（本轮未闭合）**：`check-css-contract.sh` 只校验「消费者类名是否在 CSS 中出现过」，`.foo svg` 这类**永不匹配的后代选择器**照样为 `.foo` 提供证据，因此「类名与元素同体」这类回归无法被拦下 —— 上述 `workspace-tree-chevron` 回归正是这类。文档第 190 行的可选 `check-icons.sh`（图标尺寸规则须为 `svg` 选择器而非其容器）是闭合该盲区的方向，仍留待后续。

### 遗留（非本方案范围）

- **工具卡 emoji**（`i18n/tool_cards.rs` 按工具种类给彩色 emoji）与**原生 DOM 字符串路径的图标**（`▸` @ `tui_tool_process.rs`、`×` @ `chat_image_lightbox.rs`、` ✓` @ `settings_mcp_status.rs`）：前者是独立视觉体系，后者在 Leptos 组件树之外没有替换点，均按判据保留，见第 3 步清单第 3、5 条。
- **图标尺寸字面量已清零**：`frontend/styles` 内不再有图标尺寸的 px / rem 字面量（原 `.settings-page-back svg` 的 16px 已在审查阶段收进 `--icon-lg`）；第 2 步补收范围见上。
