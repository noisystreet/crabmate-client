# 对话界面待办

由对话主路径体检整理。修复后逐项勾选（`- [x]`）或移除。

**范围**：`frontend/src/app/chat/`（composer / transcript / 查找 / 消息菜单）、会话侧栏、审批弹窗、Ask/Plan/Act。Markdown 渲染见 [`markdown_render_todo.md`](markdown_render_todo.md)（P0–P2 已完成，不要当缺口重做）。壳层通用缺陷见 [`ui_issue_todo.md`](ui_issue_todo.md)。编程 Agent 更大范围（变更集审查、还原、git）见 [`coding_agent_client.md`](coding_agent_client.md)；本文 **P2** 是该规划的 Wave 1 勾选权威。

**非目标**（不要混进对话 PR）：

- KaTeX / Mermaid（WASM 体积、流式半截、净化白名单）
- 把 Markdown 从 `innerHTML` 改成整段虚拟 DOM
- 窄屏 / Android 上开放 IDE 布局（有意锁死）
- 再扩一堆 `normalize_markdown` 启发式

## 建议落地顺序

1. P1 可访问性（审批 Esc、焦点陷阱、附件/菜单键盘、ARIA）— 与 [`ui_issue_todo.md`](ui_issue_todo.md) P1 同一批
2. 就地改用户消息并走现有 branch / regen
3. 流式中排队下一句（扩展 `ComposerStreamFollowUp`，不要再加一套 Effect）
4. 查找命中高亮；Ask/Plan/Act 贴近输入框
5. 有体积预算再做聊天代码高亮

---

## P1 · 会打断对话的可访问性

与 [`ui_issue_todo.md`](ui_issue_todo.md) **P1** 对齐；此处只列对话主路径。

- [x] 审批弹窗：焦点陷阱 + Escape 提交 `deny`（`approval_modal.rs`、`escape.rs`）
- [x] 确认框 / 新建文件：焦点陷阱；Escape 在输入框内也关闭
- [x] 图片附件改为可 Tab 的 `<button>`（`column.rs`）
- [x] 上下文菜单：打开后焦点进第一项，方向键移动；会话行 / 文件行 / 消息 / IDE 标签支持 Shift+F10
- [x] 顶栏 / 底栏菜单：`menubar` 包裹聊天「项目」触发器；Ask/Plan/Act 与侧栏视图用 `menuitemradio` + `aria-checked`
- [x] IDE 标签栏方向键切换；文件树文件行 `tabindex=0` + Enter
- [x] 当前会话 `aria-current`；置顶 / 星标 / 未保存进入可访问名

## P2 · 对话产品缺口

- [x] 就地编辑用户消息再发送（右键「编辑」；走现有 branch / regen）
- [x] 流式进行中排队下一句（`ComposerStreamFollowUp::QueuedUserMessage`；发送钮在停止旁仍可用）
- [x] 聊天查找：命中高亮落在气泡内（wrap 描边 + `<mark>`）
- [x] Ask / Plan / Act 在 composer 附近可见（从底栏移到输入条）

## P3 · 体积敏感

- [ ] 聊天闭合代码块语法高亮（语言标签 + 复制已有；高亮单独评估 WASM）

## 验证

- 单元：`stream_follow_up_gates::decide_composer_send` / `user_edit_save_blocked`、`find_highlight::split_highlight_parts` / `find_restore_scope`、`composer_follow_up` 切会话写回草稿、`session_ops::set_user_message_text`、`tui_actions_bar` 用户菜单含 `edit`
- 手测：右键编辑用户消息后发送；流式中途点编辑保存应拒绝且不截断在途流；流式中再发送出现排队芯片，结束后发出；切走会话后排队正文出现在该会话草稿；查找命中气泡内高亮；composer 旁切 Ask/Plan/Act
- Playwright：`e2e/specs/mock-approval-scenarios.spec.ts` 的 Escape 关审批（既有）
