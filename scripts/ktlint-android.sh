#!/usr/bin/env bash
# 手改 Android Kotlin（edu/crabmate）的 ktlint 检查；默认只 check，不改写。
# 用法：
#   bash scripts/ktlint-android.sh           # 检查
#   bash scripts/ktlint-android.sh --format  # 就地格式化
# 依赖：java；首次会下载钉死的 ktlint CLI 到缓存目录。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KTLINT_VERSION="${KTLINT_VERSION:-1.8.0}"
KT_DIR="${ROOT}/mobile-tauri/src-tauri/gen/android/app/src/main/java/edu/crabmate"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/crabmate-client/ktlint"
KTLINT_BIN="${CACHE_DIR}/ktlint-${KTLINT_VERSION}"
DOWNLOAD_URL="https://github.com/pinterest/ktlint/releases/download/${KTLINT_VERSION}/ktlint"

FORMAT=0
if [[ "${1:-}" == "--format" || "${1:-}" == "-F" ]]; then
  FORMAT=1
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--format|-F]" >&2
  exit 2
fi

if [[ ! -d "${KT_DIR}" ]]; then
  echo "ktlint-android: missing ${KT_DIR}" >&2
  exit 1
fi

if ! command -v java >/dev/null 2>&1; then
  echo "ktlint-android: 需要 java（OpenJDK 17+）以运行 ktlint" >&2
  exit 1
fi

ensure_ktlint() {
  if [[ -x "${KTLINT_BIN}" ]]; then
    return 0
  fi
  mkdir -p "${CACHE_DIR}"
  local tmp
  tmp="$(mktemp "${CACHE_DIR}/ktlint.download.XXXXXX")"
  echo "ktlint-android: downloading ktlint ${KTLINT_VERSION} …"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${DOWNLOAD_URL}" -o "${tmp}"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "${tmp}" "${DOWNLOAD_URL}"
  else
    echo "ktlint-android: 需要 curl 或 wget 下载 ktlint" >&2
    rm -f "${tmp}"
    exit 1
  fi
  chmod +x "${tmp}"
  mv -f "${tmp}" "${KTLINT_BIN}"
}

ensure_ktlint

ANDROID_ROOT="${ROOT}/mobile-tauri/src-tauri/gen/android"
# 仅业务包；勿扫 Tauri buildSrc / 根 gradle .kts（相对 ANDROID_ROOT）
cd "${ANDROID_ROOT}"
# 排除 Tauri 生成的 edu/crabmate/generated/**（含通配 import 等，不宜手改）
mapfile -t KT_FILES < <(
  find app/src/main/java/edu/crabmate -type f -name '*.kt' ! -path '*/generated/*' | sort
)
if [[ ${#KT_FILES[@]} -eq 0 ]]; then
  echo "ktlint-android: no .kt under edu/crabmate" >&2
  exit 1
fi

args=(--relative)
if [[ "${FORMAT}" -eq 1 ]]; then
  args+=(--format)
fi

echo "ktlint-android: ktlint ${KTLINT_VERSION} on ${#KT_FILES[@]} file(s) under edu/crabmate"
"${KTLINT_BIN}" "${args[@]}" "${KT_FILES[@]}"
