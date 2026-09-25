#!/usr/bin/env python3
"""模态骨架门禁（frontend）：`role="dialog"` / `aria-modal` 只能由共享壳发出。

把散落的手抄模态收敛到 `frontend/src/app/focusable_menu.rs` 的
`FocusableModalPanel` 之后，退化不会立刻报错：新写一个模态时照旧手抄
`role="dialog" aria-modal="true"` 照样能跑 —— 只能靠机械检查拦住。

本门禁一条规则（扫描范围仅 frontend/src，排除 focusable_menu.rs）：

  - 不得出现 `role="dialog"` / `role="alertdialog"` 字面属性；
  - 不得出现 `set_attribute("role", "dialog")` 这类命令式等价写法；
  - 不得出现任何 `aria-modal`。

改用 `FocusableModalPanel` 并传 `dialog_role`（默认 `alertdialog`，
普通弹窗传 `"dialog"`）；`labelledby` / `testid` / `stop_pointerdown` /
`on_escape` 也在壳上。

有意例外：在规则所在行写 `modal-gate: allow <理由>` 即跳过该处，
理由随代码走、不留单独清单（当前唯一豁免：命令式建 DOM 的图片灯箱）。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "frontend" / "src"
# 壳的唯一落点，本门禁对它豁免。
SHELL_MODULE = SRC_DIR / "app" / "focusable_menu.rs"

ALLOW_MARKER = "modal-gate: allow"

COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
LINE_COMMENT_RE = re.compile(r"//")

# 前面不能是标识符字符，避免把 `dialog_role="dialog"` 这种「传壳的 prop」误判成手抄。
ROLE_ATTR_RE = re.compile(r'(?<![A-Za-z0-9_])role\s*=\s*"(?:dialog|alertdialog)"')
ROLE_SET_ATTR_RE = re.compile(
    r'set_attribute\(\s*"role"\s*,\s*"(?:dialog|alertdialog)"'
)
ARIA_MODAL_RE = re.compile(r"aria-modal")


class Rule:
    def __init__(self, rel: str, line: int, message: str) -> None:
        self.rel = rel
        self.line = line
        self.message = message

    def render(self) -> str:
        return f"  {self.rel}:{self.line}  {self.message}"


def strip_block_comments(text: str) -> str:
    """块注释按等长空格抹掉，保证行号与原文一致。"""
    return COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def check_handwritten_modals() -> list[Rule]:
    rules: list[Rule] = []
    for path in sorted(SRC_DIR.rglob("*.rs")):
        if path == SHELL_MODULE:
            continue
        raw = path.read_text(encoding="utf-8")
        clean = strip_block_comments(raw)
        for lineno, (raw_line, line) in enumerate(
            zip(raw.splitlines(), clean.splitlines()), 1
        ):
            if ALLOW_MARKER in raw_line:
                continue
            comment = LINE_COMMENT_RE.search(line)
            code = line[: comment.start()] if comment else line
            rel = path.relative_to(ROOT).as_posix()
            if ROLE_ATTR_RE.search(code) or ROLE_SET_ATTR_RE.search(code):
                rules.append(
                    Rule(
                        rel,
                        lineno,
                        "手写 `role=\"dialog\"` / `\"alertdialog\"`；"
                        "请改用 `FocusableModalPanel`（`dialog_role=\"dialog\"`）",
                    )
                )
            elif ARIA_MODAL_RE.search(code):
                rules.append(
                    Rule(
                        rel,
                        lineno,
                        "手写 `aria-modal`；该属性由 `FocusableModalPanel` 统一发出",
                    )
                )
    return rules


def main() -> int:
    problems = check_handwritten_modals()
    if problems:
        print(
            "[check-modal-shell] 违规：模态的 role / aria-modal 只能由共享壳发出",
            file=sys.stderr,
        )
        for problem in problems:
            print(problem.render(), file=sys.stderr)
        print(
            "  → 改用 frontend/src/app/focusable_menu.rs 的 `FocusableModalPanel`"
            "（`dialog_role` / `labelledby` / `testid` / `stop_pointerdown` / `on_escape`）；"
            "有意例外请在该行写 `" + ALLOW_MARKER + " <理由>`",
            file=sys.stderr,
        )
        return 1
    print("[check-modal-shell] ok（focusable_menu.rs 之外无手写 role / aria-modal）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
