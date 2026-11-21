#!/usr/bin/env bash
# 桌面壳冒烟（#548 Wave 6）：真实 WebKitGTK + 真实 serve 能不能起来。
#
# 它替代被删掉的 Victauri `window_state_check`。那条断言靠 victauri 插件的
# `list_windows()` 反问壳「你现在有哪些窗口」；插件摘掉后壳不再对外提供窗口查询，
# 只能从外部观察。判据按强度排列：
#
#   1. 壳进程在观察窗内仍存活 —— 启动即崩、`setup` 返回 Err 都会在这里暴露；
#   2. 日志里既没有 `startup failed` 也没有 panic / `failed to ...`；
#   3. 壳的**子进程**里出现 WebKit（`WebKitWebProcess` / `WebKitNetworkProcess`）
#      —— WebView 引擎真的初始化并开始加载页面，而不只是「进程没死」。
#
# 为什么不查 X11 窗口：实测在无窗口管理器的 xvfb 里，壳的窗口到 43s 仍不在
# X server 的窗口树上（`xwininfo -root -tree` 报 0 children）—— 主窗口先以不可见
# 状态创建、页面加载完才 `show()`，而 debug 构建的 WASM 有 160+ MiB。把窗口当判据
# 只会得到一条稳定假红。WebKit 子进程在 8s 内就出现，才是可靠信号。
#
# 覆盖边界：这**不是**页面行为回归 —— 页面行为由 Playwright（`e2e/specs`）跑，
# 钥匙串由 `crates/crabmate-connect` 的单测跑。这里只回答一个问题：
# 「打完包的壳在本机能起来吗」。
#
# 用法：
#   ./scripts/smoke-desktop-shell.sh
#
# 环境变量：
#   CM_SMOKE_USE_XVFB      1（默认）| 0 | auto   默认在 xvfb 里跑（不打扰本机桌面）
#   CM_SMOKE_SETTLE_SECS   观察窗秒数（默认 30；拿到 WebKit 子进程即提前通过）
#   CM_SMOKE_SERVE_PORT    外部 serve 端口（默认 18080；已就绪则直接复用）
#   CM_SMOKE_SKIP_BUILD    1 = 不重新构建壳（要求二进制已存在）
#   CM_SMOKE_LOG_DIR       日志目录（默认 /tmp）
#   CM_DESKTOP_BACKEND_BIN serve 二进制路径（优先于 PATH / 同级仓）
#   CM_SMOKE_INSIDE_XVFB   内部：已由 xvfb-run 重入，勿手动设置

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="$ROOT/desktop-tauri/src-tauri"
DESKTOP_BIN="$TAURI_DIR/target/debug/crabmate-desktop"
PORT="${CM_SMOKE_SERVE_PORT:-18080}"
SERVE_URL="http://127.0.0.1:${PORT}/"
SETTLE="${CM_SMOKE_SETTLE_SECS:-30}"
LOG_DIR="${CM_SMOKE_LOG_DIR:-/tmp}"
DESKTOP_LOG="$LOG_DIR/crabmate-desktop-smoke.log"
SERVE_LOG="$LOG_DIR/crabmate-serve-smoke.log"

DESKTOP_PID=""
SERVE_PID=""
SERVE_REUSED=0

should_use_xvfb() {
    case "${CM_SMOKE_USE_XVFB:-1}" in
        1 | true | yes) return 0 ;;
        0 | false | no) return 1 ;;
        auto)
            if [ -z "${DISPLAY:-}" ] || [ "${CI:-}" = "true" ] || [ "${GITHUB_ACTIONS:-}" = "true" ]; then
                return 0
            fi
            return 1
            ;;
        *)
            echo "unknown CM_SMOKE_USE_XVFB=${CM_SMOKE_USE_XVFB} (use 1|0|auto)" >&2
            exit 1
            ;;
    esac
}

