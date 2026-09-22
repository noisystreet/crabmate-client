#!/usr/bin/env bash
# 类名契约门禁：Rust / HTML 产出的类名必须在同作用域 CSS 里有规则，
# 「有意不写规则」的钩子/层名需登记到 scripts/css_contract_allowlist.txt（须写理由）。
#
# 作用域：frontend（styles/*.css + themes/*.css + index.html 内联 <style>）、
#         connect（crates/crabmate-connect/assets/connect.html）、
#         splash（desktop-tauri/splash.html）
# 门禁（与 scripts/css_contract_check.py 一致）：未定义且未登记的类名个数必须为 0；
# 白名单失效（CSS 已定义 / 消费方已移除 / 作用域笔误）同样失败。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/css_contract_check.py" "$@"
