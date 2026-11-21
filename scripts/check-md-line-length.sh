#!/usr/bin/env bash
# Markdown 单行长度门禁：**每行**不得超过 MAX_COLS 显示列（东亚宽字符按 2 列，约 80 汉字）。
#
# 唯一豁免是围栏代码块（折行会改语义）；表格行同样受约束。CHANGELOG.md 整文件不计
# （条目必须单行 ≤800 字符、已发布段冻结）。MAX_COLS 当前 2000（2026-11 重构后的实测
# 最长），只许往下调，目标 160。口径与实现见 scripts/md_line_length_check.py。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/md_line_length_check.py" "$@"
