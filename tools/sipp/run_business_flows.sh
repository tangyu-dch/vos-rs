#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCENARIO_DIR="${ROOT_DIR}/tools/sipp/scenarios"
DATA_DIR="${ROOT_DIR}/tools/sipp/data"
LOG_DIR="${BUSINESS_LOG_DIR:-${ROOT_DIR}/target/sipp-business}"
CONFIG_FILE="${BUSINESS_CONFIG_FILE:-${ROOT_DIR}/tools/sipp/configs/business_flow.yaml}"
SEED_FILE="${BUSINESS_SEED_FILE:-${ROOT_DIR}/tools/sipp/data/business_seed.sql}"
SIPP_BIN="${SIPP_BIN:-sipp}"
EDGE_BIN="${EDGE_BIN:-${ROOT_DIR}/target/debug/sip-edge}"
EDGE_HOST="${BUSINESS_EDGE_HOST:-127.0.0.1}"
EDGE_PORT="${BUSINESS_EDGE_PORT:-5160}"
MANAGE_URL="${BUSINESS_MANAGE_URL:-http://127.0.0.1:5182/health}"
DATABASE_URL="${VOS_RS_DATABASE_URL:-postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs}"

EDGE_PID=""
LAST_GATEWAY_PID=""
POOL_BLOCKER_ACTIVE=0
declare -a CHILD_PIDS=()
declare -a PASSED=()

usage() {
    cat <<'EOF'
用法: tools/sipp/run_business_flows.sh [all|passthrough|fixed|pool|owner-failure|extension-out|extension-in]

环境变量:
  BUSINESS_CONFIG_FILE  独立 SIP Edge 配置，默认 tools/sipp/configs/business_flow.yaml
  BUSINESS_SEED_FILE    隔离业务数据 SQL，默认 tools/sipp/data/business_seed.sql
  BUSINESS_SKIP_BUILD=1 跳过 cargo build -p sip-edge
  BUSINESS_SKIP_SEED=1  跳过数据库 seed
EOF
}

log() { printf '[SIPp 业务验证] %s\n' "$*"; }
fail() { printf '[SIPp 业务验证] 失败: %s\n' "$*" >&2; exit 1; }

cleanup() {
    local status=$?
    for pid in "${CHILD_PIDS[@]:-}"; do
        kill "${pid}" 2>/dev/null || true
    done
    if [[ -n "${EDGE_PID}" ]]; then
        kill "${EDGE_PID}" 2>/dev/null || true
        wait "${EDGE_PID}" 2>/dev/null || true
    fi
    [[ "${POOL_BLOCKER_ACTIVE}" == "1" ]] && clear_pool_blocker
    if [[ ${status} -ne 0 ]]; then
        printf '[SIPp 业务验证] 日志目录: %s\n' "${LOG_DIR}" >&2
    fi
    exit "${status}"
}
trap cleanup EXIT INT TERM

require_tools() {
    command -v "${SIPP_BIN}" >/dev/null || fail "找不到 SIPp: ${SIPP_BIN}"
    command -v curl >/dev/null || fail "找不到 curl"
    command -v redis-cli >/dev/null || fail "找不到 redis-cli"
    if [[ "${BUSINESS_SKIP_SEED:-0}" != "1" ]]; then
        command -v psql >/dev/null || fail "找不到 psql"
        [[ -f "${SEED_FILE}" ]] || fail "缺少业务 seed: ${SEED_FILE}"
    fi
    [[ -f "${CONFIG_FILE}" ]] || fail "缺少业务配置: ${CONFIG_FILE}"
}

kill_port_occupants() {
    local port pid
    for port in 5160 5180 5182 5183 5190 5191; do
        while read -r pid; do
            [[ -z "${pid}" || "${pid}" == "$$" ]] && continue
            log "端口 ${port} 被 PID ${pid} 占用，结束该进程"
            kill "${pid}" 2>/dev/null || true
            sleep 1
            kill -9 "${pid}" 2>/dev/null || true
        done < <(lsof -tiTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true)
        while read -r pid; do
            [[ -z "${pid}" || "${pid}" == "$$" ]] && continue
            log "UDP 端口 ${port} 被 PID ${pid} 占用，结束该进程"
            kill "${pid}" 2>/dev/null || true
            sleep 1
            kill -9 "${pid}" 2>/dev/null || true
        done < <(lsof -tiUDP:"${port}" 2>/dev/null || true)
    done
}

