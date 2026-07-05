SHELL := /bin/bash
.DEFAULT_GOAL := help

# 如果本地存在 .env 文件，则加载其配置的环境变量
ifneq (,$(wildcard .env))
include .env
export
endif

# 定义常用的开发工具命令路径，如果环境变量中未设定，则使用默认值
CARGO ?= cargo
PYTHON ?= python3
SIPP_BIN ?= sipp

# 定义各类日志的输出目录
DEV_LOG_DIR ?= target/dev
SMOKE_LOG_DIR ?= target/sipp
FULL_FLOW_LOG_DIR ?= target/full-flow
PERF_LOG_DIR ?= target/sipp_perf

# 并发测试参数
PERF_TOTAL ?= 5000
PERF_RATE ?= 1000
PERF_CONC ?= 500
PERF_TIMEOUT ?= 60

.PHONY: help env fmt fmt-check check test clippy build quick verify smoke full-flow run-sip-edge run-cdr-worker logs clean perf perf-quick perf-all perf-report

# help: 显示 VOS-RS 所有可用的开发构建目标
help:
	@printf '%s\n' 'VOS-RS 开发构建目标 (make targets):'
	@printf '%s\n' '  make env             显示当前生效的本地开发环境变量及说明'
	@printf '%s\n' '  make fmt             格式化 Rust 代码'
	@printf '%s\n' '  make fmt-check       检查 Rust 代码格式是否规范'
	@printf '%s\n' '  make check           对整个工作区执行 cargo check 语法检查'
	@printf '%s\n' '  make test            运行 Rust 单元测试与集成测试'
	@printf '%s\n' '  make clippy          运行 clippy 静态分析 (拒绝任何警告级别错误)'
	@printf '%s\n' '  make build           构建编译 sip-edge 和 cdr-worker 服务'
	@printf '%s\n' '  make quick           快速验证：先检查格式，然后运行测试'
	@printf '%s\n' '  make smoke           运行基于 SIPp 的呼叫冒烟测试验证'
	@printf '%s\n' '  make full-flow       运行本地 SIP/RTP/RTCP/CDR 完整流程集成测试'
	@printf '%s\n' '  make verify          全面验证：运行 quick + smoke + full-flow'
	@printf '%s\n' '  make perf            运行 SIPp 并发性能测试 (默认 1000 CPS)'
	@printf '%s\n' '  make perf-quick      快速性能测试 (50/100/200 CPS，每级 1000 通)'
	@printf '%s\n' '  make perf-all        全级别性能测试 (50/100/200/500/800/1000/2000 CPS)'
	@printf '%s\n' '  make perf-report     生成性能测试报告 (带 PASS/FAIL 结果)'
	@printf '%s\n' '  make run-sip-edge    根据 .env 配置直接启动 sip-edge 边缘服务'
	@printf '%s\n' '  make run-cdr-worker  根据 .env 配置直接启动 cdr-worker 计费消费者服务'
	@printf '%s\n' '  make logs            显示当前的验证日志输出目录'

# env: 打印当前对 VOS-RS 生效的环境变量，并添加中文注释说明
env:
	@printf '%s\n' '当前生效的 VOS-RS 本地环境变量:'
	@printf '  %-32s %s\n' 'VOS_RS_SIP_UDP_BIND' "$${VOS_RS_SIP_UDP_BIND:-0.0.0.0:5060} # SIP 服务的 UDP 本地绑定监听地址"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_ADVERTISED_ADDR' "$${VOS_RS_SIP_ADVERTISED_ADDR:-127.0.0.1:5060} # SIP 外网通告播报地址（用于 SIP 报头 Contact/Via）"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_DEFAULT_GATEWAY' "$${VOS_RS_SIP_DEFAULT_GATEWAY:-<未设置>} # 未匹配到注册用户时，SIP 默认出局路由网关"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_BIND' "$${VOS_RS_SIP_TLS_BIND:-0.0.0.0:5061} # SIP TLS 本地绑定监听地址"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_CERT_PATH' "$${VOS_RS_SIP_TLS_CERT_PATH:-<未设置>} # 入站 SIP TLS 证书链 PEM 路径"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_KEY_PATH' "$${VOS_RS_SIP_TLS_KEY_PATH:-<未设置>} # 入站 SIP TLS 私钥 PEM 路径"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_CA_PATH' "$${VOS_RS_SIP_TLS_CA_PATH:-<系统 CA>} # 出站 SIP TLS 网关 CA PEM 路径"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_SERVER_NAME' "$${VOS_RS_SIP_TLS_SERVER_NAME:-<按目标 IP>} # 出站 SIP TLS 证书校验名称"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY' "$${VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY:-false} # 是否跳过出站 TLS 证书校验，仅限开发"
	@printf '  %-32s %s\n' 'VOS_RS_RTP_ADVERTISED_ADDR' "$${VOS_RS_RTP_ADVERTISED_ADDR:-127.0.0.1} # RTP 媒体中继服务外网通告播报 IP 地址"
	@printf '  %-32s %s\n' 'VOS_RS_RTP_PORT_MIN' "$${VOS_RS_RTP_PORT_MIN:-40000} # RTP 媒体端口段分配最小值"
	@printf '  %-32s %s\n' 'VOS_RS_RTP_PORT_MAX' "$${VOS_RS_RTP_PORT_MAX:-40100} # RTP 媒体端口段分配最大值"
	@printf '  %-32s %s\n' 'VOS_RS_RTP_SYMMETRIC_LEARNING' "$${VOS_RS_RTP_SYMMETRIC_LEARNING:-true} # 是否启用 RTP/RTCP 对称源地址自动学习机制"
	@printf '  %-32s %s\n' 'VOS_RS_RECORDING_ENABLED' "$${VOS_RS_RECORDING_ENABLED:-false} # 是否开启通话双声道 WAV 录音功能"
	@printf '  %-32s %s\n' 'VOS_RS_RECORDING_DIR' "$${VOS_RS_RECORDING_DIR:-target/recordings} # 通话录音文件保存根目录"
	@printf '  %-32s %s\n' 'VOS_RS_DATABASE_URL' "$${VOS_RS_DATABASE_URL:-<已禁用/未配置>} # PostgreSQL 数据库连接地址，用于直接写入 CDR"
	@printf '  %-32s %s\n' 'VOS_RS_SIP_AUTH_USERS' "$${VOS_RS_SIP_AUTH_USERS:-<未设置>} # 数据库为空时，用于自动种子填充的 SIP 用户列表 (格式如: 1001:secret,1002:secret2)"
	@printf '  %-32s %s\n' 'VOS_RS_NATS_URL' "$${VOS_RS_NATS_URL:-<已禁用/未配置>} # NATS MQ 连接地址，用于发布 CDR 事件"
	@printf '  %-32s %s\n' 'RUST_LOG' "$${RUST_LOG:-info} # Rust 程序的控制台日志级别"
	@printf '  %-32s %s\n' 'PERF_RATE' "$(PERF_RATE) # SIPp 并发测试目标 CPS"
	@printf '  %-32s %s\n' 'PERF_TOTAL' "$(PERF_TOTAL) # SIPp 并发测试总通话数"
	@printf '  %-32s %s\n' 'PERF_CONC' "$(PERF_CONC) # SIPp 并发测试最大并发数"

