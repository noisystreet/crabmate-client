#!/usr/bin/env bash
# 统一设置全部 Rust 包版本（scripts/rust-pkg-dirs.txt 列出的所有包，含各自 Cargo.lock）。
# 版本一致性由 scripts/check-boundaries.sh 守护；手动改单个包会被 CI 拒绝。
# 用法：scripts/set-version.sh <semver>   例：scripts/set-version.sh 0.5.1
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ "$#" -ne 1 ]; then
  echo "用法: $0 <semver>   例: $0 0.5.1" >&2
  exit 1
fi
NEW="$1"
if ! printf '%s' "$NEW" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'; then
  echo "error: 版本号必须是 semver（MAJOR.MINOR.PATCH[-pre][+build]），得到: $NEW" >&2
  exit 1
fi

PKG_LIST="$ROOT/scripts/rust-pkg-dirs.txt"
[ -f "$PKG_LIST" ] || { echo "error: 缺少包列表 $PKG_LIST" >&2; exit 1; }

while IFS= read -r dir; do
  toml="$dir/Cargo.toml"
  if [ ! -f "$toml" ]; then
    echo "error: 缺少 $toml" >&2
    exit 1
  fi
  old="$(grep -m1 -E '^version = ' "$toml" | sed -E 's/^version = "(.*)"$/\1/' || true)"
  if [ "$old" = "$NEW" ]; then
    echo "[set-version] $toml 已是 $NEW"
  else
    sed -i "s/^version = \"[^\"]*\"/version = \"$NEW\"/" "$toml"
    echo "[set-version] $toml: ${old:-<未找到>} -> $NEW"
  fi
  # 同步该包自身在 Cargo.lock 中的版本（-w 仅动 workspace 成员，不碰依赖钉版本）
  (cd "$dir" && (cargo update -w --offline >/dev/null 2>&1 || cargo update -w >/dev/null)) \
    || echo "[set-version] 警告: $dir 的 Cargo.lock 未同步（稍后构建时会自动更新）" >&2
done < <(grep -v -e '^#' -e '^$' "$PKG_LIST")

echo "[set-version] 全部包版本已设为 $NEW；请检查 git diff 后提交"
