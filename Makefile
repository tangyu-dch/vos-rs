SHELL := /bin/bash
.DEFAULT_GOAL := help

ifneq (,$(wildcard .env))
include .env
export
endif

CARGO ?= cargo
PYTHON ?= python3
SIPP_BIN ?= sipp
CONFIG_FILE ?= $(CURDIR)/config.yaml
SMOKE_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/smoke.yaml
FULL_FLOW_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/full_flow.yaml
FULL_FLOW_REMOTE_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/full_flow_remote.yaml
FULL_FLOW_UDS_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/full_flow_uds.yaml
FULL_FLOW_CLUSTER_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/full_flow_cluster.yaml
FULL_FLOW_HYBRID_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/full_flow_hybrid.yaml
SIP_CLUSTER_EDGE_A_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/sip_cluster_edge_a.yaml
SIP_CLUSTER_EDGE_B_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/sip_cluster_edge_b.yaml
SIP_CLUSTER_ROUTER_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/sip_cluster_router.yaml
MEDIA_EDGE_A_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/media_edge_a.yaml
MEDIA_EDGE_B_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/media_edge_b.yaml
STUN_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/stun.yaml
PERF_CONFIG_FILE ?= $(CURDIR)/tools/sipp/configs/performance.yaml

DEV_LOG_DIR ?= target/dev
SMOKE_LOG_DIR ?= target/sipp
FULL_FLOW_LOG_DIR ?= target/full-flow
PERF_LOG_DIR ?= target/sipp_bench

.PHONY: help env fmt fmt-check check lint test test-unit test-integration test-bench bench-xdp \
        clippy build build-release build-debug quick verify smoke full-flow full-flow-remote full-flow-uds full-flow-cluster full-flow-hybrid full-flow-sip-cluster full-flow-sip-cluster-failover \
        web-lint web-test web-build web-verify \
        perf perf-media perf-quick perf-all perf-report bench bench-concurrency \
        bench-concurrency-quick bench-concurrency-media bench-concurrency-recording doc \
        run-sip-router run-sip-edge run-media-edge run-api-server run-cdr-worker cluster-check logs clean test-stun

help:
	@printf '\n  VOS-RS 开发构建目标\n'
	@printf '  ─────────────────────────────────────────────\n'
	@printf '  开发工作流:\n'
	@printf '    make env             显示统一配置文件路径\n'
	@printf '    make fmt             格式化代码\n'
	@printf '    make fmt-check       检查格式\n'
	@printf '    make check           语法检查\n'
	@printf '    make lint            clippy + fmt-check\n'
	@printf '    make quick           fmt-check + test\n'
	@printf '    make verify          quick + smoke + full-flow\n'
	@printf '    make web-verify      前端 lint + test + build\n'
	@printf '  测试:\n'
	@printf '    make test            运行所有测试\n'
	@printf '    make test-unit       仅单元测试（快速）\n'
	@printf '    make test-integration 仅集成测试\n'
	@printf '    make test-bench      仅基准测试\n'
	@printf '    make test-stun       STUN 公网地址发现测试\n'
	@printf '    make full-flow-cluster 双 media-edge 调度与录音测试\n'
	@printf '    make full-flow-hybrid  本地 + 远程媒体混合调度测试\n'
	@printf '    make full-flow-sip-cluster 双 sip-edge + 原生 sip-router 测试\n'
	@printf '    make full-flow-sip-cluster-failover SIP 节点摘流与恢复测试\n'
	@printf '    make bench           运行 Criterion 基准测试\n'
	@printf '    make bench-xdp       运行 eBPF/XDP 旁路引擎极限压测 (400万+ ops/s)\n'
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
	@printf '    make bench-concurrency          持续并发全场景测试\n'
	@printf '    make bench-concurrency-quick    快速持续并发测试\n'
	@printf '    make bench-concurrency-media    RTP 中继并发测试\n'
	@printf '    make bench-concurrency-recording 录音并发测试\n'
	@printf '  运行:\n'
	@printf '    make run-sip-router  启动原生 SIP 集群入口\n'
	@printf '    make run-sip-edge    启动 sip-edge\n'
	@printf '    make run-media-edge  启动独立 media-edge\n'
	@printf '    make run-api-server  启动 api-server\n'
	@printf '    make run-cdr-worker  启动 cdr-worker\n'
	@printf '    make cluster-check   校验 SIP/媒体集群配置和编译状态\n'
	@printf '    CONFIG_FILE=...      指定 config.yaml（默认仓库根目录）\n'
	@printf '  其他:\n'
	@printf '    make doc             生成文档\n'
	@printf '    make logs            显示日志目录\n'
	@printf '    make clean           清理构建产物\n'

