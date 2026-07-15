#!/usr/bin/env bash
# 本地开发一键启动：sip-edge + api-server + 前端（三者同终端，Ctrl+C 全停）
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONFIG_FILE="${VOS_RS_CONFIG_FILE:-$ROOT/config.yaml}"

PIDS=()
cleanup() {
  echo
  echo "==> 正在停止所有服务..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

echo "==> 启动 sip-edge（参数读取 $CONFIG_FILE）"
VOS_RS_CONFIG_FILE="$CONFIG_FILE" \
  cargo run -p sip-edge &
PIDS+=($!)

echo "==> 启动 api-server (8081)"
VOS_RS_CONFIG_FILE="$CONFIG_FILE" \
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
echo " SIP/API 地址以 config.yaml 为准"
echo " 按 Ctrl+C 停止全部"
echo "========================================"
wait