# fmt: 格式化 Rust 源代码
fmt:
	@$(CARGO) fmt

# fmt-check: 检查 Rust 源代码是否满足 rustfmt 规范
fmt-check:
	@$(CARGO) fmt --check

# check: 在整个工作区中对所有 target 运行语法和类型检查
check:
	@$(CARGO) check --workspace --all-targets

# test: 运行整个项目工作区的单元测试和集成测试
test:
	@$(CARGO) test

# clippy: 静态代码分析检查，将所有警告（warnings）作为错误拒绝
clippy:
	@$(CARGO) clippy --workspace --all-targets -- -D warnings

# build: 编译打包 sip-edge 和 cdr-worker 可执行程序
build:
	@$(CARGO) build -p sip-edge -p cdr-worker

# build-release: 编译 release 版本用于性能测试
build-release:
	@$(CARGO) build --release -p sip-edge

# quick: 快捷验证工具链 (代码检查 + 运行测试)
quick: fmt-check test

# verify: 全面流程集成验证 (快速验证 + SIPp 冒烟测试 + 完整本地呼叫流)
verify: quick smoke full-flow

# smoke: 执行基于 SIPp 模拟器的高级呼叫流冒烟测试
smoke:
	@LOG_DIR="$(SMOKE_LOG_DIR)" SIPP_BIN="$(SIPP_BIN)" tools/sipp/run_smoke.sh

# full-flow: 启动集成测试，模拟完整的主被叫注册、SIP 认证、媒体中继和 CDR 计费写入流程
full-flow:
	@LOG_DIR="$(FULL_FLOW_LOG_DIR)" PYTHON="$(PYTHON)" tools/full-flow/run_full_flow.sh

# perf: 运行 SIPp 并发性能测试 (默认 1000 CPS, 5000 通)
perf: build-release
	@bash tools/sipp/run_bench_final.sh

# perf-media: 运行带 RTP 媒体的并发性能测试
perf-media: build-release
	@bash tools/sipp/run_bench_media.sh

# perf-quick: 快速性能测试 (50/100/200 CPS，每级 1000 通)
perf-quick: build-release
	@bash tools/sipp/run_bench_final.sh

# perf-all: 全级别性能测试 (50/100/200/500/800/1000/2000 CPS)
perf-all: build-release
	@bash tools/sipp/run_bench_final.sh

# perf-report: 生成性能测试报告
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

# run-sip-edge: 构建并启动 sip-edge 服务
run-sip-edge: build
	@mkdir -p "$(DEV_LOG_DIR)"
	@printf '正在启动 sip-edge 服务，绑定 UDP 地址: %s\n' "$${VOS_RS_SIP_UDP_BIND:-0.0.0.0:5060}"
	@$(CARGO) run -p sip-edge

# run-cdr-worker: 构建并启动 cdr-worker 服务 (需要配置数据库环境变量)
run-cdr-worker: build
	@if [[ -z "$${VOS_RS_DATABASE_URL:-}" ]]; then printf '%s\n' '未找到 VOS_RS_DATABASE_URL 环境变量，cdr-worker 服务启动失败'; exit 2; fi
	@printf '正在启动 cdr-worker 服务，连接 NATS 服务: %s\n' "$${VOS_RS_NATS_URL:-nats://127.0.0.1:4222}"
	@$(CARGO) run -p cdr-worker

# logs: 打印当前所有测试输出的日志目录路径
logs:
	@printf 'SIPp 冒烟测试日志目录:     %s\n' "$(SMOKE_LOG_DIR)"
	@printf 'Full-flow 完整流集成日志:   %s\n' "$(FULL_FLOW_LOG_DIR)"
	@printf '并发性能测试日志目录:       %s\n' "$(PERF_LOG_DIR)"
	@printf '开发调试日志输出目录:       %s\n' "$(DEV_LOG_DIR)"

# clean: 清理项目编译缓存和中间生成文件
clean:
	@$(CARGO) clean
