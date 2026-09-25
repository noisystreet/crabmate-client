# Web 预设主题（`data-theme`）

本目录存放 **按 slug 拆分** 的 CSS 覆层：每条 `[data-theme="…"]` 对应设置里可选的一套外观（与 `/user-data/prefs` 的 `theme` 一致）。

**命名约定**：浅色 `xxx-light`、深色 `xxx-dark`；无深浅之分的 `system` 不加后缀。深色基线 `crabmate-dark` 与默认主题 `shadcn-light` 见 `frontend/src/app_prefs.rs` 的 `THEME_SLUGS`。

## 加载顺序

Trunk 在 **`frontend/index.html`** 中于 **`styles/tokens.css` 之后** 链接本目录下的文件。  
`tokens.css` 提供间距、字号、动效时长及 **默认深色色板**（`:root`，等价于壳层选用 **`crabmate-dark`** 时的观感）。

## 明暗基线（`color-scheme`）

`tokens.css` 的 `:root` 声明 **`color-scheme: dark`**，即默认（无 `data-theme` / `crabmate-dark` / `material-dark` / `high-contrast-dark` / `shadcn-dark`）下原生控件——`<select>` 弹出层、滚动条、表单控件——按深色绘制。

**浅色预设必须在自己的主题块内覆写 `color-scheme: light`**（见 `crabmate-light.css` / `shadcn-light.css`），否则这些原生部件会沿用 `tokens.css` 的深色基线，在浅底上出现黑边弹层。深色预设无需重复声明。

`color-scheme` 不是设计 token，不参与 `--font-sans` / `--radius-*` 那套并集覆盖校验（它没有「每个主题都必须定义」的语义——未声明即继承 `:root` 的 dark），由 `scripts/css_tokens_check.py` 忽略。

## 维护清单（新增或重命名主题时）

1. **CSS**：在本目录新增 `your-slug.css`，内含完整的 `:root[data-theme="your-slug"] { … }` 色板与覆层变量（可复制 `crabmate-light.css` 再改值），**必须**定义全部 `--ide-hl-*`（keyword / string / comment / number / type / attribute / macro / key），否则 IDE 会落到 `tokens.css` 的深色字面量兜底。若新预设底色为浅色，**必须**在块内写 **`color-scheme: light`**（见上文「明暗基线」）。
2. **`index.html`**：追加一行 `<link data-trunk rel="css" href="themes/your-slug.css" />`（须在 `tokens.css` 之后）。
3. **Rust 白名单**：`frontend/src/app_prefs.rs` 中 **`THEME_SLUGS`**（偏好，可含 **`system`**）与 **`THEME_CSS_SLUGS`**（`data-theme` CSS）加入 slug；未知存储值（含已废弃的旧 slug）会回退为默认主题 **`DEFAULT_THEME_SLUG`**（`shadcn-light`）。**`system` 无对应 CSS 文件**，由 **`resolve_data_theme_slug`** 映射到 `crabmate-dark`/`crabmate-light`。
4. **文案**：`frontend/src/i18n/settings.rs` 中 **`settings_theme_preset_label`**（或等价函数）增加显示名。
5. **变更记录**：用户可见的外观变化按仓库约定写入 **`CHANGELOG.md`** 的 `[Unreleased]`（英文）；本文件即主题的权威维护说明，构建 / 引入方式见 **`frontend/README.md`**。

## 本地自定义（不提交仓库）

可复制 **`custom.example.css`** 为 `custom.css`，按需修改并在 **`index.html`** 引用（注意 `.gitignore` 是否忽略 `custom.css`，避免误提交）。

## 内置文件

| 文件 | `data-theme` | 说明 |
|------|----------------|------|
| `crabmate-light.css` | `crabmate-light` | 浅色纸灰 + 鼠尾草绿点缀（次要文字已加深至 AA 对比度）；`color-scheme: light` |
| `material-dark.css` | `material-dark` | Material 圆角 + 中性灰深色 |
| `high-contrast-dark.css` | `high-contrast-dark` | 深灰底 + 白字 + 黑白灰强调（无彩色，可读性优先） |
| `shadcn-dark.css` | `shadcn-dark` | zinc 中性近黑层级 + 1px 细边框 + 克制蓝强调 + Inter（借 shadcn/ui 设计语言，不引组件库） |
| `shadcn-light.css` | `shadcn-light` | 上一项的浅色变体（**默认主题**，shadcn 实际默认观感）：zinc-50 底 + 白卡 + zinc-200 边框 + blue-600 强调 + Inter；`color-scheme: light` |

**`crabmate-dark`** 仍由根目录 **`styles/tokens.css`** 中 `:root` 提供，无需单独文件；若希望「深色也单独成文件」便于 fork，可从 `:root` 复制变量到 `crabmate-dark.css` 并改用 `:root[data-theme="crabmate-dark"]`（须与 `tokens.css` 避免重复定义）。
