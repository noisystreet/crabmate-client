#!/usr/bin/env bash
# CSS 字面量预算门禁：frontend/styles/*.css（排除 tokens.css / themes）内，
#   1. 颜色字面量（hex / rgb() / rgba() / hsl() / hsla() / white / black）——颜色应由 var(--x) 提供；
#   2. font-size 字面量（px / rem / em）——字号应由 --text-* 等 token 或继承提供；
#   3. border-radius 字面量（px / rem / em / %，裸 0 不算，含 var() 兜底）——圆角应由 --radius-* 提供。
#
# 存量按「每文件预算」冻结在 scripts/css_literals_budget.txt，须写理由（含清理计划）。
# 门禁（与 scripts/css_literals_check.py 一致）：实际值必须等于预算值（新增即 fail，
# 清理后未同步下调预算同样 fail），且预算条目失效（文件不存在 / 三项均为 0）也 fail。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/css_literals_check.py" "$@"
