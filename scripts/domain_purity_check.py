#!/usr/bin/env python3
"""`frontend/src/domain/**` 的纯度门禁（#598）。

## 为什么需要它

`frontend/src/domain/` 是逐域**纯规则**层：它存在的全部价值是「能在 host target 上
`cargo test`」（`make test-frontend` = `cd frontend && cargo test --lib`）。一旦有人在这里
引用 `web_sys` / leptos 信号 / `crate::api`，wasm 侧照样编得过、跑得动，**host 单测却会
静默失效** —— 规则重新退回「只能靠整页 e2e 兜」，而这正是 #598 要消掉的缺口。

`AGENTS.md` 的 **Frontend domain layer** 条目给了形状；本脚本把形状变成机械约束
（即 issue 第 5 步：先立形状，再上门禁）。

## 规则

`frontend/src/domain/**/*.rs` 里出现下列任一记号即失败：

| 记号 | 为什么不行 |
|---|---|
| `web_sys` / `js_sys` | 浏览器 API：host 上要么编不过，要么运行期就坏 |
| `leptos` | 信号 / owner / `Effect`：域层不该知道谁在订阅 |
| `crate::api` | HTTP / IO |
| `SystemTime::now` / `Instant::now` | 读时钟：规则变成不可测，改由调用方注入 |
| `uuid` / `rand::` | 生成 id / 随机源：同上 |
| `spawn_local` | 异步时序属于接线职责 |

## 判定细节

- 先剥掉 `//` 行注释与 `/* */` 块注释再匹配：注释里点名 `web_sys` 是文档，不是依赖；
- 字符串字面量**不**剥：域层里拼出 `"web_sys"` 基本只有一种解释，就是真的在用；
- 就地豁免：行尾或上一行写 `domain-gate: allow <理由>`，理由为空不算豁免；
- 目录不存在直接失败：改名 / 挪位不许让门禁静默失效。
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOMAIN = ROOT / "frontend" / "src" / "domain"

# (记号, 说明)；顺序只影响输出可读性
FORBIDDEN = (
    ("web_sys", "浏览器 API 属于接线侧"),
    ("js_sys", "浏览器 API 属于接线侧"),
    ("leptos", "信号 / owner 属于接线侧"),
    ("crate::api", "HTTP / IO 属于接线侧"),
    ("SystemTime::now", "读时钟：时间由调用方注入"),
    ("Instant::now", "读时钟：时间由调用方注入"),
    ("uuid", "生成 id 由调用方注入"),
    ("rand::", "随机源由调用方注入"),
    ("spawn_local", "异步时序属于接线职责"),
)

ALLOW_MARKER = "domain-gate: allow"


def strip_comments(text: str) -> list[str]:
    """按行剥掉 `//` 与 `/* */` 注释，保持行号不变。"""
    out: list[str] = []
    in_block = False
    for raw in text.splitlines():
        buf = []
        i = 0
        while i < len(raw):
            two = raw[i : i + 2]
            if in_block:
                if two == "*/":
                    in_block = False
                    i += 2
                else:
                    i += 1
                continue
            if two == "//":
                break
            if two == "/*":
                in_block = True
                i += 2
                continue
            buf.append(raw[i])
            i += 1
        out.append("".join(buf))
    return out


def allowed(raw_lines: list[str], idx: int) -> bool:
    """行尾或上一行的 `domain-gate: allow <理由>`（理由必须非空）。"""
    for probe in (idx, idx - 1):
        if probe < 0:
            continue
        line = raw_lines[probe]
        pos = line.find(ALLOW_MARKER)
        if pos < 0:
            continue
        reason = line[pos + len(ALLOW_MARKER) :].strip().strip("*/").strip()
        if reason:
            return True
    return False


def main() -> int:
    if not DOMAIN.is_dir():
        print(f"[domain-purity] 目录不存在：{DOMAIN.relative_to(ROOT)}", file=sys.stderr)
        return 1

    files = sorted(DOMAIN.rglob("*.rs"))
    if not files:
        print("[domain-purity] 目录里没有 .rs 文件（改名了？）", file=sys.stderr)
        return 1

    hits: list[str] = []
    for path in files:
        rel = path.relative_to(ROOT)
        raw_lines = path.read_text(encoding="utf-8").splitlines()
        code_lines = strip_comments(path.read_text(encoding="utf-8"))
        for idx, code in enumerate(code_lines):
            if not code.strip():
                continue
            for token, why in FORBIDDEN:
                if token not in code:
                    continue
                if allowed(raw_lines, idx):
                    continue
                hits.append(f"{rel}:{idx + 1}: {token} — {why}")

    if hits:
        print("[domain-purity] domain 层必须是纯规则（#598）：", file=sys.stderr)
        for hit in hits:
            print(f"  {hit}", file=sys.stderr)
        print(
            "  修法：副作用留在调用点，域层只收数据；确需保留请写 `domain-gate: allow <理由>`。",
            file=sys.stderr,
        )
        return 1

    print(f"[domain-purity] ok（{len(files)} 个文件）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
