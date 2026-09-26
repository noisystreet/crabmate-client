#!/usr/bin/env bash
# 响应式断点单源校验：权威值 = frontend/src/app_prefs.rs 的 MOBILE_LAYOUT_BREAKPOINT_PX。
#
# CSS 规范不允许在 @media 条件里引用自定义属性（var() 不能用于媒体特性），
# Trunk 也不做 CSS 预处理，因此断点值在样式层只能"受约束地重复"。
# 本脚本把这份重复变成可机械校验的一致性约束：
#   1. 主断点成对互补：窄屏 `@media (max-width: N)`，宽屏 `@media (min-width: N+1)`；禁止交叉书写
#      （`max-width: N+1` 与主断点重叠，`min-width: N` 会让 N 自身归属歧义）；
#   2. 其余断点语义独立（如设置页的 900px），须在下方 EXTRA_BREAKPOINTS 显式登记；
#   3. e2e 声明的移动视口须落在窄屏侧，否则用例名不副实。
#
# 改主断点须同时改 frontend/src/app_prefs.rs —— 该常量同时驱动 matchMedia 查询与
# DOM `data-narrow-viewport` 标记（见 frontend/src/app/app_shell_effects/viewport.rs）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PREFS="frontend/src/app_prefs.rs"
CSS_DIRS=(frontend/styles frontend/themes)
E2E_MOBILE_SPEC="e2e/specs/mock-mobile-shell.spec.ts"

# 已登记的二级断点：与主断点语义独立，登记时须一并写明用途。
#   900 — frontend/styles/modal.css：设置页（.settings-layout）由双列转单列
EXTRA_BREAKPOINTS=(900)

bp="$(sed -n 's/^pub const MOBILE_LAYOUT_BREAKPOINT_PX: u32 = \([0-9][0-9]*\);.*$/\1/p' "$PREFS" | head -n 1)"
if [[ -z "${bp}" ]]; then
  echo "error: 未能在 ${PREFS} 解析 MOBILE_LAYOUT_BREAKPOINT_PX" >&2
  exit 1
fi
narrow_max="${bp}"
narrow_min=$((bp + 1))

is_extra() {
  local p="$1" e
  for e in "${EXTRA_BREAKPOINTS[@]}"; do
    [[ "${p}" == "${e}" ]] && return 0
  done
  return 1
}

problems=""
scanned=0
for dir in "${CSS_DIRS[@]}"; do
  [[ -d "${dir}" ]] || continue
  for f in "${dir}"/*.css; do
    [[ -f "${f}" ]] || continue
    while IFS= read -r m; do
      [[ -z "${m}" ]] && continue
      lineno="${m%%:*}"
      sel="${m#*:}"
      axis="$(sed -E 's/.*\(([a-z]+)-width.*/\1/' <<<"${sel}")"
      px="$(sed -E 's/.*-width:[[:space:]]*([0-9]+)px.*/\1/' <<<"${sel}")"
      scanned=$((scanned + 1))
      # 已登记的二级断点：方向自定
      if is_extra "${px}"; then continue; fi
      # 主断点：max-width:N 与 min-width:N+1 互补成对
      if [[ "${px}" == "${narrow_max}" && "${axis}" == "max" ]]; then continue; fi
      if [[ "${px}" == "${narrow_min}" && "${axis}" == "min" ]]; then continue; fi
      problems+="${f}:${lineno}: (${axis}-width: ${px}px)"$'\n'
    done < <(grep -n -o -E '@media[^{]*\((max|min)-width:[[:space:]]*[0-9]+px' "${f}" || true)
  done
done

if [[ -f "${E2E_MOBILE_SPEC}" ]]; then
  vp_w="$(sed -n 's/^const MOBILE_VIEWPORT = { width: \([0-9][0-9]*\).*$/\1/p' "${E2E_MOBILE_SPEC}" | head -n 1)"
  if [[ -n "${vp_w}" && "${vp_w}" -gt "${narrow_max}" ]]; then
    problems+="${E2E_MOBILE_SPEC}: MOBILE_VIEWPORT.width = ${vp_w} 未落在窄屏侧（> ${narrow_max}）"$'\n'
  fi
fi

if [[ -n "${problems}" ]]; then
  echo "error: 响应式断点与单源不一致（权威值 MOBILE_LAYOUT_BREAKPOINT_PX = ${narrow_max}）：" >&2
  echo "${problems}" >&2
  echo "" >&2
  echo "约定：窄屏 @media (max-width: ${narrow_max}px)（在 ${PREFS} 定义）；" >&2
  echo "      互补 @media (min-width: ${narrow_min}px)；二者不可交叉书写（会重叠）。" >&2
  echo "      其他断点须在 scripts/check-css-breakpoints.sh 的 EXTRA_BREAKPOINTS 登记并写明用途。" >&2
  exit 1
fi

echo "[check-css-breakpoints] ok (窄屏 max-width: ${narrow_max}px / 互补 min-width: ${narrow_min}px；二级断点 ${EXTRA_BREAKPOINTS[*]}；扫描 ${scanned} 处)"
