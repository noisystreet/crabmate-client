#!/usr/bin/env python3
"""XML 注释门禁：注释正文不得出现连续两个减号（`--`）。

XML 1.0 规范（§2.5 Comments）明确禁止注释正文含 `--`：

    <!-- 与 Web 背景色（CSS 变量 --bg = #07090E）对齐 -->      ← 非法

Android 的 aapt2 资源合并器会直接拒绝这种写法并报：

    Resource and asset merger: 注释中不允许出现字符串 "--"。

而注释里提到 **CSS 自定义属性**（`--bg` / `--icon-sm`）或 **命令行选项**（`--release`）时
极容易顺手写出来，本地不编译 Android 就毫无察觉，故用机械扫描拦住。

规则：扫描全仓 `*.xml`，任何 `<!-- … -->` 的**正文**中出现 `--` 即失败
（输出 `文件:行:列` + 该行原文）。`-->` 结束符本身不算 —— 它位于正文之外。

排除工具生成 / 依赖目录（`PRUNED_DIRS`）：那些 XML 由构建器产出，不是本仓维护的源文件。
注意 `mobile-tauri/src-tauri/gen/android/**/res/**` 是 **提交进仓库** 的 Tauri 移动端资源，
**不排除**（本次出错的就是它）。
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 工具生成 / 依赖目录，不是本仓维护的源文件（遍历时直接剪枝，不进目录）
PRUNED_DIRS = frozenset({".git", "target", "node_modules", "dist", "build", ".venv", ".trunk", "agent_space"})

COMMENT_RE = re.compile(r"<!--(?P<body>.*?)-->", re.S)
SNIPPET_MAX = 120


def xml_files() -> list[Path]:
    """全仓 *.xml；剪枝掉构建产物与依赖目录，故可安全覆盖 gen/android 源资源。"""
    found: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = sorted(d for d in dirnames if d not in PRUNED_DIRS)
        for name in sorted(filenames):
            if name.endswith(".xml"):
                found.append(Path(dirpath) / name)
    return sorted(found)


def scan(path: Path) -> list[tuple[int, int, str]]:
    """返回该文件里每个「注释正文含 `--`」的位置：(行号, 列号, 该行原文)。"""
    text = path.read_text(encoding="utf-8", errors="replace")
    hits: list[tuple[int, int, str]] = []
    for match in COMMENT_RE.finditer(text):
        body = match.group("body")
        if "--" not in body:
            continue
        offset = match.start("body") + body.index("--")
        line_start = text.rfind("\n", 0, offset) + 1
        line_end = text.find("\n", line_start)
        if line_end == -1:
            line_end = len(text)
        line = text[line_start:line_end].strip()
        if len(line) > SNIPPET_MAX:
            line = line[:SNIPPET_MAX] + "…"
        hits.append((text.count("\n", 0, offset) + 1, offset - line_start + 1, line))
    return hits


def main() -> int:
    files = xml_files()
    problems = [
        (path.relative_to(ROOT), line, col, snippet)
        for path in files
        for line, col, snippet in scan(path)
    ]

    if problems:
        print("[check-xml-comments] 失败：XML 注释正文出现 `--`（XML 规范 §2.5 禁止）", file=sys.stderr)
        for rel, line, col, snippet in problems:
            print(f"  {rel}:{line}:{col}: {snippet}", file=sys.stderr)
        print(
            "\n修复：注释里提到 CSS 变量 / 命令行选项时不要带连字符前缀"
            "（写 `bg` 而非 `--bg`），或改用文字描述（如「CSS 自定义属性 bg」）。",
            file=sys.stderr,
        )
        return 1

    print(f"[check-xml-comments] ok（扫描 {len(files)} 个 XML；注释内 `--` 0 处）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
