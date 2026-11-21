#!/usr/bin/env python3
"""Markdown 单行长度门禁：**每行**不得超过 `MAX_COLS` 显示列。

# 为什么是逐行上限

「统计每个文件有多少行超长、把条数冻结在基线」是个可被抵消的约束：新增 3 行长行、
顺手删掉 3 行旧长行，条数不变就算通过 —— 那是约束账本，不是约束行长。这里直接判
逐行：**任何一行**超过上限即失败，没有可交易的额度。

# 豁免只有一类：围栏代码块

``` / ~~~ 之间的内容（含围栏行本身）不计 —— 代码折行会改语义，命令行 / JSON /
日志样本一旦被折断就不再是那段东西。

**表格行不豁免**：一张宽表仍然是一条宽行，列宽也完全由作者控制（少几列、拆成两张
表都行），所以它跟正文一样受约束。这条是刻意收紧过的：早期版本把表格当豁免，结果
`client_capability_matrix.md` 里一张表成了全仓最长的一行（24501 列）而门禁看不见。

# 上限当前取 2000（下一档目标 160）

目标值是 160 显示列（与 Server 仓 `scripts/md_line_length_ratchet.py` 同口径），但仓库里
存量长行太多（1000+ 行超过 160），一次性压到 160 不现实。所以分档降：每轮把超过上限的
行清掉，再把 `MAX_COLS` 调到新的存量最长值。

分档降过两轮：24501（表格 Notes 单元格）→ 2000 → 1000。每一步都是把超上限的内容压下来
（表格里放不下的长 Notes 移到文末的「表格长备注」并折行，段落 / 列表 / 引用按各自前缀
折行），再把 `MAX_COLS` 调到新的存量最长值。

**这个数字只许往下改**：清理掉一批长行后就把它调到新的存量最长值（`--list` 会按列数
降序列出每个文件的最长行，便于挑下一档目标），逐步收敛到 160。上调等于放松约束，
PR review 与门禁 `ok` 行打印的上限值都能看见。

# 口径

宽度按**显示列**而非字符数：`unicodedata.east_asian_width` 为 `W` / `F` 的字符记
2 列，组合记号记 0 列，其余 1 列 —— 160 列约等于 80 个汉字。行尾换行符不计。

# 扫描范围

仓库内全部 `*.md`，排除：

  - 目录：`agent_space/`（gitignored 草稿，本地随便写）、任意 `node_modules/`
    （第三方 README）、`target/`、`.git/`、`.crabmate/`；
  - 文件：`CHANGELOG.md` —— 它的条目被 `scripts/check-changelog-style.sh` 要求
    **单条一行且 ≤ 800 字符**，已发布段又冻结不可重排，因此它的长行无解，只能
    整文件不计。它的行宽另由那个门禁把关。

用法：
    python3 scripts/md_line_length_check.py        # 检查
    python3 scripts/md_line_length_check.py --list # 列出每个文件的最长行（降序），
                                                   # 用来决定下一档 MAX_COLS
"""

from __future__ import annotations

import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 当前上限 = 1000 显示列（两轮重构后的存量最长）。
# 目标 160；只许往下调，每轮清理后重新取存量最长值。
MAX_COLS = 1000
# 目标值，仅用于提示与文档（不参与判定）。
TARGET_COLS = 160

# 排除目录（命中路径任一分量即跳过）。
EXCLUDED_DIRS = frozenset({"agent_space", "node_modules", "target", ".git", ".crabmate"})
# 整文件不计（理由见文件头「扫描范围」）：CHANGELOG 的条目必须单行且 ≤ 800 字符，
# 已发布段又冻结，压不下来；它的行宽由 check-changelog-style.sh 把关。
EXCLUDED_FILES = frozenset({"CHANGELOG.md"})
# 违规时最多列多少条（其余只报总数，避免刷屏）。
MAX_REPORTED = 40
SNIPPET_CHARS = 70


def display_cols(line: str) -> int:
    """按显示列计算宽度（东亚宽字符 2 列，组合记号 0 列）。"""
    cols = 0
    for ch in line:
        if unicodedata.combining(ch):
            continue
        cols += 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
    return cols


def gated_lines(text: str) -> list[tuple[int, int, str]]:
    """返回 [(行号, 显示列, 行内容), …]；只排除围栏代码块（含围栏行）。"""
    rows: list[tuple[int, int, str]] = []
    fence: str | None = None
    for lineno, raw in enumerate(text.splitlines(), 1):
        stripped = raw.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            marker = "```" if stripped.startswith("```") else "~~~"
            if fence is None:
                fence = marker
            elif fence == marker:
                fence = None
            continue
        if fence is not None:
            continue
        rows.append((lineno, display_cols(raw), raw.strip()))
    return rows


def scanned_files() -> list[Path]:
    found: list[Path] = []
    for path in sorted(ROOT.rglob("*.md")):
        rel = path.relative_to(ROOT)
        if EXCLUDED_DIRS.intersection(rel.parts) or rel.as_posix() in EXCLUDED_FILES:
            continue
        found.append(path)
    return found


def rel_of(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def check() -> int:
    problems: list[str] = []
    total = 0
    files = 0
    longest = (0, "", 0)
    scanned = 0

    for path in scanned_files():
        rel = rel_of(path)
        rows = gated_lines(path.read_text(encoding="utf-8"))
        if not rows:
            continue
        scanned += 1
        file_max = max(cols for _, cols, _ in rows)
        if file_max > longest[0]:
            lineno = next(ln for ln, cols, _ in rows if cols == file_max)
            longest = (file_max, rel, lineno)
        hits = [(ln, cols, content) for ln, cols, content in rows if cols > MAX_COLS]
        if not hits:
            continue
        files += 1
        total += len(hits)
        for lineno, cols, content in hits:
            if len(problems) < MAX_REPORTED:
                problems.append(f"  {rel}:{lineno}  {cols} 列  {content[:SNIPPET_CHARS]}")
            elif len(problems) == MAX_REPORTED:
                problems.append("  …（其余省略）")

    if problems:
        print(
            f"[check-md-line-length] 违规：{files} 个文件共 {total} 行超过上限 {MAX_COLS} 显示列"
            f"（目标 {TARGET_COLS}）：",
            file=sys.stderr,
        )
        for p in problems:
            print(p, file=sys.stderr)
        print(
            "  → 请折行；只有围栏代码块不计（表格行一样受约束）。"
            "上限只许往下调，不要为了过门禁把它调大",
            file=sys.stderr,
        )
        return 1

    print(
        f"[check-md-line-length] ok（{scanned} 个文件全部合规；上限 {MAX_COLS} 显示列，"
        f"当前最长 {longest[0]} 列 @ {longest[1]}:{longest[2]}；目标 {TARGET_COLS}）"
    )
    return 0


def list_longest() -> int:
    rows: list[tuple[int, str, int]] = []
    for path in scanned_files():
        gated = gated_lines(path.read_text(encoding="utf-8"))
        if not gated:
            continue
        cols = max(c for _, c, _ in gated)
        lineno = next(ln for ln, c, _ in gated if c == cols)
        rows.append((cols, rel_of(path), lineno))
    rows.sort(reverse=True)
    for cols, rel, lineno in rows:
        print(f"{cols:6d}  {rel}:{lineno}")
    print(f"---\n共 {len(rows)} 个文件；最长 {rows[0][0]} 列。清理后把 MAX_COLS 调到新的最大值。")
    return 0


def main(argv: list[str]) -> int:
    if "--help" in argv or "-h" in argv:
        print(__doc__)
        return 0
    if "--list" in argv:
        return list_longest()
    return check()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
