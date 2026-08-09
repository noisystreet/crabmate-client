#!/usr/bin/env bash
# Victauri E2E 一键脚本（crabmate-client；Linux headless：默认 xvfb-run）
#
# 用法: ./scripts/victauri-e2e.sh [test_binary|all|real_llm] [test_filter]
#
# serve 二进制解析（禁止 path 编译回主仓；仅定位已构建/已安装产物）：
#   1) CM_DESKTOP_BACKEND_BIN
#   2) PATH 中的 crabmate
#   3) 同级双轨 ../crabmate_agent/target/debug/crabmate（仅本地开发）
#
# 环境变量:
#   VICTAURI_USE_XVFB     1（默认）| 0 | auto
#   VICTAURI_PORT         Victauri MCP 端口（默认 7373）
#   VICTAURI_START_TIMEOUT  等待 /health 秒数（默认 120）
#   VICTAURI_MAIN_WINDOW_WAIT  health 后主窗口额外等待秒数（默认 20）
#   VICTAURI_E2E_LOG      桌面应用日志路径（默认 /tmp/crabmate-desktop-e2e.log）
#   CM_E2E_FIXTURES       默认 1（跳过连接页；隐藏窗口除非 CM_E2E_SHOW_WINDOWS / xvfb）
#   CM_E2E_SHOW_WINDOWS   默认 1（映射 WebView，否则 bridge 常失败）
#   CM_DESKTOP_SERVE_PORT 本脚本拉起的 serve 端口（默认 18080）
#   CM_DESKTOP_SERVE_URL  可覆盖（默认 http://127.0.0.1:$CM_DESKTOP_SERVE_PORT/）
#   CRABMATE_FRONTEND_DIST  可选：同步进 desktop dist（否则仅 connect/splash）
#   REAL_LLM_E2E          仅 real_llm 套件需要
#   VICTAURI_INSIDE_XVFB  内部：已由 xvfb-run 重入，勿手动设置

set -euo pipefail

# 保留原始参数（exec xvfb-run 重入时传递）
E2E_SCRIPT_ARGS=("$@")

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAURI_DIR="$ROOT/desktop-tauri/src-tauri"
DESKTOP_ROOT="$ROOT/desktop-tauri"

resolve_backend_bin() {
    if [[ -n "${CM_DESKTOP_BACKEND_BIN:-}" ]]; then
        echo "${CM_DESKTOP_BACKEND_BIN}"
        return 0
    fi
    if command -v crabmate >/dev/null 2>&1; then
        command -v crabmate
        return 0
    fi
    local sibling="${ROOT}/../crabmate_agent/target/debug/crabmate"
    if [[ -x "${sibling}" ]]; then
        echo "${sibling}"
        return 0
    fi
    return 1
}

if ! BACKEND_BIN="$(resolve_backend_bin)"; then
    echo "error: 找不到 crabmate 二进制。请设置 CM_DESKTOP_BACKEND_BIN、安装到 PATH，或先在同级主仓 cargo build -p crabmate" >&2
    exit 1
fi
# ^ 测试编排用：脚本**自行**拉起 serve（壳不再 spawn；非 sidecar）
DESKTOP_BIN="$TAURI_DIR/target/debug/crabmate-desktop"
SERVE_PORT="${CM_DESKTOP_SERVE_PORT:-18080}"
SERVE_URL="${CM_DESKTOP_SERVE_URL:-http://127.0.0.1:${SERVE_PORT}/}"
SERVE_LOG="${VICTAURI_SERVE_LOG:-/tmp/crabmate-serve-e2e.log}"

TEST="${1:-all}"
TEST_FILTER="${2:-}"
REAL_LLM="${REAL_LLM_E2E:-}"
VICTAURI_PORT="${VICTAURI_PORT:-7373}"
VICTAURI_START_TIMEOUT="${VICTAURI_START_TIMEOUT:-120}"
VICTAURI_MAIN_WINDOW_WAIT="${VICTAURI_MAIN_WINDOW_WAIT:-20}"
VICTAURI_E2E_LOG="${VICTAURI_E2E_LOG:-/tmp/crabmate-desktop-e2e.log}"

should_use_xvfb() {
    case "${VICTAURI_USE_XVFB:-1}" in
        1 | true | yes) return 0 ;;
        0 | false | no) return 1 ;;
        auto)
            if [ -z "${DISPLAY:-}" ] || [ "${CI:-}" = "true" ] || [ "${GITHUB_ACTIONS:-}" = "true" ]; then
                return 0
            fi
            return 1
            ;;
        *)
            echo "unknown VICTAURI_USE_XVFB=${VICTAURI_USE_XVFB} (use 1|0|auto)" >&2
            exit 1
            ;;
    esac
}

