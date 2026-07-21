# tools — 测试与基准工具

> **SIPp 端到端测试 + 性能压测工具集**

## 这是什么？

`tools/` 是 vos-rs 项目的 **测试与基准工具** 目录。包含：
- SIPp 场景文件（端到端 SIP 测试）
- Python RTP 发送器（媒体测试）
- 性能压测脚本

## 目录结构

```
tools/
├── sipp/                      # SIPp 端到端测试
│   ├── scenarios/             # SIPp 场景文件 (.xml)
│   ├── configs/               # 测试配置 (.yaml)
│   ├── data/                  # 测试数据 (CSV/SQL)
│   ├── run_benchmark.sh       # 性能压测脚本
│   ├── run_business_flows.sh  # 业务流程测试
│   ├── full_flow_client.py    # 完整流程客户端
│   ├── wav_rtp_sender.py      # WAV → RTP 发送器
│   ├── rtp_sender.py          # RTP 包发送器
│   └── test_speech.wav        # 测试音频 (8kHz/16bit/mono)
├── benchmark/                 # 基准测试
│   ├── bench.py               # 基准测试脚本
│   └── scenarios/             # 基准场景
└── seed_extra_cdrs.py         # 批量 CDR 种子数据
```

## SIPp 场景文件

### 压测场景

| 场景 | 文件 | 用途 |
| :--- | :--- | :--- |
| REGISTER 压测 | `scenarios/bench_register.xml` | 纯信令注册压测 |
| INVITE 呼叫 | `scenarios/cps_caller.xml` | UAC 呼叫 + 5s 通话 + BYE |
| 网关 UAS | `scenarios/gateway_uas.xml` | 模拟网关接收呼叫 |
| 出局呼叫 | `scenarios/cps_outbound.xml` | 出局呼叫压测 |

### 业务场景

| 场景 | 文件 | 用途 |
| :--- | :--- | :--- |
| 分机注册 | `business_extension_register_uac.xml` | 分机注册流程 |
| 分机呼叫 | `business_extension_uac.xml` / `_uas.xml` | 分机间呼叫 |
| 接入中继 | `business_access_uac.xml` | 接入中继呼叫 |
| 出局中继 | `business_egress_inbound_uac.xml` | 出局中继入呼 |
| 网关故障 | `business_gateway_fail_uas.xml` | 网关故障切换 |
| SRTP | `caller_sdes_srtp.xml` | SDES-SRTP 加密通话 |
| 长通话 | `caller_longcall.xml` | 长时间通话测试 |

## 性能压测

### 快速压测

```bash
# 纯信令 REGISTER (10 CPS, 100 calls)
bash sipp/run_benchmark.sh register 10 100

# 纯信令 INVITE (50 CPS, 200 calls)
bash sipp/run_benchmark.sh invite 50 200

# 带媒体 INVITE (10 CPS, 100 calls)
bash sipp/run_benchmark.sh media 10 100
```

### 完整压测流程

详见 [../docs/development/PERFORMANCE_BENCHMARK.md](../docs/development/PERFORMANCE_BENCHMARK.md)。

### 业务流程测试

```bash
# 运行全部业务流程
cd sipp && bash run_business_flows.sh

# SIP 集群故障切换测试
cd sipp && bash run_sip_cluster_failover.sh
```

## RTP 媒体测试

### WAV → RTP 发送

```bash
# 发送 WAV 文件为 RTP 流
python3 sipp/wav_rtp_sender.py sipp/test_speech.wav 127.0.0.1 40000 50 1
# 参数: <wav_file> <target_ip> <target_port> [pps] [loop]
```

### 裸 RTP 发送

```bash
# 发送固定载荷 RTP
python3 sipp/rtp_sender.py 127.0.0.1 40000 50 1000
# 参数: <target_ip> <target_port> [pps] [count]
```

## 测试配置

`configs/` 下的 YAML 文件定义不同测试拓扑：

| 配置 | 用途 |
| :--- | :--- |
| `smoke.yaml` | 冒烟测试 |
| `full_flow.yaml` | 完整流程测试 |
| `performance.yaml` | 性能测试 |
| `business_flow.yaml` | 业务流程 |
| `sip_cluster_*.yaml` | SIP 集群测试 |
| `media_edge_*.yaml` | 媒体边缘测试 |
| `stun.yaml` | STUN 穿透测试 |

## 前置要求

- SIPp 3.7+ (`brew install sipp`)
- Python 3.8+
- 测试音频 `test_speech.wav`（8kHz/16-bit/mono）

## 相关文档

- 性能压测报告：[../docs/development/PERFORMANCE_BENCHMARK.md](../docs/development/PERFORMANCE_BENCHMARK.md)
- SIPp 业务场景：[../docs/development/SIPP_BUSINESS_SCENARIOS.md](../docs/development/SIPP_BUSINESS_SCENARIOS.md)
- 基准测试：[./benchmark/README.md](./benchmark/README.md)
