#!/usr/bin/env bash
# 工具链版本单一来源（rust-toolchain.toml 的 channel）：每个 workflow 里的
# dtolnay/rust-toolchain 都必须钉到同一个版本，不许 stable / beta / nightly 浮动。
#
# 理由：pre-commit 的 clippy 钩子与 CI 的 check job 跑的是同一条命令，
# 但浮动工具链下两边不是同一个编译器，结论可能相反 —— clippy 1.100 给
# needless_bool 加了 `if cond { return true; } false` 这一分支，本地 1.98 全绿，
# CI 在 `-D warnings` 下失败，表现为「pre-commit 过了，PR 依然红」。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PIN_FILE="rust-toolchain.toml"
if [[ ! -f "$PIN_FILE" ]]; then
  echo "error: 缺少 $PIN_FILE（工具链版本单一来源）" >&2
  exit 1
fi

PIN="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$PIN_FILE" | head -n 1)"
if [[ -z "$PIN" ]]; then
  echo "error: $PIN_FILE 里找不到 channel = \"x.y.z\"" >&2
  exit 1
fi

hits=0
fail=0

settle() {
  local file="$1" ref="$2" at_line="$3" toolchain="$4"
  hits=$((hits + 1))
  case "$ref" in
    master)
      if [[ "$toolchain" != "$PIN" ]]; then
        echo "error: $file:$at_line dtolnay/rust-toolchain@master 必须带 with: toolchain: \"$PIN\"（当前：${toolchain:-未设置}）" >&2
        fail=1
      fi
      ;;
    stable | beta | nightly)
      echo "error: $file:$at_line 用了浮动的 dtolnay/rust-toolchain@$ref；本仓要求 @master + with: toolchain: \"$PIN\"" >&2
      fail=1
      ;;
    *)
      if [[ "$ref" != "$PIN" ]]; then
        echo "error: $file:$at_line dtolnay/rust-toolchain@$ref 与 $PIN_FILE 的 $PIN 不一致" >&2
        fail=1
      fi
      ;;
  esac
}

scan_file() {
  local file="$1" line=0 ref="" at_line=0 toolchain="" text=""
  while IFS= read -r raw || [[ -n "$raw" ]]; do
    line=$((line + 1))
    # 去掉行尾注释，避免 `- uses: ... # note` 这类写法干扰匹配
    text="${raw%%#*}"
    if [[ "$text" =~ uses:[[:space:]]*([^[:space:]]+) ]]; then
      if [[ -n "$ref" ]]; then settle "$file" "$ref" "$at_line" "$toolchain"; fi
      ref=""
      at_line=0
      toolchain=""
      if [[ "${BASH_REMATCH[1]}" == dtolnay/rust-toolchain@* ]]; then
        ref="${BASH_REMATCH[1]#dtolnay/rust-toolchain@}"
        at_line="$line"
      fi
      continue
    fi
    if [[ -n "$ref" && "$text" =~ ^[[:space:]]*toolchain:[[:space:]]*(.*[^[:space:]])[[:space:]]*$ ]]; then
      toolchain="${BASH_REMATCH[1]//\"/}"
    fi
  done < "$file"
  if [[ -n "$ref" ]]; then settle "$file" "$ref" "$at_line" "$toolchain"; fi
}

for f in .github/workflows/*.yml; do
  [[ -f "$f" ]] || continue
  scan_file "$f"
done

if [[ "$hits" -eq 0 ]]; then
  echo "error: .github/workflows/*.yml 里没找到 dtolnay/rust-toolchain 引用（守卫失效？）" >&2
  exit 1
fi

if [[ "$fail" -ne 0 ]]; then
  echo "改为：- uses: dtolnay/rust-toolchain@master + with: toolchain: \"$PIN\"；" >&2
  echo "升级时同时改 $PIN_FILE 的 channel（两处必须一致）。" >&2
  exit 1
fi

echo "[check-toolchain-pin] ok (rust $PIN; $hits 处 workflow 引用)"
