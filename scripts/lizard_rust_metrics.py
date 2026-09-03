#!/usr/bin/env python3
"""Rust 圈复杂度门禁（crabmate-client）。

门禁：所有扫描目标（`crates/*`、`desktop-tauri/…/src`、`mobile-tauri/…/src`、
`frontend/src`）中 **CCN > 10** 的函数个数必须为 0。
出现即失败并列出函数；不存在“允许个数上限”的模块配置。

用法：
  python3 scripts/lizard_rust_metrics.py
  python3 scripts/lizard_rust_metrics.py --list-above 10   # 额外打印清单，门禁不变
"""
from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    import lizard
except ImportError:
    print("lizard 未安装。请执行: pip install lizard", file=sys.stderr)
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
RUST_ROOTS = [
    ROOT / "crates",
    ROOT / "desktop-tauri" / "src-tauri" / "src",
    ROOT / "mobile-tauri" / "src-tauri" / "src",
    ROOT / "frontend" / "src",
]
# 圈复杂度硬门禁：超过该值的函数不允许出现。
HIGH_CCN_THRESHOLD = 10


@dataclass
class FnHit:
    ccn: int
    path: Path
    line: int
    name: str


def rust_files() -> list[Path]:
    out: list[Path] = []
    for base in RUST_ROOTS:
        if not base.is_dir():
            continue
        for p in base.rglob("*.rs"):
            if "target" in p.parts:
                continue
            out.append(p)
    return out


def _rel(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def analyze(files: list[Path], threshold: int) -> tuple[list[FnHit], int, int]:
    """返回 (CCN>threshold 命中列表, 函数总数, 最大 CCN)。"""
    hits: list[FnHit] = []
    total_fns = 0
    overall_max = 0
    for f in lizard.analyze_files([str(p) for p in files]):
        for fn in f.function_list:
            c = int(fn.cyclomatic_complexity)
            total_fns += 1
            overall_max = max(overall_max, c)
            if c > threshold:
                hits.append(FnHit(c, Path(f.filename), int(fn.start_line), fn.name))
    hits.sort(key=lambda h: (-h.ccn, _rel(h.path), h.line, h.name))
    return hits, total_fns, overall_max


def print_hits(hits: list[FnHit], limit: int = 100, stream=None) -> None:
    out = sys.stderr if stream is None else stream
    for h in hits[:limit]:
        print(f"  CCN {h.ccn}\t{_rel(h.path)}:{h.line}\t{h.name}", file=out)
    if len(hits) > limit:
        print(f"  ... 另有 {len(hits) - limit} 个", file=out)


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="检查 Rust 函数圈复杂度：CCN > 10 的函数个数必须为 0"
    )
    p.add_argument(
        "--list-above",
        type=int,
        metavar="N",
        help="额外列出 CCN>N 的函数（不改变硬门禁）",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    files = rust_files()
    if not files:
        print("lizard: 未找到 Rust 源文件", file=sys.stderr)
        return 1

    hits, total_fns, overall_max = analyze(files, HIGH_CCN_THRESHOLD)

    if args.list_above is not None:
        if args.list_above >= HIGH_CCN_THRESHOLD:
            above = [h for h in hits if h.ccn > args.list_above]
        else:
            above, _, _ = analyze(files, args.list_above)
        print(f"CCN > {args.list_above}：")
        print_hits(above, stream=sys.stdout)

    the = HIGH_CCN_THRESHOLD
    print(
        f"lizard Rust（门禁：CCN>{the} 函数个数必须为 0；"
        f"函数总数 {total_fns}，最大 CCN {overall_max}）：{len(hits)}"
    )
    if hits:
        print(f"lizard: 存在 {len(hits)} 个 CCN>{the} 的函数，门禁失败：", file=sys.stderr)
        print_hits(hits)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