# 在 Wayland 桌面上：必须 exec xvfb-run 重跑整个脚本，否则 Tauri 仍会弹到本机
maybe_reexec_under_xvfb() {
    if ! should_use_xvfb; then
        return 0
    fi
    if [ -n "${VICTAURI_INSIDE_XVFB:-}" ]; then
        return 0
    fi
    if ! command -v xvfb-run >/dev/null 2>&1; then
        echo "xvfb-run not found; install package xvfb (e.g. apt install xvfb)" >&2
        exit 1
    fi
    echo ">>> Relaunching under xvfb-run (windows shown on virtual display for JS bridge) ..." >&2
    exec env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
        WINIT_UNIX_BACKEND=x11 \
        LIBGL_ALWAYS_SOFTWARE=1 \
        xvfb-run --auto-servernum --server-args='-screen 0 1280x720x24' \
        env VICTAURI_INSIDE_XVFB=1 \
        WINIT_UNIX_BACKEND=x11 \
        LIBGL_ALWAYS_SOFTWARE=1 \
        CM_E2E_FIXTURES="${CM_E2E_FIXTURES:-1}" \
        CM_E2E_SHOW_WINDOWS="${CM_E2E_SHOW_WINDOWS:-1}" \
        CM_DESKTOP_BACKEND_BIN="${CM_DESKTOP_BACKEND_BIN:-$BACKEND_BIN}" \
        CM_DESKTOP_SERVE_PORT="${SERVE_PORT}" \
        CM_DESKTOP_SERVE_URL="${SERVE_URL}" \
        REAL_LLM_E2E="${REAL_LLM_E2E:-}" \
        VICTAURI_PORT="${VICTAURI_PORT}" \
        VICTAURI_START_TIMEOUT="${VICTAURI_START_TIMEOUT}" \
        VICTAURI_MAIN_WINDOW_WAIT="${VICTAURI_MAIN_WINDOW_WAIT}" \
        VICTAURI_E2E_LOG="${VICTAURI_E2E_LOG}" \
        bash "$0" "${E2E_SCRIPT_ARGS[@]}"
}

maybe_reexec_under_xvfb

wait_http_health() {
    local url="$1"
    local label="$2"
    local timeout="$3"
    for i in $(seq 1 "$timeout"); do
        if curl --noproxy '*' --connect-timeout 1 --max-time 2 -sf "$url" >/dev/null 2>&1; then
            echo "   ${label} OK after ${i}s"
            return 0
        fi
        if [ "$i" -eq "$timeout" ]; then
            echo "   FAILED: ${label} not healthy within ${timeout}s ($url)" >&2
            return 1
        fi
        sleep 1
    done
}

# 独立启动 serve（壳不再 spawn）。Phase 2：壳加载包内 UI，serve 默认纯 API。
# 须 CORS 放行 http://tauri.localhost（桌面 WebView Origin）。
start_serve_background() {
    : >"$SERVE_LOG"
    local serve_cwd="${ROOT}"
    if [[ -d "${ROOT}/../crabmate_agent" ]]; then
      serve_cwd="${ROOT}/../crabmate_agent"
    fi
    (
      cd "${serve_cwd}"
      exec env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
        no_proxy=127.0.0.1,localhost \
        CM_E2E_FIXTURES="${CM_E2E_FIXTURES:-1}" \
        CM_WEB_CORS_ALLOWED_ORIGINS="${CM_WEB_CORS_ALLOWED_ORIGINS:-http://tauri.localhost}" \
        "$BACKEND_BIN" serve --host 127.0.0.1 --port "$SERVE_PORT"
    ) >>"$SERVE_LOG" 2>&1 &
    echo $!
}

# 启动桌面端：剥离 Wayland。xvfb 内显示窗口（WebKit 隐藏窗不跑 JS → bridge 失效）。
start_desktop_background() {
    if [ -n "${VICTAURI_INSIDE_XVFB:-}" ]; then
        echo "   display: ${DISPLAY:-<xvfb>} + visible windows (bridge needs mapped WebView)" >&2
    else
        echo "   display: ${DISPLAY:-<unset>} + CM_E2E_SHOW_WINDOWS (prefer xvfb via VICTAURI_USE_XVFB=1)" >&2
    fi
    env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
        WINIT_UNIX_BACKEND=x11 \
        LIBGL_ALWAYS_SOFTWARE=1 \
        CM_E2E_FIXTURES="${CM_E2E_FIXTURES:-1}" \
        CM_E2E_SHOW_WINDOWS="${CM_E2E_SHOW_WINDOWS:-1}" \
        CM_DESKTOP_SERVE_URL="$SERVE_URL" \
        "$DESKTOP_BIN" >>"$VICTAURI_E2E_LOG" 2>&1 &
    echo $!
}

