#!/usr/bin/env bash
# 各 Cargo workspace：cargo audit（RustSec）+ cargo deny licenses/bans/sources。
# 与 .github/workflows/dependency-security.yml 对齐；不进 pre-commit / check.sh。
# 安装：cargo install cargo-audit cargo-deny
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

need_bin() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "$name 未安装。请执行: cargo install cargo-audit cargo-deny" >&2
    exit 1
  fi
}

need_bin cargo-audit
need_bin cargo-deny

# 与 scripts/check.sh 的 crate 根一致（各有独立 Cargo.lock）。
CARGO_DIRS=(
  desktop-tauri/src-tauri
  mobile-tauri/src-tauri
  crates/crabmate-client-api
  crates/crabmate-connect
  crates/crabmate-tui-core
  crates/crabmate-tui
  frontend
)

for rel in "${CARGO_DIRS[@]}"; do
  echo "[dependency-security] $rel"
  (
    cd "$ROOT/$rel"
    cargo audit
    # 不含 advisories：避免与 cargo audit 重复；策略见仓库根 deny.toml。
    cargo deny check licenses bans sources
  )
done

echo "[dependency-security] ok"
