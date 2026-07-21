#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOG_DIR="${FULL_FLOW_LOG_DIR:-${ROOT_DIR}/target/full-flow}"
EDGE_A_CONFIG="${SIP_CLUSTER_EDGE_A_CONFIG_FILE:-${ROOT_DIR}/tools/sipp/configs/sip_cluster_edge_a.yaml}"
EDGE_B_CONFIG="${SIP_CLUSTER_EDGE_B_CONFIG_FILE:-${ROOT_DIR}/tools/sipp/configs/sip_cluster_edge_b.yaml}"
ROUTER_CONFIG="${SIP_CLUSTER_ROUTER_CONFIG_FILE:-${ROOT_DIR}/tools/sipp/configs/sip_cluster_router.yaml}"
SIPP_BIN="${SIPP_BIN:-sipp}"
INTERNAL_SECRET="sip-cluster-test-secret"
PIDS=()

cleanup_redis() {
    local key
    while IFS= read -r key; do
        [[ -z "${key}" ]] || redis-cli del "${key}" >/dev/null
    done < <(redis-cli --scan --pattern 'vos_rs:test:sip_nodes:*')
    while IFS= read -r key; do
        [[ -z "${key}" ]] || redis-cli del "${key}" >/dev/null
    done < <(redis-cli --scan --pattern 'vos_rs:cluster:sip_dialog_routes:*')
}

