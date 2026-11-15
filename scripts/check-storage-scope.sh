#!/usr/bin/env bash
# #457 W0-4：连接作用域的 localStorage 键名只能有一处来源。
#
# 这些键要和「本页属于哪个连接」拼成分域键（`<base>#<key>`）。名字散开定义，迟早出现
# 「一个键分了域、另一个没分」的半迁移状态 —— 症状是「换了 serve 之后某一项配置还跟着
# 走」，而它只在多窗口下才现形，很难查。
#
# 规则：下列名只允许出现在 `frontend/src/api/storage_scope.rs`。
#   - crabmate-api-base-url          （serve 基址）
#   - crabmate-api-bearer-token      （纯浏览器弱存储的 Web API Bearer）
#   - crabmate-llm-thinking-mode     （思考模式两态）
#   - __CM_CONN_KEY__                （壳注入的连接标识）
#
# 例外：无。新加「与连接绑定」的存储键时，先在本模块登记常量，再由 `api::browser`
# 的分域读写使用。**应用级**偏好（语言 / 主题 / 列宽 / 草稿）不分域，也不进本名单。
#
# 用法：bash scripts/check-storage-scope.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OWNER="frontend/src/api/storage_scope.rs"
INJECTOR="desktop-tauri/src-tauri/src/desktop_main_window.rs"
PATTERN='crabmate-api-base-url|crabmate-api-bearer-token|crabmate-llm-thinking-mode|__CM_CONN_KEY__'

if [ ! -f "$OWNER" ]; then
  echo "缺少 $OWNER：连接分域键名的唯一来源" >&2
  exit 1
fi

hits="$(grep -rnE "$PATTERN" frontend/src --include='*.rs' | grep -v "^${OWNER}:" || true)"
if [ -n "$hits" ]; then
  echo "以下位置直接写了连接分域键名，它们必须只在 $OWNER 定义：" >&2
  echo "$hits" >&2
  echo "改法：从 api::storage_scope 取常量，再交给 api::browser 的分域读写。" >&2
  exit 1
fi

# 注入名两端必须同源：壳写它、前端读它，拼错只会表现为「多窗口又互相覆盖」。
for f in "$OWNER" "$INJECTOR"; do
  if ! grep -q '__CM_CONN_KEY__' "$f"; then
    echo "$f 里找不到注入名 __CM_CONN_KEY__（壳与前端必须同名）" >&2
    exit 1
  fi
done

echo "storage scope ok（分域键名单一来源：$OWNER；注入名两端一致）"
