#!/usr/bin/env python3
"""CSS 字面量预算门禁（扫描范围 = frontend/styles/*.css，排除 tokens.css）。

设计 token 的意义是「颜色与字号只有一处来源」。在 frontend/styles 里新写一个 hex 色或裸 px 字号，
视觉上不会立刻出错，却把这份来源重新打散 —— 这类退化只能靠机械计数拦住。

按文件统计三项：

  1. **颜色字面量**：`#rgb[a]` / `rgb()` / `rgba()` / `hsl()` / `hsla()` / `white` / `black`
     （`white-space` 这类属性名不算）。颜色一律应由 `var(--x)` 提供，token 见 styles/tokens.css；
  2. **font-size 字面量**：`font-size: 12px` / `1.2rem` / `1em`。字号应由 `--text-*` 等 token
     或继承提供。
  3. **border-radius 字面量**：`border-radius: 6px` / `50%` / `0.6em`（裸 `0` 不算，无尺寸语义）。
     圆角应由 `--radius-*` 提供；`var(--radius-x, 5px)` 里的兜底字面量也计入（兜底只在 token
     缺失时生效，等于把一份来源写死在样式里）。注意粒度与颜色 / 字号不同：本项**按声明**计数，
     `border-radius: 6px 6px 0 0` 这种四角写法只计 1 次（一套圆角阶梯就是一处来源），
     而颜色 / 字号按出现次数计。

`frontend/themes/*.css` 是「颜色的定义处」，天然全是字面量；`styles/tokens.css` 同理 —— 两者都不扫描。

存量已按「每文件预算」冻结在 scripts/css_literals_budget.txt：**实际值必须等于预算值**，
新增字面量要 fail，清理后不与预算同步下调同样 fail（与仓库其它白名单「失效即失败」一致）。
预算格式：`<相对路径>  font-size=<n>  color=<n>  border-radius=<n>  # <理由 / 清理计划>`；
未登记的文件预算视为 0，三项均为 0 的条目视为失效条目。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STYLES_DIR = ROOT / "frontend" / "styles"
BUDGET = ROOT / "scripts" / "css_literals_budget.txt"
# token 的定义处，天然全是字面量。
EXCLUDED = ("tokens.css",)

COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
# 只有「带主题倾向」的颜色字面量算违规：transparent / currentColor 是主题中立的，不在此列。
# `(?!-)` 排除 white-space / black-… 这类属性名。
COLOR_RE = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\b(?:white|black)\b(?!-)")
FONT_SIZE_RE = re.compile(r"font-size\s*:\s*[0-9.]+(?:px|rem|em)\b")
# 裸 0（含 `0 0 0 0` 这类四角写法）不算：无尺寸语义，不构成第二份来源。
# `var(--radius-btn, 5px)` 的兜底 5px 会计入（见文件头说明）。`border-top-left-radius` 等分角写法同样覆盖。
# 正则匹配到声明即计数一次，故 `border-radius: 6px 6px 0 0` 只计 1（见文件头说明的粒度差异）。
BORDER_RADIUS_RE = re.compile(
    r"\bborder(?:-[a-z]+)*-radius\s*:[^;{}]*?\d*\.?\d+(?:px|rem|em|%)\b"
)

METRICS = ("font-size", "color", "border-radius")
ZERO = {"font-size": 0, "color": 0, "border-radius": 0}
ENTRY_RE = re.compile(r"^(\S+)\s+font-size=(\d+)\s+color=(\d+)\s+border-radius=(\d+)$")


def strip_comments(text: str) -> str:
    """注释按等长空格抹掉，保证行号与原文一致。"""
    return COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def scanned_files() -> list[Path]:
    if not STYLES_DIR.is_dir():
        return []
    return [f for f in sorted(STYLES_DIR.glob("*.css")) if f.name not in EXCLUDED]


def measure(text: str) -> dict[str, int]:
    clean = strip_comments(text)
    return {
        "font-size": len(FONT_SIZE_RE.findall(clean)),
        "color": len(COLOR_RE.findall(clean)),
        "border-radius": len(BORDER_RADIUS_RE.findall(clean)),
    }


class BudgetError(Exception):
    pass


def load_budget() -> dict[str, tuple[dict[str, int], int]]:
    """返回 {相对路径: (各项预算, 行号)}。"""
    entries: dict[str, tuple[dict[str, int], int]] = {}
    if not BUDGET.exists():
        raise BudgetError(f"{BUDGET.relative_to(ROOT)} 不存在")
    for lineno, line in enumerate(BUDGET.read_text(encoding="utf-8").splitlines(), 1):
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        body, sep, reason = s.partition("#")
        body, reason = body.strip(), reason.strip()
        if not sep or not reason:
            raise BudgetError(f"{BUDGET.name}:{lineno} 缺少 '# 理由'：{s}")
        m = ENTRY_RE.match(body)
        if not m:
            raise BudgetError(
                f"{BUDGET.name}:{lineno} 应形如 `<路径>  font-size=<n>  color=<n>  "
                f"border-radius=<n>`：{s}"
            )
        rel = m.group(1)
        if rel in entries:
            raise BudgetError(f"{BUDGET.name}:{lineno} 重复登记 {rel}")
        entries[rel] = (
            {
                "font-size": int(m.group(2)),
                "color": int(m.group(3)),
                "border-radius": int(m.group(4)),
            },
            lineno,
        )
    return entries


def main() -> int:
    try:
        budget = load_budget()
    except BudgetError as exc:
        print(f"[check-css-literals] 预算格式错误：{exc}", file=sys.stderr)
        return 1

    actual = {
        f.relative_to(ROOT).as_posix(): measure(f.read_text(encoding="utf-8"))
        for f in scanned_files()
    }

    problems = []
    for rel, counts in actual.items():
        want = budget[rel][0] if rel in budget else ZERO
        diffs = [k for k in METRICS if counts[k] != want[k]]
        if diffs:
            detail = "；".join(f"{k} 预算 {want[k]} / 实际 {counts[k]}" for k in diffs)
            problems.append(f"  {rel}  {detail}")

    stale = []
    for rel, (want, lineno) in budget.items():
        if rel not in actual:
            stale.append(f"  {BUDGET.name}:{lineno}  {rel}  （无对应文件或不在扫描范围）")
        elif all(want[k] == 0 for k in METRICS):
            stale.append(f"  {BUDGET.name}:{lineno}  {rel}  （三项预算均为 0）")

    if problems:
        print(
            "[check-css-literals] 违规：字面量与预算不一致"
            "（新增请改用 token；清理后请同步下调 scripts/css_literals_budget.txt）：",
            file=sys.stderr,
        )
        for p in problems:
            print(p, file=sys.stderr)
        print(
            "  → 颜色：改用 var(--x)（token 定义见 frontend/styles/tokens.css）；"
            "字号：改用 var(--text-*) 等 token 或继承；"
            "圆角：改用 var(--radius-*)（去掉 var() 兜底里的 px）；"
            "确需新增请同步上调预算并写明理由",
            file=sys.stderr,
        )
    if stale:
        print("[check-css-literals] 预算失效，请删除下列条目：", file=sys.stderr)
        for s in stale:
            print(s, file=sys.stderr)
        print(
            "  → 路径须是相对仓库根的完整路径（如 frontend/styles/approval.css）；"
            "已清到 0 的条目直接删除",
            file=sys.stderr,
        )

    if problems or stale:
        return 1

    total = {k: sum(c[k] for c in actual.values()) for k in METRICS}
    print(
        f"[check-css-literals] ok（frontend/styles 内 font-size 字面量 {total['font-size']} 处，"
        f"颜色字面量 {total['color']} 处，border-radius 字面量 {total['border-radius']} 处；"
        f"预算 {len(budget)} 条，均一致）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
