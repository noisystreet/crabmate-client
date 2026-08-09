#!/usr/bin/env bash
# 准备移动壳静态资源：同步 frontend/dist + connect.html（不覆盖业务 UI）。
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile_root="$(cd "${script_dir}/.." && pwd)"
repo_root="$(cd "${mobile_root}/.." && pwd)"

dist_dest="${mobile_root}/dist"
mkdir -p "${dist_dest}"

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
  # 保留已有 connect.html：先同步 UI，再覆盖写回 connect
  rm -rf "${dist_dest}"
  mkdir -p "${dist_dest}"
  cp -a "${dist_src}/." "${dist_dest}/"
  echo "synced frontend dist (${dist_src}) -> ${dist_dest}"
else
  echo "note: no frontend dist found under mobile-tauri/dist" >&2
  echo "  run: make frontend   # or set CRABMATE_FRONTEND_DIST" >&2
  if [[ "${CM_PREPARE_REQUIRE_FRONTEND:-0}" == "1" || "${CM_PREPARE_REQUIRE_FRONTEND:-}" == "true" ]]; then
    echo "error: mobile prepare requires frontend dist" >&2
    exit 1
  fi
fi

connect_src="${repo_root}/crates/crabmate-connect/assets/connect.html"
if [[ -f "${connect_src}" ]]; then
  cp "${connect_src}" "${dist_dest}/connect.html"
  echo "copied connect.html -> ${dist_dest}"
else
  echo "error: missing ${connect_src}" >&2
  exit 1
fi

if [[ ! -f "${dist_dest}/index.html" ]]; then
  echo "warning: ${dist_dest}/index.html missing — shell cannot load business UI after connect" >&2
fi

echo "prepared mobile shell assets in ${dist_dest}"
