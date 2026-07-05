#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../" && pwd)"
SCENARIO_DIR="$ROOT_DIR/tools/sipp/scenarios"
LOG_DIR="$ROOT_DIR/target/sipp_perf"
SIPP_BIN="sipp"
LOCAL_IP="127.0.0.1"

mkdir -p "$LOG_DIR"

# Test configurations: (name total_calls rate max_concurrency)
TESTS=(
  "low_load 500 50 25"
  "mid_load 2000 200 100"
  "high_load 5000 500 250"
  "max_load 10000 1000 500"
  "burst    5000 2000 1000"
)

run_test() {
  local name="$1" total="$2" rate="$3" conc="$4"
  local edge_port=$((5060 + RANDOM % 100))
  local gw_port=$((5070 + RANDOM % 100))
  local caller_port=$((5064 + RANDOM % 100))
  local test_log="$LOG_DIR/$name"
  local edge_pid="" gw_pid=""

  mkdir -p "$test_log"
  rm -f "$test_log"/*.log "$test_log"/*.stdout

  cleanup() {
    [[ -n "$gw_pid" ]] && kill "$gw_pid" 2>/dev/null || true
    [[ -n "$edge_pid" ]] && kill "$edge_pid" 2>/dev/null || true
  }
  trap cleanup EXIT INT TERM

  # Start sip-edge
  VOS_RS_SIP_UDP_BIND="$LOCAL_IP:$edge_port" \
  VOS_RS_SIP_DEFAULT_GATEWAY="$LOCAL_IP:$gw_port" \
  VOS_RS_SIP_ADVERTISED_ADDR="$LOCAL_IP:$edge_port" \
  VOS_RS_RTP_PORT_MIN="40000" \
  VOS_RS_RTP_PORT_MAX="60000" \
  VOS_RS_SBC_ALLOW="$LOCAL_IP" \
  VOS_RS_SBC_LIMIT_CAPACITY="1000000" \
  VOS_RS_SBC_LIMIT_FILL_RATE="100000" \
  VOS_RS_SBC_MAX_CONCURRENCY="10000" \
    "$ROOT_DIR/target/release/sip-edge" >"$test_log/sip-edge.log" 2>&1 &
  edge_pid=$!
  sleep 1

  # Start gateway UAS
  "$SIPP_BIN" "$LOCAL_IP:$edge_port" \
    -sf "$SCENARIO_DIR/gateway_uas.xml" \
    -i "$LOCAL_IP" -p "$gw_port" \
    -m "$total" -aa -nostdin \
    -trace_err -error_file "$test_log/gw_errors.log" \
    >"$test_log/gw.stdout" 2>&1 &
  gw_pid=$!
  sleep 0.3

  # Run caller UAC
  local start_time end_time elapsed
  start_time=$(date +%s)
  set +e
  "$SIPP_BIN" "$LOCAL_IP:$edge_port" \
    -sf "$SCENARIO_DIR/caller_uac.xml" \
    -i "$LOCAL_IP" -p "$caller_port" \
    -s 13800138000 \
    -m "$total" -r "$rate" -l "$conc" \
    -aa -nostdin \
    -trace_err -error_file "$test_log/caller_errors.log" \
    >"$test_log/caller.stdout" 2>&1
  local caller_rc=$?
  set -e
  end_time=$(date +%s)
  elapsed=$((end_time - start_time))

  kill "$gw_pid" 2>/dev/null || true
  wait "$gw_pid" 2>/dev/null || true
  kill "$edge_pid" 2>/dev/null || true
  wait "$edge_pid" 2>/dev/null || true
  gw_pid=""
  edge_pid=""

  # Parse SIPp stats
  local stats_file=$(ls "$test_log"/caller_*.log 2>/dev/null | head -1 || echo "")
  local failed="N/A"
  local succeeded="N/A"
  local avg_resp="N/A"

  if [[ -f "$test_log/caller.stdout" ]]; then
    succeeded=$(grep -oP 'Successful call.*?: \K\d+' "$test_log/caller.stdout" 2>/dev/null || echo "0")
    failed=$(grep -oP 'Failed call.*?: \K\d+' "$test_log/caller.stdout" 2>/dev/null || echo "N/A")
    avg_resp=$(grep -oP 'Average Call Length.*?: \K[\d.]+' "$test_log/caller.stdout" 2>/dev/null || echo "N/A")
  fi

  local cps="N/A"
  if [[ "$elapsed" -gt 0 ]]; then
    cps=$(echo "scale=1; $total / $elapsed" | bc 2>/dev/null || echo "$((total / elapsed))")
  fi

  local result="PASS"
  [[ "$caller_rc" -ne 0 ]] && result="FAIL"

  echo "$result|$name|$total|$rate|$conc|${elapsed}s|${cps}|succeeded=$succeeded failed=$failed|avg_len=${avg_resp}"
}

echo "============================================================"
echo "          VOS-RS SIPp Concurrency Benchmark"
echo "============================================================"
echo ""
printf "%-6s | %-10s | %-6s | %-5s | %-5s | %-5s | %-8s | %s\n" \
  "RESULT" "TEST" "CALLS" "RATE" "CONC" "TIME" "CPS" "DETAILS"
echo "-------|------------|--------|-------|-------|-------|----------|--------------------------"

for test in "${TESTS[@]}"; do
  IFS=' ' read -r name total rate conc <<< "$test"
  echo -n "       | $name      | $total | $rate  | $conc  | "
  run_test "$name" "$total" "$rate" "$conc" 2>&1
done

echo ""
echo "============================================================"
echo "  Logs: $LOG_DIR/"
echo "============================================================"