# Wayland 桌面上 Tauri 会弹到本机：整脚本 exec 进 xvfb 再跑一次。
maybe_reexec_under_xvfb() {
    should_use_xvfb || return 0
    [ -n "${CM_SMOKE_INSIDE_XVFB:-}" ] && return 0
    if ! command -v xvfb-run >/dev/null 2>&1; then
        echo "error: 找不到 xvfb-run（apt install xvfb），或设 CM_SMOKE_USE_XVFB=0 在本机桌面跑" >&2
        exit 1
    fi
    echo ">>> 在 xvfb-run 里重入（虚拟屏上开窗，不打扰本机桌面）..." >&2
    exec env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
        WINIT_UNIX_BACKEND=x11 \
        LIBGL_ALWAYS_SOFTWARE=1 \
        xvfb-run --auto-servernum --server-args='-screen 0 1280x720x24' \
        env CM_SMOKE_INSIDE_XVFB=1 \
        CM_SMOKE_USE_XVFB="${CM_SMOKE_USE_XVFB:-1}" \
        CM_SMOKE_SETTLE_SECS="$SETTLE" \
        CM_SMOKE_SERVE_PORT="$PORT" \
        bash "$0" "$@"
}

maybe_reexec_under_xvfb

cleanup() {
    local code=$?
    if [ -n "$DESKTOP_PID" ]; then
        kill "$DESKTOP_PID" 2>/dev/null || true
        wait "$DESKTOP_PID" 2>/dev/null || true
    fi
    # 只回收**自己**起的 serve：复用的那台（端口已就绪）是别人的，不能杀。
    if [ -n "$SERVE_PID" ]; then
        kill "$SERVE_PID" 2>/dev/null || true
        wait "$SERVE_PID" 2>/dev/null || true
    fi
    return "$code"
}
trap cleanup EXIT

resolve_serve_bin() {
    if [ -n "${CM_DESKTOP_BACKEND_BIN:-}" ]; then
        printf '%s' "$CM_DESKTOP_BACKEND_BIN"
        return
    fi
    if command -v crabmate >/dev/null 2>&1; then
        command -v crabmate
        return
    fi
    local cand
    for cand in \
        "$ROOT/../crabmate_agent/target/debug/crabmate" \
        "$ROOT/../crabmate_agent/target/release/crabmate" \
        "$ROOT/../CrabMate/target/debug/crabmate" \
        "$ROOT/../server/target/debug/crabmate" \
        "$ROOT/../server/target/release/crabmate"; do
        if [ -x "$cand" ]; then
            printf '%s' "$cand"
            return
        fi
    done
    printf ''
}

serve_healthy() {
    curl --noproxy '*' --connect-timeout 1 --max-time 2 -sf \
        "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1
}

wait_http_health() {
    local timeout="$1" i
    for i in $(seq 1 "$timeout"); do
        if serve_healthy; then
            echo "   serve /health OK after ${i}s"
            return 0
        fi
        sleep 1
    done
    echo "   FAILED: serve 在 ${timeout}s 内没就绪（http://127.0.0.1:${PORT}/health）" >&2
    return 1
}

dump_logs() {
    echo "   --- tail -40 ${DESKTOP_LOG} ---" >&2
    # 注意重定向顺序：先 2>/dev/null 再把 stdout 接到 stderr，会连 stdout 一起丢进
    # /dev/null（两者都指向 /dev/null）。失败时看不到日志 —— 正是老 scripts/victauri-e2e.sh
    # 踩过的「失败零诊断」坑。
    tail -40 "$DESKTOP_LOG" >&2 2>/dev/null || true
    if [ "$SERVE_REUSED" = "1" ]; then
        echo "   (serve 是复用的已运行实例，它的日志不归本脚本)" >&2
    else
        echo "   --- tail -40 ${SERVE_LOG} ---" >&2
        tail -40 "$SERVE_LOG" >&2 2>/dev/null || true
    fi
}

# 壳是否还活着。`kill -0` 对**僵尸**也返回成功（父进程尚未 wait），单用它会把
# 「已经崩了、只是还没被回收」判成存活，把诊断引向错误方向。
desktop_alive() {
    kill -0 "$DESKTOP_PID" 2>/dev/null || return 1
    local stat
    stat="$(ps -o stat= -p "$DESKTOP_PID" 2>/dev/null || true)"
    [ -n "$stat" ] || return 1
    case "$stat" in
        Z*) return 1 ;;
    esac
    return 0
}

# 壳的子进程里出现 WebKit 引擎 = WebView 初始化并开始加载页面。
webkit_child_present() {
    pgrep -P "$DESKTOP_PID" -a 2>/dev/null | grep -qi 'webkit'
}

echo "=== 桌面壳冒烟 ==="
echo "  serve: ${SERVE_URL}"
echo "  观察窗: ${SETTLE}s"
echo "  xvfb: ${CM_SMOKE_USE_XVFB:-1}$([ -n "${CM_SMOKE_INSIDE_XVFB:-}" ] && echo ' (inside xvfb-run)')"

