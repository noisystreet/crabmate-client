#!/usr/bin/env bash
# 无 pre-commit 时的最小检查；与 .pre-commit-config.yaml 中 client-check 一致。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

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

echo "[check] ok"
