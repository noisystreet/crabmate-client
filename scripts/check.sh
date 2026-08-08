#!/usr/bin/env bash
# 无 pre-commit 时的最小检查；与 .pre-commit-config.yaml 本地钩子对齐（不含 commit-msg / typos）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "[check] forbid path deps back to Server monorepo"
bash "$ROOT/scripts/check-no-main-path.sh"

echo "[check] cargo fmt (desktop + mobile + connect)"
(cd desktop-tauri/src-tauri && cargo fmt --all -- --check)
(cd mobile-tauri/src-tauri && cargo fmt --all -- --check)
(cd crates/crabmate-connect && cargo fmt --all -- --check)

echo "[check] ensure desktop dist stubs for tauri codegen"
mkdir -p desktop-tauri/dist
[[ -f desktop-tauri/dist/index.html ]] || echo '<html></html>' > desktop-tauri/dist/index.html
[[ -f desktop-tauri/dist/splash.html ]] || echo '<html></html>' > desktop-tauri/dist/splash.html
[[ -f desktop-tauri/dist/connect.html ]] || cp crates/crabmate-connect/assets/connect.html desktop-tauri/dist/connect.html

echo "[check] cargo clippy desktop"
(cd desktop-tauri/src-tauri && cargo clippy --all-targets -- -D warnings)

echo "[check] cargo clippy mobile"
(cd mobile-tauri/src-tauri && cargo clippy --all-targets -- -D warnings)

echo "[check] cargo clippy connect"
(cd crates/crabmate-connect && cargo clippy --all-targets -- -D warnings)

echo "[check] lizard / fn-param / fn-nloc"
bash "$ROOT/scripts/lizard-rust.sh"
bash "$ROOT/scripts/fn-param-ratchet.sh"
bash "$ROOT/scripts/fn-nloc-ratchet.sh"

if command -v taplo >/dev/null 2>&1; then
  echo "[check] taplo format + lint"
  taplo format --check .
  taplo lint .
else
  echo "[check] taplo 未安装，跳过"
fi

echo "[check] ok"
