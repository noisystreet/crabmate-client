#!/usr/bin/env bash
# 图标门禁：图标须只有一个「属性模板」来源与一个「尺寸」来源。
#   1. frontend/src/**/*.rs（除 icon.rs）不得出现字面 `<svg`——一律走 Icon 组件或其 helper；
#   2. frontend/styles/*.css 里 svg 选择器规则的 width / height（含 min- / max-）
#      必须是 var(--icon-*)——三档定义见 frontend/styles/tokens.css。
#
# 有意义例外在规则所在行写 `icon-gate: allow <理由>` 即可跳过。
# 已知盲区：不校验「类名与元素是否同体」（把容器类名挂到 <svg> 上时，
# `.container svg` 后代选择器仍会给出类名已覆盖的假证据），需靠 code review 兜住。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/icons_check.py" "$@"