wait_for_victauri_health() {
    local pid="$1"
    for i in $(seq 1 "$VICTAURI_START_TIMEOUT"); do
        if curl --noproxy '*' --connect-timeout 1 --max-time 2 -sf \
            "http://127.0.0.1:${VICTAURI_PORT}/health" >/dev/null 2>&1; then
            echo "   Victauri /health OK after ${i}s"
            return 0
        fi
        if ! kill -0 "$pid" 2>/dev/null; then
            if pgrep -f "$DESKTOP_BIN" >/dev/null 2>&1; then
                sleep 1
                continue
            fi
            echo "   FAILED: desktop process exited before Victauri ready"
            echo "   --- last 40 lines of ${VICTAURI_E2E_LOG} ---"
            tail -40 "$VICTAURI_E2E_LOG" 2>/dev/null || true
            echo "   --- last 40 lines of ${SERVE_LOG} ---"
            tail -40 "$SERVE_LOG" 2>/dev/null || true
            return 1
        fi
        if [ "$i" -eq "$VICTAURI_START_TIMEOUT" ]; then
            echo "   FAILED: Victauri not healthy within ${VICTAURI_START_TIMEOUT}s"
            echo "   --- last 40 lines of ${VICTAURI_E2E_LOG} ---"
            tail -40 "$VICTAURI_E2E_LOG" 2>/dev/null || true
            return 1
        fi
        sleep 1
    done
}

echo "=== Victauri E2E ==="
echo "  test: $TEST"
echo "  real_llm: ${REAL_LLM:-no}"
echo "  xvfb: ${VICTAURI_USE_XVFB:-1}$([ -n "${VICTAURI_INSIDE_XVFB:-}" ] && echo ' (inside xvfb-run)')"
echo "  port: $VICTAURI_PORT"
echo "  serve: $SERVE_URL"

# ── Phase 1: Build desktop shell（serve 须已是外部二进制）────────
echo ""
echo ">>> Using serve binary: $BACKEND_BIN"
if [ ! -x "$BACKEND_BIN" ]; then
    echo "error: not executable: $BACKEND_BIN" >&2
    exit 1
fi

echo ">>> Preparing shell assets + building desktop ..."
cd "$ROOT"
# 可选 UI 产物：显式 CRABMATE_FRONTEND_DIST，或本仓 frontend/dist
# （同级主仓需 CRABMATE_ALLOW_SIBLING_FRONTEND=1）
if [[ -z "${CRABMATE_FRONTEND_DIST:-}" || "${CRABMATE_FRONTEND_DIST}" == "-" ]]; then
  unset CRABMATE_FRONTEND_DIST
  if [[ -f "${ROOT}/frontend/dist/index.html" ]]; then
    export CRABMATE_FRONTEND_DIST="${ROOT}/frontend/dist"
  elif [[ "${CRABMATE_ALLOW_SIBLING_FRONTEND:-0}" == "1" || "${CRABMATE_ALLOW_SIBLING_FRONTEND:-}" == "true" || "${CRABMATE_ALLOW_SIBLING_FRONTEND:-}" == "yes" ]]; then
    if [[ -f "${ROOT}/../crabmate_agent/frontend/dist/index.html" ]]; then
      export CRABMATE_FRONTEND_DIST="${ROOT}/../crabmate_agent/frontend/dist"
    fi
  fi
fi
bash "$DESKTOP_ROOT/scripts/prepare-sidecar.sh"

# E2E 须 victauri:default（JS bridge invoke）；release/check 不能长期写进 capabilities
# shellcheck source=victauri-capability.sh
source "$ROOT/scripts/victauri-capability.sh"
ensure_victauri_capability "$TAURI_DIR"
trap 'restore_victauri_capability' EXIT

cd "$TAURI_DIR"
# 显式带 feature 构建壳二进制 + 测试（插件与 ACL 一致）
cargo build --features victauri --bin crabmate-desktop 2>&1 | tail -3
cargo build --tests --features victauri 2>&1 | tail -3
echo "   done."

