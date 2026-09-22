#!/usr/bin/env python3
"""CSS 设计 token 门禁（扫描范围 = frontend/styles/*.css + frontend/themes/*.css）。

token（CSS 自定义属性）是深色 / 浅色 / 高对比主题的颜色与尺寸唯一来源。两类退化都不会报错，
只会让主题「悄悄失效」，必须机械拦截：

  1. **引用了未定义的 token**：`var(--typo)` 拼错或漏定义时整条声明静默失效（写了兜底就用兜底渲染），
     界面看起来只是「有点怪」，没有任何提示；
  2. **var() 兜底里含颜色字面量**：`var(--surface, #fff)` 的兜底在 token 已定义时永不生效，
     一旦 token 缺失就渲染出固定浅色 —— 主题化背景下这是最容易被忽略的视觉回归。

「有意使用、样式表内不可能定义」的运行时注入属性（Rust `style.setProperty` / Android IME 注入 /
渲染期内联 `style="--x: ..."`）登记在 `scripts/css_tokens_allowlist.txt`，须写理由；
白名单失效（token 已在 CSS 定义 / 已无任何引用 / 拼写笔误）同样失败。

扫描范围刻意与 scripts/check-css-breakpoints.sh 的 CSS_DIRS 一致：不含构建产物
（`frontend/dist`、`*/target`），也不含 connect / splash 等自持变量的独立迷你页面。
"""

from __future__ import annotations

import difflib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CSS_DIRS = ("frontend/styles", "frontend/themes")
ALLOWLIST = ROOT / "scripts" / "css_tokens_allowlist.txt"

COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
DEF_RE = re.compile(r"(--[A-Za-z0-9_-]+)\s*:")
VAR_RE = re.compile(r"var\(\s*(--[A-Za-z0-9_-]+)\s*(,)?")
# 只有「带主题倾向」的颜色字面量算违规：transparent / currentColor 是主题中立的，不在此列。
COLOR_RE = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(|\b(?:white|black)\b")


def strip_comments(text: str) -> str:
    """注释按等长空格抹掉，保证行号与原文一致。"""
    return COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def css_files() -> list[Path]:
    files: list[Path] = []
    for d in CSS_DIRS:
        p = ROOT / d
        if p.is_dir():
            files.extend(sorted(p.glob("*.css")))
    return files


def line_of(text: str, idx: int) -> int:
    return text.count("\n", 0, idx) + 1


def var_usages(text: str) -> list[tuple[str, str | None, int]]:
    """产出 (name, 兜底文本或 None, 起始下标)，兜底按括号配平截取。"""
    out: list[tuple[str, str | None, int]] = []
    for m in VAR_RE.finditer(text):
        name = m.group(1)
        if not m.group(2):
            out.append((name, None, m.start()))
            continue
        depth, j = 1, m.end()
        while j < len(text) and depth > 0:
            if text[j] == "(":
                depth += 1
            elif text[j] == ")":
                depth -= 1
            j += 1
        out.append((name, text[m.end() : j - 1], m.start()))
    return out


def collect() -> tuple[dict[str, str], dict[str, list[str]], list[tuple[str, int, str, str]]]:
    """返回 (已定义 token -> 首个定义位置, 被引用 token -> 引用位置列表, 颜色兜底列表)。"""
    defined: dict[str, str] = {}
    used: dict[str, list[str]] = {}
    bad_fallback: list[tuple[str, int, str, str]] = []
    for f in css_files():
        rel = f.relative_to(ROOT).as_posix()
        text = strip_comments(f.read_text(encoding="utf-8"))
        for m in DEF_RE.finditer(text):
            defined.setdefault(m.group(1), f"{rel}:{line_of(text, m.start())}")
        for name, fallback, idx in var_usages(text):
            used.setdefault(name, []).append(f"{rel}:{line_of(text, idx)}")
            if fallback is not None and COLOR_RE.search(fallback):
                bad_fallback.append(
                    (rel, line_of(text, idx), name, " ".join(fallback.split()))
                )
    return defined, used, bad_fallback


class AllowlistError(Exception):
    pass


def load_allowlist() -> dict[str, tuple[str, int]]:
    entries: dict[str, tuple[str, int]] = {}
    if not ALLOWLIST.exists():
        raise AllowlistError(f"{ALLOWLIST.relative_to(ROOT)} 不存在")
    for lineno, line in enumerate(ALLOWLIST.read_text(encoding="utf-8").splitlines(), 1):
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        key, sep, reason = s.partition("#")
        key, reason = key.strip(), reason.strip()
        if not sep or not reason:
            raise AllowlistError(f"{ALLOWLIST.name}:{lineno} 缺少 '# 理由'：{s}")
        if not key.startswith("--") or " " in key:
            raise AllowlistError(f"{ALLOWLIST.name}:{lineno} 应为 --<name>：{s}")
        if key in entries:
            raise AllowlistError(f"{ALLOWLIST.name}:{lineno} 重复登记 {key}")
        entries[key] = (reason, lineno)
    return entries


def report(
    undefined: dict[str, list[str]],
    violations: list[str],
    bad_fallback: list[tuple[str, int, str, str]],
    stale: list[str],
    allow: dict[str, tuple[str, int]],
    defined: dict[str, str],
) -> None:
    if violations:
        print(
            "[check-css-tokens] 违规：引用了未定义的 token（var() 会静默失效）：", file=sys.stderr
        )
        for name in violations:
            near = difflib.get_close_matches(name, defined.keys(), n=3, cutoff=0.65)
            hint = f"  → 相近的已定义 token：{'; '.join(near)}" if near else ""
            print(f"  {undefined[name][0]}  {name}{hint}", file=sys.stderr)
        print(
            "  → 拼写错误请改正；确需新 token 请先在 frontend/styles/tokens.css 定义；"
            "属运行时注入（Rust setProperty / Android IME / 内联 style）请登记到 "
            "scripts/css_tokens_allowlist.txt 并写明理由",
            file=sys.stderr,
        )
    if bad_fallback:
        print(
            "[check-css-tokens] 违规：var() 兜底里含颜色字面量"
            "（token 缺失时会渲染出固定浅色，掩盖问题）：",
            file=sys.stderr,
        )
        for rel, lineno, name, fallback in bad_fallback:
            print(f"  {rel}:{lineno}  var({name}, {fallback})", file=sys.stderr)
        print(
            "  → 颜色一律由 token 提供：去掉整个兜底（var(--x)），或把兜底换成已有 token",
            file=sys.stderr,
        )
    if stale:
        print(
            "[check-css-tokens] 白名单失效（CSS 已定义 / 已无引用 / 拼写笔误），请删除下列条目：",
            file=sys.stderr,
        )
        for name in stale:
            reason, lineno = allow[name]
            print(
                f"  scripts/css_tokens_allowlist.txt:{lineno}  {name}  （{reason}）",
                file=sys.stderr,
            )


def main() -> int:
    try:
        allow = load_allowlist()
    except AllowlistError as exc:
        print(f"[check-css-tokens] 白名单格式错误：{exc}", file=sys.stderr)
        return 1

    defined, used, bad_fallback = collect()
    undefined = {n: loc for n, loc in used.items() if n not in defined}
    # 白名单只赦免「未定义但被引用」；已定义或已无引用都算失效条目。
    violations = sorted(n for n in undefined if n not in allow)
    stale = sorted(k for k in allow if k in defined or k not in used)

    if violations or bad_fallback or stale:
        report(undefined, violations, bad_fallback, stale, allow, defined)
        return 1

    print(
        f"[check-css-tokens] ok（已定义 {len(defined)} 个 token，引用 {len(used)} 个；"
        f"未定义引用 0，颜色兜底 0；白名单 {len(allow)} 条）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
