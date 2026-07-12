#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# VOS-RS Stress Test Runner
#
# Starts sip-edge + SIPp gateway UAS, then runs stress_test.py for 10 minutes.
# CPS: 10-200 random walk, Call duration: 10-120s, Answer rate: 50-90%.
#
# Usage:
#   ./run_stress_test.sh [duration_sec] [wav_file]
#
# Defaults:
#   duration_sec = 600  (10 minutes)
#   wav_file     = ../sample-speech-1m.wav
# =============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/stress_test}"
RECORDING_DIR="$ROOT_DIR/recordings"

SIPP_BIN="${SIPP_BIN:-sipp}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5170}"

TOTAL_DURATION="${1:-600}"
WAV_FILE="${2:-$ROOT_DIR/tools/sample-speech-1m.wav}"

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

mkdir -p "$LOG_DIR"
mkdir -p "$RECORDING_DIR"

echo "Clearing previous recordings in: $RECORDING_DIR"
rm -f "$RECORDING_DIR"/*.wav "$RECORDING_DIR"/*.json

echo "Clearing previous logs in: $LOG_DIR"
rm -f "$LOG_DIR"/*.log "$LOG_DIR"/*.stdout

command -v "$SIPP_BIN" >/dev/null || { echo "ERROR: sipp not found"; exit 1; }
command -v "$PYTHON_BIN" >/dev/null || { echo "ERROR: python3 not found"; exit 1; }

if [[ ! -f "$WAV_FILE" ]]; then
  echo "ERROR: WAV file not found: $WAV_FILE"
  exit 1
fi

DB_URL="${VOS_RS_DATABASE_URL:-postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs}"
if command -v psql &>/dev/null; then
  echo "Resetting gateway health and ensuring default route..."
  psql "$DB_URL" -c "DELETE FROM gateway_health_status;" 2>/dev/null || true
  psql "$DB_URL" -c "INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight) VALUES ('default', '', 100, 'default', 0.0, 100) ON CONFLICT (id) DO UPDATE SET prefix = EXCLUDED.prefix, gateway_id = EXCLUDED.gateway_id;" 2>/dev/null || true
fi

echo "Building sip-edge (release)..."
cargo build --release -p sip-edge 2>&1 | tail -5

echo
echo "============================================="
echo "  VOS-RS STRESS TEST"
echo "============================================="
echo "  Duration:       ${TOTAL_DURATION}s ($((TOTAL_DURATION / 60)) min)"
echo "  CPS:            10-200 (random walk)"
echo "  Call duration:  10-120s random"
echo "  Answer rate:    50%-90% (random walk)"
echo "  WAV file:       $WAV_FILE"
echo "  Recording dir:  $RECORDING_DIR"
echo "  Edge:           $LOCAL_IP:$EDGE_PORT"
echo "  Gateway UAS:    $LOCAL_IP:$GATEWAY_PORT"
echo "============================================="
echo

# Start SIPp gateway UAS with rtp_echo (BEFORE sip-edge so OPTIONS probes succeed)
echo "Starting SIPp gateway UAS (with rtp_echo)..."
"$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" \
  -sf "$SCENARIO_DIR/gateway_longcall.xml" \
  -i "$LOCAL_IP" \
  -p "$GATEWAY_PORT" \
  -m 1000000 \
  -aa \
  -nostdin \
  -rtp_echo \
  -min_rtp_port 30000 \
  -max_rtp_port 31000 \
  -timeout "$((TOTAL_DURATION + 120))s" \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!

sleep 1
echo "Gateway UAS started (PID: $GATEWAY_PID)"

# Start sip-edge with recording + DB + NATS
echo "Starting sip-edge (release)..."
VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$GATEWAY_PORT" \
VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$EDGE_PORT" \
VOS_RS_SIP_UDP_RECEIVE_BUFFER=4194304 \
VOS_RS_SIP_UDP_SEND_BUFFER=4194304 \
VOS_RS_RTP_ADVERTISED_ADDR="$LOCAL_IP" \
VOS_RS_RTP_PORT_MIN="40000" \
VOS_RS_RTP_PORT_MAX="60000" \
VOS_RS_RTP_SYMMETRIC_LEARNING=true \
VOS_RS_SBC_ALLOW="$LOCAL_IP" \
VOS_RS_SBC_LIMIT_CAPACITY="1000000" \
VOS_RS_SBC_LIMIT_FILL_RATE="100000" \
VOS_RS_SBC_MAX_CONCURRENCY="10000" \
VOS_RS_RECORDING_ENABLED="true" \
VOS_RS_RECORDING_DIR="$RECORDING_DIR" \
VOS_RS_RECORDING_WORKERS=4 \
VOS_RS_RECORDING_QUEUE_CAPACITY=4096 \
VOS_RS_DATABASE_URL="${VOS_RS_DATABASE_URL:-postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs}" \
VOS_RS_NATS_URL="${VOS_RS_NATS_URL:-nats://127.0.0.1:4223}" \
VOS_RS_NATS_CDR_STREAM="${VOS_RS_NATS_CDR_STREAM:-VOS_RS_CDRS}" \
VOS_RS_NATS_CDR_SUBJECT="${VOS_RS_NATS_CDR_SUBJECT:-vos-rs.cdrs}" \
VOS_RS_INTERNAL_SECRET="${VOS_RS_INTERNAL_SECRET:-dev-internal-secret}" \
VOS_RS_ANTI_FRAUD_ENABLED="false" \
VOS_RS_CIRCUIT_BREAKER_FAILURE_THRESHOLD=100 \
VOS_RS_CIRCUIT_BREAKER_RECOVERY_SECS=10 \
VOS_RS_CIRCUIT_BREAKER_MIN_SAMPLES=10000 \
VOS_RS_CIRCUIT_BREAKER_MIN_SUCCESS_RATE=0.01 \
VOS_RS_SIP_AUTH_USERS=1001:secret,1002:secret \
VOS_RS_SESSION_EXPIRES_GATEWAY=7200 \
VOS_RS_SESSION_EXPIRES_CALLER=7200 \
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
echo

# Run the stress test
echo "Running stress test for ${TOTAL_DURATION}s..."
set +e
"$PYTHON_BIN" "$ROOT_DIR/tools/sipp/stress_test.py" \
  "$LOCAL_IP" "$EDGE_PORT" "$TOTAL_DURATION" "$WAV_FILE"
TEST_STATUS=$?
set -e

echo
echo "Waiting for gateway to finish..."
kill "$GATEWAY_PID" 2>/dev/null || true
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

WAV_COUNT=$(find "$RECORDING_DIR" -name '*.wav' -type f 2>/dev/null | wc -l | tr -d ' ')
JSON_COUNT=$(find "$RECORDING_DIR" -name '*.json' -type f 2>/dev/null | wc -l | tr -d ' ')

echo "  WAV files:     $WAV_COUNT"
echo "  JSON metadata: $JSON_COUNT"
echo

if [[ "$WAV_COUNT" -gt 0 ]]; then
  echo "  Top 10 recordings by size:"
  find "$RECORDING_DIR" -name '*.wav' -type f -exec ls -lhS {} + 2>/dev/null | head -10 | while read -r line; do
    echo "    $line"
  done
fi

echo
echo "============================================="

# Check CDR records in DB
DB_URL="${VOS_RS_DATABASE_URL:-postgres://tangyu@localhost/vos_rs}"
if command -v psql &>/dev/null; then
  echo
  echo "  CDR 记录 (PostgreSQL):"
  echo "  ---"
  psql "$DB_URL" -c "
SELECT status, count(*) as 数量,
       CASE WHEN recording_path IS NOT NULL THEN '有录音' ELSE '无录音' END as 录音状态,
       round(avg(duration_ms)/1000.0, 1) as 平均时长秒
FROM call_cdrs
WHERE started_at_ms > (extract(epoch from now()) * 1000 - ${TOTAL_DURATION}000)::bigint
GROUP BY status, recording_path IS NOT NULL
ORDER BY status;
" 2>/dev/null || echo "  (无法连接数据库查询 CDR)"
  echo
else
  echo "  psql 未安装，跳过数据库查询"
fi

echo
echo "============================================="
echo "Stress test status: $([ $TEST_STATUS -eq 0 ] && echo 'PASS' || echo 'FAIL')"
echo "Recordings: $WAV_COUNT"
echo "Logs: $LOG_DIR"
echo "============================================="