cleanup() {
    if ((${#PIDS[@]})); then
        kill "${PIDS[@]}" 2>/dev/null || true
        wait "${PIDS[@]}" 2>/dev/null || true
    fi
    pkill -9 -f 'sipp.*127.0.0.1:526[012]' 2>/dev/null || true
    cleanup_redis
}
trap cleanup EXIT INT TERM

require_dependencies() {
    command -v redis-cli >/dev/null || { printf '缺少 redis-cli\n'; exit 2; }
    command -v curl >/dev/null || { printf '缺少 curl\n'; exit 2; }
    command -v "${SIPP_BIN}" >/dev/null || { printf '缺少 SIPp: %s\n' "${SIPP_BIN}"; exit 2; }
    redis-cli ping >/dev/null || { printf 'Redis 6379 不可用\n'; exit 2; }
    nc -z 127.0.0.1 4222 || { printf 'NATS 4222 不可用\n'; exit 2; }
}

start_processes() {
    mkdir -p "${LOG_DIR}"
    cleanup_redis
    VOS_RS_CONFIG_FILE="${EDGE_A_CONFIG}" \
        "${ROOT_DIR}/target/release/sip-edge" >"${LOG_DIR}/sip-edge-a-failover.log" 2>&1 &
    PIDS+=("$!")
    VOS_RS_CONFIG_FILE="${EDGE_B_CONFIG}" \
        "${ROOT_DIR}/target/release/sip-edge" >"${LOG_DIR}/sip-edge-b-failover.log" 2>&1 &
    PIDS+=("$!")
    sleep 3
    VOS_RS_CONFIG_FILE="${ROUTER_CONFIG}" \
        "${ROOT_DIR}/target/release/sip-router" >"${LOG_DIR}/sip-router-failover.log" 2>&1 &
    PIDS+=("$!")
    sleep 3
}

start_gateways() {
    "${SIPP_BIN}" 127.0.0.1:5261 -sf "${ROOT_DIR}/tools/sipp/scenarios/gateway_longcall.xml" \
        -i 127.0.0.1 -p 5271 -m 24 -aa -nostdin -timeout 90s -trace_msg \
        -message_file "${LOG_DIR}/gateway-a-failover-messages.log" >/dev/null 2>&1 &
    PIDS+=("$!")
    "${SIPP_BIN}" 127.0.0.1:5262 -sf "${ROOT_DIR}/tools/sipp/scenarios/gateway_longcall.xml" \
        -i 127.0.0.1 -p 5272 -m 40 -aa -nostdin -timeout 120s -trace_msg \
        -message_file "${LOG_DIR}/gateway-b-failover-messages.log" >/dev/null 2>&1 &
    PIDS+=("$!")
    sleep 1
}

control_node() {
    local port="$1"
    local action="$2"
    curl -fsS -X POST -H "X-VOS-Token: ${INTERNAL_SECRET}" \
        "http://127.0.0.1:${port}/manage/cluster/${action}"
}

run_calls() {
    local count="$1"
    local source_port="$2"
    local call_prefix="$3"
    local output_name="$4"
    "${SIPP_BIN}" 127.0.0.1:5260 -sf "${ROOT_DIR}/tools/sipp/scenarios/caller_cluster_longcall.xml" \
        -i 127.0.0.1 -p "${source_port}" -cid_str "${call_prefix}-%u@vos-rs" \
        -s 13800138000 -m "${count}" -r 4 -l 4 -aa -nostdin -timeout 25s -timeout_error \
        -trace_err -error_file "${LOG_DIR}/${output_name}-errors.log" \
        >"${LOG_DIR}/${output_name}.log" 2>&1
    awk -F'|' '/Successful call/{gsub(/ /,"",$3); print $3}' "${LOG_DIR}/${output_name}.log"
}

invite_count() {
    local log_file="$1"
    grep 'received SIP request' "${log_file}" | grep -E -c 'method.*INVITE' || true
}

require_dependencies
start_processes

curl -fsS http://127.0.0.1:5283/health >/dev/null
curl -fsS http://127.0.0.1:5283/ready >/dev/null
curl -fsS http://127.0.0.1:5283/metrics | grep -q 'vos_rs_sip_router_active_transactions'

start_gateways
control_node 5281 drain | grep -q '"status":"draining"'
sleep 2
redis-cli get vos_rs:test:sip_nodes:sip-edge-a | grep -q '"status":"draining"'

DRAIN_SUCCESS="$(run_calls 8 5264 'aaaaaaaa-0000-4000-8000' 'caller-draining')"
A_DRAIN_INVITES="$(invite_count "${LOG_DIR}/sip-edge-a-failover.log")"
B_DRAIN_INVITES="$(invite_count "${LOG_DIR}/sip-edge-b-failover.log")"

control_node 5281 resume | grep -q '"status":"active"'
sleep 2
redis-cli get vos_rs:test:sip_nodes:sip-edge-a | grep -q '"status":"active"'
RESUME_SUCCESS="$(run_calls 16 5265 'bbbbbbbb-0000-4000-8000' 'caller-resumed')"
A_FINAL_INVITES="$(invite_count "${LOG_DIR}/sip-edge-a-failover.log")"
B_FINAL_INVITES="$(invite_count "${LOG_DIR}/sip-edge-b-failover.log")"

kill "${PIDS[0]}"
wait "${PIDS[0]}" 2>/dev/null || true
sleep 7
[[ "$(redis-cli exists vos_rs:test:sip_nodes:sip-edge-a)" == "0" ]]
CRASH_SUCCESS="$(run_calls 8 5266 'cccccccc-0000-4000-8000' 'caller-node-crashed')"
A_CRASH_INVITES="$(invite_count "${LOG_DIR}/sip-edge-a-failover.log")"
B_CRASH_INVITES="$(invite_count "${LOG_DIR}/sip-edge-b-failover.log")"

if [[ "${DRAIN_SUCCESS}" == "8" && "${A_DRAIN_INVITES}" == "0" && "${B_DRAIN_INVITES}" -gt 0 \
    && "${RESUME_SUCCESS}" == "16" && "${A_FINAL_INVITES}" -gt 0 \
    && "${B_FINAL_INVITES}" -gt "${B_DRAIN_INVITES}" && "${CRASH_SUCCESS}" == "8" \
    && "${A_CRASH_INVITES}" == "${A_FINAL_INVITES}" && "${B_CRASH_INVITES}" -gt "${B_FINAL_INVITES}" ]]; then
    printf 'SIP 集群故障场景通过：摘流=%s 通，恢复=%s 通，节点故障=%s 通；摘流期 A/B=%s/%s，恢复后=%s/%s，故障后=%s/%s INVITE\n' \
        "${DRAIN_SUCCESS}" "${RESUME_SUCCESS}" "${CRASH_SUCCESS}" \
        "${A_DRAIN_INVITES}" "${B_DRAIN_INVITES}" "${A_FINAL_INVITES}" "${B_FINAL_INVITES}" \
        "${A_CRASH_INVITES}" "${B_CRASH_INVITES}"
else
    printf 'SIP 集群故障场景失败：摘流=%s，恢复=%s，节点故障=%s；摘流期 A/B=%s/%s，恢复后=%s/%s，故障后=%s/%s INVITE\n' \
        "${DRAIN_SUCCESS}" "${RESUME_SUCCESS}" "${CRASH_SUCCESS}" \
        "${A_DRAIN_INVITES}" "${B_DRAIN_INVITES}" "${A_FINAL_INVITES}" "${B_FINAL_INVITES}" \
        "${A_CRASH_INVITES}" "${B_CRASH_INVITES}"
    exit 1
fi