env:
	@test -f "$(CONFIG_FILE)" || { printf '配置文件不存在: %s\n' "$(CONFIG_FILE)"; exit 2; }
	@printf '统一配置文件: %s\n' "$(CONFIG_FILE)"
	@awk '/^logging:/{found=1; next} found && /filter:/{gsub(/^[[:space:]]*filter:[[:space:]]*|"/, ""); print "日志过滤级别: " $$0; exit}' "$(CONFIG_FILE)"
	@printf '配置顶级分组:\n'
	@sed -n 's/^\([a-z_][a-z_]*\):.*/  - \1/p' "$(CONFIG_FILE)"

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
	@printf '文档已生成: target/doc/index.html\n'

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
	@VOS_RS_CONFIG_FILE="$(STUN_CONFIG_FILE)" \
		timeout 10 $(CARGO) run -p sip-edge 2>&1 | grep -E "STUN.*discovered|STUN.*failed|STUN.*all retries"
	@printf 'STUN test complete.\n'

bench:
	@$(CARGO) bench --workspace

bench-xdp:
	@$(CARGO) bench -p media-edge --bench xdp_media_engine_stress

quick: fmt-check test

# ─── 前端验证 ──────────────────────────────────────────

web-lint:
	@cd web && npm run lint

web-test:
	@cd web && npm test

web-build:
	@cd web && npm run build

web-verify: web-lint web-test web-build

# ─── 构建 ──────────────────────────────────────────────

build: build-debug

build-debug:
	@$(CARGO) build -p sip-router -p sip-edge -p media-edge -p api-server -p cdr-worker

build-release:
	@$(CARGO) build --release -p sip-router -p sip-edge -p media-edge -p api-server -p cdr-worker

cluster-check:
	@test -f "$(CONFIG_FILE)" || { printf '配置文件不存在: %s\n' "$(CONFIG_FILE)"; exit 2; }
	@$(CARGO) test -p sip-edge cluster::
	@$(CARGO) test -p api-server media_cluster::
	@$(CARGO) test -p sip-router
	@$(CARGO) check -p sip-router -p sip-edge -p media-edge -p api-server
	@cd web && npm run build

# ─── 集成验证 ──────────────────────────────────────────

verify: quick smoke full-flow

