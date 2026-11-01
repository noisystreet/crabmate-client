#!/usr/bin/env bash
# 终端配色门禁：`tui_mode` 里的颜色只能来自 `Palette` 的语义槽。
#   crates/crabmate-tui/src/tui_mode/**/*.rs（除 palette.rs）不得出现
#   `Color::<Named>`（Black/Red/…/White/DarkGray/Light*) 与 `Color::Rgb(0x…)` 字面量
#   ——一律走 `pal.fg` / `pal.muted` / `Palette::tone_style` / `Palette::notice_style`。
#
# 理由：命名色既不受主题影响（切主题后那几处不变），也不受无色兜底约束
# ——`Palette::mono()` 只把 palette 的槽抹成 Reset，抹不掉调用点硬写的命名色，
# 于是 `NO_COLOR` / `TERM=dumb` 下这些位置仍然带色，与 palette.rs 头部的承诺相反。
#
# 有意例外在规则所在行（或紧上一行）写 `palette-gate: allow <理由>`；理由为空即失败。
# 已知盲区：`#[cfg(test)]` 之后的代码不扫（测试夹具允许用命名色构造对比样张），
# 所以测试里写死颜色不会被这条门禁拦住。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/tui_palette_usage_check.py" "$@"
