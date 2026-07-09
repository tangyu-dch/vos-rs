#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# CPS Random Test — 随机通话时长 + 随机接听/未接听
#
# 通话场景：
#   - 70% 接听（3-15秒随机时长），产生录音 + CDR
#   - 30% 未接听，仅产生 CDR（status=canceled/failed）
#
# Usage:
#   ./run_cps_random.sh [total_calls] [cps] [answer_rate]
#
# Defaults:
#   total_calls  = 10
#   cps          = 2
#   answer_rate  = 0.7  (70% 接听)
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

TOTAL_CALLS="${1:-10}"
CALL_RATE="${2:-2}"
ANSWER_RATE="${3:-0.7}"
INBOUND_RATE="${4:-0.4}"

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

echo "Building sip-edge (release)..."
cargo build --release -p sip-edge 2>&1 | tail -5

echo
echo "============================================="
echo "  CPS RANDOM TEST"
echo "============================================="
echo "  Total calls:    $TOTAL_CALLS"
echo "  CPS:            $CALL_RATE"
echo "  Answer rate:    $(echo "$ANSWER_RATE * 100" | bc)%"
echo "  Duration:       3-15s random"
echo "  Recording dir:  $RECORDING_DIR"
echo "  Edge:           $LOCAL_IP:$EDGE_PORT"
echo "  Gateway UAS:    $LOCAL_IP:$GATEWAY_PORT"
echo "============================================="
echo

# Start sip-edge with recording + DB
echo "Starting sip-edge (release)..."
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

# Start SIPp gateway UAS with rtp_echo
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
  -timeout "120s" \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!

sleep 0.5
echo "Gateway UAS started (PID: $GATEWAY_PID)"
echo

# Run the random CPS tester
echo "Running CPS random test..."
set +e
"$PYTHON_BIN" "$ROOT_DIR/tools/sipp/cps_random.py" \
  "$LOCAL_IP" "$EDGE_PORT" \
  "$TOTAL_CALLS" "$CALL_RATE" "$ANSWER_RATE" "$INBOUND_RATE" "$DESTINATION"
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

WAV_COUNT=$(find "$RECORDING_DIR" -name '*.wav' -type f 2>/dev/null | wc -l | tr -d ' ')
JSON_COUNT=$(find "$RECORDING_DIR" -name '*.json' -type f 2>/dev/null | wc -l | tr -d ' ')

echo "  WAV files:     $WAV_COUNT"
echo "  JSON metadata: $JSON_COUNT"
echo

if [[ "$WAV_COUNT" -gt 0 ]]; then
  echo "  录音文件详情:"
  while IFS= read -r wav_file; do
    if [[ ! -f "$wav_file" ]]; then
      continue
    fi
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
  done < <(find "$RECORDING_DIR" -name '*.wav' -type f | sort)
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
SELECT call_id, caller, callee,
       status,
       CASE WHEN recording_path IS NOT NULL THEN '有录音' ELSE '无录音' END as 录音,
       round(duration_ms/1000.0, 1) as 时长秒,
       to_char(to_timestamp(started_at_ms/1000.0), 'HH24:MI:SS') as 开始时间
FROM call_cdrs
ORDER BY started_at_ms DESC
LIMIT 20;
" 2>/dev/null || echo "  (无法连接数据库查询 CDR)"
  echo
  psql "$DB_URL" -c "
SELECT status, count(*) as 数量,
       CASE WHEN recording_path IS NOT NULL THEN '有录音' ELSE '无录音' END as 录音状态
FROM call_cdrs
GROUP BY status, recording_path IS NOT NULL
ORDER BY status;
" 2>/dev/null || true
else
  echo "  psql 未安装，跳过数据库查询"
fi

echo
echo "============================================="
echo "CPS test status: $([ $CPS_STATUS -eq 0 ] && echo 'PASS' || echo 'FAIL')"
echo "Recordings: $WAV_COUNT"
echo "Logs: $LOG_DIR"
echo "Recordings: $RECORDING_DIR"
echo "============================================="
