# UI 问题待办清单

由 UI 功能体检整理，按优先级排序；修复后逐项勾选（`- [x]`）或移除。

## P0 · 明确缺陷

- [ ] `theme=system` 在 Linux 非 GNOME（KDE / XFCE / 无 gsettings）下固定浅色：`desktop-tauri/src-tauri/src/os_theme.rs` 仅探测 GNOME 的 gsettings，无 portal / KDE 回退，双通道（窗口 `.theme()` + 前端 `TAURI_OS_DARK_HINT`）一起落到 light。
- [ ] IDE 语法高亮在 material / high-contrast（均为深色）下仍用浅色 token：`frontend/styles/ide-highlight.css`、`ide-codemirror.css` 仅覆盖 `[data-theme="dark"]`，且 `--ide-hl-*` 变量未定义、靠浅色字面量兜底。

## P1 · 可访问性关键缺口

- [ ] 审批弹窗无焦点陷阱且无法 Esc 关闭：`frontend/src/app/approval_modal.rs`、`frontend/src/app/app_shell_effects/escape.rs` 未覆盖 `pending_approval`。
- [ ] 部分对话框缺焦点陷阱：`ide_new_file_modal.rs`、`shell_confirm_dialog.rs`、`ide_confirm_dialog.rs`。
- [ ] 键盘不可达：图片附件 `<label>`（`column.rs`）、右键/长按上下文菜单、顶部/底栏菜单、IDE 标签页（缺方向键）、工作区文件树文件行。
- [ ] 语义缺口：聊天模式 `role="menuitem"` 孤儿节点；单选/当前会话缺 `aria-checked` / `aria-current`；未保存/置顶/星标状态对屏幕阅读器不可见。

## P2 · 移动端边界与体验

- [ ] Android `adjustResize` 生效设备上 IME 可能双倍抬高 composer：`MainActivity.kt` 的 `--cm-ime-inset` 与 `--vv-keyboard-inset` 取 `max`，窗口已压缩时 `ime` 仍非零。
- [ ] 左侧 20px 点击盲区：`frontend/styles/shell-ds.css` `.nav-rail-edge-hit` 为 `pointer-events:auto` 且无点击处理（右侧感应条为 `none`，左右不对称）。
- [ ] 768px 断点在 Rust（`app_prefs.rs`）与多份 CSS 重复硬编码，无单一来源/校验，改漏会脱节。
- [ ] 未定义 token 硬编码 fallback（`--shell-border` / `--surface-1` / `--accent-muted` / `--accent-warn` 等），切主题时这些位置颜色不变。
- [ ] 纯浏览器宽屏触控平板（>768px 非壳）软键盘不抬高 composer。

## P3 · 次要

- [ ] `prefers-reduced-motion` 漏 2 处无限动画：会话流式徽章脉冲、克隆进度条。
- [ ] 首屏主题快照硬编码 `light`，深色用户有短暂浅色闪烁。
- [ ] 对比度风险点 `frontend/styles/shell-ds.css:303`（`--muted` 再稀释），需实测验证。
