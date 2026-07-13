#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOG="$ROOT/target/sipp_bench"
SCEN="$ROOT/tools/sipp/scenarios"
EP=5160; GP=5170; CP=5164

rm -rf "$LOG" && mkdir -p "$LOG"

kill_all() { pkill -9 -f sip-edge 2>/dev/null; pkill -9 -f sipp 2>/dev/null; }
trap kill_all EXIT

run_one() {
  local tag=$1 total=$2 rate=$3 conc=$4 timeout_sec=${5:-60}
  kill_all; sleep 1

  VOS_RS_CONFIG_FILE="${VOS_RS_CONFIG_FILE:-$ROOT/tools/sipp/configs/performance.yaml}" \
    $ROOT/target/release/sip-edge >"$LOG/${tag}_edge.log" 2>&1 &
  local edge_pid=$!
  sleep 2

  if ! kill -0 $edge_pid 2>/dev/null; then
    echo "FAIL  $tag  sip-edge crashed"
    return
  fi

  sipp 127.0.0.1:$EP -sf $SCEN/gateway_uas.xml -i 127.0.0.1 -p $GP \
    -m $total -aa -nostdin >"$LOG/${tag}_gw.log" 2>&1 &
  sleep 1

  # caller with watchdog timeout
  sipp 127.0.0.1:$EP -sf $SCEN/caller_uac.xml -i 127.0.0.1 -p $CP \
    -s 13800138000 -m $total -r $rate -l $conc \
    -aa -nostdin -trace_err -error_file "$LOG/${tag}_err.log" \
    >"$LOG/${tag}_caller.log" 2>&1 &
  local caller_pid=$!

  ( sleep "$timeout_sec"; kill -9 $caller_pid 2>/dev/null ) &
  local watchdog=$!

  wait $caller_pid 2>/dev/null
  kill $watchdog 2>/dev/null; wait $watchdog 2>/dev/null
  kill_all; sleep 0.3

  local cf="$LOG/${tag}_caller.log"
  [[ ! -f "$cf" ]] && { echo "FAIL  $tag  no log"; return; }

  local succ fail cps
  succ=$(awk -F'|' '/Successful call/{print $3}' "$cf" | tr -d ' ')
  fail=$(awk -F'|' '/Failed call/{print $3}' "$cf" | tr -d ' ')
  cps=$(awk -F'|' '/Call Rate/{print $3}' "$cf" | tr -d ' cps' | tr -d ' ')

  local st="PASS"
  [[ "$succ" == "0" ]] && st="FAIL"
  grep -q "TIMEOUT" "$cf" 2>/dev/null && st="TIMEOUT"

  printf "%-8s %-10s calls=%-6s rate=%-5s conc=%-5s  CPS=%-8s  succ=%-6s  fail=%s\n" \
    "$st" "$tag" "$total" "$rate" "$conc" "$cps" "$succ" "$fail"
}

echo "============================================================"
echo "       VOS-RS SIPp Benchmark"
echo "============================================================"
echo ""

case "${PERF_PROFILE:-standard}" in
  quick)
    run_one "50cps" 500 50 25 30
    run_one "100cps" 1000 100 50 30
    run_one "200cps" 2000 200 100 30
    ;;
  standard)
    run_one "500cps" 3000 500 250 30
    run_one "800cps" 5000 800 400 30
    run_one "1000cps" 5000 1000 500 30
    ;;
  all)
    run_one "50cps" 500 50 25 30
    run_one "100cps" 1000 100 50 30
    run_one "200cps" 2000 200 100 30
    run_one "500cps" 3000 500 250 30
    run_one "800cps" 5000 800 400 30
    run_one "1000cps" 5000 1000 500 30
    run_one "2000cps" 5000 2000 1000 30
    ;;
  *) echo "Unknown PERF_PROFILE: $PERF_PROFILE" >&2; exit 2 ;;
esac

echo ""
echo "============================================================"
echo "  Logs: $LOG/"
echo "============================================================"
