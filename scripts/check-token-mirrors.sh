#!/usr/bin/env bash
# 设计令牌镜像门禁（单一来源 = frontend/styles/tokens.css 的 :root）：
#   1. Android 原生层镜像：res/values/colors.xml 的 cm_<token> 必须逐项等于 tokens.css 的 --<token>，
#      且该文件不得混入其它裸颜色；
#   2. Android 主题：res/values*/themes.xml 不得出现颜色字面量，应用主题四个底色必须引用
#      @color/cm_bg，且必须显式声明系统栏图标用浅色（windowLightStatusBar /
#      windowLightNavigationBar = false）；
#   3. 首绘底色：frontend/index.html 内联样式、desktop-tauri/splash.html 与
#      crates/crabmate-connect/assets/connect.html 的 bg0 必须等于 --bg。
#
# 三条规则都是精确相等、无白名单：改值必须两侧同改，声明被改名 / 搬走同样失败（防门禁静默失效）。
# 门禁实现与 scripts/token_mirrors_check.py 一致。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/token_mirrors_check.py" "$@"
