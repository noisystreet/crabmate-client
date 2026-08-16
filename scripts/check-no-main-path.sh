#!/usr/bin/env bash
# 禁止壳仓 Cargo 依赖 path 回 Server 开发树（路径 A / P3.3）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 匹配典型误用：相对 path 指向 crabmate_agent / CrabMate crates，或绝对 path 回主仓。
PATTERN='path\s*=\s*"[^"]*(crabmate_agent|CrabMate/crates|/CrabMate/)'

hits=""
if command -v rg >/dev/null 2>&1; then
  hits="$(rg -n --glob '**/Cargo.toml' -e "$PATTERN" . || true)"
elif command -v grep >/dev/null 2>&1; then
  hits="$(grep -Rn --include='Cargo.toml' -E "$PATTERN" . || true)"
else
  echo "error: check-no-main-path 需要 rg 或 grep（CI 请 apt install ripgrep）" >&2
  exit 1
fi

if [[ -n "${hits}" ]]; then
  echo "error: 发现 path 依赖回 Server monorepo（禁止）：" >&2
  echo "${hits}" >&2
  echo "契约请钉 git crabmate + protocol（rev / 日后 crates.io 0.4.0）；connect 仅用本仓 path。" >&2
  exit 1
fi

echo "[check-no-main-path] ok"
