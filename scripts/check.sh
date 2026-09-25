#!/usr/bin/env bash
# 无 pre-commit 时的最小检查；与 .pre-commit-config.yaml 本地钩子对齐（不含 commit-msg / typos / e2e）。
# 含 ktlint-android（需 java）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 全仓 Rust 包单一列表（与 Makefile fmt/clippy/clean、check-boundaries.sh 共用）
RUST_PKG_DIRS="$(grep -v -e '^#' -e '^$' "$ROOT/scripts/rust-pkg-dirs.txt")"

echo "[check] forbid path deps back to Server monorepo"
bash "$ROOT/scripts/check-no-main-path.sh"

echo "[check] dependency boundary rules (client-api purity / connect default / contract pin / version sync)"
bash "$ROOT/scripts/check-boundaries.sh"

echo "[check] responsive breakpoints single source (matches MOBILE_LAYOUT_BREAKPOINT_PX)"
bash "$ROOT/scripts/check-css-breakpoints.sh"

echo "[check] class-name contract (consumer classes must have CSS rules or be allowlisted)"
bash "$ROOT/scripts/check-css-contract.sh"

echo "[check] design tokens (referenced tokens must be defined; no color literal in var() fallback; themes must cover the same tokens)"
bash "$ROOT/scripts/check-css-tokens.sh"

echo "[check] css literals budget (per-file font-size / color literals in frontend/styles must match css_literals_budget.txt)"
bash "$ROOT/scripts/check-css-literals.sh"

echo "[check] icons (no literal <svg> outside icon.rs; svg rule sizes must be var(--icon-*))"
bash "$ROOT/scripts/check-icons.sh"

echo "[check] modal shell (role / aria-modal only emitted by focusable_menu.rs)"
bash "$ROOT/scripts/check-modal-shell.sh"

echo "[check] xml comments (no double-dash inside XML comments)"
bash "$ROOT/scripts/check-xml-comments.sh"

echo "[check] cargo fmt (all packages in scripts/rust-pkg-dirs.txt)"
while IFS= read -r dir; do
  echo "[check] cargo fmt $dir"
  (cd "$dir" && cargo fmt --all -- --check)
done <<< "$RUST_PKG_DIRS"

echo "[check] ensure desktop/mobile dist stubs for tauri codegen"
bash "$ROOT/scripts/ensure-tauri-dist-stubs.sh"

echo "[check] cargo clippy (all packages except frontend; frontend uses wasm32 toolchain)"
while IFS= read -r dir; do
  if [ "$dir" = "frontend" ]; then continue; fi
  echo "[check] cargo clippy $dir"
  (cd "$dir" && cargo clippy --all-targets -- -D warnings)
done <<< "$RUST_PKG_DIRS"

echo "[check] cargo clippy frontend (wasm32)"
rustup target add wasm32-unknown-unknown 2>/dev/null || true
(cd frontend && cargo clippy --target wasm32-unknown-unknown --all-targets --all-features -- -D warnings)

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

echo "[check] ktlint Android (edu/crabmate)"
bash "$ROOT/scripts/ktlint-android.sh"

echo "[check] ok"
