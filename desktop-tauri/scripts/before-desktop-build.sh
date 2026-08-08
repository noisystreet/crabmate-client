#!/usr/bin/env bash
# Tauri `beforeBuildCommand`：release 包必须带优化后的业务 UI，再同步壳静态资源。
#
# - 默认：`trunk build --release`（需 trunk + wasm-opt）→ 再 `prepare-sidecar.sh`
# - `CM_PREPARE_SKIP_FRONTEND=1` 或 `CRABMATE_FRONTEND_DIST=-`：跳过 UI 构建（CI stub）
# - 已设 `CRABMATE_FRONTEND_DIST=<path>`：不重建，只同步该目录（仍校验非 debug 体积，除非 SKIP）
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop_root="$(cd "${script_dir}/.." && pwd)"
repo_root="$(cd "${desktop_root}/.." && pwd)"
frontend_dir="${repo_root}/frontend"

skip_frontend=0
if [[ "${CM_PREPARE_SKIP_FRONTEND:-0}" == "1" || "${CM_PREPARE_SKIP_FRONTEND:-}" == "true" || "${CM_PREPARE_SKIP_FRONTEND:-}" == "yes" ]]; then
  skip_frontend=1
elif [[ "${CRABMATE_FRONTEND_DIST:-}" == "-" ]]; then
  skip_frontend=1
fi

if [[ "${skip_frontend}" -eq 0 ]]; then
  if [[ -n "${CRABMATE_FRONTEND_DIST:-}" ]]; then
    echo "note: CRABMATE_FRONTEND_DIST=${CRABMATE_FRONTEND_DIST} (skip trunk; sync only)" >&2
  else
    command -v trunk >/dev/null 2>&1 || {
      echo "error: trunk not found (cargo install trunk)" >&2
      exit 1
    }
    command -v wasm-opt >/dev/null 2>&1 || {
      echo "error: wasm-opt not found; trunk build --release would emit an empty .wasm" >&2
      echo "  install: cargo install wasm-opt" >&2
      exit 1
    }
    rustup target add wasm32-unknown-unknown 2>/dev/null || true
    echo "building release frontend (trunk build --release)…" >&2
    (cd "${frontend_dir}" && unset NO_COLOR && trunk build --release)
  fi
  # 防止把 debug 的 ~100MB+ WASM 打进 .deb
  export CM_PREPARE_REQUIRE_OPTIMIZED_WASM=1
fi

bash "${script_dir}/prepare-sidecar.sh"
