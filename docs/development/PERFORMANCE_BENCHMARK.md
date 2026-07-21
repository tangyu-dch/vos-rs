# 性能压测报告

> VOS-RS SIP 信令与媒体性能基准测试

## 1. 测试环境

### 1.1 硬件

| 项目 | 规格 |
| :--- | :--- |
| 机型 | Apple MacBook Pro (M1 Pro, 2021) |
| CPU | Apple M1 Pro (ARM64) |
| 物理核心 | 8 性能核 + 2 能效核 = 10 核 |
| 逻辑核心 | 10 |
| 内存 | 16 GB unified memory |
| 磁盘 | Apple SSD (NVMe) |

### 1.2 软件

| 项目 | 版本 |
| :--- | :--- |
| 操作系统 | macOS 26.5.1 (Darwin 25.5.0 ARM64) |
| Rust | stable (edition 2021) |
| SIPp | 3.7.7-TLS-PCAP-SHA256 |
| PostgreSQL | 15.x (OrbStack 容器) |
| NATS | 2.x (OrbStack 容器) |
| Redis | 7.x (OrbStack 容器) |

### 1.3 服务构建

| 服务 | 构建模式 | 二进制路径 |
| :--- | :--- | :--- |
| sip-edge | release (optimized) | `target/release/sip-edge` |
| api-server | debug | `target/debug/api-server` |
| cdr-worker | debug | `target/debug/cdr-worker` |

### 1.4 sip-edge 配置（压测专用）

压测时需关闭以下配置以消除非信令处理开销：

```yaml
sip_edge:
  routing:
    gateway_health_checks_enabled: false  # 关闭网关 OPTIONS 健康探测
  billing:
    balance_enforcement_enabled: false    # 关闭余额检查（避免零余额拦截）
  recording:
    enabled: false                         # 关闭录音
  webhooks:
    enabled: false                         # 关闭 Webhook 推送
```

## 2. 压测方法论

### 2.1 测试场景

| 场景 | 描述 | 场景文件 |
| :--- | :--- | :--- |
| **纯信令 REGISTER** | SIP 注册请求，测 SIP 解析 + DB 写入 | `scenarios/bench_register.xml` |
| **纯信令 INVITE** | SIP 呼叫建立 + 释放，测 B2BUA 完整流程 | `scenarios/cps_caller.xml` |
| **带媒体 INVITE** | SIP 呼叫 + RTP 流转发，测媒体中继 | `scenarios/cps_caller.xml` + `-rtp_stream` |

### 2.2 拓扑

```text
纯信令 REGISTER:
  SIPp UAC ──REGISTER──→ sip-edge (5060) ──→ PostgreSQL

纯信令 INVITE:
  SIPp UAC ──INVITE──→ sip-edge (5060) ──INVITE──→ SIPp UAS (5190)
                       (B2BUA 转发)                  (模拟网关)

带媒体 INVITE:
  SIPp UAC ──INVITE+RTP──→ sip-edge (5060) ──INVITE+RTP──→ SIPp UAS (5190)
                           (B2BUA + RTP Relay)
```

### 2.3 执行脚本

```bash
# 纯信令 REGISTER 压测
bash tools/sipp/run_benchmark.sh register [CPS] [COUNT]

# 纯信令 INVITE 压测 (自动启动 UAS)
bash tools/sipp/run_benchmark.sh invite [CPS] [COUNT]

# 带媒体 INVITE 压测 (自动启动 UAS)
bash tools/sipp/run_benchmark.sh media [CPS] [COUNT]
```

### 2.4 CPS 档位

| 档位 | CPS | 通话数 | 单通话时长 | 预期峰值并发 |
| :--- | :--- | :--- | :--- | :--- |
| 低载 | 10 | 100 | 5s | 50 |
| 中载 | 50 | 200 | 5s | 250 |
| 高载 | 100 | 300 | 5s | 500 |
| 极限 | 200 | 400 | 5s | 1000 |

