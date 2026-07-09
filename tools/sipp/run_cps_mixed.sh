#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 混合 CPS 测试 — SIPp (SIP 信令) + Python RTP Sender (音频发送)
# 坐席号码从号码库存中取 (4001-4005 分配给落地网关 gw-local-1)
# =============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/target/cps_rec}"
RECORDING_DIR="$ROOT_DIR/recording"
TMP_SCENARIOS="$LOG_DIR/scenarios"
WAV_FILE="$ROOT_DIR/tools/sipp/test_speech.wav"

SIPP_BIN="${SIPP_BIN:-sipp}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
LOCAL_IP="${LOCAL_IP:-127.0.0.1}"
EDGE_PORT="${EDGE_PORT:-5062}"
GATEWAY_PORT="${GATEWAY_PORT:-5170}"

TOTAL_CALLS="${1:-200}"
CALL_RATE="${2:-10}"
INBOUND_RATE="${3:-0.4}"

# 坐席号码池（从号码库存分配给落地网关 gw-local-1）
AGENT_NUMBERS=("4001" "4002" "4003" "4004" "4005")

EDGE_PID=""
GATEWAY_PID=""
RTP_PID=""

cleanup() {
  echo
  echo "Cleaning up..."
  [[ -n "$RTP_PID" ]] && kill "$RTP_PID" 2>/dev/null || true
  [[ -n "$GATEWAY_PID" ]] && kill "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill "$EDGE_PID" 2>/dev/null || true
  sleep 1
  [[ -n "$RTP_PID" ]] && kill -9 "$RTP_PID" 2>/dev/null || true
  [[ -n "$GATEWAY_PID" ]] && kill -9 "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$EDGE_PID" ]] && kill -9 "$EDGE_PID" 2>/dev/null || true
  pkill -f "sipp.*cps_" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR" "$RECORDING_DIR" "$TMP_SCENARIOS"
