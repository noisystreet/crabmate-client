#!/usr/bin/env bash
# 跨包依赖边界的机械检查（AGENTS.md Hard Constraints 的可验证子集）：
#   1. crabmate-client-api 纯度：禁止 reqwest / tokio / tauri / web-sys / wasm-bindgen
#   2. crabmate-connect 默认 feature 不含 Tauri（tauri 必须为 optional 依赖）
#   3. 契约钉形状：crabmate = { version, default-features = false, features = ["protocol"] }，
#      全仓唯一形状、版本一致；禁止钉旧包名 crabmate-sse-protocol
#   4. 全部 Rust 包版本一致（以 scripts/rust-pkg-dirs.txt 首个包为基准）
# 包目录单一来源：scripts/rust-pkg-dirs.txt
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PKG_LIST="$ROOT/scripts/rust-pkg-dirs.txt"
if [ ! -f "$PKG_LIST" ]; then
  echo "error: 缺少包列表 $PKG_LIST" >&2
  exit 1
fi

fail() {
  echo "error: $1" >&2
  exit 1
}

# 1. crabmate-client-api 纯度（无 IO / 无壳依赖）
CLIENT_API_TOML="$ROOT/crates/crabmate-client-api/Cargo.toml"
[ -f "$CLIENT_API_TOML" ] || fail "缺少 $CLIENT_API_TOML"
if grep -Eq '^(reqwest|tokio|tauri|web-sys|wasm-bindgen)\s*=' "$CLIENT_API_TOML"; then
  echo "error: crabmate-client-api 引入了禁用依赖（reqwest/tokio/tauri/web-sys/wasm-bindgen）：" >&2
  grep -En '^(reqwest|tokio|tauri|web-sys|wasm-bindgen)\s*=' "$CLIENT_API_TOML" >&2
  echo "该 crate 是纯逻辑共享层，网络 / 异步运行时 / 壳绑定一律放在上层。" >&2
  exit 1
fi
echo "[check-boundaries] crabmate-client-api 纯度 ok"

# 2. crabmate-connect 默认 feature 不含 Tauri
CONNECT_TOML="$ROOT/crates/crabmate-connect/Cargo.toml"
[ -f "$CONNECT_TOML" ] || fail "缺少 $CONNECT_TOML"
grep -Eq '^default = \[\]' "$CONNECT_TOML" \
  || fail "crabmate-connect [features] 缺少 default = []（默认构建不得拉入 Tauri）"
grep -Eq '^tauri = .*optional = true' "$CONNECT_TOML" \
  || fail "crabmate-connect 的 tauri 依赖必须声明 optional = true（由壳经 feature = [\"tauri\"] 启用）"
echo "[check-boundaries] crabmate-connect 默认 feature ok"

# 3. 契约钉形状：全仓 Cargo.toml 中对 crates.io crabmate 的直接依赖必须命中同一形状
PIN_PATTERN='^crabmate = \{ version = "[^"]+", default-features = false, features = \["protocol"\] \}$'
pin_versions=""
while IFS= read -r toml; do
  [ -f "$toml" ] || continue
  while IFS= read -r line; do
    if ! printf '%s\n' "$line" | grep -Eq "$PIN_PATTERN"; then
      echo "error: 契约依赖未命中钉形状（crates.io crabmate + default-features = false + features = [\"protocol\"]）：" >&2
      echo "  $toml: $line" >&2
      echo "允许的形状：crabmate = { version = \"<semver>\", default-features = false, features = [\"protocol\"] }" >&2
      exit 1
    fi
    v="$(printf '%s\n' "$line" | sed -E 's/^crabmate = \{ version = "([^"]+)".*$/\1/')"
    if [ -n "$pin_versions" ] && [ "$v" != "$pin_versions" ]; then
      fail "crabmate 契约钉版本不一致：$pin_versions vs $v（$toml）"
    fi
    pin_versions="$v"
  done < <(grep '^crabmate = ' "$toml" || true)
done < <(git ls-files '*/Cargo.toml' 'Cargo.toml')
echo "[check-boundaries] 契约钉形状 ok（version = $pin_versions）"

if grep -Rn --include='Cargo.toml' 'crabmate-sse-protocol' . >/dev/null 2>&1; then
  echo "error: 发现旧包名 crabmate-sse-protocol（已改名 crates.io crabmate，禁止钉回）：" >&2
  grep -Rn --include='Cargo.toml' 'crabmate-sse-protocol' . >&2
  exit 1
fi

# 4. 全部 Rust 包版本一致（基准 = 列表首个包）
base_version=""
base_dir=""
while IFS= read -r dir; do
  toml="$dir/Cargo.toml"
  [ -f "$toml" ] || fail "包列表中的 $toml 不存在"
  v="$(grep -m1 -E '^version = ' "$toml" | sed -E 's/^version = "(.*)"$/\1/' || true)"
  if [ -z "$v" ]; then
    fail "$toml 未找到 [package] version 行（独立单包 workspace 必须自带版本）"
  fi
  if [ -z "$base_version" ]; then
    base_version="$v"
    base_dir="$dir"
  elif [ "$v" != "$base_version" ]; then
    fail "包版本不一致：$dir = $v，基准 $base_dir = $base_version（用 scripts/set-version.sh 统一改版）"
  fi
done < <(grep -v -e '^#' -e '^$' "$PKG_LIST")
echo "[check-boundaries] 版本一致性 ok（基准 $base_dir = $base_version）"

echo "[check-boundaries] ok"