## 3. 测试结果

### 3.1 当前状态：sip-edge 请求处理异常

**问题**：sip-edge 收到 SIP 请求（REGISTER/INVITE）后不返回响应，SIPp 等待 32 秒后超时。

**现象**：
- sip-edge 日志显示 `received SIP request method=REGISTER/INVITE`
- 之后无任何响应日志（无 100/180/200/401/404）
- SIPp 重传 5 次后超时

**排查发现**：
1. 网关 OPTIONS 健康探测全部超时（SIPp UAS 不处理 OPTIONS）→ 已关闭
2. 余额检查可能拦截 → 已关闭 `balance_enforcement_enabled`
3. 关闭上述配置后，sip-edge 仍不响应 → 需深入调试

**待排查方向**：
1. sip-edge 的 SIP 请求处理线程是否被阻塞（DB 查询死锁？）
2. sip_users 表是否有 1001 用户记录（REGISTER 需要查用户表）
3. auth 模块是否在等待认证信息导致卡住
4. tokio runtime 线程饥饿（定时器任务占用过多 CPU？）

### 3.2 预期性能目标

修复请求处理问题后，预期性能（基于架构设计）：

| 场景 | 目标 CPS | 目标并发 | 备注 |
| :--- | :--- | :--- | :--- |
| 纯信令 REGISTER | 2000+ | N/A | 只测 SIP 解析 + DB 写入 |
| 纯信令 INVITE | 1000+ | 5000+ | B2BUA 完整流程 |
| 带媒体 INVITE | 500+ | 2000+ | 含 RTP 中继 |

### 3.3 性能基线（待补充）

> 以下表格在 sip-edge 请求处理问题修复后填充实际数据。

#### 纯信令 REGISTER

| CPS | 通话数 | 成功 | 失败 | 实际 CPS | 平均响应时间 | 备注 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| 10 | 100 | - | - | - | - | 待测 |
| 50 | 200 | - | - | - | - | 待测 |
| 100 | 300 | - | - | - | - | 待测 |
| 200 | 400 | - | - | - | - | 待测 |

#### 纯信令 INVITE

| CPS | 通话数 | 成功 | 失败 | 实际 CPS | 平均建立时间 | 平均通话时长 | 备注 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| 10 | 100 | - | - | - | - | - | 待测 |
| 50 | 200 | - | - | - | - | - | 待测 |
| 100 | 300 | - | - | - | - | - | 待测 |
| 200 | 400 | - | - | - | - | - | 待测 |

#### 带媒体 INVITE

| CPS | 通话数 | 成功 | 失败 | 实际 CPS | 平均建立时间 | RTP 丢包率 | 备注 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| 10 | 100 | - | - | - | - | - | 待测 |
| 50 | 200 | - | - | - | - | - | 待测 |
| 100 | 300 | - | - | - | - | - | 待测 |
| 200 | 400 | - | - | - | - | - | 待测 |

## 4. 压测操作步骤

### 4.1 环境准备

```bash
# 1. 确保 PostgreSQL / NATS / Redis 已启动
pg_isready -h 127.0.0.1 -p 5432
nc -z 127.0.0.1 4222  # NATS
nc -z 127.0.0.1 6379  # Redis

# 2. release 构建 sip-edge
cargo build --release -p sip-edge

# 3. 修改 config.yaml 关闭非必要开销 (见 1.4 节)

# 4. 启动 sip-edge
nohup target/release/sip-edge > /tmp/sip-edge.log 2>&1 &

# 5. 验证 sip-edge 监听
lsof -iUDP:5060 -P -n
```

### 4.2 执行压测

