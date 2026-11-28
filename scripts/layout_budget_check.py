#!/usr/bin/env python3
"""目录聚合行数预算 + 根级平铺冻结（#597）。

## 为什么需要它

`scripts/fn-nloc-ratchet.sh` 的 920 行单文件硬帽是对的，但它已经成了本仓**事实上的
模块划分依据**：全仓 651 个 `.rs` 里 26 个落在 850–920 行、16 个 ≥900 行、`> 920` 为
**0** 个，最硬的证据是 `tui_mode/worker.rs` 与 `tui_mode/mod.rs` **正好 920 行整**；98 个
文件的注释里明说「以免 / 压 920 行门禁」。于是模块边界由「哪个文件还有余量」决定，而不是
由职责决定 —— 拆成 8 个 900 行文件是免费通行证。

本门禁补的是**聚合**维度：一个域的总行数也要有预算，且新增文件要有唯一落点。

## 规则

- **Rule A · 目录预算**：`scripts/layout_budget.txt` 登记的每个目录，`.rs` 物理行数合计
  必须**等于**预算值（写多写少都 fail —— 少写等于给后续增长留免费额度）。
  `scope=recursive` 递归统计，`scope=direct` 只统计直接子文件。
- **Rule A2 · 不许嵌套登记**：`scope=recursive` 的条目不得是另一条目的祖先 —— 否则同一批
  行数被统计两次，「预算」的含义就没了。
- **Rule B · 根级平铺冻结**：`scripts/layout_root_flat.txt` 逐文件冻结
  `frontend/src/app/*.rs`：新增根级文件 fail，清单里的文件消失也 fail（双向 = stale 检查）。
- **Rule C · 归属守恒（目前只对 `frontend/src/app`）**：
  `app` 递归总行数 == 根级 direct 预算 + 各直接子目录 recursive 预算之和。任何未登记的新目录、
  或搬进来却没同步预算的行数，都会以「未归属 N 行」的形式暴露。

## 为什么覆盖规则只在 `app/` 试点

一旦按行数全仓强制「≥ N 行必须登记」，`crates/crabmate-tui/src`（66,459）与它内部的
`tui_mode`（57,503）这种嵌套会让「登记哪个」变成没有答案的问题。所以：

- `frontend/src/app/**` 用 Rule C 做**试点**（这也是 #598 之后唯一有模板的域树）；
- 其它根（`crates/*/src`、`frontend/src/api`、…）按需登记，新增大目录由评审把关。

## 用法

- 默认：跑三条规则，全绿退出 0；
- `--print-baseline`：按当前实测值打印两份基线（理由列是 `# TODO`，须人工补），
  失败信息里会提示它；
- 理由列**必填**且不许留 `TODO` —— 没有理由的预算，下一个人不知道它能不能降。
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUDGET_FILE = ROOT / "scripts" / "layout_budget.txt"
FLAT_FILE = ROOT / "scripts" / "layout_root_flat.txt"

# 与 fn_nloc_rust_metrics.py 同一组扫描根（排除 target/），`--print-candidates` 用
SCAN_ROOTS = (
    ROOT / "crates",
    ROOT / "desktop-tauri" / "src-tauri" / "src",
    ROOT / "mobile-tauri" / "src-tauri" / "src",
    ROOT / "frontend" / "src",
)

# 「一个域」的规模下限：≥ 3,000 行才值得有独立预算（`--print-candidates` 的阈值）
MIN_DOMAIN_LINES = 3000

# Rule C 的试点域根（相对路径）
DOMAIN_ROOT = "frontend/src/app"

# 「其它根」**按需登记**：选哪些由评审判断，数字永远实测。
# 例如 `crates/crabmate-tui/src`（66,459）与其中的 `tui_mode`（57,503）是嵌套关系，
# 只有后者有意义（11 个 `#[path]` 旁路，见 #597）—— 阈值算不出这种取舍。
OTHER_ROOTS = (
    "crates/crabmate-client-api/src",
    "crates/crabmate-tool-card/src",
    "crates/crabmate-tui/src/tui_mode",
    "frontend/src/api",
    "frontend/src/i18n",
)

# Rule B 冻结的是这个目录下**直接**的 .rs
FLAT_DIR = ROOT / "frontend" / "src" / "app"

TODO_REASON = "TODO"


def _lines(path: Path) -> int:
    with path.open("r", encoding="utf-8", errors="replace") as f:
        return sum(1 for _ in f)


def _rust_files(base: Path) -> list[Path]:
    return [p for p in base.rglob("*.rs") if "target" not in p.parts]


def _recursive_lines(base: Path) -> int:
    return sum(_lines(p) for p in _rust_files(base))


def _direct_lines(base: Path) -> int:
    return sum(_lines(p) for p in base.glob("*.rs"))


def _actual_lines(rel: str, scope: str) -> int | None:
    """相对路径的实际行数；目录不存在返回 None。"""
    base = ROOT / rel
    if not base.is_dir():
        return None
    return _direct_lines(base) if scope == "direct" else _recursive_lines(base)


class Entry:
    __slots__ = ("rel", "lines", "scope", "reason")

    def __init__(self, rel: str, lines: int, scope: str, reason: str) -> None:
        self.rel = rel
        self.lines = lines
        self.scope = scope
        self.reason = reason


def _parse_budget(path: Path) -> tuple[list[Entry], list[str]]:
    entries: list[Entry] = []
    errors: list[str] = []
    seen: dict[str, int] = {}
    if not path.is_file():
        return entries, [f"预算文件不存在：{path.relative_to(ROOT)}"]
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        body, _, reason = line.partition("#")
        fields = body.split()
        if len(fields) != 3:
            errors.append(f"{path.name}:{lineno} 格式应为 `<dir> lines=<n> scope=<recursive|direct> # <理由>`")
            continue
        rel, lines_field, scope_field = fields
        if not lines_field.startswith("lines=") or not scope_field.startswith("scope="):
            errors.append(f"{path.name}:{lineno} 字段名必须是 `lines=` / `scope=`")
            continue
        try:
            budget = int(lines_field[len("lines=") :])
        except ValueError:
            errors.append(f"{path.name}:{lineno} `lines=` 不是整数")
            continue
        scope = scope_field[len("scope=") :]
        if scope not in ("recursive", "direct"):
            errors.append(f"{path.name}:{lineno} `scope=` 只能是 recursive / direct")
            continue
        reason = reason.strip()
        if not reason or reason.upper().startswith(TODO_REASON):
            errors.append(f"{path.name}:{lineno} `{rel}` 缺理由（TODO 未替换）")
            continue
        if rel in seen:
            errors.append(f"{path.name}:{lineno} `{rel}` 重复登记（首次在第 {seen[rel]} 行）")
            continue
        seen[rel] = lineno
        entries.append(Entry(rel, budget, scope, reason))
    return entries, errors


def _parse_flat(path: Path) -> tuple[set[str], list[str]]:
    if not path.is_file():
        return set(), [f"冻结清单不存在：{path.relative_to(ROOT)}"]
    files: set[str] = set()
    errors: list[str] = []
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line in files:
            errors.append(f"{path.name}:{lineno} `{line}` 重复")
            continue
        files.add(line)
    return files, errors


def _check_budget(entries: list[Entry]) -> list[str]:
    problems: list[str] = []
    for e in entries:
        actual = _actual_lines(e.rel, e.scope)
        if actual is None:
            problems.append(f"{e.rel} 已不存在（陈旧条目，请删除）")
            continue
        if actual == 0 and e.lines > 0:
            problems.append(f"{e.rel} 已无 .rs 文件（陈旧条目，请删除）")
            continue
        if actual != e.lines:
            delta = actual - e.lines
            hint = "变胖了" if delta > 0 else "变瘦了，请把预算下调到实际值"
            problems.append(f"{e.rel} 预算 {e.lines} ≠ 实际 {actual}（{delta:+d}，{hint}）")
    # Rule A2：递归条目不得是另一条目的祖先
    for a in entries:
        if a.scope != "recursive":
            continue
        for b in entries:
            if a.rel != b.rel and b.rel.startswith(a.rel + "/"):
                problems.append(
                    f"{a.rel}（recursive）同时登记了 {b.rel} —— 嵌套登记会重复计数"
                )
    return problems


def _check_flat(frozen: set[str]) -> list[str]:
    actual = {str(p.relative_to(ROOT)) for p in FLAT_DIR.glob("*.rs")}
    problems = [
        f"新增根级文件 {rel} —— 新文件必须落在域目录（#597），或先登记预算再冻结"
        for rel in sorted(actual - frozen)
    ]
    problems += [
        f"清单里的 {rel} 已不存在（陈旧条目，请删除）" for rel in sorted(frozen - actual)
    ]
    return problems


def _check_conservation(entries: list[Entry]) -> list[str]:
    """Rule C：app 递归总行数 == 根级 direct + 各直接子目录 recursive。"""
    domain = ROOT / DOMAIN_ROOT
    if not domain.is_dir():
        return [f"试点域根不存在：{DOMAIN_ROOT}（改名了？请同步本脚本与预算文件）"]
    total = _recursive_lines(domain)
    accounted = 0
    covered: set[str] = set()
    for e in entries:
        if not e.rel.startswith(DOMAIN_ROOT + "/"):
            continue
        rest = e.rel[len(DOMAIN_ROOT) + 1 :]
        if "/" in rest:
            continue  # 孙目录由父条目覆盖
        covered.add(rest)
        accounted += e.lines
    root_entry = next((e for e in entries if e.rel == DOMAIN_ROOT), None)
    if root_entry is None:
        return [f"{DOMAIN_ROOT} 本身没有 direct 预算条目"]
    accounted += root_entry.lines
    if accounted == total:
        return []
    known = {p.name for p in domain.iterdir() if p.is_dir()}
    unregistered = sorted(known - covered)
    detail = f"（未登记的子目录：{', '.join(unregistered)}）" if unregistered else ""
    return [
        f"{DOMAIN_ROOT} 有 {total - accounted:+d} 行未归属{detail}"
        f" —— 新目录/新行数必须登记到 {BUDGET_FILE.name}"
    ]


def _print_baseline() -> None:
    print(f"# ===== {BUDGET_FILE.name}（实测基线；理由列须人工补）=====")
    print("# 格式：<dir>  lines=<n>  scope=<recursive|direct>   # <理由 / 目标>")
    print(f"{DOMAIN_ROOT}  lines={_direct_lines(ROOT / DOMAIN_ROOT)}  scope=direct   # {TODO_REASON}")
    for child in sorted(p for p in (ROOT / DOMAIN_ROOT).iterdir() if p.is_dir()):
        rel = f"{DOMAIN_ROOT}/{child.name}"
        if _rust_files(child):
            print(f"{rel}  lines={_recursive_lines(child)}  scope=recursive   # {TODO_REASON}")
    for rel in OTHER_ROOTS:
        print(f"{rel}  lines={_recursive_lines(ROOT / rel)}  scope=recursive   # {TODO_REASON}")
    print()
    print(f"# ===== {FLAT_FILE.name}（当前根级 .rs；只减不增）=====")
    _print_flat()


def _print_candidates() -> int:
    """列出扫描根下所有 ≥ MIN_DOMAIN_LINES 的目录（含嵌套），供人工挑登记项。"""
    rows: list[tuple[int, str]] = []
    for root in SCAN_ROOTS:
        if not root.is_dir():
            continue
        dirs = {d for d in root.rglob("*") if d.is_dir() and "target" not in d.parts}
        dirs.add(root)
        for d in dirs:
            total = _recursive_lines(d)
            if total >= MIN_DOMAIN_LINES:
                rows.append((total, str(d.relative_to(ROOT))))
    if not rows:
        print("没有 ≥ %d 行的目录" % MIN_DOMAIN_LINES, file=sys.stderr)
        return 1
    print(f"# ≥ {MIN_DOMAIN_LINES} 行的目录（嵌套关系会同时列出，登记时自己去重）")
    for total, rel in sorted(rows, key=lambda r: -r[0]):
        inside = "  <= 本项已覆盖子目录" if any(
            other != rel and other.startswith(rel + "/") for _, other in rows
        ) else ""
        print(f"{rel:45s} {total:7d}{inside}")
    return 0


def _print_flat() -> None:
    for p in sorted(FLAT_DIR.glob("*.rs")):
        print(str(p.relative_to(ROOT)))


def main(argv: list[str]) -> int:
    if "--print-baseline" in argv:
        _print_baseline()
        return 0
    if "--print-flat" in argv:
        _print_flat()
        return 0
    if "--print-candidates" in argv:
        return _print_candidates()

    entries, errors = _parse_budget(BUDGET_FILE)
    frozen, flat_errors = _parse_flat(FLAT_FILE)
    problems = errors + flat_errors
    if not problems:
        problems = _check_budget(entries) + _check_flat(frozen) + _check_conservation(entries)

    if problems:
        print("[layout-budget] 目录聚合预算 / 根级冻结（#597）：", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        print(
            f"  用法：`python3 scripts/layout_budget_check.py --print-baseline` 重列实测基线"
            f"（{BUDGET_FILE.name} / {FLAT_FILE.name}）。",
            file=sys.stderr,
        )
        return 1

    total = sum(e.lines for e in entries)
    print(
        f"layout-budget: {len(entries)} 个目录条目 / 共 {total} 行；"
        f"根级冻结 {len(frozen)} 个文件"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