# ── Phase 2: Kill old processes ─────────────────────────────
echo ""
echo ">>> Killing old processes ..."
pkill -9 -f 'src-tauri/target/debug/crabmate-desktop' 2>/dev/null || true
pkill -9 -f 'target/debug/crabmate-desktop' 2>/dev/null || true
pkill -9 -f "crabmate serve" 2>/dev/null || true
sleep 2
rm -rf /tmp/victauri/*/
echo "   done."

# ── Phase 3: Start serve, then desktop ──────────────────────
echo ""
echo ">>> Starting crabmate serve on ${SERVE_URL} ..."
SERVE_PID=$(start_serve_background) || exit 1
echo "   serve PID: $SERVE_PID"
echo "   serve log: $SERVE_LOG"
if ! wait_http_health "http://127.0.0.1:${SERVE_PORT}/health" "serve /health" 60; then
    echo "   --- last 40 lines of ${SERVE_LOG} ---"
    tail -40 "$SERVE_LOG" 2>/dev/null || true
    kill "$SERVE_PID" 2>/dev/null || true
    exit 1
fi

echo ""
echo ">>> Starting app (Tauri + WebView required for Victauri; xvfb keeps it off-screen) ..."
cd "$TAURI_DIR"
: >"$VICTAURI_E2E_LOG"
APP_PID=$(start_desktop_background) || exit 1
echo "   PID: $APP_PID"
echo "   log: $VICTAURI_E2E_LOG"

# ── Phase 4: Wait for Victauri health + main window ─────────
echo ">>> Waiting for Victauri server (http://127.0.0.1:${VICTAURI_PORT}/health) ..."
wait_for_victauri_health "$APP_PID"

echo ">>> Waiting for main window (page load, ${VICTAURI_MAIN_WINDOW_WAIT}s) ..."
sleep "$VICTAURI_MAIN_WINDOW_WAIT"

# Phase 2：业务 UI 在壳包内；serve 纯 API 时根路径 404 为预期。
echo ">>> Checking serve /health (API-only OK) ..."
if ! curl --noproxy '*' --connect-timeout 2 --max-time 5 -sf \
  "http://127.0.0.1:${SERVE_PORT}/health" >/dev/null; then
  echo "   WARN: serve /health failed after start; DOM bridge tests may fail" >&2
fi

# 确认桌面 dist 有业务 UI（prepare-sidecar 应已同步）。
if [[ ! -f "${ROOT}/desktop-tauri/dist/index.html" ]]; then
  echo "   WARN: desktop-tauri/dist/index.html missing; run make prepare-sidecar" >&2
fi

# ── Phase 5: Clean stale discovery dirs ────────────────────
for d in /tmp/victauri/*/port; do
    [ -e "$d" ] || continue
    port=$(cat "$d" 2>/dev/null || true)
    dir=$(dirname "$d")
    if [ "$port" != "$VICTAURI_PORT" ]; then
        rm -rf "$dir"
    fi
done

# ── Phase 6: Run tests ──────────────────────────────────────
echo ""
echo ">>> Running tests ..."
cd "$TAURI_DIR"
export VICTAURI_E2E=1
export CM_E2E_FIXTURES="${CM_E2E_FIXTURES:-1}"
export CM_DESKTOP_SERVE_URL="$SERVE_URL"
export VICTAURI_PORT
unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY

EXIT=0

find_test_bin() {
    local name="$1"
    find target/debug/deps -name "${name}-*" -not -name '*.d' 2>/dev/null | head -1
}

# 共用同一 WebView：套件内必须串行，否则 seed/reload 互踩。
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-1}"

if [ "$TEST" = "real_llm" ]; then
    export REAL_LLM_E2E=1
    BIN=$(find_test_bin victauri_real_llm)
    if [ -n "$BIN" ]; then
        "$BIN" --test-threads=1 || EXIT=$?
    else
        cargo test --features victauri --test victauri_real_llm -- --nocapture --test-threads=1 || EXIT=$?
    fi
elif [ "$TEST" = "all" ]; then
    cargo test --features victauri --no-fail-fast --no-run 2>/dev/null || true
    for name in victauri_e2e victauri_session_crud victauri_prefs_theme victauri_status_bar \
        victauri_settings victauri_settings2 victauri_keyboard victauri_conversation \
        victauri_user_data victauri_pagination victauri_visible_messages \
        victauri_sse_stub victauri_sse_more victauri_scroll_send victauri_ide_layout \
        victauri_two_turn victauri_turn_layout victauri_real_llm; do
        BIN=$(find_test_bin "$name")
        if [ -n "$BIN" ]; then
            echo ">>> $name"
            "$BIN" --test-threads=1 || EXIT=$?
        fi
    done
else
    cargo test --features victauri --test "$TEST" -- "$TEST_FILTER" --nocapture --test-threads=1 || EXIT=$?
fi

# ── Phase 7: Cleanup ────────────────────────────────────────
echo ""
echo ">>> Stopping app + serve ..."
kill "$APP_PID" 2>/dev/null || true
kill "$SERVE_PID" 2>/dev/null || true
pkill -f 'target/debug/crabmate-desktop' 2>/dev/null || true
pkill -f "crabmate serve --host 127.0.0.1 --port ${SERVE_PORT}" 2>/dev/null || true
pkill -f "serve --host 127.0.0.1 --port ${SERVE_PORT}" 2>/dev/null || true
wait 2>/dev/null || true

echo ""
if [ "$EXIT" -eq 0 ]; then
    echo "=== ALL PASSED ==="
else
    echo "=== FAILED (exit code $EXIT) ==="
fi
exit "$EXIT"