rm -f "$RECORDING_DIR"/*.wav "$RECORDING_DIR"/*.json

command -v "$SIPP_BIN" >/dev/null || { echo "ERROR: sipp not found"; exit 1; }

echo "Building sip-edge (release)..."
cargo build --release -p sip-edge 2>&1 | tail -3

# 生成场景文件（每个坐席号码 × 每种时长 × 呼入/呼出）
DURATIONS="3000 5000 8000 10000 13000 15000"
SCENARIO_COUNT=0

for agent in "${AGENT_NUMBERS[@]}"; do
  for pause_ms in $DURATIONS; do
    # 呼出场景：坐席号码 → 外部号码
    scenario_file="$TMP_SCENARIOS/cps_out_${agent}_${pause_ms}.xml"
    cat > "$scenario_file" << XMLEOF
<?xml version="1.0" encoding="ISO-8859-1" ?>
<!DOCTYPE scenario SYSTEM "sipp.dtd">
<scenario name="CPS outbound ${agent} ${pause_ms}ms">
  <send retrans="500">
    <![CDATA[
      INVITE sip:13800138000@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      Max-Forwards: 70
      From: "${agent}" <sip:${agent}@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:13800138000@[remote_ip]:[remote_port]>
      Call-ID: [call_id]
      CSeq: 1 INVITE
      X-Call-Direction: outbound
      Contact: <sip:${agent}@[local_ip]:[local_port]>
      Content-Type: application/sdp
      Content-Length: [len]

      v=0
      o=cps 1 1 IN IP[local_ip_type] [local_ip]
      s=VOS-RS CPS
      c=IN IP[media_ip_type] [media_ip]
      t=0 0
      m=audio [media_port] RTP/AVP 0 8 101
      a=rtpmap:0 PCMU/8000
      a=rtpmap:8 PCMA/8000
      a=rtpmap:101 telephone-event/8000
      a=fmtp:101 0-16
    ]]>
  </send>
  <recv response="100" optional="true" />
  <recv response="180" optional="true" />
  <recv response="200" rtd="true" crlf="true" />
  <send>
    <![CDATA[
      ACK sip:13800138000@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: "${agent}" <sip:${agent}@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:13800138000@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 1 ACK
      Contact: <sip:${agent}@[local_ip]:[local_port]>
      Max-Forwards: 70
      Content-Length: 0
    ]]>
  </send>
  <pause milliseconds="${pause_ms}" />
  <send retrans="500">
    <![CDATA[
      BYE sip:13800138000@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: "${agent}" <sip:${agent}@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:13800138000@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 2 BYE
      Max-Forwards: 70
      Content-Length: 0
    ]]>
  </send>
  <recv response="200" />
  <ResponseTimeRepartition value="10, 20, 50, 100, 200, 500, 1000"/>
  <CallLengthRepartition value="10, 50, 100, 500, 1000, 5000"/>
</scenario>
XMLEOF
    SCENARIO_COUNT=$((SCENARIO_COUNT + 1))

    # 呼入场景：外部号码 → 坐席号码
    scenario_file="$TMP_SCENARIOS/cps_in_${agent}_${pause_ms}.xml"
    cat > "$scenario_file" << XMLEOF
<?xml version="1.0" encoding="ISO-8859-1" ?>
<!DOCTYPE scenario SYSTEM "sipp.dtd">
<scenario name="CPS inbound ${agent} ${pause_ms}ms">
  <send retrans="500">
    <![CDATA[
      INVITE sip:${agent}@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      Max-Forwards: 70
      From: "ext-2001" <sip:2001@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:${agent}@[remote_ip]:[remote_port]>
      Call-ID: [call_id]
      CSeq: 1 INVITE
      X-Call-Direction: inbound
      Contact: <sip:2001@[local_ip]:[local_port]>
      Content-Type: application/sdp
      Content-Length: [len]

      v=0
      o=cps 1 1 IN IP[local_ip_type] [local_ip]
      s=VOS-RS CPS
      c=IN IP[media_ip_type] [media_ip]
      t=0 0
      m=audio [media_port] RTP/AVP 0 8 101
      a=rtpmap:0 PCMU/8000
      a=rtpmap:8 PCMA/8000
      a=rtpmap:101 telephone-event/8000
      a=fmtp:101 0-16
    ]]>
  </send>
  <recv response="100" optional="true" />
  <recv response="180" optional="true" />
  <recv response="200" rtd="true" crlf="true" />
  <send>
    <![CDATA[
      ACK sip:${agent}@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: "ext-2001" <sip:2001@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:${agent}@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 1 ACK
      Contact: <sip:2001@[local_ip]:[local_port]>
      Max-Forwards: 70
      Content-Length: 0
    ]]>
  </send>
  <pause milliseconds="${pause_ms}" />
  <send retrans="500">
    <![CDATA[
      BYE sip:${agent}@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/[transport] [local_ip]:[local_port];branch=[branch]
      From: "ext-2001" <sip:2001@[local_ip]:[local_port]>;tag=[pid]vosrsCps[call_number]
      To: <sip:${agent}@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 2 BYE
      Max-Forwards: 70
      Content-Length: 0
    ]]>
  </send>
  <recv response="200" />
  <ResponseTimeRepartition value="10, 20, 50, 100, 200, 500, 1000"/>
  <CallLengthRepartition value="10, 50, 100, 500, 1000, 5000"/>
</scenario>
XMLEOF
    SCENARIO_COUNT=$((SCENARIO_COUNT + 1))
  done
done

echo "Generated $SCENARIO_COUNT scenario files"

echo
echo "============================================="
echo "  MIXED CPS TEST (SIPp + RTP Sender)"
echo "============================================="
echo "  Total calls:    $TOTAL_CALLS"
echo "  CPS:            $CALL_RATE"
echo "  Inbound rate:   $(echo "$INBOUND_RATE * 100" | bc)%"
echo "  Agent numbers:  ${AGENT_NUMBERS[*]}"
echo "  Durations:      3s-15s random"
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
RUST_LOG="${RUST_LOG:-sip_edge=info}" \
  "$ROOT_DIR/target/release/sip-edge" >"$LOG_DIR/sip-edge.log" 2>&1 &
EDGE_PID=$!
sleep 2

if ! kill -0 "$EDGE_PID" 2>/dev/null; then
  echo "ERROR: sip-edge failed to start"
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
  -min_rtp_port 20000 \
  -max_rtp_port 29999 \
  -timeout "$((TOTAL_CALLS * 15 / CALL_RATE + 120))s" \
  -trace_err \
  -error_file "$LOG_DIR/gateway_errors.log" \
  >"$LOG_DIR/gateway.stdout" 2>&1 &
GATEWAY_PID=$!
sleep 0.5
echo "Gateway UAS started (PID: $GATEWAY_PID)"

# Start RTP sender
echo "Starting RTP sender..."
"$PYTHON_BIN" "$ROOT_DIR/tools/sipp/rtp_sender.py" \
  "$EDGE_PORT" "$WAV_FILE" "100" \
  >"$LOG_DIR/rtp_sender.log" 2>&1 &
RTP_PID=$!
sleep 0.5
echo "RTP sender started (PID: $RTP_PID)"

# Calculate call distribution
OUTBOUND_CALLS=$(echo "$TOTAL_CALLS * (1 - $INBOUND_RATE)" | bc | cut -d. -f1)
INBOUND_CALLS=$(echo "$TOTAL_CALLS - $OUTBOUND_CALLS" | bc)
NUM_AGENTS=${#AGENT_NUMBERS[@]}
OUTBOUND_PER_AGENT=$((OUTBOUND_CALLS / NUM_AGENTS))
INBOUND_PER_AGENT=$((INBOUND_CALLS / NUM_AGENTS))
OUTBOUND_REMAINDER=$((OUTBOUND_CALLS - OUTBOUND_PER_AGENT * NUM_AGENTS))
INBOUND_REMAINDER=$((INBOUND_CALLS - INBOUND_PER_AGENT * NUM_AGENTS))

echo
echo "  Outbound: $OUTBOUND_CALLS, Inbound: $INBOUND_CALLS"
echo "  Per agent: ~$OUTBOUND_PER_AGENT outbound + ~$INBOUND_PER_AGENT inbound"
echo

# Launch SIPp instances
PIDS=()
for agent in "${AGENT_NUMBERS[@]}"; do
  for pause_ms in $DURATIONS; do
    # Outbound
    calls=$((OUTBOUND_PER_AGENT / 6))
    [ $OUTBOUND_REMAINDER -gt 0 ] && calls=$((calls + 1)) && OUTBOUND_REMAINDER=$((OUTBOUND_REMAINDER - 1))
    [ $calls -gt 0 ] && {
      "$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" -sf "$TMP_SCENARIOS/cps_out_${agent}_${pause_ms}.xml" \
        -i "$LOCAL_IP" -m "$calls" -r "$CALL_RATE" -l 50 -nostdin \
        -trace_err -error_file "$LOG_DIR/err_out_${agent}_${pause_ms}.log" \
        >"$LOG_DIR/sipp_out_${agent}_${pause_ms}.stdout" 2>&1 &
      PIDS+=($!)
    }

    # Inbound
    calls=$((INBOUND_PER_AGENT / 6))
    [ $INBOUND_REMAINDER -gt 0 ] && calls=$((calls + 1)) && INBOUND_REMAINDER=$((INBOUND_REMAINDER - 1))
    [ $calls -gt 0 ] && {
      "$SIPP_BIN" "$LOCAL_IP:$EDGE_PORT" -sf "$TMP_SCENARIOS/cps_in_${agent}_${pause_ms}.xml" \
        -i "$LOCAL_IP" -m "$calls" -r "$CALL_RATE" -l 50 -nostdin \
        -trace_err -error_file "$LOG_DIR/err_in_${agent}_${pause_ms}.log" \
        >"$LOG_DIR/sipp_in_${agent}_${pause_ms}.stdout" 2>&1 &
      PIDS+=($!)
    }
  done
done

echo "  Launched ${#PIDS[@]} SIPp instances"
echo "  Waiting for completion..."

for pid in "${PIDS[@]}"; do
  wait "$pid" 2>/dev/null || true
done

echo "SIPp callers done. Waiting for gateway..."
wait "$GATEWAY_PID" 2>/dev/null || true

echo
echo "============================================="
echo "  RESULTS"
echo "============================================="

TOTAL_SUCCESSFUL=0
TOTAL_FAILED=0
for f in "$LOG_DIR"/sipp_out_*.stdout "$LOG_DIR"/sipp_in_*.stdout; do
  [ -f "$f" ] || continue
  s=$(grep "Successful call" "$f" 2>/dev/null | tail -1 | awk '{print $NF}' || echo "0")
  fl=$(grep "Failed call" "$f" 2>/dev/null | tail -1 | awk '{print $NF}' || echo "0")
  TOTAL_SUCCESSFUL=$((TOTAL_SUCCESSFUL + ${s:-0}))
  TOTAL_FAILED=$((TOTAL_FAILED + ${fl:-0}))
done

WAV_COUNT=$(find "$RECORDING_DIR" -name '*.wav' -type f 2>/dev/null | wc -l | tr -d ' ')

echo "  SIPp Successful: $TOTAL_SUCCESSFUL"
echo "  SIPp Failed:     $TOTAL_FAILED"
echo "  WAV files:       $WAV_COUNT"
echo

# Recording verification
echo "  录音文件验证:"
for f in $(find "$RECORDING_DIR" -name '*.wav' -type f | head -10 | sort); do
  sz=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo "0")
  if [ "$sz" -gt 100 ]; then
    dur=$(python3 -c "print(round(($sz - 44) / 32000.0, 1))" 2>/dev/null || echo "?")
    echo "    OK   $(basename $f)  ${dur}s  ${sz} bytes"
  else
    echo "    EMPTY $(basename $f)  ${sz} bytes"
  fi
done

echo
psql postgres://tangyu@localhost/vos_rs -c "
SELECT direction, status, count(*) FROM call_cdrs GROUP BY direction, status ORDER BY direction, status;
" 2>/dev/null || true

echo
ANS=$(psql -t -A postgres://tangyu@localhost/vos_rs -c "SELECT count(*) FROM call_cdrs WHERE status='answered'" 2>/dev/null || echo "?")
REC=$(psql -t -A postgres://tangyu@localhost/vos_rs -c "SELECT count(*) FROM call_cdrs WHERE recording_path IS NOT NULL" 2>/dev/null || echo "?")
echo "  CDR answered: $ANS / CDR with recording: $REC / WAV: $WAV_COUNT"
echo "============================================="
