#!/usr/bin/env bash
# 本地开发一键启动：sip-edge + api-server + 前端（三者同终端，Ctrl+C 全停）
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export DATABASE_URL="${DATABASE_URL:-postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs}"
export VOS_RS_RECORDING_DIR="${VOS_RS_RECORDING_DIR:-$ROOT/target/recordings}"
mkdir -p "$VOS_RS_RECORDING_DIR"

PIDS=()
cleanup() {
  echo
  echo "==> 正在停止所有服务..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

echo "==> 启动 sip-edge (SIP 5090 + 管理 API 8082)"
VOS_RS_SIP_UDP_BIND=127.0.0.1:5090 \
VOS_RS_SIP_DEFAULT_GATEWAY=127.0.0.1:5070 \
VOS_RS_DATABASE_URL="$DATABASE_URL" \
VOS_RS_MANAGE_BIND=127.0.0.1:8082 \
VOS_RS_RECORDING_ENABLED=true \
VOS_RS_RECORDING_DIR="$VOS_RS_RECORDING_DIR" \
RUST_LOG=sip_edge=info \
  cargo run -p sip-edge &
PIDS+=($!)

echo "==> 启动 api-server (8081)"
API_PORT=8081 \
DATABASE_URL="$DATABASE_URL" \
VOS_RS_RECORDING_DIR="$VOS_RS_RECORDING_DIR" \
VOS_RS_MANAGE_BASE=http://127.0.0.1:8082 \
  cargo run -p api-server &
PIDS+=($!)

echo "==> 启动前端 (3001)"
( cd web && VITE_API_TARGET=http://localhost:8081 npm run dev ) &
PIDS+=($!)

echo
echo "========================================"
echo " 前端:           http://localhost:3001"
echo " API:            http://localhost:8081"
echo " sip-edge 管理:  http://localhost:8082"
echo " SIP UDP/TCP:    127.0.0.1:5090"
echo " 按 Ctrl+C 停止全部"
echo "========================================"
wait