smoke:
	@printf 'Smoke test: running SIPp basic call flow...\n'
	@$(CARGO) build --release -p sip-edge 2>/dev/null
	@mkdir -p "$(SMOKE_LOG_DIR)"
	@VOS_RS_CONFIG_FILE="$(SMOKE_CONFIG_FILE)" \
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
	kill $$EDGE_PID 2>/dev/null; wait $$EDGE_PID 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; \
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
	@rm -f target/test_recordings/*.wav
	@VOS_RS_CONFIG_FILE="$(FULL_FLOW_CONFIG_FILE)" \
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
	kill $$EDGE_PID 2>/dev/null; wait $$EDGE_PID 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; pkill -9 -f wav_rtp_sender 2>/dev/null; \
	if [ "$$SUCC" = "1" ] && [ "$$WAV_COUNT" -ge "1" ]; then \
		printf 'FULL-FLOW PASS: %s call succeeded, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
	else \
		printf 'FULL-FLOW FAIL: %s call, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
		exit 1; \
	fi

full-flow-remote:
	@printf 'Full-flow Remote test: decoupled SIP signaling + remote RTP media + remote recording...\n'
	@$(CARGO) build --release -p sip-edge -p media-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)" target/test_recordings
	@rm -f target/test_recordings/*.wav
	@VOS_RS_CONFIG_FILE="$(FULL_FLOW_REMOTE_CONFIG_FILE)" \
	target/release/media-edge >"$(FULL_FLOW_LOG_DIR)/media-edge.log" 2>&1 & \
	MEDIA_EDGE_PID=$$!; sleep 2; \
	VOS_RS_CONFIG_FILE="$(FULL_FLOW_REMOTE_CONFIG_FILE)" \
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
	kill $$EDGE_PID $$MEDIA_EDGE_PID 2>/dev/null; wait $$EDGE_PID $$MEDIA_EDGE_PID 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; pkill -9 -f wav_rtp_sender 2>/dev/null; \
	if [ "$$SUCC" = "1" ] && [ "$$WAV_COUNT" -ge "1" ]; then \
		printf 'FULL-FLOW REMOTE PASS: %s call succeeded, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
	else \
		printf 'FULL-FLOW REMOTE FAIL: %s call, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
		exit 1; \
	fi

full-flow-uds:
	@printf 'Full-flow UDS test: decoupled SIP signaling + UDS IPC + remote RTP media + remote recording...\n'
	@$(CARGO) build --release -p sip-edge -p media-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)" target/test_recordings
	@rm -f target/test_recordings/*.wav
	@rm -f /tmp/media-edge-test.sock
	@VOS_RS_CONFIG_FILE="$(FULL_FLOW_UDS_CONFIG_FILE)" \
	target/release/media-edge >"$(FULL_FLOW_LOG_DIR)/media-edge.log" 2>&1 & \
	MEDIA_EDGE_PID=$$!; sleep 2; \
	VOS_RS_CONFIG_FILE="$(FULL_FLOW_UDS_CONFIG_FILE)" \
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
	kill $$EDGE_PID $$MEDIA_EDGE_PID 2>/dev/null; wait $$EDGE_PID $$MEDIA_EDGE_PID 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; pkill -9 -f wav_rtp_sender 2>/dev/null; rm -f /tmp/media-edge-test.sock; \
	if [ "$$SUCC" = "1" ] && [ "$$WAV_COUNT" -ge "1" ]; then \
		printf 'FULL-FLOW UDS PASS: %s call succeeded, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
	else \
		printf 'FULL-FLOW UDS FAIL: %s call, %s WAV files\n' "$$SUCC" "$$WAV_COUNT"; \
		exit 1; \
	fi

full-flow-cluster:
	@printf 'Full-flow Cluster test: two media-edge nodes + Call-ID affinity + recording...\n'
	@$(CARGO) build --release -p sip-edge -p media-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)" target/test_recordings
	@rm -f target/test_recordings/*.wav /tmp/media-edge-a.sock /tmp/media-edge-b.sock
	@VOS_RS_CONFIG_FILE="$(MEDIA_EDGE_A_CONFIG_FILE)" \
	target/release/media-edge >"$(FULL_FLOW_LOG_DIR)/media-edge-a.log" 2>&1 & \
	MEDIA_A_PID=$$!; \
	VOS_RS_CONFIG_FILE="$(MEDIA_EDGE_B_CONFIG_FILE)" \
	target/release/media-edge >"$(FULL_FLOW_LOG_DIR)/media-edge-b.log" 2>&1 & \
	MEDIA_B_PID=$$!; sleep 2; \
	VOS_RS_CONFIG_FILE="$(FULL_FLOW_CLUSTER_CONFIG_FILE)" \
	target/release/sip-edge >"$(FULL_FLOW_LOG_DIR)/edge-cluster.log" 2>&1 & \
	EDGE_PID=$$!; sleep 4; \
	RTP_PIDS=""; \
	for PORT in 40000 40002 41000 41002; do \
		python3 tools/sipp/wav_rtp_sender.py tools/sipp/test_speech.wav 127.0.0.1 $$PORT 50 4 >/dev/null 2>&1 & \
		RTP_PIDS="$$RTP_PIDS $$!"; \
	done; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/gateway_longcall.xml \
		-i 127.0.0.1 -p 5170 -m 2 -aa -nostdin >/dev/null 2>&1 & \
	sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/caller_longcall.xml \
		-i 127.0.0.1 -p 5164 -s 13800138000 -m 2 -r 1 -l 2 -aa -nostdin \
		> "$(FULL_FLOW_LOG_DIR)/caller-cluster.log" 2>&1; \
	sleep 8; \
	SUCC=$$(awk -F'|' '/Successful call/{gsub(/ /,"",$$3); print $$3}' "$(FULL_FLOW_LOG_DIR)/caller-cluster.log"); \
	WAV_COUNT=$$(ls target/test_recordings/*.wav 2>/dev/null | wc -l); \
	A_ALLOC=$$(grep -c 'allocated media relay endpoint' "$(FULL_FLOW_LOG_DIR)/media-edge-a.log" || true); \
	B_ALLOC=$$(grep -c 'allocated media relay endpoint' "$(FULL_FLOW_LOG_DIR)/media-edge-b.log" || true); \
	kill $$EDGE_PID $$MEDIA_A_PID $$MEDIA_B_PID 2>/dev/null; \
	wait $$EDGE_PID $$MEDIA_A_PID $$MEDIA_B_PID 2>/dev/null || true; \
	kill $$RTP_PIDS 2>/dev/null; wait $$RTP_PIDS 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; \
	rm -f /tmp/media-edge-a.sock /tmp/media-edge-b.sock; \
	if [ "$$SUCC" = "2" ] && [ "$$WAV_COUNT" -ge "2" ] && [ "$$A_ALLOC" -ge "2" ] && [ "$$B_ALLOC" -ge "2" ]; then \
		printf 'FULL-FLOW CLUSTER PASS: %s calls, %s WAV, node-a=%s, node-b=%s allocations\n' "$$SUCC" "$$WAV_COUNT" "$$A_ALLOC" "$$B_ALLOC"; \
	else \
		printf 'FULL-FLOW CLUSTER FAIL: %s calls, %s WAV, node-a=%s, node-b=%s allocations\n' "$$SUCC" "$$WAV_COUNT" "$$A_ALLOC" "$$B_ALLOC"; \
		exit 1; \
	fi

full-flow-hybrid:
	@printf 'Full-flow Hybrid test: local media + remote media-edge scheduling...\n'
	@$(CARGO) build --release -p sip-edge -p media-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)" target/test_recordings
	@rm -f target/test_recordings/*.wav /tmp/media-edge-a.sock
	@VOS_RS_CONFIG_FILE="$(MEDIA_EDGE_A_CONFIG_FILE)" \
	target/release/media-edge >"$(FULL_FLOW_LOG_DIR)/media-edge-hybrid.log" 2>&1 & \
	MEDIA_PID=$$!; sleep 2; \
	VOS_RS_CONFIG_FILE="$(FULL_FLOW_HYBRID_CONFIG_FILE)" \
	target/release/sip-edge >"$(FULL_FLOW_LOG_DIR)/edge-hybrid.log" 2>&1 & \
	EDGE_PID=$$!; sleep 4; \
	RTP_PIDS=""; \
	for PORT in 40000 40002 41000 41002; do \
		python3 tools/sipp/wav_rtp_sender.py tools/sipp/test_speech.wav 127.0.0.1 $$PORT 50 4 >/dev/null 2>&1 & \
		RTP_PIDS="$$RTP_PIDS $$!"; \
	done; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/gateway_longcall.xml \
		-i 127.0.0.1 -p 5170 -m 2 -aa -nostdin >/dev/null 2>&1 & \
	sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5160 -sf tools/sipp/scenarios/caller_longcall.xml \
		-i 127.0.0.1 -p 5164 -s 13800138000 -m 2 -r 1 -l 2 -aa -nostdin \
		> "$(FULL_FLOW_LOG_DIR)/caller-hybrid.log" 2>&1; \
	sleep 8; \
	SUCC=$$(awk -F'|' '/Successful call/{gsub(/ /,"",$$3); print $$3}' "$(FULL_FLOW_LOG_DIR)/caller-hybrid.log"); \
	WAV_COUNT=$$(ls target/test_recordings/*.wav 2>/dev/null | wc -l); \
	LOCAL_ALLOC=$$(grep -c 'allocated media relay endpoint (lock-free)' "$(FULL_FLOW_LOG_DIR)/edge-hybrid.log" || true); \
	REMOTE_ALLOC=$$(grep -c 'allocated media relay endpoint' "$(FULL_FLOW_LOG_DIR)/media-edge-hybrid.log" || true); \
	kill $$EDGE_PID $$MEDIA_PID 2>/dev/null; wait $$EDGE_PID $$MEDIA_PID 2>/dev/null || true; \
	kill $$RTP_PIDS 2>/dev/null; wait $$RTP_PIDS 2>/dev/null || true; pkill -9 -f sipp 2>/dev/null; \
	rm -f /tmp/media-edge-a.sock; \
	if [ "$$SUCC" = "2" ] && [ "$$WAV_COUNT" -ge "2" ] && [ "$$LOCAL_ALLOC" -ge "2" ] && [ "$$REMOTE_ALLOC" -ge "2" ]; then \
		printf 'FULL-FLOW HYBRID PASS: %s calls, %s WAV, local=%s, remote=%s allocations\n' "$$SUCC" "$$WAV_COUNT" "$$LOCAL_ALLOC" "$$REMOTE_ALLOC"; \
	else \
		printf 'FULL-FLOW HYBRID FAIL: %s calls, %s WAV, local=%s, remote=%s allocations\n' "$$SUCC" "$$WAV_COUNT" "$$LOCAL_ALLOC" "$$REMOTE_ALLOC"; \
		exit 1; \
	fi

full-flow-sip-cluster:
	@printf 'Full-flow SIP Cluster test: sip-router + two sip-edge nodes...\n'
	@command -v redis-cli >/dev/null || { printf '缺少 redis-cli\n'; exit 2; }
	@redis-cli ping >/dev/null || { printf 'Redis 6379 不可用\n'; exit 2; }
	@nc -z 127.0.0.1 4222 || { printf 'NATS 4222 不可用\n'; exit 2; }
	@$(CARGO) build --release -p sip-router -p sip-edge 2>/dev/null
	@mkdir -p "$(FULL_FLOW_LOG_DIR)"
	@for KEY in $$(redis-cli --scan --pattern 'vos_rs:test:sip_nodes:*'); do redis-cli del "$$KEY" >/dev/null; done
	@for KEY in $$(redis-cli --scan --pattern 'vos_rs:cluster:sip_dialog_routes:*'); do redis-cli del "$$KEY" >/dev/null; done
	@VOS_RS_CONFIG_FILE="$(SIP_CLUSTER_EDGE_A_CONFIG_FILE)" \
	target/release/sip-edge >"$(FULL_FLOW_LOG_DIR)/sip-edge-a.log" 2>&1 & \
	EDGE_A_PID=$$!; \
	VOS_RS_CONFIG_FILE="$(SIP_CLUSTER_EDGE_B_CONFIG_FILE)" \
	target/release/sip-edge >"$(FULL_FLOW_LOG_DIR)/sip-edge-b.log" 2>&1 & \
	EDGE_B_PID=$$!; sleep 3; \
	VOS_RS_CONFIG_FILE="$(SIP_CLUSTER_ROUTER_CONFIG_FILE)" \
	target/release/sip-router >"$(FULL_FLOW_LOG_DIR)/sip-router.log" 2>&1 & \
	ROUTER_PID=$$!; sleep 3; \
	NODE_COUNT=$$(redis-cli --scan --pattern 'vos_rs:test:sip_nodes:*' | wc -l | tr -d ' '); \
	$(SIPP_BIN) 127.0.0.1:5261 -sf tools/sipp/scenarios/gateway_longcall.xml \
		-i 127.0.0.1 -p 5271 -m 16 -aa -nostdin -timeout 60s -trace_msg \
		-message_file "$(FULL_FLOW_LOG_DIR)/gateway-a-messages.log" >/dev/null 2>&1 & \
	GATEWAY_A_PID=$$!; \
	$(SIPP_BIN) 127.0.0.1:5262 -sf tools/sipp/scenarios/gateway_longcall.xml \
		-i 127.0.0.1 -p 5272 -m 16 -aa -nostdin -timeout 60s -trace_msg \
		-message_file "$(FULL_FLOW_LOG_DIR)/gateway-b-messages.log" >/dev/null 2>&1 & \
	GATEWAY_B_PID=$$!; sleep 1; \
	$(SIPP_BIN) 127.0.0.1:5260 -sf tools/sipp/scenarios/caller_cluster_longcall.xml \
		-i 127.0.0.1 -p 5264 -cid_str '00000000-0000-4000-8000-%u@vos-rs' \
		-s 13800138000 -m 16 -r 4 -l 4 -aa -nostdin -timeout 20s -timeout_error -trace_err \
		-trace_msg -message_file "$(FULL_FLOW_LOG_DIR)/caller-sip-cluster-messages.log" \
		-error_file "$(FULL_FLOW_LOG_DIR)/caller-sip-cluster-errors.log" \
		>"$(FULL_FLOW_LOG_DIR)/caller-sip-cluster.log" 2>&1; \
	SUCC=$$(awk -F'|' '/Successful call/{gsub(/ /,"",$$3); print $$3}' "$(FULL_FLOW_LOG_DIR)/caller-sip-cluster.log"); \
	A_INVITES=$$(grep 'received SIP request' "$(FULL_FLOW_LOG_DIR)/sip-edge-a.log" | grep -E -c 'method.*INVITE' || true); \
	B_INVITES=$$(grep 'received SIP request' "$(FULL_FLOW_LOG_DIR)/sip-edge-b.log" | grep -E -c 'method.*INVITE' || true); \
	kill $$ROUTER_PID $$EDGE_A_PID $$EDGE_B_PID $$GATEWAY_A_PID $$GATEWAY_B_PID 2>/dev/null; \
	wait $$ROUTER_PID $$EDGE_A_PID $$EDGE_B_PID $$GATEWAY_A_PID $$GATEWAY_B_PID 2>/dev/null || true; \
	pkill -9 -f sipp 2>/dev/null; \
	for KEY in $$(redis-cli --scan --pattern 'vos_rs:test:sip_nodes:*'); do redis-cli del "$$KEY" >/dev/null; done; \
	for KEY in $$(redis-cli --scan --pattern 'vos_rs:cluster:sip_dialog_routes:*'); do redis-cli del "$$KEY" >/dev/null; done; \
	if [ "$$NODE_COUNT" = "2" ] && [ "$$SUCC" = "16" ] && [ "$$A_INVITES" -gt "0" ] && [ "$$B_INVITES" -gt "0" ]; then \
		printf 'FULL-FLOW SIP CLUSTER PASS: nodes=%s, calls=%s, edge-a=%s, edge-b=%s INVITE\n' "$$NODE_COUNT" "$$SUCC" "$$A_INVITES" "$$B_INVITES"; \
	else \
		printf 'FULL-FLOW SIP CLUSTER FAIL: nodes=%s, calls=%s, edge-a=%s, edge-b=%s INVITE\n' "$$NODE_COUNT" "$$SUCC" "$$A_INVITES" "$$B_INVITES"; \
		exit 1; \
	fi

full-flow-sip-cluster-failover:
	@printf 'SIP 集群摘流与恢复故障场景测试...\n'
	@$(CARGO) build --release -p sip-router -p sip-edge 2>/dev/null
	@$(CARGO) test -p sip-router test_two_router_instances_share_dialog_owner_in_redis -- --ignored
	@$(CARGO) test -p sip-edge test_inter_node_egress_waits_for_matching_ack -- --ignored
	@FULL_FLOW_LOG_DIR="$(FULL_FLOW_LOG_DIR)" \
	SIP_CLUSTER_EDGE_A_CONFIG_FILE="$(SIP_CLUSTER_EDGE_A_CONFIG_FILE)" \
	SIP_CLUSTER_EDGE_B_CONFIG_FILE="$(SIP_CLUSTER_EDGE_B_CONFIG_FILE)" \
	SIP_CLUSTER_ROUTER_CONFIG_FILE="$(SIP_CLUSTER_ROUTER_CONFIG_FILE)" \
	SIPP_BIN="$(SIPP_BIN)" tools/sipp/run_sip_cluster_failover.sh

# ─── 性能测试 ──────────────────────────────────────────

perf: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario signaling --total 1000 --cps 200 --duration 35 --sustain 30

perf-media: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario media_relay --total 500 --cps 100 --duration 35 --sustain 30

perf-quick: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario all --total 50 --cps 10 --duration 15 --sustain 10

perf-all: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario all --total 500 --cps 100 --duration 35 --sustain 30

perf-report:
	@printf '\n========================================\n'
	@printf '       VOS-RS 性能测试报告\n'
	@printf '========================================\n'
	@latest_run=$$(ls -td target/benchmark/* 2>/dev/null | head -n 1); \
	if [ -n "$$latest_run" ]; then \
		for report in $$latest_run/*/report.md; do \
			if [ -f "$$report" ]; then \
				cat "$$report"; \
				printf '\n----------------------------------------\n'; \
			fi; \
			done; \
	else \
		printf '未找到任何测试报告。\n'; \
	fi

