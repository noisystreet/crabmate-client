#!/usr/bin/env bash
# 模态骨架门禁：模态的 role / aria-modal 只能由共享壳发出。
#   frontend/src/**/*.rs（除 app/focusable_menu.rs）不得出现
#   role="dialog" / role="alertdialog" / set_attribute("role", "dialog") / aria-modal
#   ——一律走 FocusableModalPanel。
#
# 有意例外在规则所在行写 `modal-gate: allow <理由>` 即可跳过。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/modal_shell_check.py" "$@"
