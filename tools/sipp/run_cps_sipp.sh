#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# SIPp-based CPS Test — 使用 SIPp 作为呼叫发起方（比 Python 高效 10x+）
#
# Usage: ./run_cps_sipp.sh [total_calls] [cps] [concurrent]
# =============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/cps_rec}"
RECORDING_DIR="$ROOT_DIR/recording"

SIPP_BIN="${SIPP_BIN:-sipp}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5170}"
DESTINATION="${DESTINATION:-13800138000}"

TOTAL_CALLS="${1:-200}"
CALL_RATE="${2:-10}"
CONCURRENT="${3:-50}"

EDGE_PID=""
GATEWAY_PID=""

cleanup() {
  echo
  echo "Cleaning up..."
  [[ -n "$GATEWAY_PID" ]] && kill "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill "$EDGE_PID" 2>/dev/null || true
  sleep 1
  [[ -n "$GATEWAY_PID" ]] && kill -9 "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill -9 "$EDGE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR" "$RECORDING_DIR"

echo "Clearing previous recordings..."
rm -f "$RECORDING_DIR"/*.wav "$RECORDING_DIR"/*.json

command -v "$SIPP_BIN" >/dev/null || { echo "ERROR: sipp not found"; exit 1; }

echo "Building sip-edge (release)..."
cargo build --release -p sip-edge 2>&1 | tail -3

echo
echo "============================================="
echo "  SIPp CPS TEST"
echo "============================================="
echo "  Total calls:    $TOTAL_CALLS"
echo "  CPS:            $CALL_RATE"
echo "  Concurrent:     $CONCURRENT"
echo "  Recording dir:  $RECORDING_DIR"
echo "  Edge:           $LOCAL_IP:$EDGE_PORT"
echo "  Gateway UAS:    $LOCAL_IP:$GATEWAY_PORT"
echo "============================================="
echo

# Start sip-edge
echo "Starting sip-edge..."
VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$GATEWAY_PORT" \
VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_RTP_PORT_MIN="30000" \
VOS_RS_RTP_PORT_MAX="60000" \
VOS_RS_SBC_ALLOW="$LOCAL_IP" \
VOS_RS_SBC_LIMIT_CAPACITY="1000000" \
VOS_RS_SBC_LIMIT_FILL_RATE="100000" \
VOS_RS_SBC_MAX_CONCURRENCY="10000" \
VOS_RS_RECORDING_ENABLED="true" \
VOS_RS_RECORDING_DIR="$RECORDING_DIR" \
VOS_RS_ANTI_FRAUD_ENABLED="false" \
VOS_RS_DATABASE_URL="${VOS_RS_DATABASE_URL:-postgres://tangyu@localhost/vos_rs}" \
RUST_LOG="${RUST_LOG:-sip_edge=info,media=info}" \
  "$ROOT_DIR/target/release/sip-edge" >"$LOG_DIR/sip-edge.log" 2>&1 &
EDGE_PID=$!
sleep 2

if ! kill -0 "$EDGE_PID" 2>/dev/null; then
  echo "ERROR: sip-edge failed to start"
  tail -20 "$LOG_DIR/sip-edge.log"
  exit 1
fi
echo "sip-edge started (PID: $EDGE_PID)"

# Start SIPp gateway UAS
echo "Starting SIPp gateway UAS..."
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/gateway_longcall.xml" \
  -i "$LOCAL_IP" \
  -p "$GATEWAY_PORT" \
  -m "$TOTAL_CALLS" \
  -aa \
  -nostdin \
  -rtp_echo \
  -min_rtp_port 30000 \
  -max_rtp_port 31000 \
  -timeout "$((TOTAL_CALLS / CALL_RATE + 120))s" \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!
sleep 0.5
echo "Gateway UAS started (PID: $GATEWAY_PID)"

# Run SIPp as caller UAC
echo
echo "Running SIPp CPS caller ($TOTAL_CALLS calls, $CALL_RATE CPS, $CONCURRENT concurrent)..."
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/cps_caller.xml" \
  -i "$LOCAL_IP" \
  -m "$TOTAL_CALLS" \
  -r "$CALL_RATE" \
  -l "$CONCURRENT" \
  -nostdin \
  -trace_err \
  -error_file "$LOG_DIR/caller_errors.log" \
  >"$LOG_DIR/caller.stdout" 2>&1
CPS_EXIT=$?

echo
echo "Waiting for gateway to finish..."
wait "$GATEWAY_PID" 2>/dev/null || true
GATEWAY_PID=""

kill "$EDGE_PID" 2>/dev/null || true
sleep 1
EDGE_PID=""

echo
echo "============================================="
echo "  RECORDING VERIFICATION"
echo "============================================="

WAV_COUNT=$(find "$RECORDING_DIR" -name '*.wav' -type f 2>/dev/null | wc -l | tr -d ' ')
JSON_COUNT=$(find "$RECORDING_DIR" -name '*.json' -type f 2>/dev/null | wc -l | tr -d ' ')

echo "  WAV files:     $WAV_COUNT"
echo "  JSON metadata: $JSON_COUNT"
echo

if [[ "$WAV_COUNT" -gt 0 ]]; then
  echo "  Per-file check:"
  while IFS= read -r wav_file; do
    if [[ ! -f "$wav_file" ]]; then continue; fi
    data_size=$(python3 -c "
import struct
with open('$wav_file', 'rb') as f:
    f.seek(40)
    data = f.read(4)
    if len(data) >= 4:
        print(struct.unpack('<I', data)[0])
    else:
        print(0)
" 2>/dev/null || echo "0")
    duration_sec=$(python3 -c "print(round($data_size / 32000.0, 1))" 2>/dev/null || echo "0")
    filename=$(basename "$wav_file")
    echo "    $filename  (${duration_sec}s, ${data_size} bytes)"
  done < <(find "$RECORDING_DIR" -name '*.wav' -type f | head -20 | sort)
fi

# Parse SIPp caller stats
echo
echo "  SIPp caller stats:"
grep -E "Successful|Failed|Call Rate|Total Calls" "$LOG_DIR/caller.stdout" 2>/dev/null | head -5

echo
echo "  Gateway stats:"
grep -E "Successful|Failed|Total Calls|Call Rate" "$LOG_DIR/gateway.stdout" 2>/dev/null | tail -4

echo "============================================="
echo "CPS test status: $([ $CPS_EXIT -eq 0 ] && echo 'PASS' || echo 'FAIL')"
echo "Recordings: $WAV_COUNT"
echo "Logs: $LOG_DIR"
