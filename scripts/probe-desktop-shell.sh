#!/usr/bin/env bash
# 桌面壳自检探针（无头）：xvfb 里跑**真 WebKitGTK + 真壳**，把 wasm panic 与 DOM 状态抓回本地。
#
# 为什么需要它：Chromium / Playwright 全绿的用例覆盖不到 owner 生命周期、微任务时序这类
# 引擎差异 —— 而真桌面上「点了没反应」往往正是那些差异造成的 wasm panic（wasm32 上 panic 是
# trap，panic 之后 `Drop` 不跑、`spawn_local` 链一起死，界面就只剩一张画）。排查方法见
# `docs/design/desktop_shell_debugging.md`。
#
# 它做的事（与那份文档 §2 一一对应）：
#   1. 起一台**独立** serve（自带已知 Bearer，不碰用户自己的实例锁）；
#   2. 起一个本地收集端（python3 http，页面 POST 过来）；
#   3. 把探针脚本内联进 `desktop-tauri/dist/index.html`，`touch build.rs` 重编壳（资产是内嵌的，
#      不重编就不生效）；
#   4. xvfb 里启动壳，用独立 `XDG_RUNTIME_DIR` 绕开单实例插件（否则第二个进程静默 exit 0）；
#   5. 观察窗口结束 → 打印 panic 条数与探针日志 → 还原 dist。
#
# 用法：
#   bash scripts/probe-desktop-shell.sh                          # 最小探针（只报 panic / 存活 / slots）
#   bash scripts/probe-desktop-shell.sh --probe /tmp/p.js        # 换成自己的交互探针
#   bash scripts/probe-desktop-shell.sh --with-ctl --settle 60   # 左栏「可连实例」用真 ctl
#
# 参数（也可用同名环境变量）：
#   --probe <file.js>   探针脚本（默认内置最小探针：panic + 存活 + lane 计数）
#   --settle <secs>     观察窗口，默认 45（CM_PROBE_SETTLE_SECS）
#   --serve-port <p>    独立 serve 端口，默认 18080（CM_PROBE_SERVE_PORT）
#   --probe-port <p>    收集端端口，默认 4177（CM_PROBE_COLLECT_PORT）
#   --bearer <token>    两端共用的 Bearer，默认 probe-token（CM_PROBE_BEARER）
#   --with-ctl          把真实 `$XDG_RUNTIME_DIR/crabmate` 链进独立运行时目录（可连实例才是真的）
#   --keep              保留工作目录（探针 / serve 日志 / shell 日志 / XDG 目录）
#   --skip-build        不重编壳（探针没变时才可复用）
#   --skip-ui           不重编业务 UI（`make frontend`；连续复跑时省时间）
#   --no-restore        跑完不还原 `desktop-tauri/dist`
#   --help
#
# 退出码：探针日志里出现 panic（`CONSOLE-ERR`）→ 1；否则 0。日志路径会打印出来。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

SERVE_PORT="${CM_PROBE_SERVE_PORT:-18080}"
PROBE_PORT="${CM_PROBE_COLLECT_PORT:-4177}"
SETTLE="${CM_PROBE_SETTLE_SECS:-45}"
BEARER="${CM_PROBE_BEARER:-probe-token}"
PROBE_JS=""
WITH_CTL=0
KEEP=0
SKIP_BUILD=0
RESTORE=1
# 默认先构建业务 UI：`desktop-tauri/dist` 的来源是 `frontend/dist`，后者不重编就会拿旧 wasm
# 跑出「假结论」（本次踩过：改了前端却读到旧行为）。`--skip-ui` 供连续复跑时省那 6 秒。
SKIP_UI=0

WORK=""
SERVE_PID=""
COLLECT_PID=""
SHELL_PID=""

