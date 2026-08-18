#!/usr/bin/env bash
# Playwright E2E 一键脚本（Client 仓）
#
# 构建/定位 UI dist → 启动纯 API crabmate serve + crabmate-web（回环托管 UI）
# → 运行 Playwright → 停止前后端。
#
# 用法:
#   ./scripts/e2e-playwright.sh                          # 全部测试
#   ./scripts/e2e-playwright.sh specs/mock-tool-call-scenarios.spec.ts
#   ./scripts/e2e-playwright.sh --headed
#
# 环境变量:
#   CRABMATE_PORT          后端 API 端口（默认 8080）
#   CRABMATE_WEB_PORT      前端静态端口 crabmate-web（默认 4173）
#   CRABMATE_BIN           crabmate 二进制（优先）
#   CRABMATE_SERVER_DIR    Server 仓路径（默认同级 ../crabmate_agent 或 ../CrabMate）
#   CRABMATE_WEB_BIN       crabmate-web 二进制（优先；默认 PATH / 本仓 target / 自动构建）
#   CM_WEB_STATIC_DIR      UI dist（默认本仓 frontend/dist；经 --root 传给 crabmate-web）
#   CM_E2E_BUILD_FRONTEND  为 1 且 dist 缺失时执行 make frontend
#   E2E_DIR                Playwright 目录（默认 e2e/）
#
# 说明：Server 默认纯 API（不传 --with-web）；SPA 由客户端自托管 `crabmate-web`
# 在回环上托管，经 `#cm_api_base=` 交接指向纯 API serve（跨 Origin，靠
# CM_WEB_CORS_ALLOWED_ORIGINS 放行 web Origin）。

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${CRABMATE_PORT:-8080}"
WEB_PORT="${CRABMATE_WEB_PORT:-4173}"
API_BASE="http://127.0.0.1:${PORT}"
E2E_DIR="${E2E_DIR:-$ROOT/e2e}"
BACKEND_PID=""
WEB_PID=""
EXIT_CODE=0

resolve_server_dir() {
  if [[ -n "${CRABMATE_SERVER_DIR:-}" ]]; then
    echo "${CRABMATE_SERVER_DIR}"
    return
  fi
  for d in "${ROOT}/../crabmate_agent" "${ROOT}/../CrabMate"; do
    if [[ -f "${d}/Cargo.toml" ]]; then
      echo "$(cd "$d" && pwd)"
      return
    fi
  done
  echo ""
}

if [[ -z "${CM_WEB_STATIC_DIR:-}" ]]; then
  export CM_WEB_STATIC_DIR="${ROOT}/frontend/dist"
fi

if [[ ! -f "${CM_WEB_STATIC_DIR}/index.html" ]]; then
  if [[ "${CM_E2E_BUILD_FRONTEND:-0}" == "1" ]]; then
    echo ">>> 构建 frontend（CM_E2E_BUILD_FRONTEND=1）..."
    (cd "$ROOT" && make frontend)
  fi
fi

if [[ ! -f "${CM_WEB_STATIC_DIR}/index.html" ]]; then
  echo "错误: 未找到 UI dist：${CM_WEB_STATIC_DIR}/index.html" >&2
  echo "      请先: make frontend" >&2
  echo "      或: export CM_WEB_STATIC_DIR=/path/to/frontend/dist" >&2
  exit 1
fi
echo ">>> UI dist: ${CM_WEB_STATIC_DIR}"

cleanup() {
  if [[ -n "$WEB_PID" ]]; then
    echo ""
    echo ">>> 停止 crabmate-web (PID $WEB_PID)..."
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  if [[ -n "$BACKEND_PID" ]]; then
    echo ">>> 停止后端 (PID $BACKEND_PID)..."
    kill "$BACKEND_PID" 2>/dev/null || true
    wait "$BACKEND_PID" 2>/dev/null || true
    echo ">>> 后端已停止"
  fi
}
trap cleanup EXIT

if command -v lsof &>/dev/null; then
  for p in "$PORT" "$WEB_PORT"; do
    if lsof -ti ":$p" >/dev/null 2>&1; then
      echo "!!! 端口 $p 已被占用，请先释放或设置 CRABMATE_PORT / CRABMATE_WEB_PORT"
      exit 1
    fi
  done
else
  echo ">>> lsof 不可用，跳过端口检查"
fi

export RUST_LOG="${CM_E2E_RUST_LOG:-warn}"
SERVER_DIR="$(resolve_server_dir)"

