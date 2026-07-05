#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/sipp}"

SIPP_BIN="${SIPP_BIN:-sipp}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5070}"
CALLER_PORT="${CALLER_PORT:-5064}"
DESTINATION="${DESTINATION:-13800138000}"
SIPP_TIMEOUT="${SIPP_TIMEOUT:-15s}"

EDGE_PID=""
GATEWAY_PID=""

cleanup() {
  if [[ -n "$GATEWAY_PID" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
    kill "$GATEWAY_PID" 2>/dev/null || true
  fi

  if [[ -n "$EDGE_PID" ]] && kill -0 "$EDGE_PID" 2>/dev/null; then
    kill "$EDGE_PID" 2>/dev/null || true
  fi
}

print_failure_logs() {
  echo
  echo "SIPp smoke failed. Logs are in: $LOG_DIR"
  for file in sip-edge.log gateway.stdout gateway_errors.log caller.stdout caller_errors.log; do
    if [[ -s "$LOG_DIR/$file" ]]; then
      echo
      echo "==> $file"
      tail -n 80 "$LOG_DIR/$file"
    fi
  done
}

trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR"/*.log "$LOG_DIR"/*.stdout

command -v "$SIPP_BIN" >/dev/null

cargo build -p sip-edge

VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$GATEWAY_PORT" \
VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$EDGE_PORT" \
  "$ROOT_DIR/target/debug/sip-edge" >"$LOG_DIR/sip-edge.log" 2>&1 &
EDGE_PID=$!

sleep 0.5

"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/gateway_uas.xml" \
  -i "$LOCAL_IP" \
  -p "$GATEWAY_PORT" \
  -m 1 \
  -aa \
  -nostdin \
  -timeout "$SIPP_TIMEOUT" \
  -timeout_error \
  -trace_err \
  -trace_msg \
  -message_file "$LOG_DIR/gateway_messages.log" \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!

sleep 0.5

set +e
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/caller_uac.xml" \
  -i "$LOCAL_IP" \
  -p "$CALLER_PORT" \
  -s "$DESTINATION" \
  -m 1 \
  -r 1 \
  -l 1 \
  -aa \
  -nostdin \
  -timeout "$SIPP_TIMEOUT" \
  -timeout_error \
  -trace_err \
  -trace_msg \
  -message_file "$LOG_DIR/caller_messages.log" \
  -error_file "$LOG_DIR/caller_errors.log" \
  >"$LOG_DIR/caller.stdout" 2>&1
CALLER_STATUS=$?

wait "$GATEWAY_PID"
GATEWAY_STATUS=$?
GATEWAY_PID=""
set -e

if [[ "$CALLER_STATUS" -ne 0 || "$GATEWAY_STATUS" -ne 0 ]]; then
  print_failure_logs
  exit 1
fi

echo "SIPp smoke passed."
echo "Logs: $LOG_DIR"
