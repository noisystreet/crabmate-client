#!/usr/bin/env bash
# Markdown 单行长度门禁：**每行**不得超过 MAX_COLS 显示列（东亚宽字符按 2 列，约 80 汉字）。
#
# 唯一豁免是围栏代码块（折行会改语义）；表格行同样受约束。MAX_COLS 当前等于存量最长
# 行（24501），只许往下调，目标 160。口径与实现见 scripts/md_line_length_check.py。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/md_line_length_check.py" "$@"
