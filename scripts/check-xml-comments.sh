#!/usr/bin/env bash
# XML 注释门禁：注释正文不得出现连续两个减号（`--`）—— XML 规范 §2.5 禁止，
# aapt2 资源合并器会直接报「注释中不允许出现字符串 "--"」。
#
# 典型踩坑：注释里写 CSS 自定义属性（`--bg` / `--icon-sm`）或命令行选项（`--release`）。
#
# 作用域：全仓 *.xml（剪枝构建产物与依赖目录；gen/android 的源资源在扫描范围内）。
# 门禁（与 scripts/xml_comments_check.py 一致）：注释正文含 `--` 的文件数必须为 0。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/xml_comments_check.py" "$@"
