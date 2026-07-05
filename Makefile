SHELL := /bin/bash
.DEFAULT_GOAL := help

ifneq (,$(wildcard .env))
include .env
export
endif

CARGO ?= cargo
PYTHON ?= python3
SIPP_BIN ?= sipp

DEV_LOG_DIR ?= target/dev
SMOKE_LOG_DIR ?= target/sipp
FULL_FLOW_LOG_DIR ?= target/full-flow
PERF_LOG_DIR ?= target/sipp_perf

PERF_TOTAL ?= 5000
PERF_RATE ?= 1000
PERF_CONC ?= 500
PERF_TIMEOUT ?= 60

.PHONY: help env fmt fmt-check check lint test test-unit test-integration test-bench \
        clippy build build-release build-debug quick verify smoke full-flow \
        perf perf-media perf-quick perf-all perf-report bench doc \
        run-sip-edge run-cdr-worker logs clean test-stun

help:
	@printf '\n  VOS-RS 开发构建目标\n'
	@printf '  ─────────────────────────────────────────────\n'
	@printf '  开发工作流:\n'
	@printf '    make env             显示当前环境变量\n'
	@printf '    make fmt             格式化代码\n'
	@printf '    make fmt-check       检查格式\n'
	@printf '    make check           语法检查\n'
	@printf '    make lint            clippy + fmt-check\n'
	@printf '    make quick           fmt-check + test\n'
	@printf '    make verify          quick + smoke + full-flow\n'
	@printf '  测试:\n'
	@printf '    make test            运行所有测试\n'
	@printf '    make test-unit       仅单元测试（快速）\n'
	@printf '    make test-integration 仅集成测试\n'
	@printf '    make test-bench      仅基准测试\n'
	@printf '    make test-stun       STUN 公网地址发现测试\n'
	@printf '    make bench           运行 Criterion 基准测试\n'
	@printf '  构建:\n'
	@printf '    make build           debug 构建\n'
	@printf '    make build-release   release 构建\n'
	@printf '    make build-debug     debug 构建（显式）\n'
	@printf '  性能测试:\n'
	@printf '    make perf            SIPp 并发测试 (1000 CPS)\n'
	@printf '    make perf-quick      快速测试 (50/100/200 CPS)\n'
	@printf '    make perf-all        全级别测试\n'
	@printf '    make perf-media      带 RTP 媒体的性能测试\n'
	@printf '    make perf-report     生成测试报告\n'
	@printf '  运行:\n'
	@printf '    make run-sip-edge    启动 sip-edge\n'
	@printf '    make run-cdr-worker  启动 cdr-worker\n'
	@printf '  其他:\n'
	@printf '    make doc             生成文档\n'
	@printf '    make logs            显示日志目录\n'
	@printf '    make clean           清理构建产物\n'

env:
	@printf '%s\n' '当前生效的 VOS-RS 环境变量:'
	@printf '  %-38s %s\n' 'VOS_RS_SIP_UDP_BIND'          "$${VOS_RS_SIP_UDP_BIND:-0.0.0.0:5060}"
	@printf '  %-38s %s\n' 'VOS_RS_SIP_ADVERTISED_ADDR'   "$${VOS_RS_SIP_ADVERTISED_ADDR:-127.0.0.1:5060}"
	@printf '  %-38s %s\n' 'VOS_RS_SIP_DEFAULT_GATEWAY'   "$${VOS_RS_SIP_DEFAULT_GATEWAY:-<未设置>}"
	@printf '  %-38s %s\n' 'VOS_RS_SIP_AUTH_USERS'        "$${VOS_RS_SIP_AUTH_USERS:-<未设置>}"
	@printf '  %-38s %s\n' 'VOS_RS_RTP_ADVERTISED_ADDR'   "$${VOS_RS_RTP_ADVERTISED_ADDR:-127.0.0.1}"
	@printf '  %-38s %s\n' 'VOS_RS_RTP_PORT_MIN'          "$${VOS_RS_RTP_PORT_MIN:-40000}"
	@printf '  %-38s %s\n' 'VOS_RS_RTP_PORT_MAX'          "$${VOS_RS_RTP_PORT_MAX:-40100}"
	@printf '  %-38s %s\n' 'VOS_RS_RECORDING_ENABLED'     "$${VOS_RS_RECORDING_ENABLED:-false}"
	@printf '  %-38s %s\n' 'VOS_RS_RECORDING_DIR'         "$${VOS_RS_RECORDING_DIR:-target/recordings}"
	@printf '  %-38s %s\n' 'VOS_RS_DATABASE_URL'          "$${VOS_RS_DATABASE_URL:-<未设置>}"
	@printf '  %-38s %s\n' 'VOS_RS_NATS_URL'              "$${VOS_RS_NATS_URL:-<未设置>}"
	@printf '  %-38s %s\n' 'VOS_RS_STUN_SERVER'           "$${VOS_RS_STUN_SERVER:-<未设置>}"
	@printf '  %-38s %s\n' 'VOS_RS_UPNP_ENABLED'          "$${VOS_RS_UPNP_ENABLED:-false}"
	@printf '  %-38s %s\n' 'RUST_LOG'                     "$${RUST_LOG:-info}"