```bash
# 纯信令 REGISTER: 10/50/100/200 CPS
for CPS in 10 50 100 200; do
  bash tools/sipp/run_benchmark.sh register $CPS 200
done

# 纯信令 INVITE: 10/50/100/200 CPS
for CPS in 10 50 100 200; do
  bash tools/sipp/run_benchmark.sh invite $CPS 200
done

# 带媒体 INVITE: 10/50/100 CPS
for CPS in 10 50 100; do
  bash tools/sipp/run_benchmark.sh media $CPS 100
done
```

### 4.3 收集结果

SIPp 输出关键指标：

| 指标 | 含义 |
| :--- | :--- |
| `Outgoing calls created` | 发起的通话总数 |
| `Successful call` | 成功完成的通话数 |
| `Failed call` | 失败的通话数 |
| `Call Rate (cumulative)` | 实际平均 CPS |
| `Response Time 1` | INVITE → 200 OK 平均建立时间 |
| `Call Length` | 平均通话时长 |

### 4.4 监控

压测期间同步监控：

```bash
# sip-edge CPU/内存
top -pid $(pgrep -f "target/release/sip-edge")

# sip-edge 管理 API
curl http://127.0.0.1:8082/manage/active-calls | jq .

# PostgreSQL 连接数
psql -c "SELECT count(*) FROM pg_stat_activity WHERE datname='vos_rs';"

# Prometheus 指标
curl http://127.0.0.1:8082/metrics | grep -E "vos_rs_calls|vos_rs_cps"
```

## 5. 已知问题与排查

### 5.1 sip-edge 不响应 SIP 请求

**症状**：sip-edge 收到 REGISTER/INVITE 后不返回任何响应，SIPp 32 秒后超时。

**排查步骤**：

1. 检查 sip-edge 日志是否有 `received SIP request` 记录
   ```bash
   tail -f /tmp/sip-edge.log | grep "received SIP request"
   ```
2. 检查是否为认证拦截（REGISTER 需要认证）
   - 查 `sip_users` 表是否有对应用户
   - 临时关闭认证：设置 `VOS_RS_AUTH_ENABLED=false`
3. 检查是否为余额拦截
   - 临时关闭：`config.yaml` 中 `billing.balance_enforcement_enabled: false`
4. 检查是否为网关健康检查拦截
   - 临时关闭：`config.yaml` 中 `routing.gateway_health_checks_enabled: false`
5. 检查 tokio runtime 是否线程饥饿
   - 用 `top -pid` 观察 CPU 使用率
   - 检查是否有阻塞操作（std::sync::Mutex + sync I/O）

### 5.2 网关 OPTIONS 探测超时

**症状**：所有网关的 OPTIONS 健康探测超时，网关被标记为不健康。

**原因**：SIPp UAS 只处理 INVITE，不响应 OPTIONS。

**解决**：压测时关闭健康检查 `gateway_health_checks_enabled: false`。

### 5.3 零余额拦截

**症状**：INVITE 收到 403 Forbidden，原因是主叫账户余额为 0。

**解决**：
- 压测时关闭：`balance_enforcement_enabled: false`
- 或给测试账户充值：`UPDATE billing_accounts SET balance = 10000 WHERE username = '1001';`

## 6. 相关资源

- 压测脚本：[../../tools/sipp/run_benchmark.sh](../../tools/sipp/run_benchmark.sh)
- REGISTER 场景：[../../tools/sipp/scenarios/bench_register.xml](../../tools/sipp/scenarios/bench_register.xml)
- INVITE 场景：[../../tools/sipp/scenarios/cps_caller.xml](../../tools/sipp/scenarios/cps_caller.xml)
- UAS 场景：[../../tools/sipp/scenarios/gateway_uas.xml](../../tools/sipp/scenarios/gateway_uas.xml)
- SIPp 业务场景：[./SIPP_BUSINESS_SCENARIOS.md](./SIPP_BUSINESS_SCENARIOS.md)
- 架构分析：[../architecture/VOS_RS_ARCHITECTURE_ANALYSIS.md](../architecture/VOS_RS_ARCHITECTURE_ANALYSIS.md)
