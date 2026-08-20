# Markdown 渲染待办

由聊天 Markdown 模块体检整理。修复后逐项勾选（`- [x]`）或移除。

**范围**：`frontend/src/markdown.rs`（normalize → pulldown-cmark → ammonia）、`frontend/src/markdown/sanitize.rs`（任务列表 / `language-*` 白名单）、`frontend/src/app/chat/tui_line_markdown.rs`（流式按块冻结）、`frontend/src/message_render.rs`、`frontend/styles/layout-chat.css`（`.chat-tui-line` / `.msg-md-prose`）。变更集模态与聊天主列共用 `to_safe_html`；闭合 transcript 块已复用 `.msg-md-prose`。

**非目标**（不要当缺口做）：

- `ENABLE_SMART_PUNCTUATION`（会改代码里的引号与 `--`）
- Wikilink / YAML front matter
- 把整段 Markdown 改成虚拟 DOM、丢掉 `ammonia` + `innerHTML`（架构重写，单独立项）
- 数学（KaTeX）/ Mermaid：依赖重，见文末「单独立项」

## 建议落地顺序

1. P0 白名单（任务列表 `input` + `code` 的 `language-*`）
2. 对齐 `.chat-tui-line` 与 `.msg-md-prose`
3. 闭合代码块：语言标签 + 复制；高亮第二步
4. 流式斜体/链接、`~~~` 围栏、引用单独成块
5. 按需开 `ENABLE_GFM` alert

---

## P0 · 开关开了但用户看不见

- [x] **任务列表是空开的开关**：`markdown.rs` 已 `ENABLE_TASKLISTS`，pulldown-cmark 输出 `<input type="checkbox">`；ammonia 默认 tags 不含 `input`，复选框被剥掉。白名单 `input[type=checkbox][disabled]`（只读），并补 `.chat-tui-line` / `.msg-md-prose` 任务列表样式。测试：`- [ ]` / `- [x]` 渲染后仍有 checkbox，且不可被点击改状态。
  - 落地：`frontend/src/markdown/sanitize.rs` 放行 `input`，并强制 `type=checkbox` + `disabled`（注入的 `type=text` 也会被改成只读勾选框）；CSS `pointer-events: none`。
- [x] **`language-*` class 被剥掉**：围栏会生成 `class="language-rust"`；ammonia `allowed_classes` 默认为空，语言信息到不了 DOM。允许 `pre`/`code` 上的 `language-*`。测试：` ```rust ` 输出 HTML 含 `language-rust`。这是语法高亮与代码块工具条的前置条件。
  - 落地：`code`/`pre` 的 `class` 经 `attribute_filter` 只保留 `language-` + ASCII token（含 `+` `.` `-` `#`，如 `c++` / `c#`）；其它 class 丢掉。
- [x] **主聊天样式远薄于变更集模态**：`.chat-tui-line` 只有 margin、表格线、横向滚动；h1–h6 字号、`blockquote`、`hr`、链接色、`pre` 底、行内 `code` 底只写在 `.msg-md-prose`（`changelist_modal.rs`）。把 prose 抽到共用层，或让 transcript 复用 `msg-md-prose` 子集，避免两套观感。
  - 落地：闭合块 / 围栏包装增加 `msg-md-prose`（`--active` / `--plain` / `--blank` 不加）；任务列表 CSS 用 `li:has(input[type="checkbox"])`（含宽松列表里包在 `<p>` 中的 checkbox）。

## P1 · 体验缺口

- [x] **闭合代码块 UX**：无语法高亮、无语言标签、不能单块复制（整条消息可复制，见 `tui_actions_bar.rs`）。IDE 已有 `ide_syntax_highlight.rs` / CodeMirror，聊天 `pre`/`code` 未用。先做语言标签 + Copy；高亮第二步，控制 WASM 体积。依赖 P0 的 `language-*` 白名单。
  - 落地：`markdown/code_block.rs` 在净化后包 `md-code-block` 工具条；点击 `[data-md-copy-code]` 复制 `pre` 文本（聊天 transcript 与变更集模态）。高亮仍不做。
