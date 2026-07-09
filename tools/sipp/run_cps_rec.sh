#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# CPS Recording Test for VOS-RS
#
# Runs a Calls-Per-Second test where every call carries real PCMU RTP media
# for the configured duration (default 10s), producing a WAV recording per call.
#
# Recordings are written to: $ROOT_DIR/recording/
#
# Why this script exists:
#   The existing bench scripts (run_perf.sh, run_bench*.sh) do NOT enable
#   recording, do NOT send RTP media, and end calls in ~100ms — so no
#   recordings are produced. This script fixes all three issues.
#
# Usage:
#   ./run_cps_rec.sh [total_calls] [cps] [duration_sec]
#
# Defaults:
#   total_calls  = 5
#   cps          = 1
#   duration_sec = 10
# =============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/cps_rec}"
RECORDING_DIR="$ROOT_DIR/recording"

SIPP_BIN="${SIPP_BIN:-sipp}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5170}"
DESTINATION="${DESTINATION:-13800138000}"

TOTAL_CALLS="${1:-5}"
CALL_RATE="${2:-1}"
CALL_DURATION="${3:-10}"

EDGE_PID=""
GATEWAY_PID=""

cleanup() {
  echo
  echo "Cleaning up..."
  [[ -n "$GATEWAY_PID" ]] && kill "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill "$EDGE_PID" 2>/dev/null || true
  # Give processes a moment to flush recordings
  sleep 1
  [[ -n "$GATEWAY_PID" ]] && kill -9 "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill -9 "$EDGE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"
mkdir -p "$RECORDING_DIR"

# Clear previous recordings so we can verify cleanly
echo "Clearing previous recordings in: $RECORDING_DIR"
rm -f "$RECORDING_DIR"/*.wav "$RECORDING_DIR"/*.json

echo "Clearing previous logs in: $LOG_DIR"
rm -f "$LOG_DIR"/*.log "$LOG_DIR"/*.stdout

command -v "$SIPP_BIN" >/dev/null || { echo "ERROR: sipp not found"; exit 1; }
command -v "$PYTHON_BIN" >/dev/null || { echo "ERROR: python3 not found"; exit 1; }

echo "Building sip-edge (release)..."
cargo build --release -p sip-edge 2>&1 | tail -5

echo
echo "============================================="
echo "  CPS RECORDING TEST"
echo "============================================="
echo "  Total calls:    $TOTAL_CALLS"
echo "  CPS:            $CALL_RATE"
echo "  Call duration:  ${CALL_DURATION}s"
echo "  Recording dir:  $RECORDING_DIR"
echo "  Edge:           $LOCAL_IP:$EDGE_PORT"
echo "  Gateway UAS:    $LOCAL_IP:$GATEWAY_PORT"
echo "============================================="
echo

# Start sip-edge with recording enabled
echo "Starting sip-edge (release) with recording..."
VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$GATEWAY_PORT" \
VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_RTP_PORT_MIN="40000" \
VOS_RS_RTP_PORT_MAX="60000" \
VOS_RS_SBC_ALLOW="$LOCAL_IP" \
VOS_RS_SBC_LIMIT_CAPACITY="1000000" \
VOS_RS_SBC_LIMIT_FILL_RATE="100000" \
VOS_RS_SBC_MAX_CONCURRENCY="10000" \
VOS_RS_RECORDING_ENABLED="true" \
VOS_RS_RECORDING_DIR="$RECORDING_DIR" \
VOS_RS_ANTI_FRAUD_ENABLED="false" \
VOS_RS_DATABASE_URL="${VOS_RS_DATABASE_URL:-postgres://tangyu@localhost/vos_rs}" \
RUST_LOG="${RUST_LOG:-sip_edge=info,media=debug}" \
  "$ROOT_DIR/target/release/sip-edge" >"$LOG_DIR/sip-edge.log" 2>&1 &
EDGE_PID=$!

sleep 2
if ! kill -0 "$EDGE_PID" 2>/dev/null; then
  echo "ERROR: sip-edge failed to start"
  tail -20 "$LOG_DIR/sip-edge.log"
  exit 1
fi
echo "sip-edge started (PID: $EDGE_PID)"

# Start SIPp gateway UAS with rtp_echo so it echoes RTP back through the relay,
# creating bidirectional media flow for recording (caller→relay→gateway→relay→caller).
echo "Starting SIPp gateway UAS (with rtp_echo)..."
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
  -timeout "$((CALL_DURATION + 30))s" \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!

sleep 0.5
echo "Gateway UAS started (PID: $GATEWAY_PID)"
echo

# Run the Python CPS tester
echo "Running CPS recording test..."
set +e
"$PYTHON_BIN" "$ROOT_DIR/tools/sipp/cps_recording.py" \
  "$LOCAL_IP" "$EDGE_PORT" \
  "$TOTAL_CALLS" "$CALL_RATE" "$CALL_DURATION" "$DESTINATION"
CPS_STATUS=$?
set -e

echo
echo "Waiting for gateway to finish..."
wait "$GATEWAY_PID" 2>/dev/null || true
GATEWAY_PID=""

# Stop sip-edge
kill "$EDGE_PID" 2>/dev/null || true
sleep 1
EDGE_PID=""

echo
echo "============================================="
echo "  RECORDING VERIFICATION"
echo "============================================="

# Count WAV files
WAV_COUNT=$(find "$RECORDING_DIR" -name '*.wav' -type f 2>/dev/null | wc -l | tr -d ' ')
JSON_COUNT=$(find "$RECORDING_DIR" -name '*.json' -type f 2>/dev/null | wc -l | tr -d ' ')

echo "  WAV files:     $WAV_COUNT"
echo "  JSON metadata: $JSON_COUNT"
echo "  Expected:      $TOTAL_CALLS"
echo

# Check each WAV file for minimum duration (10s = 80000 samples at 8kHz stereo)
# WAV data size = samples * channels * bits_per_sample/8 = 80000 * 2 * 2 = 320000 bytes for 10s
MIN_DATA_BYTES=$((80000 * 2 * 2))  # 10s at 8kHz, 16-bit, stereo
VALID_COUNT=0
SHORT_COUNT=0

if [[ "$WAV_COUNT" -gt 0 ]]; then
  echo "  Per-file check (min ${CALL_DURATION}s audio):"
  while IFS= read -r wav_file; do
    if [[ ! -f "$wav_file" ]]; then
      continue
    fi
    # Extract data size from WAV header (bytes 40-43, little-endian)
    data_size=$(python3 -c "
import struct, sys
with open('$wav_file', 'rb') as f:
    f.seek(40)
    data = f.read(4)
    if len(data) >= 4:
        print(struct.unpack('<I', data)[0])
    else:
        print(0)
" 2>/dev/null || echo "0")
    # Duration = data_bytes / (sample_rate * channels * bits_per_sample/8)
    # = data_bytes / (8000 * 2 * 2) = data_bytes / 32000
    duration_sec=$(python3 -c "print(round($data_size / 32000.0, 1))" 2>/dev/null || echo "0")
    filename=$(basename "$wav_file")
    if [[ "$data_size" -ge "$MIN_DATA_BYTES" ]]; then
      echo "    OK   $filename  (${duration_sec}s, ${data_size} bytes)"
      VALID_COUNT=$((VALID_COUNT + 1))
    else
      echo "    SHORT $filename  (${duration_sec}s, ${data_size} bytes)"
      SHORT_COUNT=$((SHORT_COUNT + 1))
    fi
  done < <(find "$RECORDING_DIR" -name '*.wav' -type f | sort)
fi

echo
echo "  Valid recordings (>=${CALL_DURATION}s):  $VALID_COUNT"
echo "  Short recordings:                        $SHORT_COUNT"
echo "  Missing recordings:                      $((TOTAL_CALLS - WAV_COUNT))"
echo "============================================="

echo
echo "CPS test status: $([ $CPS_STATUS -eq 0 ] && echo 'PASS' || echo 'FAIL')"
echo "Recordings: $VALID_COUNT/$TOTAL_CALLS valid"
echo "Logs: $LOG_DIR"
echo "Recordings: $RECORDING_DIR"

# Exit with failure if any recording is missing or short
if [[ "$VALID_COUNT" -ne "$TOTAL_CALLS" ]]; then
  echo "WARNING: Not all calls produced valid ${CALL_DURATION}s recordings."
  exit 1
fi