# ── 前置：UI dist + serve 二进制 + 壳二进制 ──────────────────
if [ ! -f "$ROOT/frontend/dist/index.html" ]; then
    echo "error: 缺少 frontend/dist/index.html（壳加载包内 UI）。先跑 make frontend" >&2
    exit 1
fi

SERVE_BIN="$(resolve_serve_bin)"
if [ -z "$SERVE_BIN" ] || [ ! -x "$SERVE_BIN" ]; then
    echo "error: 找不到 crabmate serve 可执行文件（设 CM_DESKTOP_BACKEND_BIN / 放进 PATH / 同级克隆 Server 仓）" >&2
    exit 1
fi
echo ">>> serve 二进制：${SERVE_BIN}"

echo ">>> 准备壳静态资源 ..."
bash "$ROOT/desktop-tauri/scripts/prepare-sidecar.sh"

if [ "${CM_SMOKE_SKIP_BUILD:-0}" != "1" ]; then
    echo ">>> 构建壳（cargo build --bin crabmate-desktop）..."
    (cd "$TAURI_DIR" && cargo build --bin crabmate-desktop 2>&1 | tail -3)
fi
if [ ! -x "$DESKTOP_BIN" ]; then
    echo "error: 缺少壳二进制 ${DESKTOP_BIN}" >&2
    exit 1
fi

# ── serve：已就绪则复用，否则自己起 ────────────────────────
# 先清空桌面日志：serve 阶段就失败时，别把**上一轮**的壳日志当成本轮证据打出来。
: >"$DESKTOP_LOG"
if serve_healthy; then
    echo ">>> 复用已在监听的 serve（${PORT}）"
    SERVE_REUSED=1
else
    echo ">>> 启动 serve (${PORT}) ..."
    : >"$SERVE_LOG"
    (
        cd "$ROOT"
        # 本机可能已有 serve 占着同一工作区的会话库实例锁（用户自己的 8080/8081…）；
        # 冒烟只需要 /health + /user-data，绕开实例锁即可，不影响那台在跑的。
        exec env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
            no_proxy=127.0.0.1,localhost \
            CM_SERVE_DISABLE_INSTANCE_LOCK=1 \
            "$SERVE_BIN" serve --host 127.0.0.1 --port "$PORT"
    ) >>"$SERVE_LOG" 2>&1 &
    SERVE_PID=$!
    if ! wait_http_health 60; then
        dump_logs
        exit 1
    fi
fi

# ── 起壳：跳过连接页，直接开包内 UI ─────────────────────────
echo ">>> 启动壳（CM_DESKTOP_SKIP_CONNECT=1 → ${SERVE_URL}）..."
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
    WINIT_UNIX_BACKEND=x11 \
    LIBGL_ALWAYS_SOFTWARE=1 \
    CM_DESKTOP_SKIP_CONNECT=1 \
    CM_DESKTOP_SERVE_URL="$SERVE_URL" \
    "$DESKTOP_BIN" >>"$DESKTOP_LOG" 2>&1 &
DESKTOP_PID=$!
echo "   PID: $DESKTOP_PID"

# ── 观察窗：存活 + 无失败日志 + WebKit 子进程 ───────────────
echo ">>> 观察（最多 ${SETTLE}s，WebKit 子进程出现即通过）..."
elapsed_ms=0
webkit_seen=""
while [ "$elapsed_ms" -lt "$((SETTLE * 1000))" ]; do
    if ! desktop_alive; then
        echo "!!! 壳进程在 $((elapsed_ms / 1000))s 就退出了" >&2
        dump_logs
        exit 1
    fi
    if webkit_child_present; then
        webkit_seen="yes"
        break
    fi
    sleep 1
    elapsed_ms=$((elapsed_ms + 1000))
done

if grep -qE 'startup failed|panicked at|failed to (create|show|focus)' "$DESKTOP_LOG" 2>/dev/null; then
    echo "!!! 日志里出现启动失败 / panic / 窗口创建或显示失败" >&2
    dump_logs
    exit 1
fi

if [ -z "$webkit_seen" ]; then
    echo "!!! 壳进程活着，但 ${SETTLE}s 内没出现 WebKit 子进程（WebView 没起来）" >&2
    dump_logs
    exit 1
fi

echo "   OK: 进程存活 + 日志干净 + WebKit 子进程已起"
echo ""
echo "=== 壳冒烟通过（日志：${DESKTOP_LOG}）==="
