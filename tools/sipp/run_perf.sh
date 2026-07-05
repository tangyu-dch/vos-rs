#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/sipp_perf}"

SIPP_BIN="${SIPP_BIN:-sipp}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5070}"
CALLER_PORT="${CALLER_PORT:-5064}"
DESTINATION="${DESTINATION:-13800138000}"

# Performance test parameters
TOTAL_CALLS="${TOTAL_CALLS:-500}"
CALL_RATE="${CALL_RATE:-100}"         # Calls per second
MAX_CONCURRENCY="${MAX_CONCURRENCY:-50}" # Limit concurrent calls

EDGE_PID=""
GATEWAY_PID=""

cleanup() {
  echo "Cleaning up processes..."
  if [[ -n "$GATEWAY_PID" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
    kill "$GATEWAY_PID" 2>/dev/null || true
  fi

  if [[ -n "$EDGE_PID" ]] && kill -0 "$EDGE_PID" 2>/dev/null; then
    kill "$EDGE_PID" 2>/dev/null || true
  fi
}

trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR"/*.log "$LOG_DIR"/*.stdout

command -v "$SIPP_BIN" >/dev/null || { echo "sipp bin not found"; exit 1; }

echo "Building sip-edge in Release mode..."
cargo build --release -p sip-edge

echo "Starting sip-edge in Release mode..."
VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$GATEWAY_PORT" \
VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_RTP_PORT_MIN="40000" \
VOS_RS_RTP_PORT_MAX="60000" \
VOS_RS_SBC_ALLOW="$LOCAL_IP" \
VOS_RS_SBC_LIMIT_CAPACITY="1000000" \
VOS_RS_SBC_LIMIT_FILL_RATE="100000" \
VOS_RS_SBC_MAX_CONCURRENCY="10000" \
  "$ROOT_DIR/target/release/sip-edge" >"$LOG_DIR/sip-edge.log" 2>&1 &
EDGE_PID=$!

sleep 1

echo "Starting gateway (UAS) to expect $TOTAL_CALLS calls..."
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/gateway_uas.xml" \
  -i "$LOCAL_IP" \
  -p "$GATEWAY_PORT" \
  -m "$TOTAL_CALLS" \
  -aa \
  -nostdin \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!

sleep 0.5

echo "Starting caller (UAC) concurrency test..."
echo "  Total Calls: $TOTAL_CALLS"
echo "  Call Rate: $CALL_RATE CPS"
echo "  Max Concurrency: $MAX_CONCURRENCY"
echo ""

START_TIME=$(date +%s)

set +e
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/caller_uac.xml" \
  -i "$LOCAL_IP" \
  -p "$CALLER_PORT" \
  -s "$DESTINATION" \
  -m "$TOTAL_CALLS" \
  -r "$CALL_RATE" \
  -l "$MAX_CONCURRENCY" \
  -aa \
  -nostdin \
  -trace_err \
  -error_file "$LOG_DIR/caller_errors.log" \
  >"$LOG_DIR/caller.stdout" 2>&1
CALLER_STATUS=$?

if [[ "$CALLER_STATUS" -ne 0 ]]; then
  echo "Caller test failed, stopping gateway..."
  kill "$GATEWAY_PID" 2>/dev/null || true
fi

wait "$GATEWAY_PID" 2>/dev/null
GATEWAY_STATUS=$?
GATEWAY_PID=""
set -e

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

echo "============================================="
echo "             PERFORMANCE REPORT"
echo "============================================="
if [[ "$CALLER_STATUS" -eq 0 && "$GATEWAY_STATUS" -eq 0 ]]; then
  echo "Result: SUCCESS (0 calls failed)"
  CPS=$(echo "scale=2; $TOTAL_CALLS / $ELAPSED" | bc 2>/dev/null || echo "$((TOTAL_CALLS / (ELAPSED + 1)))")
  echo "Elapsed Time: ${ELAPSED} seconds"
  echo "Average Throughput: ${CPS} CPS"
else
  echo "Result: FAILED"
  echo "Caller status: $CALLER_STATUS"
  echo "Gateway status: $GATEWAY_STATUS"
  echo ""
  echo "Last 20 lines of caller stdout:"
  tail -n 20 "$LOG_DIR/caller.stdout" || true
  echo ""
  echo "Last 20 lines of gateway stdout:"
  tail -n 20 "$LOG_DIR/gateway.stdout" || true
fi
echo "============================================="