fmt:
	@$(CARGO) fmt

fmt-check:
	@$(CARGO) fmt --check

check:
	@$(CARGO) check --workspace --all-targets

lint: clippy fmt-check

clippy:
	@$(CARGO) clippy --workspace --all-targets -- -D warnings

doc:
	@$(CARGO) doc --workspace --no-deps
	@printf '文档已生成: target/doc/%s/index.html\n' $(CARGO)

# ─── 测试 ──────────────────────────────────────────────

test:
	@$(CARGO) test --workspace

test-unit:
	@$(CARGO) test --workspace --lib --test '*' 2>/dev/null || $(CARGO) test --workspace

test-integration:
	@$(CARGO) test --workspace --test '*' -- --nocapture

test-bench:
	@$(CARGO) test --workspace --benches

test-stun:
	@printf 'Testing STUN discovery...\n'
	@RUST_LOG=info VOS_RS_STUN_SERVER=stun.l.google.com:19302 \
		VOS_RS_SIP_UDP_BIND=127.0.0.1:5160 VOS_RS_SIP_ADVERTISED_ADDR=127.0.0.1:5160 \
		timeout 10 $(CARGO) run -p sip-edge 2>&1 | grep -E "STUN.*discovered|STUN.*failed|STUN.*all retries"
	@printf 'STUN test complete.\n'

bench:
	@$(CARGO) bench --workspace

quick: fmt-check test

# ─── 构建 ──────────────────────────────────────────────

build: build-debug

build-debug:
	@$(CARGO) build -p sip-edge -p cdr-worker

build-release:
	@$(CARGO) build --release -p sip-edge -p cdr-worker

# ─── 集成验证 ──────────────────────────────────────────

verify: quick smoke full-flow

smoke:
	@printf 'Smoke test: running SIPp basic call flow...\n'
	@$(CARGO) build --release -p sip-edge 2>/dev/null
	@mkdir -p "$(SMOKE_LOG_DIR)"
	@VOS_RS_SIP_UDP_BIND="127.0.0.1:5160" \
	VOS_RS_SIP_DEFAULT_GATEWAY="127.0.0.1:5170" \
	VOS_RS_SIP_ADVERTISED_ADDR="127.0.0.1:5160" \
	VOS_RS_RTP_PORT_MIN=40000 VOS_RS_RTP_PORT_MAX=60010 \
	VOS_RS_SBC_ALLOW=127.0.0.1 \
	target/release/sip-edge >"$(SMOKE_LOG_DIR)/edge.log" 2>&1 & \
	EDGE_PID=$$!; sleep 2; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/gateway_uas.xml \
		-i 127.0.0.1 -p 5170 -m 1 -aa -nostdin >/dev/null 2>&1 & \
	sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/caller_uac.xml \
		-i 127.0.0.1 -p 5164 -s 13800138000 -m 1 -r 1 -l 1 -aa -nostdin \
		> "$(SMOKE_LOG_DIR)/caller.log" 2>&1; \
	SUCC=$$(awk -F'|' '/Successful call/{gsub(/ /,"",$$3); print $$3}' "$(SMOKE_LOG_DIR)/caller.log"); \
	FAIL=$$(awk -F'|' '/Failed call/{gsub(/ /,"",$$3); print $$3}' "$(SMOKE_LOG_DIR)/caller.log"); \
	kill $$EDGE_PID 2>/dev/null; pkill -9 -f sipp 2>/dev/null; \
	if [ "$$SUCC" = "1" ] && [ "$$FAIL" = "0" ]; then \
		printf 'SMOKE PASS: %s succeeded, %s failed\n' "$$SUCC" "$$FAIL"; \
	else \
		printf 'SMOKE FAIL: %s succeeded, %s failed\n' "$$SUCC" "$$FAIL"; \
		exit 1; \
	fi

