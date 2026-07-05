#!/usr/bin/env bash
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../" && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="$ROOT_DIR/target/sipp_perf_$(date +%Y%m%d_%H%M%S)"
SIPP_BIN="sipp"
LOCAL_IP="127.0.0.1"

mkdir -p "$LOG_DIR"

cleanup() {
  pkill -f "sip-edge" 2>/dev/null || true
  pkill -f "sipp" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

run_one() {
  local tag="$1" total="$2" rate="$3" conc="$4"
  local ep=$((5060 + RANDOM % 400))
  local gp=$((5070 + RANDOM % 400))
  local cp=$((5064 + RANDOM % 400))
  local d="$LOG_DIR/$tag"
  mkdir -p "$d"

  cleanup

  # start sip-edge
  VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$ep" \
  VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$gp" \
  VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$ep" \
  VOS_RS_RTP_PORT_MIN=40000 VOS_RS_RTP_PORT_MAX=60000 \
  VOS_RS_SBC_ALLOW="$LOCAL_IP" \
  VOS_RS_SBC_LIMIT_CAPACITY=1000000 \
  VOS_RS_SBC_LIMIT_FILL_RATE=100000 \
  VOS_RS_SBC_MAX_CONCURRENCY=10000 \
    "$ROOT_DIR/target/release/sip-edge" >/dev/null 2>&1 &
  local edge_pid=$!
  sleep 1

  # start gateway UAS
  "$SIPP_BIN" "$LOCAL_IP:$ep" \
    -sf "$SCENARIO_DIR/gateway_uas.xml" \
    -i "$LOCAL_IP" -p "$gp" -m "$total" -aa -nostdin \
    >"$d/gw.log" 2>&1 &
  local gw_pid=$!
  sleep 0.3

  # caller UAC
  set +e
  "$SIPP_BIN" "$LOCAL_IP:$ep" \
    -sf "$SCENARIO_DIR/caller_uac.xml" \
    -i "$LOCAL_IP" -p "$cp" -s 13800138000 \
    -m "$total" -r "$rate" -l "$conc" \
    -aa -nostdin -trace_err -error_file "$d/err.log" \
    >"$d/caller.log" 2>&1
  local rc=$?
  set -e

  cleanup
  sleep 0.3

  # parse results
  local succ fail cps rpl avg_len
  succ=$(grep -oP 'Successful call\s+\|\s+[\d ]+\|\s+\K\d+' "$d/caller.log" 2>/dev/null || echo "?")
  fail=$(grep -oP 'Failed call\s+\|\s+[\d ]+\|\s+\K\d+' "$d/caller.log" 2>/dev/null || echo "?")
  cps=$(grep -oP 'Call Rate\s+\|\s+[\d.]+ cps\s+\|\s+\K[\d.]+' "$d/caller.log" 2>/dev/null | tail -1 || echo "?")
  avg_len=$(grep -oP 'Response Time 1\s+\|\s+\K[\d:.\-]+' "$d/caller.log" 2>/dev/null | tail -1 || echo "?")

  local status="PASS"
  [[ "$rc" -ne 0 ]] && status="FAIL"
  [[ "$succ" == "0" ]] && status="FAIL"

  printf "%-6s | %-12s | %6s | %5s | %5s | %5s CPS | succ=%-6s fail=%-5s | avg_resp=%s\n" \
    "$status" "$tag" "$total" "$rate" "$conc" "$cps" "$succ" "$fail" "$avg_len"
}

echo "============================================================"
echo "        VOS-RS SIPp Concurrency Benchmark (UDP)"
echo "============================================================"
echo ""
printf "%-6s | %-12s | %6s | %5s | %5s | %10s | %s\n" \
  "RESULT" "TEST" "CALLS" "RATE" "CONC" "CPS" "DETAILS"
echo "-------|--------------|--------|-------|-------|------------|-------------------------------"

# 逐级加压
run_one "01_50cps"       500    50   25
run_one "02_100cps"     1000   100   50
run_one "03_200cps"     2000   200  100
run_one "04_500cps"     3000   500  250
run_one "05_800cps"     5000   800  400
run_one "06_1000cps"    5000  1000  500
run_one "07_2000cps"    5000  2000 1000
run_one "08_burst3k"    5000  3000 1500

echo ""
echo "============================================================"
echo "  Logs: $LOG_DIR/"
echo "============================================================"
