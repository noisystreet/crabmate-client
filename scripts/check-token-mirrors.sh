#!/usr/bin/env bash
# 设计令牌镜像门禁（单一来源 = frontend/styles/tokens.css 的 :root）：
#   1. Android 原生层镜像：res/values/colors.xml 的 cm_<token> 必须逐项等于 tokens.css 的 --<token>，
#      且该文件不得混入其它裸颜色；
#   2. Android 主题：res/values*/themes.xml 不得出现颜色字面量，应用主题四个底色必须引用
#      @color/cm_bg，且必须显式声明系统栏图标用浅色（windowLightStatusBar /
#      windowLightNavigationBar = false）；
#   3. 首绘底色：frontend/index.html 内联样式与
#      crates/crabmate-connect/assets/connect.html 的 bg0 必须等于 --bg；
#   4. 主题里的颜色引用必须真实存在：res/values*/themes.xml 引用的每个 @color/<name>
#      都要在 res/values*/colors.xml 里有定义（拼错否则只在 AAPT 构建期报错）；
#   5. 手写 Kotlin（java/edu/crabmate）只许消费 R.color.cm_*，用别的颜色资源即绕过镜像链；
#   6. 桌面原生窗口底色：desktop-tauri/src-tauri/src/desktop_main_window.rs 的 BOOT_SHELL_BG
#      必须等于 --bg（WebView 在缩放 / 重绘间隙会露出这一层）。
#
# 六条规则都是精确相等、无白名单：改值必须两侧同改，声明被改名 / 搬走同样失败（防门禁静默失效）。
# 其中 3 / 6 两条还要求声明「恰好命中一次」。
# 门禁实现与 scripts/token_mirrors_check.py 一致。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/token_mirrors_check.py" "$@"
