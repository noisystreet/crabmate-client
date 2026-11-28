#!/usr/bin/env python3
"""#602：跨包直接依赖的 lockstep 门禁（声明维度硬失败，lock 差异只报告）。

## 为什么需要它

本仓 8 个 Rust 包是 8 个**独立** workspace（ADR-0004），各有自己的 `Cargo.lock`。
第三方依赖因此有 8 份真相：谁都可以在其中一个包里把 `reqwest` 从 0.13 升到 0.14、
或退回 0.12，其余 7 个包照常编译。症状不响：同一个二进制里出现两套 HTTP 栈 /
两套 TLS 提供者，或者同一个上游 bug 要在两个包里各修一次、各测一次。

## 规则 A · 声明一致性（硬失败）

同一 crate 被多个包**直接声明**时，`version` req 必须字面相同。
本仓 path 依赖（`crabmate-*`）天然一致；契约 pin `crabmate` 不参与比较 ——
它的 byte-identical（crates.io semver 或 git rev 两种形状）由
`scripts/check-boundaries.sh` 负责，两处规则重叠只会双报。

## 规则 B · 同一 manifest 内自相矛盾（硬失败）

同一包在 `dependencies` / `dev-dependencies` / `build-dependencies` /
`target.'cfg(...)'.dependencies` 里重复声明同一 crate 且 req 不同。

## 报告 C · lock 解析版本矩阵（不失败）

直接依赖在 8 份 `Cargo.lock` 里的解析版本；同一 compat 边界（`0.x` 取 `0.minor`，
其余取 `major`）内 patch 不齐的条目以 `warning:` 列出，并给出对齐命令。

**为什么不把它做成硬失败**：8 个包的 target 与 feature 不同（wasm32 / Android /
desktop），`cargo update` 的落点天然错开一两个 patch。做成硬失败后，任何一次依赖
升级都要同时把 8 份 lock 改齐，否则门禁就红 —— 那是把「依赖升级」变成「8 包连动」，
换来的只是 patch 对齐，没有真实收益（同一个 lock 内 Cargo 只选一个兼容版本，
跨 lock 的 patch 差异不影响任何单个产物的正确性）。

## 判据为什么不是「同 crate 同 major 全仓唯一」

Cargo 允许同一 lock 内多 major 共存；不同 lock 的传递依赖树更是随 target / feature
变化。实测本仓 `windows-sys`（4 个版本）、`phf`、`html5ever`、`getrandom` 等 50+ 个
crate 在各包并不一致，且这是**正常的**。硬要求全仓一致会一次红掉 50+ 项，门禁随即
失去意义。真正可控且有害的是**声明维度**的分叉 —— 那才是人能写错、且会造成重复
编译与语义分叉的地方。
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    print(
        "error: 本脚本需要 Python 3.11+（import tomllib）。仓里其它 check 脚本只用标准库老 API，"
        "这是第一个要求 3.11 的；CI 是 ubuntu-latest（当前 3.12），本地若更老请升级解释器，"
        "不要改用正则手写 TOML 解析。",
        file=sys.stderr,
    )
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
PKG_LIST = ROOT / "scripts" / "rust-pkg-dirs.txt"

# 契约 pin：byte-identical 由 scripts/check-boundaries.sh 守，这里不重复比较。
CONTRACT_CRATE = "crabmate"

DEP_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def package_dirs() -> list[str]:
    dirs = []
    for raw in PKG_LIST.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line and not line.startswith("#"):
            dirs.append(line)
    return dirs


def iter_dep_tables(manifest: dict):
    """产出 (段名, {crate: spec})，含 target.'cfg(..)'.dependencies。"""
    for section in DEP_SECTIONS:
        table = manifest.get(section)
        if isinstance(table, dict):
            yield section, table
    targets = manifest.get("target")
    if isinstance(targets, dict):
        for cfg, body in targets.items():
            if not isinstance(body, dict):
                continue
            for section in DEP_SECTIONS:
                table = body.get(section)
                if isinstance(table, dict):
                    yield f"target.'{cfg}'.{section}", table


def req_of(spec) -> str:
    """把一条依赖声明归一成可比较的 req 字面量。"""
    if isinstance(spec, dict):
        if "version" in spec:
            return str(spec["version"])
        if "path" in spec:
            return "<path>"
        if "git" in spec:
            return "<git>"
        if spec.get("workspace") is True:
            return "<workspace>"
        return "<unspecified>"
    return str(spec)


def compat_key(version: str) -> str:
    """semver 兼容边界：0.x 用 0.minor，其余用 major。"""
    base = version.split("+", 1)[0].split("-", 1)[0]
    parts = base.split(".")
    if not parts:
        return "?"
    if parts[0] == "0":
        return f"0.{parts[1]}" if len(parts) > 1 else "0"
    return parts[0]


def parse_version(version: str) -> tuple:
    base = version.split("+", 1)[0].split("-", 1)[0]
    out = []
    for piece in base.split("."):
        out.append(int(piece) if piece.isdigit() else 0)
    while len(out) < 3:
        out.append(0)
    return tuple(out[:3])


def load_manifests() -> dict[str, dict]:
    manifests = {}
    for rel in package_dirs():
        path = ROOT / rel / "Cargo.toml"
        if not path.is_file():
            print(f"error: 缺少 {rel}/Cargo.toml（scripts/rust-pkg-dirs.txt 列了它）", file=sys.stderr)
            sys.exit(1)
        manifests[rel] = tomllib.loads(path.read_text(encoding="utf-8"))
    return manifests


def check_declarations(manifests: dict[str, dict]) -> tuple[dict[str, set], int]:
    """规则 A / B。返回 (crate -> req 集合, 失败数)。"""
    # crate -> req -> [包]
    by_crate: dict[str, dict[str, list[str]]] = {}
    failures = 0

    for rel, manifest in manifests.items():
        seen_in_pkg: dict[str, dict[str, str]] = {}
        for section, table in iter_dep_tables(manifest):
            for name, spec in table.items():
                req = req_of(spec)
                seen_in_pkg.setdefault(name, {})[req] = section
                by_crate.setdefault(name, {}).setdefault(req, []).append(f"{rel} [{section}]")
        # 规则 B：同一 manifest 内同一 crate 出现互相矛盾的声明
        for name, reqs in seen_in_pkg.items():
            if len(reqs) > 1:
                detail = "、".join(f"{req}（{sec}）" for req, sec in sorted(reqs.items()))
                print(f"error: {rel}/Cargo.toml 对 {name} 有互相矛盾的声明：{detail}", file=sys.stderr)
                failures += 1

    # 规则 A：跨包声明一致性
    for name in sorted(by_crate):
        if name == CONTRACT_CRATE:
            continue
        reqs = by_crate[name]
        if len(reqs) == 1:
            continue
        print(f"error: 直接依赖 {name} 的 version 声明跨包不一致：", file=sys.stderr)
        for req, who in sorted(reqs.items()):
            print(f"  {req}: {', '.join(sorted(who))}", file=sys.stderr)
        failures += 1

    return {name: set(reqs) for name, reqs in by_crate.items()}, failures


def load_locks() -> dict[str, dict[str, set]]:
    """crate -> lock 相对路径 -> 版本集合。"""
    out: dict[str, dict[str, set]] = {}
    for rel in package_dirs():
        lock = ROOT / rel / "Cargo.lock"
        if not lock.is_file():
            print(f"error: 缺少 {rel}/Cargo.lock（本仓每包各自一份）", file=sys.stderr)
            sys.exit(1)
        data = tomllib.loads(lock.read_text(encoding="utf-8"))
        for pkg in data.get("package", []):
            name = pkg.get("name")
            version = pkg.get("version")
            if not name or not version:
                continue
            out.setdefault(name, {}).setdefault(rel, set()).add(version)
    return out


def report_lock_matrix(direct_crates: dict[str, set], locks: dict[str, dict[str, set]], verbose: bool) -> int:
    """报告 C：直接依赖在 lock 里的解析版本；同 compat 边界内 patch 不齐的只提醒。

    `direct_crates` 是规则 A 收集到的「本仓直接声明过的 crate → 该 crate 出现过的 req 集合」，
    这里只用到 key（crate 名）。

    默认只列「跨 lock 出现多于一个版本」的条目（含 note）；`--verbose` 时打全量矩阵，
    免得 pre-commit 每次都刷 40 行。

    范围说明：规则 A / 本报告都**只看 `version` req 与解析版本，不看 `features` /
    `default-features`**。跨包的 features 差异不会造成重复编译问题 —— 每个包是独立
    二进制，features 只在本包 lock 内做 unification，不像 version 那样会让同一份代码
    在一个产物里出现两遍。
    """
    warnings = 0
    if verbose:
        print("[check-dep-lockstep] 直接依赖在 Cargo.lock 里的解析版本：")
    for name in sorted(direct_crates):
        per_lock = locks.get(name)
        if not per_lock:
            if verbose:
                print(f"  {name}: <不在任何 lock 里>")
            continue
        versions = sorted({v for vs in per_lock.values() for v in vs}, key=parse_version)

        by_compat: dict[str, set] = {}
        for version in versions:
            by_compat.setdefault(compat_key(version), set()).add(version)

        if verbose:
            print(f"  {name}: {', '.join(versions)}")
        for key, group in sorted(by_compat.items()):
            if len(group) < 2:
                continue
            warnings += 1
            newest = max(group, key=parse_version)
            print(
                f"warning: {name} 在 compat {key} 内 patch 不齐：{', '.join(sorted(group, key=parse_version))}"
                f"（对齐：在各包目录跑 cargo update -p {name} --precise {newest}；不阻断门禁）"
            )
        # 跨 compat 边界并存多为本包传递依赖的产物，不是漂移（声明一致已由规则 A 保证）。
        if verbose and len(by_compat) > 1:
            print(
                f"  note: {name} 存在多个 compat 边界（{', '.join(sorted(by_compat))}）——"
                "多为本包传递依赖的产物，不是漂移"
            )
    return warnings


def main() -> int:
    if not PKG_LIST.is_file():
        print(f"error: 缺少 {PKG_LIST.relative_to(ROOT)}", file=sys.stderr)
        return 1

    manifests = load_manifests()
    direct, failures = check_declarations(manifests)
    locks = load_locks()
    verbose = "--verbose" in sys.argv[1:]
    warnings = report_lock_matrix(direct, locks, verbose)

    if failures:
        print(
            "改为：把同一个 crate 的 version req 在所有 Cargo.toml 里写成同一个字面量"
            "（升级时 8 个包一起改，别再让它们各走各的）。",
            file=sys.stderr,
        )
        return 1

    print(
        f"[check-dep-lockstep] ok（{len(manifests)} 个包 / {len(direct)} 个直接依赖 / "
        f"{len(package_dirs())} 份 Cargo.lock；patch 提示 {warnings} 条，不阻断）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
