#!/usr/bin/env bash
# make web-release：trunk --release UI + crabmate-web 二进制 + .deb
# CI 打包门禁可设 CM_WEB_SKIP_FRONTEND=1，跳过 trunk，只用已有/stub 的 frontend/dist。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_DIR="${ROOT}/crates/crabmate-web-host"

if [[ "${CM_WEB_SKIP_FRONTEND:-0}" == "1" ]]; then
  echo "skipping trunk (CM_WEB_SKIP_FRONTEND=1)" >&2
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

  echo "building release frontend (trunk build --release)…" >&2
  rustup target add wasm32-unknown-unknown 2>/dev/null || true
  (cd "${ROOT}/frontend" && unset NO_COLOR && trunk build --release)
fi

echo "building crabmate-web…" >&2
cargo build --release --manifest-path "${HOST_DIR}/Cargo.toml"

bash "${ROOT}/scripts/pack-web-deb.sh"
