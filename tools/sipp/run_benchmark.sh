#!/usr/bin/env bash
# VOS-RS 压测脚本: 纯信令 / 带媒体，支持单档或批量 CPS 档位
#
# 用法:
#   bash run_benchmark.sh signal  <cps> <count>   # 单档纯信令压测
#   bash run_benchmark.sh media   <cps> <count>   # 单档带媒体压测
#   bash run_benchmark.sh signal  batch           # 批量纯信令 (10/50/100/200/500 CPS)
#   bash run_benchmark.sh media   batch           # 批量带媒体
#
# 前置条件:
#   1. sip-edge 已用 VOS_RS_AUTH_BYPASS=true 启动并监听 5060
#   2. config.yaml 中 balance_enforcement_enabled=false, gateway_health_checks_enabled=false
#   3. 数据库 sip_routes 表中 prefix=9 路由到 sipp-egress (127.0.0.1:5190)
#   4. RTP 端口范围 (50000-50998) 未被占用
#
# 输出: 紧凑表格行，可拼接为 Markdown 表格

set -euo pipefail

MODE="${1:-signal}"
CPS_ARG="${2:-10}"
COUNT_ARG="${3:-100}"

SIP_EDGE="127.0.0.1:5060"
SCEN_DIR="$(cd "$(dirname "$0")" && pwd)/scenarios"
UAC_SCENARIO="$SCEN_DIR/bench_uac.xml"
UAS_SCENARIO="$SCEN_DIR/bench_uas.xml"
SERVICE="91001"
UAS_PORT=5190
UAC_PORT=6060

# 确保 bench_uas 在 UAS_PORT 上运行
ensure_uas() {
  if ! pgrep -f "bench_uas.xml -p $UAS_PORT" >/dev/null 2>&1; then
    echo "[setup] 启动 sipp UAS 在 UDP $UAS_PORT..." >&2
    sipp -sf "$UAS_SCENARIO" -p "$UAS_PORT" -nd -m 1000000 -bg >/dev/null 2>&1 || true
    sleep 1
  fi
}

# 单次压测，输出一行结果
run_single() {
  local mode="$1"
  local cps="$2"
  local count="$3"
  local extra=""
  if [ "$mode" = "media" ]; then
    extra=""
  fi

  # 每次 run_single 前杀死残留的 bench_uac 实例
  pkill -9 -f "bench_uac" 2>/dev/null || true
  sleep 0.5

  # 清理 Redis 资源租约 (前一轮残留)
  redis-cli --scan --pattern 'vos_rs:{resource-leases}*' 2>/dev/null | \
    xargs -I {} redis-cli DEL {} >/dev/null 2>&1 || true

  local start_ts end_ts elapsed_real
  start_ts=$(date +%s.%N)

  # SIPp 输出捕获: 成功/失败/响应时间/通话时长/实际cps
  # -l 设为 max(cps*2, 200) 确保并发足够，-rp 1s 明确每秒发送速率
  local conc
  conc=$((cps * 2))
  [ $conc -lt 200 ] && conc=200
  local out
  out=$(sipp -s "$SERVICE" -sf "$UAC_SCENARIO" \
    -m "$count" -r "$cps" -rp 1s -l "$conc" \
    -timeout 180s \
    $extra -nd "$SIP_EDGE" 2>&1 || true)

  end_ts=$(date +%s.%N)
  elapsed_real=$(awk "BEGIN { printf \"%.3f\", $end_ts - $start_ts }")

  # 从输出中提取关键指标
  local succ fail rt cl rate elapsed
  succ=$(echo "$out" | grep "Successful call" | tail -1 | awk -F'|' '{gsub(/ /,"",$3); print $3}')
  fail=$(echo "$out" | grep "Failed call" | tail -1 | awk -F'|' '{gsub(/ /,"",$3); print $3}')
  rt=$(echo "$out" | grep "Response Time 1" | tail -1 | awk -F'|' '{gsub(/ /,"",$3); print $3}')
  cl=$(echo "$out" | grep "Call Length" | tail -1 | awk -F'|' '{gsub(/ /,"",$3); print $3}')
  rate=$(echo "$out" | grep "Call Rate" | tail -1 | awk -F'|' '{gsub(/ cps/,"",$3); gsub(/ /,"",$3); print $3}')
  elapsed=$(echo "$out" | grep "Elapsed Time" | tail -1 | awk -F'|' '{gsub(/ /,"",$3); print $3}')

  # 空值兜底
  succ=${succ:-0}; fail=${fail:-0}; rt=${rt:--}; cl=${cl:--}; rate=${rate:-0}; elapsed=${elapsed:--}

  # 计算实际吞吐 CPS
  local actual_cps
  actual_cps=$(awk "BEGIN { if ($elapsed_real > 0) printf \"%.1f\", $succ / $elapsed_real; else print \"0\" }")

  printf "| %-7s | %4s | %6s | %6s | %6s | %14s | %14s | %7s | %8s |\n" \
    "$mode" "$cps" "$count" "$succ" "$fail" "$rt" "$cl" "$rate" "$actual_cps"
}

case "$MODE" in
  signal|media)
    if [ "$CPS_ARG" = "batch" ]; then
      # 批量档位: 10/50/100/200/500 CPS, 每档 CPS×10 通话 (最少 100)
      echo "========== 批量压测: mode=$MODE =========="
      printf "| %-7s | %4s | %6s | %6s | %6s | %14s | %14s | %7s | %8s |\n" \
        "mode" "cps" "count" "succ" "fail" "resp_time" "call_len" "rate" "actual"
      printf "|---------|------|--------|--------|--------|----------------|----------------|---------|----------|\n"
      ensure_uas
      for cps in 200 500 1000 1200 1500; do
        count=$((cps * 5))
        [ $count -lt 500 ] && count=500
        run_single "$MODE" "$cps" "$count"
      done
    else
      ensure_uas
      printf "| %-7s | %4s | %6s | %6s | %6s | %14s | %14s | %7s | %8s |\n" \
        "mode" "cps" "count" "succ" "fail" "resp_time" "call_len" "rate" "actual"
      printf "|---------|------|--------|--------|--------|----------------|----------------|---------|----------|\n"
      run_single "$MODE" "$CPS_ARG" "$COUNT_ARG"
    fi
    ;;
  *)
    echo "用法: bash $0 {signal|media} <cps|batch> [count]"
    echo "  signal  - 纯信令 INVITE 压测"
    echo "  media   - INVITE + RTP 流压测"
    echo "  cps     - 每秒呼叫数 (10/50/100/200/500) 或 batch"
    echo "  count   - 总通话数 (单档模式, 默认 100)"
    exit 1
    ;;
esac
