#!/usr/bin/env python3
"""#558 项 5/6 的机械门禁：`ACTIVE` 作用域的读者，以及零参包装的调用面。

## 为什么需要它

`connection_scope::ACTIVE` 是「当前**前台** lane 的那份作用域」的指针（thread_local）。
任何**跨 lane** 的路径一旦经它取基址 / Bearer，请求就打到前台那条 serve —— 这正是
#569「切 lane 没反应 / 两台 serve 互相污染」的成因。#558 把验收口径收窄为「`ACTIVE`
只服务前台天然唯一的调用点」，本脚本给出**约束机制**（谁可以读它）。

## 规则 A · 读者白名单（`frontend/src/api/**`，`connection_scope.rs` 本体除外）

读 `ACTIVE` 的零参口 `active_holder` / `bearer` / `is_active` / `activate` /
`install_new_holder` 只允许出现在 `scripts/active_scope_allowlist.txt` 里，每条须写理由。
新增一处不登记 → 失败；登记了但代码里已经消失（陈旧）→ 也失败。

## 规则 B · 调用面预算 ratchet（`frontend/src/**`）

零参包装 `api_base_url` / `api_url` / `prepare_api_auth` / `auth_headers` /
`web_api_bearer_token` 的**每文件调用次数**必须等于 `scripts/active_scope_budget.txt`。

- 多一处 → 失败：会被后台 lane 并发触发的路径必须改用 `*_on(holder)` 显式带作用域。
- 少一处 → 也失败：预算只收紧不放松（ratchet，不是 floor）；改少了就同步下调。

## 判定细节

不算命中：注释行 / 行内注释 / 字符串字面量 / `fn name(` 定义行 / 方法调用（`x.name(`）/
其它模块的同名限定调用（`other::name(`）。`connection_scope::name(` 算命中。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "frontend" / "src"
SCOPE_FILE = "api/connection_scope.rs"
ALLOWLIST = ROOT / "scripts" / "active_scope_allowlist.txt"
BUDGET = ROOT / "scripts" / "active_scope_budget.txt"

# 读 `ACTIVE` 的零参全局口（规则 A）
GLOBAL_IDENTS = ("active_holder", "bearer", "is_active", "activate", "install_new_holder")
# 零参包装：转调上面的口，读到的永远是「前台那份」（规则 B）
WRAPPER_IDENTS = (
    "api_base_url",
    "api_url",
    "prepare_api_auth",
    "auth_headers",
    "web_api_bearer_token",
)

CALL_RE = r"(?<![\w.$])(?:\w+\s*::\s*)*{ident}\s*\("


def code_only(line: str) -> str:
    """抹掉字符串字面量与行尾注释，只留可判定的代码文本。

    只处理 `"`（Rust 里 `'` 多为生命周期标注，误抹反而会漏判），够用且不产生假阴性。
    """
    out: list[str] = []
    in_str = False
    escaped = False
    i = 0
    while i < len(line):
        ch = line[i]
        if escaped:
            out.append(" ")
            escaped = False
            i += 1
            continue
        if in_str:
            if ch == "\\":
                escaped = True
                out.append(" ")
                i += 1
                continue
            if ch == '"':
                in_str = False
            out.append(" ")
            i += 1
            continue
        if ch == '"':
            in_str = True
            out.append(" ")
            i += 1
            continue
        if ch == "/" and line[i + 1 : i + 2] == "/":
            break
        out.append(ch)
        i += 1
    return "".join(out)


def calls_in(code: str, ident: str) -> bool:
    """该行代码里是否有 `ident(...)` 调用（方法调用 `x.ident(` 不算）。

    带 `::` 限定的写法与裸调用一样算命中 —— 否则 `crate::api::browser::api_url(...)`
    这种完整路径会被漏判（实测过一次，规则 B 因此没抓到探针）。全仓除 `api/browser.rs`
    与 `connection_scope.rs` 外没有这些名字的函数（`is_active` 的同名都是 `&self` 方法，
    或不在 `api/` 层的局部闭包），所以放宽不会误报。
    """
    if re.search(rf"\bfn\s+{ident}\s*\(", code):
        return False  # 定义行
    return bool(re.search(CALL_RE.format(ident=ident), code))


def scan(idents: tuple[str, ...], base: Path) -> dict[tuple[str, str], list[int]]:
    hits: dict[tuple[str, str], list[int]] = {}
    for path in sorted(base.rglob("*.rs")):
        rel = str(path.relative_to(SRC)).replace("\\", "/")
        if rel == SCOPE_FILE:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, raw in enumerate(text.splitlines(), 1):
            if raw.lstrip().startswith("//"):
                continue
            code = code_only(raw)
            if not code.strip():
                continue
            for ident in idents:
                if calls_in(code, ident):
                    hits.setdefault((rel, ident), []).append(lineno)
    return hits


def parse_allowlist() -> tuple[set[tuple[str, str]], list[str]]:
    entries: set[tuple[str, str]] = set()
    problems: list[str] = []
    if not ALLOWLIST.exists():
        return entries, [f"{ALLOWLIST.relative_to(ROOT)}: 文件缺失"]
    for lineno, raw in enumerate(ALLOWLIST.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        body, _, reason = line.partition("#")
        if "::" not in body:
            problems.append(f"{ALLOWLIST.name}:{lineno}: 格式应为 `api/xxx.rs :: ident  # 理由`")
            continue
        path, _, ident = (part.strip() for part in body.partition("::"))
        if not reason.strip():
            problems.append(f"{ALLOWLIST.name}:{lineno}: `{path} :: {ident}` 缺理由")
            continue
        entries.add((path, ident))
    return entries, problems


def parse_budget() -> tuple[dict[str, int], list[str]]:
    budget: dict[str, int] = {}
    problems: list[str] = []
    if not BUDGET.exists():
        return budget, [f"{BUDGET.relative_to(ROOT)}: 文件缺失"]
    for lineno, raw in enumerate(BUDGET.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 2 or not parts[1].isdigit():
            problems.append(f"{BUDGET.name}:{lineno}: 格式应为 `<path> <count>`")
            continue
        budget[parts[0]] = int(parts[1])
    return budget, problems


def main() -> int:
    if "--print-baseline" in sys.argv:
        print("== 规则 A 基线（读 ACTIVE 的口）")
        for (rel, ident), lines in sorted(scan(GLOBAL_IDENTS, SRC / "api").items()):
            print(f"  {rel} :: {ident}  x{len(lines)}  {lines}")
        print("\n== 规则 B 基线（零参包装调用面）")
        per_file: dict[str, int] = {}
        for (rel, _ident), lines in scan(WRAPPER_IDENTS, SRC).items():
            per_file[rel] = per_file.get(rel, 0) + len(lines)
        for rel, count in sorted(per_file.items()):
            print(f"  frontend/src/{rel}  {count}")
        print(f"\n文件数: {len(per_file)}  调用点: {sum(per_file.values())}")
        return 0

    failures: list[str] = []

    # ---- 规则 A ----
    allowed, problems = parse_allowlist()
    failures.extend(problems)
    global_hits = scan(GLOBAL_IDENTS, SRC / "api")
    for (rel, ident), lines in sorted(global_hits.items()):
        if (rel, ident) in allowed:
            continue
        where = ", ".join(f"{rel}:{n}" for n in lines)
        failures.append(
            f"规则 A: `{rel} :: {ident}` 读了 ACTIVE（{where}）——"
            f" 跨 lane 路径请改用收 holder 的版本；确属前台唯一则登记到 "
            f"scripts/active_scope_allowlist.txt 并写理由"
        )
    for rel, ident in sorted(allowed - set(global_hits)):
        failures.append(
            f"规则 A: 白名单条目陈旧 —— `{rel} :: {ident}` 在代码里已不存在，请删除该条"
        )

    # ---- 规则 B ----
    budget, problems = parse_budget()
    failures.extend(problems)
    actual: dict[str, int] = {}
    for (rel, _ident), lines in scan(WRAPPER_IDENTS, SRC).items():
        actual[rel] = actual.get(rel, 0) + len(lines)
    for rel, count in sorted(actual.items()):
        key = f"frontend/src/{rel}"
        if key not in budget:
            failures.append(
                f"规则 B: {key} 新增了 {count} 处零参包装调用，但预算里没有该文件 ——"
                f" 跨 lane 路径请改用 `*_on(holder)`；确属前台唯一则把 `{key} {count}` 写进 "
                f"scripts/active_scope_budget.txt"
            )
        elif count > budget[key]:
            failures.append(
                f"规则 B: {key} 调用面 {budget[key]} -> {count}（上限 {budget[key]}）——"
                f" 跨 lane 路径请改用 `*_on(holder)` 显式带作用域"
            )
        elif count < budget[key]:
            failures.append(
                f"规则 B: {key} 调用面已收紧 {budget[key]} -> {count}，"
                f" 请把 scripts/active_scope_budget.txt 里的数字下调（预算不放宽）"
            )
    for key, count in sorted(budget.items()):
        rel = key[len("frontend/src/") :] if key.startswith("frontend/src/") else key
        if actual.get(rel, 0) == 0:
            failures.append(f"规则 B: 预算条目陈旧 —— {key}（{count}）在代码里已无调用，请删除该条")

    if failures:
        print("active scope 门禁失败（#558 项 5/6）：\n")
        for item in failures:
            print(f"  - {item}")
        print(
            "\n说明：`ACTIVE` 只是「当前前台 lane 的那份作用域」。会被后台 lane 并发触发的"
            " fetch / SSE / 审批轮询必须显式带 `holder`（`api_url_on` / `prepare_api_auth_on` /"
            " `auth_headers_for` / `*_scoped_local_storage_raw_on`）。"
        )
        return 1

    print("active scope ok: 规则 A 读者全部登记，规则 B 调用面与预算一致")
    return 0


if __name__ == "__main__":
    sys.exit(main())