seed_business_data() {
    [[ "${BUSINESS_SKIP_SEED:-0}" == "1" ]] && return
    log "写入 sipp-* 隔离中继、号码和策略"
    psql "${DATABASE_URL}" -f "${SEED_FILE}" >"${LOG_DIR}/seed.log" 2>&1
}

prepare_pool_blocker() {
    local expires lease_value
    expires=$(( $(date +%s) + 300 ))
    lease_value=$'861380020101\x1fsipp-egress'
    redis-cli HSET 'vos_rs:{resource-leases}:calls' sipp-capacity-blocker "${lease_value}" >/dev/null
    redis-cli ZADD 'vos_rs:{resource-leases}:call-expiry' "${expires}" sipp-capacity-blocker >/dev/null
    redis-cli ZADD 'vos_rs:{resource-leases}:number:861380020101' "${expires}" sipp-capacity-blocker >/dev/null
    redis-cli ZADD 'vos_rs:{resource-leases}:gateway:sipp-egress' "${expires}" sipp-capacity-blocker >/dev/null
    POOL_BLOCKER_ACTIVE=1
}

clear_pool_blocker() {
    command -v redis-cli >/dev/null 2>&1 || return
    redis-cli HDEL 'vos_rs:{resource-leases}:calls' sipp-capacity-blocker >/dev/null 2>&1 || true
    redis-cli ZREM 'vos_rs:{resource-leases}:call-expiry' sipp-capacity-blocker >/dev/null 2>&1 || true
    redis-cli ZREM 'vos_rs:{resource-leases}:number:861380020101' sipp-capacity-blocker >/dev/null 2>&1 || true
    redis-cli ZREM 'vos_rs:{resource-leases}:gateway:sipp-egress' sipp-capacity-blocker >/dev/null 2>&1 || true
    POOL_BLOCKER_ACTIVE=0
}

start_edge() {
    kill_port_occupants
    if [[ "${BUSINESS_SKIP_BUILD:-0}" != "1" ]]; then
        log "构建 sip-edge"
        cargo build -p sip-edge >"${LOG_DIR}/build.log" 2>&1
    fi
    [[ -x "${EDGE_BIN}" ]] || fail "sip-edge 不可执行: ${EDGE_BIN}"
    log "启动独立 sip-edge ${EDGE_HOST}:${EDGE_PORT}"
    VOS_RS_CONFIG_FILE="${CONFIG_FILE}" "${EDGE_BIN}" >"${LOG_DIR}/edge.log" 2>&1 &
    EDGE_PID=$!
    for _ in {1..40}; do
        if curl -fsS "${MANAGE_URL}" >/dev/null 2>&1; then
            return
        fi
        kill -0 "${EDGE_PID}" 2>/dev/null || fail "sip-edge 启动失败，请查看 edge.log"
        sleep 0.25
    done
    fail "sip-edge 健康检查超时: ${MANAGE_URL}"
}

wait_for_process() {
    local pid=$1 name=$2
    if ! wait "${pid}"; then
        fail "${name} SIPp 断言失败"
    fi
}

stop_process() {
    local pid=$1
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
}

assert_egress_message() {
    local name=$1 expected_file=$2 caller callee message_file
    IFS=';' read -r caller callee < <(tail -n 1 "${DATA_DIR}/${expected_file}")
    message_file="${LOG_DIR}/${name}-egress-messages.log"
    grep -Eq "INVITE sip:${callee}@" "${message_file}" \
        || fail "${name} 落地 Request-URI 未使用预期被叫 ${callee}"
    grep -Eq "From:.*sip:${caller}@" "${message_file}" \
        || fail "${name} 落地主叫未按策略变换为 ${caller}"
}

start_gateway() {
    local name=$1 ip=$2 port=$3 expected=$4 scenario=${5:-business_gateway_uas.xml}
    local log_prefix="${LOG_DIR}/${name}"
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/${scenario}" \
        -inf "${DATA_DIR}/${expected}" -i "${ip}" -p "${port}" -l 1 \
        -timeout 15 -aa -nostdin -trace_err -trace_msg \
        -error_file "${log_prefix}-errors.log" -message_file "${log_prefix}-messages.log" \
        >"${log_prefix}.log" 2>&1 &
    LAST_GATEWAY_PID=$!
    CHILD_PIDS+=("${LAST_GATEWAY_PID}")
}

