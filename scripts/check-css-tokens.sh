#!/usr/bin/env bash
# CSS 设计 token 门禁：frontend/styles + frontend/themes 内 `var(--x)` 引用的 token 必须在该范围内
# 有定义，且 var() 兜底禁止颜色字面量（token 缺失时会渲染固定浅色，掩盖主题回归）。
#
# 「有意使用、样式表内不可能定义」的运行时注入属性（Rust setProperty / Android IME / 内联 style）
# 登记到 scripts/css_tokens_allowlist.txt，须写理由。
#
# 门禁（与 scripts/css_tokens_check.py 一致）：未定义引用、颜色兜底、白名单失效三项必须全为 0。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/css_tokens_check.py" "$@"
