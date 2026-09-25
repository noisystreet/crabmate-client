#!/usr/bin/env python3
"""图标门禁（frontend）：图标只有一个「属性模板」来源与一个「尺寸」来源。

第 1、2 步把散落的手写内联 `<svg>` 与各档尺寸字面量收敛到
frontend/src/icon.rs（`Icon` 组件 + `--icon-*`），但这两类退化都不会立刻报错：
再手写一个 `<svg>` 照样能跑，再写一个 `width: 14px` 也照样能跑 —— 只能靠机械检查拦住。

本门禁两条规则（扫描范围仅 frontend；connect / splash 无共享 Icon 组件，不在范围内）：

  1. **`frontend/src/**/*.rs`（除 icon.rs）不得出现字面 `<svg`** —— 图标一律走
     `Icon` 组件或其 helper；`icon.rs` 是模板的唯一落点。
  2. **`frontend/styles/*.css` 里 `svg` 选择器规则的 `width` / `height`
     （含 min- / max-）必须是 `var(--icon-*)`** —— 三个档位定义在 tokens.css。

`font-size` 与颜色字面量不在此门禁（check-css-literals.sh 已全量覆盖）。

有意例外：在规则所在行（Rust 该行 / CSS 该规则前的注释里）写
`icon-gate: allow <理由>` 即跳过该处，理由随代码走、不留单独清单。

**已知盲区**：本门禁不校验「类名与元素是否同体」。若把某个容器类名直接挂到
`<svg>` 上，`.container svg` 这类后代选择器仍会给出「类名已覆盖」的假证据
（check-css-contract.sh 只查类名出现过），本门禁也看不到 —— 2026-09-25 的
`workspace-tree-chevron` 回归即属此类，需靠 code review 兜住。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "frontend" / "src"
STYLES_DIR = ROOT / "frontend" / "styles"
# 模板的唯一落点，规则 1 对它豁免。
ICON_MODULE = SRC_DIR / "icon.rs"

ALLOW_MARKER = "icon-gate: allow"

COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
LINE_COMMENT_RE = re.compile(r"//")
SVG_LITERAL_RE = re.compile(r"<svg\b")

# `svg` 必须是类型选择器：前面是行首或组合符 / 空白，后面是行尾或组合符 / 伪类 / 属性选择器。
# 这样 `.foo-svg` / `.btn-send-icon-svg` 这类「类名里带 svg」的规则不会被误判。
SVG_SELECTOR_RE = re.compile(r"(?:^|[\s,>+~(])svg(?=$|[\s,>+~.:#\[]|\)|\s*$)")
SIZE_PROP_RE = re.compile(r"(?:min-|max-)?(?:width|height)\s*:\s*([^;}]+)")
SIZE_FROM_TOKEN_RE = re.compile(r"^var\(--icon-[a-z0-9-]+\)(?:\s*!important)?$")


class Rule:
    def __init__(self, rel: str, line: int, message: str) -> None:
        self.rel = rel
        self.line = line
        self.message = message

    def render(self) -> str:
        return f"  {self.rel}:{self.line}  {self.message}"


def strip_block_comments(text: str) -> str:
    """块注释按等长空格抹掉，保证行号与原文一致。"""
    return COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def line_of(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def check_rust_literals() -> list[Rule]:
    rules: list[Rule] = []
    for path in sorted(SRC_DIR.rglob("*.rs")):
        if path == ICON_MODULE:
            continue
        raw = path.read_text(encoding="utf-8")
        clean = strip_block_comments(raw)
        for lineno, (raw_line, line) in enumerate(
            zip(raw.splitlines(), clean.splitlines()), 1
        ):
            comment = LINE_COMMENT_RE.search(line)
            code = line[: comment.start()] if comment else line
            if ALLOW_MARKER in raw_line:
                continue
            if SVG_LITERAL_RE.search(code):
                rules.append(
                    Rule(
                        path.relative_to(ROOT).as_posix(),
                        lineno,
                        "出现字面 `<svg`；请改用 crate::icon 的 `Icon` 组件或 helper",
                    )
                )
    return rules


def iter_rules(css: str, base: int = 0, lead: int = 0):
    """按花括号配平产出 (选择器, 规则体, 规则体起点, 前置区起点, 选择器终点)。

    `@` 开头者递归进去（保证 `@media` 内嵌套的 svg 规则也被检查）。
    「前置区」= 上一条规则结束到本规则选择器结束的原文区间，用于就地豁免标记。
    """
    i, n, start, prev_end = 0, len(css), 0, lead
    while i < n:
        ch = css[i]
        if ch == "{":
            selector = css[start:i].strip()
            depth, j = 1, i + 1
            while j < n and depth:
                if css[j] == "{":
                    depth += 1
                elif css[j] == "}":
                    depth -= 1
                j += 1
            body = css[i + 1 : j - 1]
            if selector.startswith("@"):
                yield from iter_rules(body, base + i + 1, 0)
            else:
                yield selector, body, base + i + 1, base + prev_end, base + i
            prev_end = start = i = j
        elif ch in "};":
            prev_end = start = i = i + 1
        else:
            i += 1


def check_css_sizes() -> list[Rule]:
    rules: list[Rule] = []
    if not STYLES_DIR.is_dir():
        return rules
    for path in sorted(STYLES_DIR.glob("*.css")):
        rel = path.relative_to(ROOT).as_posix()
        raw = path.read_text(encoding="utf-8")
        css = strip_block_comments(raw)
        for selector, body, body_start, lead_start, selector_end in iter_rules(css):
            if not SVG_SELECTOR_RE.search(selector):
                continue
            if ALLOW_MARKER in raw[lead_start:selector_end]:
                continue
            for match in SIZE_PROP_RE.finditer(body):
                value = match.group(1).strip()
                if SIZE_FROM_TOKEN_RE.match(value):
                    continue
                index = body_start + match.start()
                rules.append(
                    Rule(
                        rel,
                        line_of(css, index),
                        f"`{selector}` 的尺寸 `{value}` 不是 `var(--icon-*)`；"
                        "三档定义见 frontend/styles/tokens.css",
                    )
                )
    return rules


def main() -> int:
    problems = check_rust_literals() + check_css_sizes()
    if problems:
        print(
            "[check-icons] 违规：图标须走共享组件与尺寸 token"
            "（模板唯一来源 frontend/src/icon.rs；尺寸唯一来源 --icon-*）",
            file=sys.stderr,
        )
        for problem in problems:
            print(problem.render(), file=sys.stderr)
        print(
            "  → 内联 SVG 改用 `Icon` / `icon_*` helper；"
            "尺寸改用 var(--icon-sm|md|lg)；有意例外请在该行写 `" + ALLOW_MARKER + " <理由>`",
            file=sys.stderr,
        )
        return 1
    print("[check-icons] ok（icon.rs 之外无字面 `<svg`；svg 规则尺寸均来自 --icon-*）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
