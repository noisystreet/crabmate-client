#!/usr/bin/env bash
# 同步连接页 +（可选）业务 UI 到桌面 / 移动 dist。
# Phase 2：业务 UI 在壳内加载；connect.html 不得覆盖 index.html。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${ROOT}/crates/crabmate-connect/assets/connect.html"
if [[ ! -f "${SRC}" ]]; then
  echo "missing ${SRC}" >&2
  exit 1
fi

sync_connect() {
  local dest="$1"
  mkdir -p "$(dirname "${dest}")"
  cp "${SRC}" "${dest}"
  echo "synced connect.html -> ${dest}"
}

# 桌面：connect.html 与 index.html 并存
if [[ -d "${ROOT}/desktop-tauri/dist" ]] || mkdir -p "${ROOT}/desktop-tauri/dist"; then
  sync_connect "${ROOT}/desktop-tauri/dist/connect.html"
fi

# 移动：connect.html 为启动页；index.html 留给业务 UI
mkdir -p "${ROOT}/mobile-tauri/dist"
sync_connect "${ROOT}/mobile-tauri/dist/connect.html"

# 若尚无业务 UI，提示（不失败；prepare-mobile / prepare-sidecar 负责拷贝）
if [[ ! -f "${ROOT}/mobile-tauri/dist/index.html" ]]; then
  echo "note: mobile-tauri/dist/index.html missing — run: make prepare-mobile  (or make frontend && sync)" >&2
fi