- [x] **裸 URL 自动成链**：正文中的 `http://` / `https://` 收成 `<a target=_blank>`（`frontend/src/markdown/autolink.rs`）；scheme 大小写不敏感；中文路径与 `http://[::1]/` 保留；行内 code、围栏、已有 Markdown 链接内不处理；`javascript:` 不成链。流式活跃行同样生效。关 Markdown 时不成链。测试见 `markdown.rs` / `autolink.rs` / `tui_line_markdown.rs`。
- [x] **流式行内斜体与 Markdown 链接**：`tui_line_markdown.rs` 的 `stream_inline_safe_html` 只处理成对 `**` / `` ` `` / `~~` 与裸 URL。扩展扫描：`*em*` / `_em_`、已成对 `[text](url)`；半截标记保持转义字面量。测试对齐现有「半截 `**` 不着色」约定。
  - 落地：`markdown/stream_inline.rs`；`_em_` 避开 `snake_case`；流式链接只接受 `http(s)`。
- [x] **流式围栏只认 \`\`\`**：`is_fence_marker` / `open_block_is_fence_buffer` 只看 \`\`\`；CommonMark 的 `~~~` 未闭合时不会走纯文本缓冲。识别 `~~~`，闭合后仍走 `to_safe_html`。
- [x] **引用不成块**：`BlockKind` 只有 Paragraph / Table / List；`>` 引用当段落，可能和后文粘在同一 pending。增加 Blockquote，空行或类型切换时单独冻结。
- [x] **GFM alert**：pulldown-cmark 0.13 的 `ENABLE_GFM` 目前覆盖 `[!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]`。模型常用；需同步 ammonia 放行 `blockquote` 上的 `markdown-alert-*` class，并补 CSS。不要顺手打开 `ENABLE_SMART_PUNCTUATION`。
- [x] **安全回归**：`markdown.rs` 仅测 `<script>` 剥离。补 `javascript:` / `data:` 链接与图片用例（scheme 应被拦）；评估远程 `![img](https://…)`（跟踪像素）是否加 `referrerpolicy=no-referrer` 或默认不加载。`chat_links_open_in_new_tab` 是字符串替换，改白名单时一并收紧，避免漏 `target`。
  - 落地：ammonia `a[target=_blank]` + `img[referrerpolicy=no-referrer]`；测 `javascript:` 链接与 `data:` 图被剥。
- [x] **工作区工具图**：闭合 Markdown `![alt](plots/a.png)` 改写为 `/workspace/file/raw?path=`；ammonia 仅放行该相对 URL（仍剥 `../` 与 `data:`）；聊天 DOM 用 Bearer fetch 后换成 `blob:`（`<img>` 不带鉴权头）。不含 svg。

## P2 · 清理与语义

- [x] **`ENABLE_HEADING_ATTRIBUTES` 几乎无效**：解析 `{#id .class}`，ammonia 默认剥 `id`/`class`。聊天里标题锚点价值低：关掉该选项，或白名单带前缀的安全 `id`。选一种，不要留空开开关。
  - 落地：关掉 `ENABLE_HEADING_ATTRIBUTES`（不在 DOM 上挂标题 `id`，避免和壳层锚点冲突）。`{#id}` 会留在标题可见文本里（模型很少这么写）。
- [x] **模型误写修补仍是启发式**：`normalize_markdown_for_render` 覆盖全角冒号+围栏、句末+`**`、行内 \`\`\`、粘连 `//`、ATX 空格、表头`||`分隔行。未覆盖嵌套列表、`~~~`、引用粘连。按真实助手日志加用例，避免无限 `replace`。
  - 落地：`~~~` 与 \`\`\` 走同一套行内/行尾/粘连 `//` 规则；句末 `。` / `！` / `？` 后的 `>` 拆成引用（不拆 `：>`，避免 `阈值：> 0`）；`-规范` / `1.下一步` 补空格（不改 `-rf` / `1.0`、不把 `*em*` 当列表）。**不**改嵌套列表缩进。
- [x] **回合内 `h1` 抢页面层级**：助手消息可产出 `h1`。考虑把消息内 `h1` 降为 `h3`/`h4`，或限制回合内标题层级；表格可补 caption（非必须）。
  - 落地：净化后 `<h1>`→`<h3>`、`<h2>`→`<h4>`（`###` 仍为 `h3`）。表格 caption 不做。

## 单独立项（本清单不排期）

以下不要混进上面的 PR：

- 数学：`ENABLE_MATH` + KaTeX（WASM 体积、ammonia 标签、流式半截公式）
- Mermaid / 图渲染
- 把 Markdown 从 `innerHTML` 迁到 Leptos 虚拟 DOM

## 验证

- 单元：`frontend/src/markdown.rs`、`tui_line_markdown.rs` 现有 `#[test]` / wasm-bindgen 测试扩覆盖
- 手测：助手流式输出含表格、任务列表、围栏、引用、链接；开关 `markdown_render=false` 仍纯文本转义
- 样式：同一段 Markdown 在聊天 transcript 与变更集模态上标题/代码/引用观感一致