run_access_case() {
    local name=$1 source_ip=$2 source_port=$3 input=$4 expected=$5
    log "场景 ${name}: ${source_ip}:${source_port} -> sipp-egress"
    local gateway_pid
    start_gateway "${name}-egress" 127.0.0.1 5190 "${expected}"
    gateway_pid=${LAST_GATEWAY_PID}
    sleep 0.4
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_access_uac.xml" \
        -inf "${DATA_DIR}/${input}" -i "${source_ip}" -p "${source_port}" -m 1 -l 1 \
        -timeout 15 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/${name}-errors.log" -message_file "${LOG_DIR}/${name}-messages.log" \
        >"${LOG_DIR}/${name}.log" 2>&1 || fail "${name} 接入侧呼叫失败"
    stop_process "${gateway_pid}"
    assert_egress_message "${name}" "${expected}"
    PASSED+=("${name}")
}

run_owner_failure() {
    log "场景 owner-failure: 号码归属落地 503 后必须终止，不得跨中继冒用号码"
    local failed_pid sentinel_pid
    start_gateway owner-failure-primary-egress 127.0.0.1 5191 business_expected_failover.csv business_gateway_fail_uas.xml
    failed_pid=${LAST_GATEWAY_PID}
    python3 "${ROOT_DIR}/tools/sipp/assert_no_sip.py" 127.0.0.1 5190 --timeout 3 \
        --output "${LOG_DIR}/owner-failure-unexpected-sip.log" &
    sentinel_pid=$!
    CHILD_PIDS+=("${sentinel_pid}")
    sleep 0.4
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_access_rejected_uac.xml" \
        -inf "${DATA_DIR}/business_access_failover.csv" -i 127.0.0.1 -p 5167 -m 1 -l 1 \
        -timeout 15 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/owner-failure-errors.log" -message_file "${LOG_DIR}/owner-failure-messages.log" \
        >"${LOG_DIR}/owner-failure.log" 2>&1 || fail "owner 失败未返回预期 503"
    stop_process "${failed_pid}"
    assert_egress_message owner-failure-primary business_expected_failover.csv
    wait_for_process "${sentinel_pid}" "跨中继禁止断言"
    PASSED+=("owner-failure")
}

run_extension_out() {
    log "场景 extension-out: 分机 Digest REGISTER 后呼出"
    local gateway_pid
    start_gateway extension-out-egress 127.0.0.1 5190 business_expected_extension_outbound.csv
    gateway_pid=${LAST_GATEWAY_PID}
    sleep 0.4
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_extension_uac.xml" \
        -inf "${DATA_DIR}/business_extension_outbound.csv" -i 127.0.0.1 -p 5180 -m 1 -l 1 \
        -timeout 20 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/extension-out-errors.log" -message_file "${LOG_DIR}/extension-out-messages.log" \
        >"${LOG_DIR}/extension-out.log" 2>&1 || fail "分机呼出失败"
    stop_process "${gateway_pid}"
    assert_egress_message extension-out business_expected_extension_outbound.csv
    PASSED+=("extension-out")
}

