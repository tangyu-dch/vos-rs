#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOG="$ROOT/target/sipp_bench_media"
SCEN="$ROOT/tools/sipp/scenarios"
RTP_SENDER="$ROOT/tools/sipp/rtp_range_sender.py"
WAV_FILE="$ROOT/tools/sipp/test_speech.wav"
SIPP_BIN="${SIPP_BIN:-sipp}"
EP=${EDGE_PORT:-5160}; GP=${GATEWAY_PORT:-5170}; CP=${CALLER_PORT:-5164}
RTP_MIN=${RTP_MIN:-40000}
RTP_MAX=${RTP_MAX:-65000}
MEDIA_PORT_COUNT=${MEDIA_PORT_COUNT:-2048}
MEDIA_PPS=${MEDIA_PPS:-20000}
MEDIA_RECORDING_ENABLED=${MEDIA_RECORDING_ENABLED:-false}
MEDIA_RECORDING_DIR=${MEDIA_RECORDING_DIR:-$ROOT/target/sipp_bench_media_recordings}
SIP_UDP_RECEIVE_BUFFER=${VOS_RS_SIP_UDP_RECEIVE_BUFFER:-4194304}
SIP_UDP_SEND_BUFFER=${VOS_RS_SIP_UDP_SEND_BUFFER:-4194304}
RECORDING_WORKERS=${VOS_RS_RECORDING_WORKERS:-4}
RECORDING_QUEUE_CAPACITY=${VOS_RS_RECORDING_QUEUE_CAPACITY:-4096}
MEDIA_LEVELS=${MEDIA_LEVELS:-"50:500:400:45 100:1000:800:45 200:1600:1400:45 300:1800:2200:50 500:2500:3500:60"}
MEDIA_RUST_LOG=${MEDIA_RUST_LOG:-info}

rm -rf "$LOG" && mkdir -p "$LOG"
rm -rf "$MEDIA_RECORDING_DIR" && mkdir -p "$MEDIA_RECORDING_DIR"

kill_all() {
  pkill -9 -f sip-edge 2>/dev/null
  pkill -9 -f sipp 2>/dev/null
  pkill -9 -f rtp_range_sender.py 2>/dev/null
}
trap kill_all EXIT

command -v "$SIPP_BIN" >/dev/null || { echo "ERROR: sipp not found"; exit 1; }

extract_metric() {
  local key=$1 file=$2
  awk -v key="$key" '
    /clearing RTP relay target/ {
      for (i = 1; i <= NF; i++) {
        if (index($i, key "=") == 1) {
          value = $i
          sub(key "=", "", value)
          sub(/[^0-9].*$/, "", value)
          sum += value + 0
        }
      }
    }
    END { print sum + 0 }
  ' "$file"
}

