#!/usr/bin/env bash
# 对 `crates/`、`desktop-tauri/.../src`、`mobile-tauri/.../src`、`frontend/src` 做圈复杂度（CCN）扫描，使用 lizard（https://github.com/terryyin/lizard）。
# 未安装时：pip install lizard
#
# 门禁（与 scripts/lizard_rust_metrics.py 一致）：全局 CCN>10 的函数个数必须为 0，
# 出现即失败并列出函数；没有按模块的个数上限配置。
# 额外参数原样传给 Python，例如：
#   bash scripts/lizard-rust.sh --list-above 10
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! python3 -c "import lizard" 2>/dev/null; then
  echo "lizard 未安装。请执行: pip install lizard" >&2
  echo "（或: uv pip install lizard；检查见 .pre-commit-config.yaml 中 lizard-rust）" >&2
  exit 1
fi
exec python3 "$ROOT/scripts/lizard_rust_metrics.py" "$@"