bench-concurrency: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario all --total 500 --cps 100 --duration 35 --sustain 30

bench-concurrency-quick: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario signaling --total 50 --cps 10 --duration 15 --sustain 10

bench-concurrency-media: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario media_relay --total 500 --cps 100 --duration 35 --sustain 30

bench-concurrency-recording: build-release
	@$(PYTHON) tools/benchmark/bench.py --scenario recording --total 500 --cps 100 --duration 35 --sustain 30

# ─── 运行 ──────────────────────────────────────────────

run-sip-router: build-debug
	@VOS_RS_CONFIG_FILE="$(CONFIG_FILE)" $(CARGO) run -p sip-router

run-sip-edge: build-debug
	@mkdir -p "$(DEV_LOG_DIR)"
	@VOS_RS_CONFIG_FILE="$(CONFIG_FILE)" $(CARGO) run -p sip-edge

run-media-edge: build-debug
	@VOS_RS_CONFIG_FILE="$(CONFIG_FILE)" $(CARGO) run -p media-edge

run-api-server:
	@VOS_RS_CONFIG_FILE="$(CONFIG_FILE)" $(CARGO) run -p api-server

run-cdr-worker: build-debug
	@VOS_RS_CONFIG_FILE="$(CONFIG_FILE)" $(CARGO) run -p cdr-worker

logs:
	@printf 'SIPp 冒烟测试日志:     %s\n' "$(SMOKE_LOG_DIR)"
	@printf '完整流集成测试日志:   %s\n' "$(FULL_FLOW_LOG_DIR)"
	@printf '性能测试日志:         %s\n' "$(PERF_LOG_DIR)"
	@printf '开发调试日志:         %s\n' "$(DEV_LOG_DIR)"

clean:
	@$(CARGO) clean
	@rm -rf target/test_recordings target/sipp target/sipp_bench target/sipp_bench_media target/full-flow
	@rm -f *.log *.aiff