full-flow:
	@printf 'Full-flow test: SIP signaling + RTP media + recording...\n'
	@$(CARGO) build --release -p sip-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)" target/test_recordings
	@VOS_RS_SIP_UDP_BIND="127.0.0.1:5160" \
	VOS_RS_SIP_DEFAULT_GATEWAY="127.0.0.1:5170" \
	VOS_RS_SIP_ADVERTISED_ADDR="127.0.0.1:5160" \
	VOS_RS_RTP_PORT_MIN=40000 VOS_RS_RTP_PORT_MAX=60010 \
	VOS_RS_SBC_ALLOW=127.0.0.1 VOS_RS_SBC_LIMIT_CAPACITY=1000000 \
	VOS_RS_SBC_LIMIT_FILL_RATE=100000 VOS_RS_SBC_MAX_CONCURRENCY=10000 \
	VOS_RS_RECORDING_ENABLED=true VOS_RS_RECORDING_DIR=$$(pwd)/target/test_recordings \
	RUST_LOG=info \
	target/release/sip-edge >"$(FULL_FLOW_LOG_DIR)/edge.log" 2>&1 & \
	EDGE_PID=$$!; sleep 3; \
	python3 tools/sipp/wav_rtp_sender.py tools/sipp/test_speech.wav 127.0.0.1 40000 50 2 >/dev/null 2>&1 & \
	python3 tools/sipp/wav_rtp_sender.py tools/sipp/test_speech.wav 127.0.0.1 40002 50 2 >/dev/null 2>&1 & \
	sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/gateway_longcall.xml \
		-i 127.0.0.1 -p 5170 -m 1 -aa -nostdin >/dev/null 2>&1 & \
	sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/caller_longcall.xml \
		-i 127.0.0.1 -p 5164 -s 13800138000 -m 1 -r 1 -l 1 -aa -nostdin \
		> "$(FULL_FLOW_LOG_DIR)/caller.log" 2>&1; \
	sleep 8; \
	SUCC=$$(awk -F'|' '/Successful call/{gsub(/ /,"",$$3); print $$3}' "$(FULL_FLOW_LOG_DIR)/caller.log"); \
	WAV_COUNT=$$(ls target/test_recordings/*.wav 2>/dev/null | wc -l); \
	kill $$EDGE_PID 2>/dev/null; pkill -9 -f sipp 2>/dev/null; pkill -9 -f wav_rtp_sender 2>/dev/null; \
	if [ "$$SUCC" = "1" ] && [ "$$WAV_COUNT" -ge "1" ]; then \
		printf 'FULL-FLOW PASS: %s call succeeded, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
	else \
		printf 'FULL-FLOW FAIL: %s call, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
		exit 1; \
	fi

# ─── 性能测试 ──────────────────────────────────────────

perf: build-release
	@bash tools/sipp/run_bench_final.sh

perf-media: build-release
	@bash tools/sipp/run_bench_media.sh

perf-quick: build-release
	@bash tools/sipp/run_bench_final.sh

perf-all: build-release
	@bash tools/sipp/run_bench_final.sh

perf-report: perf-all
	@printf '\n========================================\n'
	@printf '       VOS-RS 性能测试报告\n'
	@printf '========================================\n'
	@printf '%-10s | %-8s | %-8s | %-8s | %s\n' "目标CPS" "实际CPS" "成功" "失败" "日志"
	@printf '%-10s-+-%-8s-+-%-8s-+-%-8s-+-%s\n' "----------" "--------" "--------" "--------" "--------"
	@for rate in 50 100 200 500 800 1000 2000; do \
		SUCC=$$(awk -F'|' '/Successful call/{print $$3}' "$(PERF_LOG_DIR)/$${rate}cps_caller.log" 2>/dev/null | tr -d ' '); \
		FAIL=$$(awk -F'|' '/Failed call/{print $$3}' "$(PERF_LOG_DIR)/$${rate}cps_caller.log" 2>/dev/null | tr -d ' '); \
		CPS=$$(awk -F'|' '/Call Rate/{print $$3}' "$(PERF_LOG_DIR)/$${rate}cps_caller.log" 2>/dev/null | tr -d ' cps' | tr -d ' '); \
		[ -z "$$SUCC" ] && SUCC="超时"; \
		[ -z "$$FAIL" ] && FAIL="-"; \
		[ -z "$$CPS" ] && CPS="-"; \
		printf '%-10s | %-8s | %-8s | %-8s | %s\n' "$${rate}cps" "$$CPS" "$$SUCC" "$$FAIL" "$(PERF_LOG_DIR)/$${rate}cps_caller.log"; \
	done
	@printf '\n日志目录: %s\n' "$(PERF_LOG_DIR)"
	@pkill -9 -f sip-edge 2>/dev/null; pkill -9 -f sipp 2>/dev/null

# ─── 运行 ──────────────────────────────────────────────

run-sip-edge: build-debug
	@mkdir -p "$(DEV_LOG_DIR)"
	@$(CARGO) run -p sip-edge

run-cdr-worker: build-debug
	@if [[ -z "$${VOS_RS_DATABASE_URL:-}" ]]; then printf 'Error: VOS_RS_DATABASE_URL not set\n'; exit 2; fi
	@$(CARGO) run -p cdr-worker

logs:
	@printf 'SIPp 冒烟测试日志:     %s\n' "$(SMOKE_LOG_DIR)"
	@printf '完整流集成测试日志:   %s\n' "$(FULL_FLOW_LOG_DIR)"
	@printf '性能测试日志:         %s\n' "$(PERF_LOG_DIR)"
	@printf '开发调试日志:         %s\n' "$(DEV_LOG_DIR)"

clean:
	@$(CARGO) clean
	@rm -rf target/test_recordings target/sipp target/sipp_bench target/sipp_bench_media target/full-flow
	@rm -f *.log *.aiff
