#!/usr/bin/env bash
# 跨包依赖 lockstep 门禁（#602；扫描范围 = scripts/rust-pkg-dirs.txt 里每个包的
# Cargo.toml + Cargo.lock）：
#   规则 A · 同一 crate 被多个包**直接声明**时，version req 必须字面相同。
#            契约 pin `crabmate` 不参与比较（byte-identical 由 check-boundaries.sh 守）。
#   规则 B · 同一 manifest 内对同一 crate 的声明不得自相矛盾（dependencies /
#            dev-dependencies / build-dependencies / target.'cfg(..)'.dependencies）。
#   报告 C · 直接依赖在 8 份 Cargo.lock 里的解析版本；同 compat 边界内 patch 不齐
#            只打 warning，**不阻断**（8 个包 target 不同，升级落点天然错开）。
#
# 判据不做「同 crate 同 major 全仓唯一」：Cargo 允许同 lock 内多 major 共存，
# 不同 lock 的传递依赖树随 target / feature 变化（实测 50+ 个 crate 天然不一致）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/dep_lockstep_check.py" "$@"