start_backend() {
  # Server 默认纯 API：E2E 不再依赖 --with-web。SPA 由客户端 crabmate-web 托管。
  local serve_args=(serve --port "$PORT")
  if [[ -n "${CRABMATE_BIN:-}" ]]; then
    echo ">>> 启动后端 (CRABMATE_BIN=$CRABMATE_BIN)..."
    "$CRABMATE_BIN" "${serve_args[@]}" &
    BACKEND_PID=$!
    return
  fi
  if command -v crabmate >/dev/null 2>&1; then
    echo ">>> 启动后端 (PATH crabmate)..."
    crabmate "${serve_args[@]}" &
    BACKEND_PID=$!
    return
  fi
  for cand in \
    "${SERVER_DIR:+$SERVER_DIR/target/debug/crabmate}" \
    "${SERVER_DIR:+$SERVER_DIR/target/release/crabmate}"; do
    if [[ -n "$cand" && -x "$cand" ]]; then
      echo ">>> 启动后端 ($cand)..."
      "$cand" "${serve_args[@]}" &
      BACKEND_PID=$!
      return
    fi
  done
  if [[ -n "$SERVER_DIR" && -f "$SERVER_DIR/Cargo.toml" ]]; then
    echo ">>> 启动后端 (cargo run @ $SERVER_DIR)..."
    (cd "$SERVER_DIR" && cargo run -- "${serve_args[@]}") &
    BACKEND_PID=$!
    return
  fi
  echo "错误: 未找到 crabmate serve。" >&2
  echo "      设置 CRABMATE_BIN=/path/to/crabmate，或将 crabmate 放入 PATH，" >&2
  echo "      或同级克隆 Server 仓（../crabmate_agent）并 cargo build -p crabmate --features web。" >&2
  exit 1
}

# 浏览器 Origin 须进 serve 的 CORS 白名单（跨 Origin API 调用）。
allow_web_origin_in_cors() {
  local origin="http://127.0.0.1:${WEB_PORT}"
  local current="${CM_WEB_CORS_ALLOWED_ORIGINS:-}"
  if [[ -z "$current" ]]; then
    export CM_WEB_CORS_ALLOWED_ORIGINS="$origin"
  elif [[ ",$current," != *",$origin,"* ]]; then
    export CM_WEB_CORS_ALLOWED_ORIGINS="$current,$origin"
  fi
  echo ">>> CORS 白名单: $CM_WEB_CORS_ALLOWED_ORIGINS"
}

resolve_web_bin() {
  if [[ -n "${CRABMATE_WEB_BIN:-}" ]]; then
    echo "${CRABMATE_WEB_BIN}"
    return
  fi
  if command -v crabmate-web >/dev/null 2>&1; then
    command -v crabmate-web
    return
  fi
  for cand in \
    "${ROOT}/crates/crabmate-web-host/target/debug/crabmate-web" \
    "${ROOT}/crates/crabmate-web-host/target/release/crabmate-web"; do
    if [[ -x "$cand" ]]; then
      echo "$cand"
      return
    fi
  done
  echo ""
}

build_web_bin() {
  local host_dir="${ROOT}/crates/crabmate-web-host"
  if [[ ! -f "$host_dir/Cargo.toml" ]]; then
    return 1
  fi
  echo ">>> 构建 crabmate-web（本仓 crates/crabmate-web-host）..."
  (cd "$host_dir" && cargo build --bin crabmate-web)
}

start_web_host() {
  local bin
  bin="$(resolve_web_bin)"
  if [[ -z "$bin" ]]; then
    if ! build_web_bin; then
      echo "错误: 未找到 crabmate-web。" >&2
      echo "      设置 CRABMATE_WEB_BIN=/path/to/crabmate-web，或先构建本仓 crates/crabmate-web-host。" >&2
      exit 1
    fi
    bin="$(resolve_web_bin)"
  fi
  echo ">>> 启动 crabmate-web ($bin) @ :$WEB_PORT (api-base $API_BASE)..."
  "$bin" --listen "127.0.0.1:${WEB_PORT}" \
    --root "$CM_WEB_STATIC_DIR" \
    --api-base "$API_BASE" \
    --no-open &
  WEB_PID=$!
}

# 先设 CORS 再起 serve：serve 继承该 env（浏览器 Origin 须进白名单，跨 Origin API 调用）。
allow_web_origin_in_cors
start_backend
start_web_host

echo ">>> 等待后端就绪 (:$PORT)..."
for i in $(seq 1 60); do
  if curl -s --connect-timeout 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo ">>> 后端就绪"
    break
  fi
  if [[ $i -eq 60 ]]; then
    echo "!!! 后端启动超时"
    exit 1
  fi
  sleep 1
done

echo ">>> 等待 crabmate-web 就绪 (:$WEB_PORT)..."
for i in $(seq 1 30); do
  if curl -s --connect-timeout 2 -o /dev/null "http://127.0.0.1:$WEB_PORT/" >/dev/null 2>&1; then
    echo ">>> crabmate-web 就绪"
    break
  fi
  if [[ $i -eq 30 ]]; then
    echo "!!! crabmate-web 启动超时"
    exit 1
  fi
  sleep 1
done

echo ">>> 运行 Playwright 测试..."
echo "    参数: $*"
echo ""

(
  cd "$E2E_DIR"
  if [[ ! -d node_modules ]]; then
    npm ci
  fi
  CRABMATE_PORT="$PORT" \
    CRABMATE_WEB_PORT="$WEB_PORT" \
    CRABMATE_API_BASE="$API_BASE" \
    no_proxy=127.0.0.1,localhost \
    npx playwright test "$@"
) || EXIT_CODE=$?

if [[ $EXIT_CODE -eq 0 ]]; then
  echo ""
  echo ">>> 全部测试通过"
else
  echo ""
  echo "!!! 测试失败 (exit=$EXIT_CODE)"
fi

exit $EXIT_CODE
