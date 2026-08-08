#!/usr/bin/env bash
# 准备桌面壳静态资源：连接页 / 闪屏；可选同步业务 UI dist。
# 桌面壳**不再**打包或校验 crabmate sidecar 二进制。
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop_root="$(cd "${script_dir}/.." && pwd)"
repo_root="$(cd "${desktop_root}/.." && pwd)"

dist_dest="${desktop_root}/dist"
mkdir -p "${dist_dest}"

# 业务 UI 产物：
#   CM_PREPARE_SKIP_FRONTEND=1 或 CRABMATE_FRONTEND_DIST=-  → 不同步 UI（保留已有 dist，供 CI stub）
#   CRABMATE_FRONTEND_DIST=<path>                            → 用该目录
#   否则本仓 frontend/dist
#   CRABMATE_ALLOW_SIBLING_FRONTEND=1 时才回落 ../crabmate_agent/frontend/dist
dist_src=""
skip_frontend=0
if [[ "${CM_PREPARE_SKIP_FRONTEND:-0}" == "1" || "${CM_PREPARE_SKIP_FRONTEND:-}" == "true" || "${CM_PREPARE_SKIP_FRONTEND:-}" == "yes" ]]; then
  skip_frontend=1
elif [[ "${CRABMATE_FRONTEND_DIST:-}" == "-" ]]; then
  skip_frontend=1
elif [[ -n "${CRABMATE_FRONTEND_DIST:-}" ]]; then
  dist_src="${CRABMATE_FRONTEND_DIST}"
elif [[ -f "${repo_root}/frontend/dist/index.html" ]]; then
  dist_src="${repo_root}/frontend/dist"
elif [[ "${CRABMATE_ALLOW_SIBLING_FRONTEND:-0}" == "1" || "${CRABMATE_ALLOW_SIBLING_FRONTEND:-}" == "true" || "${CRABMATE_ALLOW_SIBLING_FRONTEND:-}" == "yes" ]]; then
  if [[ -f "${repo_root}/../crabmate_agent/frontend/dist/index.html" ]]; then
    dist_src="${repo_root}/../crabmate_agent/frontend/dist"
  fi
fi

if [[ "${skip_frontend}" -eq 1 ]]; then
  echo "note: skip frontend sync (CM_PREPARE_SKIP_FRONTEND or CRABMATE_FRONTEND_DIST=-)" >&2
elif [[ -n "${dist_src}" && -f "${dist_src}/index.html" ]]; then
  rm -rf "${dist_dest}"
  cp -a "${dist_src}" "${dist_dest}"
  echo "synced frontend dist (${dist_src}) -> ${dist_dest}"
else
  echo "note: no frontend dist found (ok for shell-only; connect/splash still copied)" >&2
  echo "  run: make frontend   # or set CRABMATE_FRONTEND_DIST" >&2
fi

# 启动画面
splash_src="${desktop_root}/splash.html"
if [[ -f "${splash_src}" ]]; then
  cp "${splash_src}" "${dist_dest}/splash.html"
  echo "copied splash.html -> ${dist_dest}"
fi

# 桌面/移动共用连接页（源：crates/crabmate-connect/assets）
connect_src="${repo_root}/crates/crabmate-connect/assets/connect.html"
if [[ -f "${connect_src}" ]]; then
  cp "${connect_src}" "${dist_dest}/connect.html"
  echo "copied connect.html -> ${dist_dest}"
else
  echo "error: missing ${connect_src}" >&2
  exit 1
fi

if [[ -d "${desktop_root}/binaries" ]]; then
  echo "note: desktop-tauri/binaries/ is unused (shell does not spawn serve); safe to delete locally" >&2
fi

echo "prepared desktop shell assets in ${dist_dest}"
