#!/usr/bin/env python3
"""类名契约门禁：消费者（Rust / HTML）产出的类名必须在同作用域 CSS 里有规则，否则须在白名单登记。

入口：bash scripts/check-css-contract.sh（等价于 python3 scripts/css_contract_check.py）

设计要点
--------
1. 按「作用域」隔离判定，避免跨作用域误报：
   - frontend：样式来源 = frontend/styles/*.css + frontend/themes/*.css + index.html 内联 <style>
               消费方   = frontend/src/**/*.rs + frontend/index.html
   - connect ：自包含页面（内联 style + 自身 class 消费）
   - splash  ：自包含页面（内联 style + 自身 class 消费）
2. 采集启发式与 P0 审计（agent_space/ui_css_audit/audit_class_contract.py）同源：不猜测
   「全量 kebab 字符串」，只采信带证据的候选（attr / tuple / cond / fnbody / cooc / isstate）；
   只作 data-* 属性值的 token 记为噪声，不判违规。
3. 作用域边界（有意为之）：第三方 vendor JS 运行时生成的类名（CodeMirror 的 cm-* 等）
   不在契约内，故消费方只收本仓自有 Rust / HTML。
4. 白名单 scripts/css_contract_allowlist.txt 登记「有意不写规则」的类名；失效条目
   （CSS 已定义 / 消费方已移除 / scope 拼写错误）同样报错，防止白名单腐烂。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOWLIST = ROOT / "scripts/css_contract_allowlist.txt"

# ---------------------------------------------------------------- 作用域定义

SCOPES = [
    {
        "name": "frontend",
        "css_globs": ["frontend/styles/*.css", "frontend/themes/*.css"],
        "css_files": ["frontend/index.html"],  # 内联 <style>
        "consumers": ["frontend/index.html"],
        "consumer_globs": ["frontend/src/**/*.rs"],
    },
    {
        "name": "connect",
        "css_globs": [],
        "css_files": ["crates/crabmate-connect/assets/connect.html"],
        "consumers": ["crates/crabmate-connect/assets/connect.html"],
        "consumer_globs": [],
    },
    {
        "name": "splash",
        "css_globs": [],
        "css_files": ["desktop-tauri/splash.html"],
        "consumers": ["desktop-tauri/splash.html"],
        "consumer_globs": [],
    },
]

# ---------------------------------------------------------------- 文本工具

IDENT_CLASS = re.compile(r"^[a-z][a-z0-9]*(?:-{1,2}[a-z0-9]+)*$")
PLACEHOLDER = re.compile(r"\{[^}]*\}")


def strip_rust_comments(src: str) -> str:
    """去掉 Rust 注释，保留字符串字面量与换行（行号可比对）。"""
    out: list[str] = []
    i, n = 0, len(src)
    state = "code"  # code | line | block | str | char | raw
    raw_hashes = 0
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if state == "code":
            if c == "/" and nxt == "/":
                state, i = "line", i + 2
                continue
            if c == "/" and nxt == "*":
                state, i = "block", i + 2
                continue
            if c == "r" and (nxt == '"' or nxt == "#"):
                m = re.match(r'r(#+)?"', src[i:])
                if m:
                    raw_hashes = len(m.group(1) or "")
                    out.append(src[i : i + m.end()])
                    i += m.end()
                    state = "raw"
                    continue
            if c == '"':
                state = "str"
            elif c == "'":
                # 区分生命周期 'a 与字符字面量 'a'
                if not re.match(r"'([^'\\]|\\.)'", src[i:]) and not re.match(
                    r"'\\(x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)'", src[i:]
                ):
                    out.append(c)
                    i += 1
                    continue
                state = "char"
            out.append(c)
            i += 1
        elif state == "line":
            if c == "\n":
                state = "code"
                out.append(c)
            i += 1
        elif state == "block":
            if c == "*" and nxt == "/":
                state, i = "code", i + 2
                continue
            if c == "\n":
                out.append(c)
            i += 1
        elif state in ("str", "char"):
            out.append(c)
            if c == "\\":
                out.append(nxt)
                i += 2
                continue
            if (state == "str" and c == '"') or (state == "char" and c == "'"):
                state = "code"
            i += 1
        elif state == "raw":
            if c == '"':
                tail = src[i + 1 : i + 1 + raw_hashes]
                if tail == "#" * raw_hashes:
                    out.append(c + tail)
                    i += 1 + raw_hashes
                    state = "code"
                    continue
            out.append(c)
            i += 1
    return "".join(out)


def strip_css_comments(css: str) -> str:
    return re.sub(r"/\*.*?\*/", "", css, flags=re.S)


def css_rules(css: str):
    """返回 [(prelude, depth)]，prelude 为 '{' 之前的文本。"""
    css = strip_css_comments(css)
    rules, buf, depth = [], [], 0
    for ch in css:
        if ch == "{":
            rules.append(("".join(buf).strip(), depth))
            buf, depth = [], depth + 1
        elif ch == "}":
            depth = max(0, depth - 1)
            buf = []
        else:
            buf.append(ch)
    return rules


CLASS_SEL = re.compile(r"\.(-?[A-Za-z_][A-Za-z0-9_-]*)")


def classes_in_css(css: str) -> set[str]:
    found: set[str] = set()
    for prelude, _depth in css_rules(css):
        if prelude.startswith("@"):
            continue
        for m in CLASS_SEL.finditer(prelude):
            found.add(m.group(1))
    return found


def inline_style_of_html(text: str) -> str:
    return "\n".join(m.group(1) for m in re.finditer(r"<style[^>]*>(.*?)</style>", text, re.S))


def class_attr_values_html(text: str) -> list[tuple[str, int]]:
    """HTML 里的 class="..." → [(value, 行号)]"""
    out = []
    body = re.sub(r"<style[^>]*>.*?</style>", "", text, flags=re.S)
    for m in re.finditer(r"""class\s*=\s*["']([^"']*)["']""", body):
        out.append((m.group(1), body[: m.start()].count("\n") + 1))
    return out


# ---------------------------------------------------------------- token 拆分


def tokens_of(value: str):
    """把 'a b--c is-x--{k}' 拆成 (静态 token 集合, 动态前缀集合)。"""
    static, dynamic = set(), set()
    for raw in re.split(r"[\s,]+", value):
        tok = raw.strip()
        if not tok:
            continue
        if "{" in tok:
            prefix = PLACEHOLDER.sub("", tok).strip()
            while prefix.endswith("-"):
                prefix = prefix[:-1]
            if prefix:
                dynamic.add(prefix)
            continue
        if IDENT_CLASS.match(tok):
            static.add(tok)
    return static, dynamic


# ---------------------------------------------------------------- Rust 采集

RE_ATTR = re.compile(r"""class\s*=\s*["']([^"']*)["']""")
RE_ATTR_MOVE = re.compile(r'''class\s*=\s*move\s*\|\|[^{;"\n]*?"([^"]*)"''')
RE_TUPLE = re.compile(r'''class\s*=\s*\(\s*"([^"]*)"''')
RE_COND = re.compile(r"""class\s*:\s*([A-Za-z_][A-Za-z0-9_-]*)\s*=""")
RE_LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
RE_FN_CLASS = re.compile(r"\bfn\s+([A-Za-z0-9_]*_class)\b")
RE_ISSTATE = re.compile(r'"\s*(is-[a-z0-9][a-z0-9-]*)\s*"')
RE_DATAATTR = re.compile(r"""data-[A-Za-z0-9-]+\s*=\s*["']([^"']*)["']""")


def common_prefix_len(a: str, b: str) -> int:
    n = 0
    for x, y in zip(a, b):
        if x != y:
            break
        n += 1
    return n


def near_miss(tok: str, css_classes: list[str], min_prefix: int = 8) -> list[str]:
    """与 token 共享较长前缀的既有 CSS 类名——报错时提示「命名漂移」。"""
    hits = [
        (common_prefix_len(tok, c), c)
        for c in css_classes
        if min_prefix <= common_prefix_len(tok, c) < max(len(tok), len(c))
    ]
    hits.sort(key=lambda x: (-x[0], x[1]))
    return [c for _n, c in hits[:3]]


def fn_bodies(code: str) -> list[tuple[str, int, str]]:
    """粗粒度提取 fn *_class 函数体（大括号配平），返回 (名字, 起始偏移, 函数体)。"""
    bodies: list[tuple[str, int, str]] = []
    for m in RE_FN_CLASS.finditer(code):
        name, i, n = m.group(1), m.end(), len(code)
        # 单测函数名也可能以 _class 结尾，不是「状态 → 类名」构造器
        if re.search(r"#\[\s*test\s*\]", code[max(0, m.start() - 200) : m.start()]):
            continue
        start = code.find("{", i)
        if start < 0:
            continue
        depth, j = 0, start
        while j < n:
            if code[j] == "{":
                depth += 1
            elif code[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        bodies.append((name, start, code[start : j + 1]))
    return bodies


def class_list_shaped(lit: str, known_css: set[str]) -> bool:
    """字面量是否「整条都是类名列表」。

    把 i18n 文案、测试断言句、HTML 片段挡在外面——只要出现一个既非已知 CSS 类名、
    又不符合 kebab 类名形态的 token，整条丢弃。
    """
    toks = [t for t in re.split(r"[\s,]+", lit) if t]
    if not toks:
        return False
    for t in toks:
        if t in known_css or "{" in t or IDENT_CLASS.match(t):
            continue
        return False
    return True


def collect_rust(path: Path, known_css: set[str]):
    """返回 {token: {kind: [行号]}} 与 {dynamic_prefix: {kind: [行号]}}。"""
    raw = path.read_text(encoding="utf-8", errors="replace")
    code = strip_rust_comments(raw)
    ev: dict[str, dict[str, list[int]]] = {}
    dyn: dict[str, dict[str, list[int]]] = {}

    def add(store, tok, kind, line):
        store.setdefault(tok, {}).setdefault(kind, []).append(line)

    def line_of(pos):
        return code[:pos].count("\n") + 1

    for rex, kind in ((RE_ATTR, "attr"), (RE_ATTR_MOVE, "attr"), (RE_TUPLE, "tuple")):
        for m in rex.finditer(code):
            s, d = tokens_of(m.group(1))
            for t in s:
                add(ev, t, kind, line_of(m.start(1)))
            for p in d:
                add(dyn, p, kind, line_of(m.start(1)))

    for m in RE_COND.finditer(code):
        add(ev, m.group(1), "cond", line_of(m.start(1)))

    for m in RE_ISSTATE.finditer(code):
        add(ev, m.group(1), "isstate", line_of(m.start(1)))

    for _name, start, body in fn_bodies(code):
        for m in RE_LITERAL.finditer(body):
            lit = m.group(1)
            # match 分支的 scrutinee（"user" => ...）不是类名
            if re.match(r"\s*=>", body[m.end() :]):
                continue
            if not class_list_shaped(lit, known_css):
                continue
            s, d = tokens_of(lit)
            for t in s:
                if "-" in t:  # 单词 token 无法与英文散文区分，只收 kebab
                    add(ev, t, "fnbody", line_of(start + m.start(0)))
            for p in d:
                add(dyn, p, "fnbody", line_of(start + m.start(0)))

    for m in RE_LITERAL.finditer(code):
        lit = m.group(1)
        if not class_list_shaped(lit, known_css):
            continue
        s, _d = tokens_of(lit)
        if not (s & known_css):
            continue
        for t in s:
            if "-" in t:
                add(ev, t, "cooc", line_of(m.start(1)))

    for m in RE_DATAATTR.finditer(code):
        s, _d = tokens_of(m.group(1))
        for t in s:
            add(ev, t, "dataattr", line_of(m.start(1)))

    return ev, dyn


# ---------------------------------------------------------------- 作用域扫描


def css_origins(scope) -> dict[str, set[str]]:
    origin: dict[str, set[str]] = {}
    for pattern in scope["css_globs"]:
        for p in sorted(ROOT.glob(pattern)):
            for c in classes_in_css(p.read_text(encoding="utf-8", errors="replace")):
                origin.setdefault(c, set()).add(str(p.relative_to(ROOT)))
    for rel in scope["css_files"]:
        p = ROOT / rel
        if not p.exists():
            continue
        text = p.read_text(encoding="utf-8", errors="replace")
        for c in classes_in_css(inline_style_of_html(text)):
            origin.setdefault(c, set()).add(f"{rel} (inline <style>)")
    return origin


def scope_consumers(scope, known_css: set[str]):
    """返回 (静态 token 证据, 动态前缀证据)；证据值均为 '路径:行号'。"""
    used: dict[str, dict[str, list[str]]] = {}
    dyn_used: dict[str, dict[str, list[str]]] = {}

    def touch(store, tok, kind, loc):
        store.setdefault(tok, {}).setdefault(kind, []).append(loc)

    for rel in scope["consumers"]:
        p = ROOT / rel
        if not p.exists():
            continue
        text = p.read_text(encoding="utf-8", errors="replace")
        for value, line in class_attr_values_html(text):
            s, d = tokens_of(value)
            for t in s:
                touch(used, t, "htmlattr", f"{rel}:{line}")
            for pf in d:
                touch(dyn_used, pf, "htmlattr", f"{rel}:{line}")
        for m in RE_DATAATTR.finditer(text):
            s, _d = tokens_of(m.group(1))
            for t in s:
                touch(used, t, "dataattr", f"{rel}:{m.start()}")

    for pattern in scope["consumer_globs"]:
        for p in sorted(ROOT.glob(pattern)):
            ev, dyn = collect_rust(p, known_css)
            rel = str(p.relative_to(ROOT))
            for tok, kinds in ev.items():
                for kind, lines in kinds.items():
                    for ln in lines:
                        touch(used, tok, kind, f"{rel}:{ln}")
            for pf, kinds in dyn.items():
                for kind, lines in kinds.items():
                    for ln in lines:
                        touch(dyn_used, pf, kind, f"{rel}:{ln}")

    return used, dyn_used


def undefined_tokens(scope_name: str, known_css: set[str], used, dyn_used):
    """返回 {token: (证据串, 归一化证据键)}，只含「CSS 无规则」的类名。"""
    found: dict[str, tuple[str, tuple[str, ...]]] = {}
    for tok, kinds in used.items():
        if tok in known_css:
            continue
        if set(kinds) == {"dataattr"}:
            continue  # 只作 data-* 属性值，不是类名
        if scope_name != "frontend" and set(kinds) <= {"htmlattr", "dataattr"}:
            continue  # 非 frontend 作用域的 HTML 属性只在自身页面内消费
        refs = sorted({x for v in kinds.values() for x in v})
        found[tok] = ("+".join(sorted(kinds)), tuple(refs))
    for pf, kinds in dyn_used.items():
        hit = [c for c in known_css if c.startswith(pf + "-") or c == pf]
        if hit:
            continue
        refs = sorted({x for v in kinds.values() for x in v})
        found[pf + "-{dynamic}"] = ("+".join(sorted(kinds)), tuple(refs))
    return found


# ---------------------------------------------------------------- 白名单


class AllowlistError(Exception):
    pass


def load_allowlist(path: Path, scope_names: set[str]) -> dict[tuple[str, str], tuple[str, int]]:
    entries: dict[tuple[str, str], tuple[str, int]] = {}
    if not path.exists():
        raise AllowlistError(f"{path.relative_to(ROOT)} 不存在")
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        key, sep, reason = s.partition("#")
        key, reason = key.strip(), reason.strip()
        if not sep or not reason:
            raise AllowlistError(f"{path.name}:{lineno} 缺少 '# 理由'：{s}")
        scope, sep2, token = key.partition(":")
        if not sep2 or not token:
            raise AllowlistError(f"{path.name}:{lineno} 应为 <scope>:<token>：{s}")
        if scope not in scope_names:
            raise AllowlistError(f"{path.name}:{lineno} 未知作用域 {scope}")
        if (scope, token) in entries:
            raise AllowlistError(f"{path.name}:{lineno} 重复登记 {key}")
        entries[(scope, token)] = (reason, lineno)
    return entries


# ---------------------------------------------------------------- 主流程


def main() -> int:
    scope_names = {s["name"] for s in SCOPES}
    try:
        allow = load_allowlist(ALLOWLIST, scope_names)
    except AllowlistError as exc:
        print(f"[check-css-contract] 白名单格式错误：{exc}", file=sys.stderr)
        return 1

    report: dict[str, dict[str, tuple[str, tuple[str, ...]]]] = {}
    css_classes: dict[str, list[str]] = {}
    for scope in SCOPES:
        origin = css_origins(scope)
        known = set(origin)
        used, dyn_used = scope_consumers(scope, known)
        report[scope["name"]] = undefined_tokens(scope["name"], known, used, dyn_used)
        css_classes[scope["name"]] = sorted(known)

    violations: list[tuple[str, str, str, tuple[str, ...], list[str]]] = []
    for scope_name, undef in report.items():
        for tok, (evidence, refs) in sorted(undef.items()):
            if (scope_name, tok) in allow:
                continue
            near = near_miss(tok, css_classes[scope_name]) if "{dynamic}" not in tok else []
            violations.append((scope_name, tok, evidence, refs, near))

    seen = {(scope_name, tok) for scope_name, undef in report.items() for tok in undef}
    stale = sorted(k for k in allow if k not in seen)

    if violations or stale:
        if violations:
            print(
                "[check-css-contract] 违规：Rust / HTML 产出类名，但同作用域 CSS 无对应规则：",
                file=sys.stderr,
            )
            for scope_name, tok, evidence, refs, near in violations:
                print(f"  {scope_name}:{tok}  [{evidence}]  {refs[0]}", file=sys.stderr)
                if near:
                    print(f"      前缀相近的既有 CSS 类名：{'; '.join(near)}", file=sys.stderr)
            print(
                "  → 属真实缺失请在对应 CSS 补规则；属「有意不写规则」的钩子/层名，"
                "登记到 scripts/css_contract_allowlist.txt 并写明理由",
                file=sys.stderr,
            )
        if stale:
            print(
                "[check-css-contract] 白名单失效（CSS 已定义 / 消费方已移除 / 作用域笔误），"
                "请删除下列条目：",
                file=sys.stderr,
            )
            for scope_name, tok in stale:
                reason, lineno = allow[(scope_name, tok)]
                print(
                    f"  scripts/css_contract_allowlist.txt:{lineno}  {scope_name}:{tok}  （{reason}）",
                    file=sys.stderr,
                )
        return 1

    summary = "；".join(
        f"{name} 未定义 {len(undef)}" for name, undef in sorted(report.items())
    )
    print(f"[check-css-contract] ok（{summary}；白名单 {len(allow)} 条）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