log() { echo ">>> $*"; }
die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    sed -n '3,40p' "$0" | sed -e 's/^# \{0,1\}//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
    --probe)
        PROBE_JS="${2:?--probe 需要文件路径}"
        shift 2
        ;;
    --settle)
        SETTLE="${2:?--settle 需要秒数}"
        shift 2
        ;;
    --serve-port)
        SERVE_PORT="${2:?--serve-port 需要端口}"
        shift 2
        ;;
    --probe-port)
        PROBE_PORT="${2:?--probe-port 需要端口}"
        shift 2
        ;;
    --bearer)
        BEARER="${2:?--bearer 需要值}"
        shift 2
        ;;
    --with-ctl) WITH_CTL=1 && shift ;;
    --keep) KEEP=1 && shift ;;
    --skip-build) SKIP_BUILD=1 && shift ;;
    --skip-ui) SKIP_UI=1 && shift ;;
    --no-restore) RESTORE=0 && shift ;;
    -h | --help) usage ;;
    *) die "未知参数：$1（--help 看用法）" ;;
    esac
done

restore_dist() {
    [[ "${RESTORE}" -eq 1 ]] || return 0
    bash "${ROOT}/desktop-tauri/scripts/prepare-sidecar.sh" >/dev/null 2>&1 || true
}

cleanup() {
    set +e
    [[ -n "${SHELL_PID}" ]] && kill -TERM "-${SHELL_PID}" 2>/dev/null
    [[ -n "${COLLECT_PID}" ]] && kill -TERM "${COLLECT_PID}" 2>/dev/null
    [[ -n "${SERVE_PID}" ]] && kill -TERM "${SERVE_PID}" 2>/dev/null
    restore_dist
    if [[ -n "${WORK}" && "${KEEP}" -eq 0 ]]; then
        rm -rf "${WORK}"
    elif [[ -n "${WORK}" ]]; then
        log "工作目录保留：${WORK}"
    fi
}
trap cleanup EXIT

port_busy() {
    # 只看本机监听：能连上就算占用（避免依赖 ss / lsof）。
    (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3>&-; return 0; }
    return 1
}

wait_http() {
    local url="$1" secs="$2" i
    for ((i = 0; i < secs; i++)); do
        if curl -s -m 2 -o /dev/null "$url" 2>/dev/null; then return 0; fi
        sleep 1
    done
    return 1
}

resolve_serve_bin() {
    if [[ -n "${CM_DESKTOP_BACKEND_BIN:-}" ]]; then
        printf '%s' "${CM_DESKTOP_BACKEND_BIN}"
        return
    fi
    if command -v crabmate >/dev/null 2>&1; then
        command -v crabmate
        return
    fi
    printf ''
}

# ── 前置 ────────────────────────────────────────────────────
command -v xvfb-run >/dev/null 2>&1 || die "找不到 xvfb-run（apt install xvfb）"
command -v python3 >/dev/null 2>&1 || die "找不到 python3（收集端与 dist 注入要用）"
command -v curl >/dev/null 2>&1 || die "找不到 curl（探活用）"
[[ -f "${ROOT}/frontend/dist/index.html" ]] || die "缺少 frontend/dist/index.html，先跑 make frontend"
[[ -z "${PROBE_JS}" || -f "${PROBE_JS}" ]] || die "探针文件不存在：${PROBE_JS}"

SERVE_BIN="$(resolve_serve_bin)"
[[ -n "${SERVE_BIN}" && -x "${SERVE_BIN}" ]] || die "找不到 crabmate serve（设 CM_DESKTOP_BACKEND_BIN 或放进 PATH）"

