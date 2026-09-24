#!/usr/bin/env bash
# CSS 设计 token 门禁（扫描范围 = frontend/styles/*.css + frontend/themes/*.css）：
#   1. `var(--x)` 引用的 token 必须在该范围内有定义；
#   2. `var()` 兜底禁止颜色字面量（token 缺失时会渲染固定浅色，掩盖主题回归）；
#   3. 主题覆盖完整性：某个主题覆盖过的 token，其余主题必须同样显式覆盖（否则该主题下静默沿用
#      默认深色值）。规则按并集机械推导，主题身份变量（字体族 / 圆角档）在脚本内登记豁免。
#
# 「有意使用、样式表内不可能定义」的运行时注入属性（Rust setProperty / Android IME / 内联 style）
# 登记到 scripts/css_tokens_allowlist.txt，须写理由。
#
# 门禁（与 scripts/css_tokens_check.py 一致）：未定义引用、颜色兜底、主题覆盖缺口、白名单失效
# 四项必须全为 0。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/css_tokens_check.py" "$@"