run_extension_in() {
    local registered_user ignored_password expected_caller
    log "场景 extension-in: 落地中继呼入 DID 并投递已注册分机"
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_extension_register_uac.xml" \
        -inf "${DATA_DIR}/business_extension_inbound.csv" -i 127.0.0.1 -p 5180 -m 1 -l 1 \
        -timeout 10 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/extension-in-register-errors.log" \
        -message_file "${LOG_DIR}/extension-in-register-messages.log" \
        >"${LOG_DIR}/extension-in-register.log" 2>&1 || fail "分机 Digest 注册失败"
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_extension_uas.xml" \
        -inf "${DATA_DIR}/business_extension_inbound.csv" -i 127.0.0.1 -p 5180 -m 1 -l 1 \
        -timeout 20 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/extension-in-uas-errors.log" -message_file "${LOG_DIR}/extension-in-uas-messages.log" \
        >"${LOG_DIR}/extension-in-uas.log" 2>&1 &
    local extension_pid=$!
    CHILD_PIDS+=("${extension_pid}")
    sleep 1
    "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_egress_inbound_uac.xml" \
        -inf "${DATA_DIR}/business_egress_inbound.csv" -i 127.0.0.1 -p 5190 -m 1 -l 1 \
        -timeout 20 -aa -nostdin -trace_err -trace_msg \
        -error_file "${LOG_DIR}/extension-in-uac-errors.log" -message_file "${LOG_DIR}/extension-in-uac-messages.log" \
        >"${LOG_DIR}/extension-in-uac.log" 2>&1 || fail "落地中继呼入 DID 失败"
    wait_for_process "${extension_pid}" "分机呼入接听侧"
    IFS=';' read -r registered_user ignored_password expected_caller \
        < <(tail -n 1 "${DATA_DIR}/business_extension_inbound.csv")
    grep -Eq "INVITE sip:${registered_user}@" "${LOG_DIR}/extension-in-uas-messages.log" \
        || fail "DID 未投递到预期分机 ${registered_user}"
    grep -Eq "From:.*sip:${expected_caller}@" "${LOG_DIR}/extension-in-uas-messages.log" \
        || fail "DID 呼入主叫未透传为 ${expected_caller}"
    PASSED+=("extension-in")
}

main() {
    local selected=${1:-all}
    [[ "${selected}" == "-h" || "${selected}" == "--help" ]] && { usage; exit 0; }
    mkdir -p "${LOG_DIR}"
    require_tools
    seed_business_data
    start_edge
    case "${selected}" in
        all)
            run_access_case passthrough 127.0.0.1 5164 business_access_passthrough.csv business_expected_passthrough.csv
            run_access_case fixed 127.0.0.1 5165 business_access_fixed.csv business_expected_fixed.csv
            log "场景 pool: 第一号码满载，第二号码及其 owner 落地应成功"
            local pool_gateway_pid
            prepare_pool_blocker
            start_gateway pool-egress 127.0.0.1 5191 business_expected_pool.csv
            pool_gateway_pid=${LAST_GATEWAY_PID}
            sleep 0.4
            "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_access_uac.xml" \
                -inf "${DATA_DIR}/business_access_pool.csv" -i 127.0.0.1 -p 5166 -m 1 -l 1 \
                -timeout 15 -aa -nostdin -trace_err -trace_msg \
                -error_file "${LOG_DIR}/pool-errors.log" -message_file "${LOG_DIR}/pool-messages.log" \
                >"${LOG_DIR}/pool.log" 2>&1 || fail "pool 容量回退呼叫失败"
            stop_process "${pool_gateway_pid}"
            assert_egress_message pool business_expected_pool.csv
            clear_pool_blocker
            PASSED+=("pool")
            run_extension_out
            run_extension_in
            run_owner_failure
            ;;
        passthrough) run_access_case passthrough 127.0.0.1 5164 business_access_passthrough.csv business_expected_passthrough.csv ;;
        fixed) run_access_case fixed 127.0.0.1 5165 business_access_fixed.csv business_expected_fixed.csv ;;
        pool)
            prepare_pool_blocker
            start_gateway pool-egress 127.0.0.1 5191 business_expected_pool.csv
            pool_gateway_pid=${LAST_GATEWAY_PID}
            sleep 0.4
            "${SIPP_BIN}" "${EDGE_HOST}:${EDGE_PORT}" -sf "${SCENARIO_DIR}/business_access_uac.xml" \
                -inf "${DATA_DIR}/business_access_pool.csv" -i 127.0.0.1 -p 5166 -m 1 -l 1 \
                -timeout 15 -aa -nostdin >"${LOG_DIR}/pool.log" 2>&1 || fail "pool 容量回退呼叫失败"
            stop_process "${pool_gateway_pid}"
            assert_egress_message pool business_expected_pool.csv
            clear_pool_blocker
            PASSED+=("pool")
            ;;
        owner-failure) run_owner_failure ;;
        extension-out) run_extension_out ;;
        extension-in) run_extension_in ;;
        *) usage; fail "未知场景: ${selected}" ;;
    esac
    log "全部通过: ${PASSED[*]}"
    log "信令日志: ${LOG_DIR}"
}

main "$@"