run_one() {
  local tag=$1 total=$2 rate=$3 conc=$4 timeout_sec=${5:-60}
  kill_all; sleep 1

  # Start sip-edge
  VOS_RS_CONFIG_FILE="${VOS_RS_CONFIG_FILE:-$ROOT/tools/sipp/configs/performance.yaml}" \
  RUST_LOG="$MEDIA_RUST_LOG" \
    $ROOT/target/release/sip-edge >"$LOG/${tag}_edge.log" 2>&1 &
  local edge_pid=$!
  sleep 2

  if ! kill -0 $edge_pid 2>/dev/null; then
    echo "FAIL  $tag  sip-edge crashed"
    return
  fi

  # Start gateway UAS
  "$SIPP_BIN" 127.0.0.1:$EP -sf $SCEN/gateway_longcall.xml -i 127.0.0.1 -p $GP \
    -m $total -aa -nostdin -rtp_echo -min_rtp_port 20000 -max_rtp_port 29999 \
    -timeout "$((timeout_sec + 15))s" \
    >"$LOG/${tag}_gw.log" 2>&1 &
  local gateway_pid=$!
  sleep 1

  python3 -u "$RTP_SENDER" \
    --wav "$WAV_FILE" \
    --target-ip 127.0.0.1 \
    --port-min "$RTP_MIN" \
    --port-count "$MEDIA_PORT_COUNT" \
    --duration "$timeout_sec" \
    --pps "$MEDIA_PPS" \
    >"$LOG/${tag}_rtp.log" 2>&1 &
  local rtp_pid=$!

  # Start caller UAC with watchdog timeout
  "$SIPP_BIN" 127.0.0.1:$EP -sf $SCEN/caller_longcall.xml -i 127.0.0.1 -p $CP \
    -s 13800138000 -m $total -r $rate -l $conc \
    -aa -nostdin -timeout "${timeout_sec}s" -trace_err -error_file "$LOG/${tag}_err.log" \
    >"$LOG/${tag}_caller.log" 2>&1 &
  local caller_pid=$!

  ( sleep "$((timeout_sec + 10))"; kill $caller_pid 2>/dev/null; sleep 5; kill -9 $caller_pid 2>/dev/null ) &
  local watchdog=$!

  wait $caller_pid 2>/dev/null
  kill $watchdog 2>/dev/null; wait $watchdog 2>/dev/null
  kill $rtp_pid 2>/dev/null; wait $rtp_pid 2>/dev/null
  kill $gateway_pid 2>/dev/null; wait $gateway_pid 2>/dev/null
  kill $edge_pid 2>/dev/null; wait $edge_pid 2>/dev/null
  sleep 0.3

  local cf="$LOG/${tag}_caller.log"
  [[ ! -f "$cf" ]] && { echo "FAIL  $tag  no log"; return; }

  local succ fail complete_cps
  succ=$(awk -F'|' '/Successful call/{print $3}' "$cf" | tr -d ' ')
  fail=$(awk -F'|' '/Failed call/{print $3}' "$cf" | tr -d ' ')
  complete_cps=$(awk -F'|' '/Call Rate/{print $3}' "$cf" | tr -d ' cps' | tr -d ' ')

  local st="PASS"
  if [[ -z "$succ" || -z "$fail" || -z "$complete_cps" ]]; then
    st="INVALID"
    succ="${succ:-?}"
    fail="${fail:-?}"
    complete_cps="${complete_cps:-?}"
  elif [[ "$succ" == "0" || "$fail" != "0" ]]; then
    st="FAIL"
  fi
  grep -q "TIMEOUT" "$cf" 2>/dev/null && st="TIMEOUT"

  local rtp_sent rtp_received rtp_forwarded rtp_recorded rtp_recording_dropped
  rtp_sent=$(awk -F'[ =]' '/RTP sent/{for(i=1;i<=NF;i++) if($i=="packets") print $(i+1)}' "$LOG/${tag}_rtp.log" | tail -1)
  rtp_received=$(extract_metric received_packets "$LOG/${tag}_edge.log")
  rtp_forwarded=$(extract_metric forwarded_packets "$LOG/${tag}_edge.log")
  rtp_recorded=$(extract_metric recorded_packets "$LOG/${tag}_edge.log")
  rtp_recording_dropped=$(extract_metric recording_dropped_packets "$LOG/${tag}_edge.log")

  printf "%-8s %-14s calls=%-6s target_cps=%-5s conc=%-5s  complete_cps=%-8s succ=%-6s fail=%-6s rtp_sent=%-8s recv=%-8s fwd=%-8s rec=%-8s rec_drop=%s\n" \
    "$st" "$tag" "$total" "$rate" "$conc" "$complete_cps" "$succ" "$fail" "${rtp_sent:-0}" "$rtp_received" "$rtp_forwarded" "$rtp_recorded" "$rtp_recording_dropped"
}

echo "============================================================"
echo "  VOS-RS SIPp Benchmark (with RTP media)"
echo "============================================================"
echo ""
echo "Media: ${MEDIA_PPS}pps across ${MEDIA_PORT_COUNT} relay RTP ports, recording=${MEDIA_RECORDING_ENABLED}"
echo "SIP UDP buffers: receive=${SIP_UDP_RECEIVE_BUFFER}, send=${SIP_UDP_SEND_BUFFER}; recording workers=${RECORDING_WORKERS}, queue=${RECORDING_QUEUE_CAPACITY}"
echo "Levels: ${MEDIA_LEVELS}"
echo ""

for level in $MEDIA_LEVELS; do
  IFS=: read -r rate total conc timeout_sec <<< "$level"
  run_one "${rate}cps_media" "$total" "$rate" "$conc" "$timeout_sec"
done

echo ""
echo "============================================================"
echo "  Logs: $LOG/"
echo "============================================================"
