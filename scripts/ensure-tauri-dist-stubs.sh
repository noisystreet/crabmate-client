#!/usr/bin/env bash
# 为桌面/移动 Tauri codegen 确保 frontendDist 目录存在（二者均 gitignore）。
# 供 scripts/check.sh 与 pre-commit 共用，避免 CI 干净 checkout 与本地「已有 dist」行为不一致。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ensure_shell_dist() {
  local dir="$1"
  local with_splash="${2:-0}"
  mkdir -p "$dir"
  [[ -f "${dir}/index.html" ]] || echo '<html></html>' >"${dir}/index.html"
  [[ -f "${dir}/connect.html" ]] || cp crates/crabmate-connect/assets/connect.html "${dir}/connect.html"
  if [[ "${with_splash}" == "1" ]]; then
    [[ -f "${dir}/splash.html" ]] || echo '<html></html>' >"${dir}/splash.html"
  fi
}

ensure_shell_dist desktop-tauri/dist 1
ensure_shell_dist mobile-tauri/dist 0
