#!/usr/bin/env python3
"""终端配色门禁：`tui_mode` 里的颜色必须全部来自 `Palette` 的语义槽。

为什么需要这条：`Palette::resolve_with` 在 `NO_COLOR` / `TERM=dumb` 下走 `mono()`，
把 13 个槽抹成 `Reset`；但调用点如果直接写 `Color::LightRed`，那个颜色**根本不经过
palette**——切主题不变色，无色模式也照样带色。同一份文件里两种写法并存时，
屏幕上就会出现「一半跟主题、一半不跟」的自相矛盾状态。

例外：本行或紧上一行写 `palette-gate: allow <理由>`。只写 `allow` 不给理由视为未豁免
（否则 `allow` 会退化成静默开关）。

测试代码不扫，两条判据：
  1. 行级——`#[cfg(test)]` 之后一律跳过（同文件内联的 `mod tests` 够用）；
  2. 文件级——`*_tests.rs` / `tests.rs` 整个跳过。这类文件由父模块用
     `#[path = "..."] mod tests;` 挂进来，文件**自己**没有 `#[cfg(test)]` 属性，
     只靠行级判据会把它们当成生产代码误报（`md_tests.rs` 就是这种）。
测试夹具需要用命名色构造对比样张，且不进渲染路径。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TUI_MODE = ROOT / "crates" / "crabmate-tui" / "src" / "tui_mode"
# palette.rs 是唯一允许出现色值的地方（它就是「语义槽 → 颜色」的映射表本体）。
PALETTE_RS = TUI_MODE / "palette.rs"

# ratatui 的命名色恒指 xterm 16 色的固定取值，与主题无关。`Reset` 不在其中：
# 它的语义就是「这一格不设色」，是合法的无色表达。
NAMED_COLORS = (
    "Black|Red|Green|Yellow|Blue|Magenta|Cyan|Gray|DarkGray|White|"
    "LightRed|LightGreen|LightYellow|LightBlue|LightMagenta|LightCyan"
)
NAMED_RE = re.compile(rf"Color::(?:{NAMED_COLORS})\b")
# 只抓字面 RGB（`Color::Rgb(0x..`）。`Color::Rgb(p.0[0], ..)` 是把背景图像素交回给
# ratatui，值与主题无关，不算硬写色值。
RGB_LITERAL_RE = re.compile(r"Color::Rgb\(\s*0x")
ALLOW_RE = re.compile(r"palette-gate:\s*allow(?:\s+(.*))?$")
TEST_MOD_ATTR = "#[cfg(test)]"


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT))


def is_test_file(path: Path) -> bool:
    """`#[path]` 挂进来的测试模块：文件自己不带 `#[cfg(test)]`，只能靠文件名认。"""
    return path.name == "tests.rs" or path.name.endswith("_tests.rs")


def live_lines(lines: list[str]):
    """产出 (0-based 下标, 行内容)，到第一个 `#[cfg(test)]` 之前为止。"""
    for idx, line in enumerate(lines):
        if line.strip().startswith(TEST_MOD_ATTR):
            return
        yield idx, line


def excused(lines: list[str], idx: int) -> bool:
    """本行或紧上一行写了带理由的豁免标记？"""
    for probe in (idx, idx - 1):
        if not 0 <= probe < len(lines):
            continue
        m = ALLOW_RE.search(lines[probe])
        if not m:
            continue
        # 写了 `allow` 却没写理由 → 不算豁免，让它红（避免 allow 变成静默开关）。
        return bool((m.group(1) or "").strip())
    return False


def main() -> int:
    if not TUI_MODE.is_dir():
        print(f"{rel(TUI_MODE)} 不存在", file=sys.stderr)
        return 1
    files = sorted(
        p
        for p in TUI_MODE.rglob("*.rs")
        if p.resolve() != PALETTE_RS.resolve() and not is_test_file(p)
    )
    if not files:
        print(f"{rel(TUI_MODE)} 下没有 .rs 文件；门禁会静默失效", file=sys.stderr)
        return 1
    failures: list[str] = []
    for path in files:
        lines = path.read_text(encoding="utf-8").split("\n")
        for idx, line in live_lines(lines):
            if line.lstrip().startswith("//"):
                continue
            for pattern, what in ((NAMED_RE, "命名色"), (RGB_LITERAL_RE, "RGB 字面量")):
                for hit in pattern.finditer(line):
                    if excused(lines, idx):
                        continue
                    failures.append(
                        f"{rel(path)}:{idx + 1} 出现{what} `{hit.group(0)}`；"
                        "终端颜色必须走 Palette 的语义槽（pal.fg / pal.muted / "
                        "Palette::tone_style / Palette::notice_style），"
                        "否则切主题不生效、NO_COLOR 下仍然带色"
                    )
    if failures:
        print("终端配色门禁失败（单一来源 = tui_mode/palette.rs 的 Palette 语义槽）：", file=sys.stderr)
        for item in failures:
            print(f"  - {item}", file=sys.stderr)
        return 1
    print(f"tui palette usage ok（{len(files)} 个文件）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
