#!/usr/bin/env bash
# Playwright E2E 一键脚本（Client 仓）
#
# 构建/定位 UI dist → 启动 crabmate serve → 运行 Playwright → 停止后端。
#
# 用法:
#   ./scripts/e2e-playwright.sh                          # 全部测试
#   ./scripts/e2e-playwright.sh specs/mock-tool-call-scenarios.spec.ts
#   ./scripts/e2e-playwright.sh --headed
#
# 环境变量:
#   CRABMATE_PORT          后端端口（默认 8080）
#   CRABMATE_BIN           crabmate 二进制（优先）
#   CRABMATE_SERVER_DIR    Server 仓路径（默认同级 ../crabmate_agent 或 ../CrabMate）
#   CM_WEB_STATIC_DIR      UI dist（默认本仓 frontend/dist）
#   CM_E2E_BUILD_FRONTEND  为 1 且 dist 缺失时执行 make frontend
#   E2E_DIR                Playwright 目录（默认 e2e/）
#
# 说明：Server 默认纯 API；本脚本托管 SPA，启动时始终传 --with-web。

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${CRABMATE_PORT:-8080}"
E2E_DIR="${E2E_DIR:-$ROOT/e2e}"
BACKEND_PID=""
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
  if [[ -n "$BACKEND_PID" ]]; then
    echo ""
    echo ">>> 停止后端 (PID $BACKEND_PID)..."
    kill "$BACKEND_PID" 2>/dev/null || true
    wait "$BACKEND_PID" 2>/dev/null || true
    echo ">>> 后端已停止"
  fi
}
trap cleanup EXIT

if command -v lsof &>/dev/null; then
  if lsof -ti ":$PORT" >/dev/null 2>&1; then
    echo "!!! 端口 $PORT 已被占用，请先释放或设置 CRABMATE_PORT"
    exit 1
  fi
else
  echo ">>> lsof 不可用，跳过端口检查"
fi

export RUST_LOG="${CM_E2E_RUST_LOG:-warn}"
SERVER_DIR="$(resolve_server_dir)"

start_backend() {
  # Server 默认不挂 SPA；E2E 需要 UI，必须显式 --with-web（配合 CM_WEB_STATIC_DIR）。
  local serve_args=(serve --with-web --port "$PORT")
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
  echo "      或同级克隆 Server 仓（../crabmate_agent）并 cargo build -p crabmate。" >&2
  exit 1
}

start_backend

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

echo ">>> 运行 Playwright 测试..."
echo "    参数: $*"
echo ""

(
  cd "$E2E_DIR"
  if [[ ! -d node_modules ]]; then
    npm ci
  fi
  CRABMATE_PORT="$PORT" no_proxy=127.0.0.1,localhost npx playwright test "$@"
) || EXIT_CODE=$?

if [[ $EXIT_CODE -eq 0 ]]; then
  echo ""
  echo ">>> 全部测试通过"
else
  echo ""
  echo "!!! 测试失败 (exit=$EXIT_CODE)"
fi

exit $EXIT_CODE