DESKTOP_BIN="${ROOT}/desktop-tauri/src-tauri/target/debug/crabmate-desktop"
port_busy "${SERVE_PORT}" && die "端口 ${SERVE_PORT} 已被占用（换 --serve-port，或先停掉那台 serve）"
port_busy "${PROBE_PORT}" && die "端口 ${PROBE_PORT} 已被占用（换 --probe-port）"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/cm-probe-XXXXXX")"
mkdir -p "${WORK}/rt" "${WORK}/data" "${WORK}/cache"
chmod 700 "${WORK}/rt"
if [[ "${WITH_CTL}" -eq 1 ]]; then
    # 单实例插件靠 XDG_RUNTIME_DIR 查重，所以这里必须是独立目录；ctl socket 用软链接回真实例。
    if [[ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/crabmate/ctl.sock" ]]; then
        ln -sfn "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/crabmate" "${WORK}/rt/crabmate"
    else
        log "未找到 ctl socket，--with-ctl 忽略（左栏不会有「可连实例」）"
    fi
fi

# ── 探针脚本（默认：只报 panic / 存活 / lane 计数）────────────
cat >"${WORK}/probe.js" <<'PROBE_HEAD'
(function () {
  var LOG = "http://127.0.0.1:__PROBE_PORT__/r";
  function R(m) {
    try {
      fetch(LOG, { method: "POST", mode: "no-cors", body: String(m).slice(0, 2400) });
    } catch (e) {}
  }
  // initialization_script / 内联注入都可能重复执行，探针必须幂等。
  if (window.__CM_PROBE__) return;
  window.__CM_PROBE__ = true;

  window.addEventListener("error", function (e) {
    R("ERROR " + (e.message || "") + " @" + (e.filename || "") + ":" + (e.lineno || ""));
  });
  window.addEventListener("unhandledrejection", function (e) {
    var r = e && e.reason;
    R("REJECT " + String((r && r.message) || r).slice(0, 600));
  });
  var _ce = console.error;
  console.error = function () {
    try {
      R(
        "CONSOLE-ERR " +
          Array.prototype.map
            .call(arguments, function (a) {
              return String((a && a.stack) || a).slice(0, 900);
            })
            .join(" "),
      );
    } catch (e) {}
    return _ce.apply(console, arguments);
  };

  var t0 = Date.now();
  (function tick() {
    var active = document.querySelector(".lane-slot--active");
    R(
      "ALIVE slots=" +
        document.querySelectorAll(".lane-slot").length +
        " activeLane=" +
        (active ? active.getAttribute("data-lane") : "none") +
        " composer=" +
        !!document.querySelector('[data-testid="chat-composer-input"]') +
        " t=" +
        Math.round((Date.now() - t0) / 1000) +
        "s",
    );
    if (Date.now() - t0 < 90000) setTimeout(tick, 10000);
  })();
})();
PROBE_HEAD

if [[ -n "${PROBE_JS}" ]]; then
    log "使用外部探针：${PROBE_JS}"
    cp "${PROBE_JS}" "${WORK}/probe.js"
fi
# 收集端端口要写进探针（先替换占位，再注入 dist）
python3 - "${WORK}/probe.js" "${PROBE_PORT}" <<'PY'
import pathlib, sys
path, port = pathlib.Path(sys.argv[1]), sys.argv[2]
path.write_text(path.read_text(encoding="utf-8").replace("__PROBE_PORT__", port), encoding="utf-8")
PY

# ── 收集端 ──────────────────────────────────────────────────
cat >"${WORK}/collector.py" <<'PY'
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = sys.argv[1]
PORT = int(sys.argv[2])


class H(BaseHTTPRequestHandler):
    def _ok(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()

    def do_GET(self):  # noqa: N802
        with open(LOG, "a", encoding="utf-8") as f:
            f.write("GET ping\n")
        self._ok()

    def do_POST(self):  # noqa: N802
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n).decode("utf-8", "replace") if n else ""
        with open(LOG, "a", encoding="utf-8") as f:
            f.write(body.replace("\n", " | ") + "\n")
        self._ok()

    def log_message(self, *a):
        pass


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY

log "起收集端（:${PROBE_PORT} → ${WORK}/probe.log）"
python3 "${WORK}/collector.py" "${WORK}/probe.log" "${PROBE_PORT}" &
COLLECT_PID=$!
if ! wait_http "http://127.0.0.1:${PROBE_PORT}/ping" 10; then
    die "收集端没起来（端口 ${PROBE_PORT}）"
fi

# ── 独立 serve ─────────────────────────────────────────────
log "起独立 serve（:${SERVE_PORT}，Bearer 与环境变量同源）"
(
    cd "${ROOT}"
    exec env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
        no_proxy=127.0.0.1,localhost \
        CM_SERVE_DISABLE_INSTANCE_LOCK=1 \
        CM_WEB_API_BEARER_TOKEN="${BEARER}" \
        "${SERVE_BIN}" serve --host 127.0.0.1 --port "${SERVE_PORT}"
) >"${WORK}/serve.log" 2>&1 &
SERVE_PID=$!
if ! wait_http "http://127.0.0.1:${SERVE_PORT}/health" 60; then
    echo "--- serve 日志：" >&2
    tail -20 "${WORK}/serve.log" >&2 || true
    die "serve 没在 ${SERVE_PORT} 就绪"
fi

# ── 注入探针 + 重编壳 ──────────────────────────────────────
# 顺序：先同步 dist（拿到干净的 index.html 当基线），再内联探针，再 touch build.rs 重编 ——
# 壳把资产内嵌进二进制，不重编就不生效（`grep` 二进制找不到明文是因为压缩存放）。
if [[ "${SKIP_BUILD}" -eq 0 ]]; then
    if [[ "${SKIP_UI}" -eq 0 ]]; then
        log "构建业务 UI（make frontend；连续复跑可 --skip-ui）"
        (cd "${ROOT}" && make frontend 2>&1 | tail -2)
    fi
    log "同步 dist 并注入探针"
    bash "${ROOT}/desktop-tauri/scripts/prepare-sidecar.sh" >/dev/null
    python3 - "${ROOT}/desktop-tauri/dist/index.html" "${WORK}/probe.js" <<'PY'
import pathlib, sys
index, probe = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
html = index.read_text(encoding="utf-8")
if "</body>" not in html:
    raise SystemExit("index.html 里找不到 </body>")
js = probe.read_text(encoding="utf-8")
index.write_text(html.replace("</body>", "<script>\n" + js + "\n</script>\n</body>", 1), encoding="utf-8")
PY
    log "重编壳（资产内嵌，必须 touch build.rs）"
    touch "${ROOT}/desktop-tauri/src-tauri/build.rs"
    (cd "${ROOT}/desktop-tauri/src-tauri" && cargo build --bin crabmate-desktop 2>&1 | tail -2)
else
    log "跳过构建（--skip-build）：假设上一个二进制里已经含探针"
fi
[[ -x "${DESKTOP_BIN}" ]] || die "缺少壳二进制：${DESKTOP_BIN}"

# ── 在 xvfb 里启动壳 ──────────────────────────────────────
log "启动壳（观察 ${SETTLE}s；日志 ${WORK}/shell.log）"
# setsid：让壳与 xvfb-run 自成进程组，超时后整组 TERM，避免残留 Xvfb / WebKit 子进程。
setsid env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
    WINIT_UNIX_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 \
    XDG_RUNTIME_DIR="${WORK}/rt" XDG_DATA_HOME="${WORK}/data" XDG_CACHE_HOME="${WORK}/cache" \
    CM_DESKTOP_SKIP_CONNECT=1 \
    CM_DESKTOP_SERVE_URL="http://127.0.0.1:${SERVE_PORT}/" \
    CM_WEB_API_BEARER_TOKEN="${BEARER}" \
    xvfb-run --auto-servernum --server-args='-screen 0 1280x900x24' \
    "${DESKTOP_BIN}" \
    >"${WORK}/shell.log" 2>&1 &
SHELL_PID=$!

sleep "${SETTLE}"
kill -TERM "-${SHELL_PID}" 2>/dev/null || true
wait "${SHELL_PID}" 2>/dev/null || true
SHELL_PID=""

# ── 结果 ───────────────────────────────────────────────────
panics="$(grep -c 'CONSOLE-ERR' "${WORK}/probe.log" 2>/dev/null || true)"
panics="${panics:-0}"

echo
echo "==================== 探针结果 ===================="
echo "panic（CONSOLE-ERR）: ${panics}"
echo "-------------------------------------------------"
if [[ -s "${WORK}/probe.log" ]]; then
    tail -40 "${WORK}/probe.log"
else
    echo "（收集端没收到任何消息：页面可能没加载 —— 看 ${WORK}/shell.log）"
    tail -20 "${WORK}/shell.log" 2>/dev/null || true
fi
echo "================================================="
echo "日志：${WORK}/probe.log / ${WORK}/shell.log / ${WORK}/serve.log"
echo "判定口径见 docs/design/desktop_shell_debugging.md（panic 是硬失败；ALIVE 里的 slots / activeLane 是状态）"

[[ "${panics}" -eq 0 ]]
