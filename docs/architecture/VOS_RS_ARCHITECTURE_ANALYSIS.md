# VOS-RS 项目全面逻辑梳理与昆石 VOS 对比分析

> 文档生成时间: 2026-07-08  
> 项目版本: vos-rs 0.1  
> 总代码量: ~25,850 行 Rust + ~15,000 行 TypeScript/React

---

## 目录

1. [项目架构总览](#1-项目架构总览)
2. [SIP 信令处理流程](#2-sip-信令处理流程)
3. [媒体/RTP 处理](#3-媒体rtp-处理)
4. [路由与网关管理](#4-路由与网关管理)
5. [CDR 与计费系统](#5-cdr-与计费系统)
6. [录音系统](#6-录音系统)
7. [SBC 安全模块](#7-sbc-安全模块)
8. [认证与鉴权](#8-认证与鉴权)
9. [拓扑隐藏](#9-拓扑隐藏)
10. [NAT 穿透](#10-nat-穿透)
11. [管理 API 与前端](#11-管理-api-与前端)
12. [数据库设计](#12-数据库设计)
13. [与昆石 VOS 对比分析](#13-与昆石-vos-对比分析)
14. [不合理项与优化建议](#14-不合理项与优化建议)

---

## 1. 项目架构总览

### 1.1 系统架构图

```
┌─────────────────────────────────────────────────────────────┐
│                    Web 管理界面 (React)                       │
│  Dashboard │ ActiveCalls │ CDR │ Users │ Gateways │ Routes  │
│  Numbers │ Registrations │ Reports │ Rates │ Accounts       │
└──────────────────────────┬──────────────────────────────────┘
                           │ HTTP REST
┌──────────────────────────▼──────────────────────────────────┐
│                    api-server (Axum)                          │
│  30+ REST 端点 │ PostgreSQL │ 录音存储                        │
└──────────────────────────┬──────────────────────────────────┘
                           │ PostgreSQL + NATS JetStream
┌──────────────────────────▼──────────────────────────────────┐
│                    sip-edge (B2BUA)                           │
│  SIP 信令 │ RTP Relay │ 录音 │ SBC │ 认证 │ 路由             │
│  9138 行 main.rs │ UDP/TCP/TLS/WebSocket                     │
└───────┬──────────────────────────────────┬──────────────────┘
        │ SIP/UDP                          │ SIP/UDP
┌───────▼───────┐                  ┌───────▼───────┐
│   呼叫方       │                  │   网关/对端    │
│  (Caller)     │                  │  (Gateway)    │
└───────────────┘                  └───────────────┘
```

### 1.2 模块清单

| 模块 | 路径 | 行数 | 职责 |
|------|------|------|------|
| sip-core | `crates/sip-core/` | ~2000 | SIP 消息解析 (Request/Response/HeaderMap/SipUri) |
| rtp-core | `crates/rtp-core/` | ~1500 | RTP/RTCP 协议 (RtpPacket, RtcpPacket, TelephoneEvent) |
| sdp-core | `crates/sdp-core/` | ~800 | SDP 解析 (SessionDescription, RtpEndpoint) |
| call-core | `crates/call-core/` | ~1200 | 呼叫状态机、路由(LCR)、CDR 生成、网关健康追踪 |
| cdr-core | `crates/cdr-core/` | ~1800 | CDR 存储(PostgreSQL)、数据模型、DTMF 事件 |
| storage-core | `crates/storage-core/` | ~600 | 录音存储抽象 (Local/OSS/Dual) |
| sip-edge | `services/sip-edge/` | ~12000 | SIP B2BUA 核心 (main.rs 9138行 + 子模块) |
| api-server | `services/api-server/` | ~560 | REST API (Axum) |
| cdr-worker | `services/cdr-worker/` | ~200 | NATS CDR 消费者 |
| web | `web/` | ~15000 | React 管理界面 |

---

## 2. SIP 信令处理流程

### 2.1 INVITE 处理 (呼入)

**源文件**: `services/sip-edge/src/main.rs:3090-3319`

```
1. 收到 INVITE
2. 检查是否 in-dialog (To header 有 tag) → 转到 in-dialog 处理
3. 检查 draining 状态 → 503 Service Unavailable
4. 检查用户并发数 → 486 Busy Here (超过 sbc_max_concurrency)
5. 检查来源是否为 peer gateway
   - 非 peer: Digest 认证 → 401 Unauthorized
6. 查询注册表 → 目标用户是否已注册
   - 已注册: 直接转发到注册地址
   - 未注册: 路由表查询
7. SDP 重写 (媒体中继)
8. 拓扑隐藏 (生成 external Call-ID)
9. 构建出站 INVITE → 转发到网关
10. 返回 100 Trying 给呼叫方
```

**关键数据结构**:
- `InboundTransaction`: 存储每通呼叫的双侧状态 (caller/gateway)
- `DialogLeg`: Caller 或 Gateway 侧

### 2.2 200 OK 处理 (应答)

**源文件**: `services/sip-edge/src/main.rs:2253-2650`

```
1. 收到 200 OK (来自网关或注册用户)
2. 查找对应的 InboundTransaction
3. 如果是外部 Call-ID → 转换为内部 Call-ID
4. 解析 SDP Answer → 更新媒体端点
5. 配对 RTP 中继端口 (caller ↔ gateway)
6. 启动录音 (如果启用)
7. 解析 Session-Expires → 存储到 transaction
8. 转发 200 OK 给呼叫方
9. 发送 ACK 给网关
```

### 2.3 BYE 处理

**源文件**: `services/sip-edge/src/main.rs:3663-3776`

```
1. 收到 BYE (任意一侧)
2. 收集 RTCP 质量指标 (caller/gateway)
3. 计算 MOS 分数
4. 收集 DTMF 数字
5. 调用 CallManager.handle_inbound_termination()
6. 清理 RTP 中继目标
7. 返回 200 OK
8. 生成 CDR (通过 completed_cdrs)
```

### 2.4 CANCEL 处理

**源文件**: `services/sip-edge/src/main.rs:3754-3766`

```
1. 收到 CANCEL (呼叫方超时)
2. 与 BYE 相同的清理流程
3. 特殊处理: 取消出站 INVITE 客户端事务
   - 通过 external Call-ID 查找客户端事务
   - 调用 cancel_client_transaction()
   - 避免 32s 超时生成虚假 503
```

### 2.5 REGISTER 处理

**源文件**: `services/sip-edge/src/registrar.rs:48-80`

```
1. 收到 REGISTER
2. 提取 AOR (Address of Record)
3. 处理 Contact 头:
   - expires=0: 注销
   - expires>0: 注册/刷新
   - 无 Contact: 查询当前绑定
4. 存储到内存 HashMap + PostgreSQL (可选)
5. 返回 200 OK + Contact 列表 + expires
```

### 2.6 REFER 处理 (呼叫转接)

**源文件**: `services/sip-edge/src/main.rs:3843-3950`

```
1. 收到 REFER (盲转)
2. 提取 Refer-To URI
3. 构建新 INVITE 到目标
4. 返回 202 Accepted
5. 发送 NOTIFY (sipfrag) 通知转接进度
6. 转接完成后清理原呼叫
```

---

## 3. 媒体/RTP 处理

### 3.1 RTP 中继架构

**源文件**: `services/sip-edge/src/media.rs`

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  Caller RTP  │────▶│  Relay Port  │────▶│ Gateway RTP │
│  (实际端口)   │     │  (分配端口)   │     │  (实际端口)  │
└─────────────┘     └──────────────┘     └─────────────┘
                           │
                    ┌──────▼──────┐
                    │   录音模块   │
                    │  WAV 文件    │
                    └─────────────┘
```

### 3.2 端口分配策略

- **端口范围**: 默认 40000-40100 (环境变量可配置)
- **分配方式**: 线性扫描，跳过已占用端口
- **对称 RTP**: 自动学习对端实际地址 (symmetric_rtp_learning)
- **RTCP**: 偶数端口 RTP，奇数端口 RTCP (port+1)

### 3.3 RTP 转发逻辑

```rust
// 伪代码
loop {
    recv(socket, buf) → packet
    if packet is RTP {
        forward_to_target(target_socket, packet)
        record_to_wav(recorder, packet)  // 如果启用录音
        process_dtmf(packet)             // 如果是 DTMF
    }
    if packet is RTCP {
        parse_receiver_report()
        calculate_rtt_jitter_loss()
        forward_to_target()
    }
}
```

### 3.4 录音实现

- **格式**: WAV (8kHz, stereo, 16-bit PCM)
- **写入**: 同步文件 I/O (Arc<Mutex<WavCallRecorder>>)
- **声道分离**: Caller 左声道，Gateway 右声道
- **存储后端**: Local / OSS (S3兼容) / Dual (双写)

---

## 4. 路由与网关管理

### 4.1 路由算法 (LCR + Weighted Load Balancing)

**源文件**: `crates/call-core/src/routing.rs`

```rust
// 路由选择优先级:
1. 最长前缀匹配 (Longest Prefix Match) — prefix length DESC
2. 更高优先级值 (Higher Priority = More Preferred) — priority DESC
3. 更低成本 (Lower Cost = LCR) — cost ASC
4. 同等条件下加权随机 (Weighted Random) — weight DESC/random
5. 健康状态过滤 (Circuit Breaker)
6. 容量检查 (max_capacity / max_concurrent)
7. 方向过滤 (inbound/outbound/both)
```

**示例**:
```
目标: 8613800138000
路由表:
  r1: prefix="86",   priority=100, cost=0.50, weight=100 → gw1
  r2: prefix="8613", priority=200, cost=0.30, weight=200 → gw2
  r3: prefix="8613", priority=200, cost=0.30, weight=100 → gw3
  r4: prefix="8613", priority=100, cost=0.40, weight=100 → gw4

选择顺序: r2/r3 (最长前缀8613 + 最高优先级200 + 同成本，按 weight 加权随机) → r4 → r1
```

### 4.2 网关健康追踪 (Circuit Breaker)

**源文件**: `crates/call-core/src/routing.rs:137-354`

```
状态机:
  Healthy ──失败→ Circuit Open ──恢复间隔→ Half-Open ──成功→ Healthy
                                    │
                                    └──失败→ Circuit Open

阈值:
  failure_threshold: 5 (连续失败次数)
  recovery_interval: 30s (半开探测间隔)
  min_success_rate: 0.3 (最低成功率)
  min_samples: 10 (最少样本数)
```

### 4.3 并发控制

- **用户级**: `sbc_max_concurrency` (默认10)
- **网关级**: `max_capacity` (每个网关独立配置)
- **分配级**: `gateway_number_assignments` 表

### 4.4 网关类型

| 类型 | 说明 | 用途 |
|------|------|------|
| gateway | 落地网关 | 对接运营商/PSTN |
| peer | 对接网关 | 对接其他 VoIP 系统 |
| extension | 分机 | 自动生成 |

---

## 5. CDR 与计费系统

### 5.1 CDR 生成流程

```
1. INVITE → CallManager.handle_inbound_invite()
   - 创建 Call 对象
   - 记录 started_at
   - 读取 X-Call-Direction (inbound/outbound)

2. 180 Ringing → Call.mark_ringing()

3. 200 OK → Call.mark_answered()
   - 记录 answered_at

4. BYE/CANCEL → CallManager.handle_inbound_termination()
   - 记录 ended_at
   - 收集 RTCP 质量指标
   - 计算 MOS 分数
   - 收集 DTMF 数字
   - CallCdr::from_completed_call()

5. CDR 写入
   - 路径1: 直接 PostgreSQL insert
   - 路径2: NATS JetStream → cdr-worker → PostgreSQL
   - 去重: ON CONFLICT (call_id, started_at) DO NOTHING
```

### 5.2 CDR 数据模型

**数据库表**: `call_cdrs`

| 字段 | 类型 | 说明 |
|------|------|------|
| call_id | TEXT | 呼叫唯一标识 |
| caller | TEXT | 主叫号码 |
| callee | TEXT | 被叫号码 |
| started_at | TIMESTAMPTZ | 开始时间 |
| answered_at | TIMESTAMPTZ | 应答时间 |
| ended_at | TIMESTAMPTZ | 结束时间 |
| duration_ms | BIGINT | 总时长(ms) |
| billable_duration_ms | BIGINT | 计费时长(ms) |
| status | TEXT | 状态 (answered/canceled/failed) |
| caller_rtcp_* | DOUBLE | 主叫侧 RTCP 指标 |
| gateway_rtcp_* | DOUBLE | 网关侧 RTCP 指标 |
| mos | DOUBLE | MOS 分数 |
| dtmf_digits | TEXT | DTMF 数字序列 |
| recording_path | TEXT | 录音文件路径 |
| direction | VARCHAR(10) | 方向 (inbound/outbound) |

### 5.3 计费系统

**表结构**:
- `billing_rates`: 费率表 (prefix + rate_per_minute)
- `billing_accounts`: 账户表 (username + balance)
- `billing_ledger`: 账本 (call_id + amount + balance_after)

**计费逻辑**:
```rust
amount = (billable_duration_ms / 60000.0) * rate_per_minute
balance_after = balance_before - amount
```

---

## 6. 录音系统

### 6.1 录音架构

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  RTP Stream  │────▶│ WAV Recorder │────▶│ Storage     │
│  (实时)      │     │  (同步写入)   │     │ Local/OSS   │
└─────────────┘     └──────────────┘     └─────────────┘
```

### 6.2 录音参数

- **采样率**: 8kHz
- **声道**: 2 (stereo)
- **位深**: 16-bit PCM
- **WAV 头**: 44 bytes
- **时长计算**: `(size_bytes - 44) / 32000`

### 6.3 存储后端

| 后端 | config.yaml 映射 (connections.s3) | 说明 |
|------|-----------------------------------|------|
| Local | `backend: "local"` | 本地文件系统 |
| OSS / S3 | `backend: "s3"` | S3 兼容对象存储 |

### 6.4 录音 API

```
GET  /api/recordings/:call_id/audio → CDR 详情录音播放/下载
GET  /api/recordings/:call_id/audio → 下载音频 (WAV)
```

---

## 7. SBC 安全模块

### 7.1 功能列表

| 功能 | 实现 / system_configs | 说明 |
|------|----------------------|------|
| IP ACL (白名单) | 数据库/系统设置中配置白名单字段 | 逗号分隔的 CIDR |
| IP ACL (黑名单) | 数据库/系统设置中配置黑名单字段 | 逗号分隔的 CIDR |
| 令牌桶限速 | `sbc_rate_limit_capacity` | 桶容量 (默认2000.0) |
| 令牌桶填充率 | `sbc_rate_limit_fill_rate` | 每秒填充 (默认500.0) |
| 用户并发限制 | `sbc_max_concurrency` | 用户最大并发 (默认2000) |

### 7.2 检查顺序

```
1. IP 黑名单检查 → 拒绝
2. IP 白名单检查 (如果配置) → 不在白名单则拒绝
3. 令牌桶限速 → 超速拒绝
4. 用户并发数检查 → 超限返回 486
```

---

## 8. 认证与鉴权

### 8.1 Digest 认证

**源文件**: `services/sip-edge/src/auth.rs`

```
1. 收到 INVITE (非 peer gateway)
2. 检查 Authorization 头
   - 无: 返回 401 + WWW-Authenticate
3. 解析 Digest 参数 (username, realm, nonce, response, ...)
4. 验证动态 nonce (时间戳 + 签名)
5. 防重放: nonce + cnonce + nc 去重
6. 验证密码 (从环境变量或数据库)
7. 通过 → 继续处理
```

### 8.2 认证配置

- **账户存储**：数据库 `sip_users` 表
- **Realm**：默认使用 `"vos-rs"`
- **Secret Key**：启动时随机生成

---

## 9. 拓扑隐藏

### 9.1 实现方式

**源文件**: `services/sip-edge/src/topology.rs`

```
1. Call-ID 翻译
   - 内部 Call-ID: 原始 SIP 消息的 Call-ID
   - 外部 Call-ID: MD5(internal + timestamp) @ advertised_addr
   - 映射存储: DashMap<internal ↔ external>

2. Via 头重写
   - 替换为 advertised_addr

3. Contact 头重写
   - 替换为 advertised_addr
```

### 9.2 Call-ID 映射

```rust
let external_call_id = format!(
    "{}@{}",
    &md5_hex[..16],
    edge_config.advertised_addr.split(':').next().unwrap_or("vos-rs")
);
edge_state.register_call_id_mapping(&internal_call_id, &external_call_id);
```

---

## 10. NAT 穿透

### 10.1 STUN

- **配置**：`config.yaml` 中配置 `sip_edge.nat_traversal.stun_server`
- **功能**：发现公网 IP 地址
- **Keepalive**：每 30s 发送 STUN Binding Request

### 10.2 UPnP

- **配置**：`services/sip-edge/src/upnp.rs`
- **功能**：自动端口映射

### 10.3 TURN

- **配置**：`services/sip-edge/src/turn.rs`
- **功能**：中继穿越对称 NAT

### 10.4 Symmetric RTP

- **配置**：数据库 `system_configs` 表中配置 `rtp_symmetric_learning` (默认值为 `true`)
- **功能**：从收到的第一个 RTP 包学习对端实际地址

---

## 11. 管理 API 与前端

### 11.1 REST API 端点

| 端点 | 方法 | 功能 |
|------|------|------|
| `/api/dashboard/stats` | GET | 仪表板统计 |
| `/api/dashboard/trend` | GET | 小时趋势 |
| `/api/cdrs` | GET | CDR 列表 (分页/过滤) |
| `/api/cdrs/:call_id` | GET | 单条 CDR |
| `/api/cdrs/:call_id/dtmf` | GET | DTMF 事件 |
| `/api/users` | GET/POST | 用户 CRUD |
| `/api/gateways` | GET/POST | 网关 CRUD |
| `/api/routes` | GET/POST | 路由 CRUD |
| `/api/registrations` | GET | 注册列表 |
| `/api/recordings/:call_id/audio` | GET | CDR 录音播放/下载 |
| `/api/recordings/:call_id/audio` | GET | 下载录音 |
| `/api/reports/summary` | GET | 报表汇总 |
| `/api/reports/export` | GET | 导出 CSV |
| `/api/rates` | GET/POST | 费率 CRUD |
| `/api/accounts` | GET | 账户列表 |
| `/api/accounts/:username/credit` | POST | 充值 |
| `/api/ledger` | GET | 账本 |
| `/api/billing/reconcile` | POST | 对账 |
| `/api/calls/active` | GET | 活跃呼叫 |
| `/api/calls/:call_id/terminate` | POST | 终止呼叫 |
| `/api/route-preview` | GET | 路由预览 |
| `/api/numbers` | GET/POST | 号码库存 CRUD |
| `/api/anti-fraud/rules` | GET/POST | 反欺诈规则 |

### 11.2 前端页面

| 页面 | 路由 | 功能 |
|------|------|------|
| Dashboard | `/` | 仪表板 (统计+趋势图) |
| ActiveCalls | `/active-calls` | 活跃呼叫监控 |
| CDR | `/cdr` | 通话记录列表+详情+录音播放 |
| Users | `/users` | SIP 用户管理 |
| Gateways | `/gateways` | 落地网关管理 |
| PeerGateways | `/peer-gateways` | 对接网关管理 |
| Routes | `/routes` | 路由管理 |
| Registrations | `/registrations` | 注册状态 |
| Numbers | `/numbers` | 号码库存管理 |
| CDR Recording | CDR 详情 | 录音播放与下载 |
| Reports | `/reports` | 报表 (7天/30天/自定义) |
| Rates | `/rates` | 费率管理 |
| Accounts | `/accounts` | 账户管理 |
| AntiFraud | `/anti-fraud` | 反欺诈规则 |

### 11.3 技术栈

- **前端**: React 18 + TypeScript + Arco Design + Vite + ECharts
- **后端**: Rust + Axum + PostgreSQL + NATS JetStream
- **设计系统**: Indigo 主色 (#6366f1) + Violet 强调色 (#8b5cf6)

---

## 12. 数据库设计

### 12.1 核心表

```sql
-- CDR 表
call_cdrs (
  id BIGSERIAL PRIMARY KEY,
  call_id TEXT NOT NULL,
  caller TEXT,
  callee TEXT,
  started_at TIMESTAMPTZ,
  answered_at TIMESTAMPTZ,
  ended_at TIMESTAMPTZ,
  duration_ms BIGINT,
  billable_duration_ms BIGINT,
  status TEXT,
  failure_status_code INTEGER,
  failure_reason TEXT,
  caller_rtcp_loss_rate DOUBLE PRECISION,
  caller_rtcp_jitter_ms DOUBLE PRECISION,
  caller_rtcp_rtt_ms INTEGER,
  gateway_rtcp_loss_rate DOUBLE PRECISION,
  gateway_rtcp_jitter_ms DOUBLE PRECISION,
  gateway_rtcp_rtt_ms INTEGER,
  mos DOUBLE PRECISION,
  dtmf_digits TEXT,
  recording_path TEXT,
  direction VARCHAR(10) DEFAULT 'outbound',
  inserted_at TIMESTAMPTZ DEFAULT now(),
  UNIQUE (call_id, started_at)
)

-- SIP 用户
sip_users (username TEXT PK, password TEXT, created_at TIMESTAMPTZ)

-- SIP 网关
sip_gateways (
  id TEXT PK, host TEXT, port INTEGER, transport TEXT,
  max_capacity INTEGER, gateway_type VARCHAR(20) DEFAULT 'peer',
  prefix_rules TEXT, supports_registration BOOLEAN,
  caller_id_mode TEXT, virtual_caller TEXT,
  current_concurrent INTEGER, account_id BIGINT,
  max_concurrent INTEGER, created_at TIMESTAMPTZ
)

-- 路由
sip_routes (
  id TEXT PK, prefix TEXT, priority INTEGER,
  gateway_id TEXT REFERENCES sip_gateways(id),
  cost DOUBLE PRECISION DEFAULT 0.0,
  time_start TEXT, time_end TEXT,
  created_at TIMESTAMPTZ
)

-- DTMF 事件
dtmf_events (
  id BIGSERIAL PK, call_id TEXT, digit TEXT,
  source TEXT, timestamp_ms BIGINT,
  rtp_timestamp BIGINT, duration_ms INTEGER,
  volume INTEGER, inserted_at TIMESTAMPTZ
)

-- 注册
sip_registrations (
  aor TEXT, contact_uri TEXT, received_from TEXT,
  expires_at TIMESTAMPTZ, updated_at TIMESTAMPTZ,
  path TEXT, PRIMARY KEY (aor, contact_uri)
)

-- 费率
billing_rates (id TEXT PK, prefix TEXT, rate_per_minute DOUBLE PRECISION)

-- 账户
billing_accounts (username TEXT PK, balance DOUBLE PRECISION, currency TEXT)

-- 账本
billing_ledger (
  id BIGSERIAL PK, call_id TEXT UNIQUE, username TEXT,
  duration_ms BIGINT, rate_per_minute DOUBLE PRECISION,
  amount DOUBLE PRECISION, balance_after DOUBLE PRECISION
)

-- 号码库存
number_inventory (number TEXT PK, username TEXT, status TEXT)
```

---

## 13. 与昆石 VOS 对比分析

### 13.1 架构对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **语言** | Rust | C/C++ |
| **架构** | 单体 B2BUA | 分布式集群 |
| **部署** | 单实例 | 多节点集群 |
| **性能** | 高 (Rust 零开销抽象) | 极高 (成熟优化) |
| **并发模型** | Tokio async | 多进程/多线程 |
| **存储** | PostgreSQL + NATS | Oracle/MySQL + 专有存储 |
| **Web 管理** | React SPA | Web 管理台 |
| **API** | RESTful (Axum) | REST + 私有协议 |

### 13.2 SIP 信令对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **INVITE 处理** | ✅ 完整 | ✅ 完整 |
| **100rel/PRACK** | ✅ 支持 | ✅ 支持 |
| **Session Timer** | ✅ 支持 | ✅ 支持 |
| **REFER (转接)** | ✅ 盲转 | ✅ 盲转 + 咨询转 |
| **UPDATE** | ✅ 支持 | ✅ 支持 |
| **MESSAGE** | ✅ 支持 | ✅ 支持 |
| **SUBSCRIBE/NOTIFY** | ⚠️ 部分 | ✅ 完整 (BLF等) |
| **PUBLISH** | ❌ 未实现 | ✅ 支持 |
| **出站代理** | ⚠️ 简单 | ✅ 完整 (多跳路由) |

### 13.3 媒体处理对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **RTP 中继** | ✅ 完整 | ✅ 完整 |
| **RTCP 质量监控** | ✅ 完整 (RTT/Jitter/Loss) | ✅ 完整 |
| **录音** | ✅ WAV + 转码后处理 (Opus/AMR 压缩) | ✅ 多格式 (本地/NFS/OSS) |
| **DTMF** | ✅ RFC 2833 + SIP INFO | ✅ RFC 2833 + SIP INFO + Inband |
| **编解码转码** | ✅ 完整支持 (支持通过 ffmpeg 异步后处理将 WAV 转码为 Opus/AMR 压缩格式) | ✅ 完整 (G.711/G.729/Opus) |
| **会议桥** | ✅ 完整支持 (实现周期为 20ms 的 Mix-Minus 音频混音，支持 SIP 呼入与自动拆线清理) | ✅ 支持 |
| **SRTP** | ✅ 完整支持 (SDES-SRTP 加解密及 offer/answer 信令协商已打通) | ✅ 支持 |
| **Video** | ❌ 未实现 | ⚠️ 有限支持 |


### 13.4 路由与计费对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **LCR (最低成本路由)** | ✅ 前缀+优先级+成本 | ✅ 完整 LCR + 时间路由 |
| **时间路由** | ✅ 已支持 (支持时段过滤) | ✅ 完整 (时间段路由) |
| **费率表** | ✅ 基础 | ✅ 多维费率 (时段/目的地/账户) |
| **实时计费** | ✅ 完整支持 (已支持呼叫前余额预检、实时扣费和看门狗定时器超时强制拆线) | ✅ 完整 (预付费/后付费) |
| **网关健康追踪** | ✅ Circuit Breaker | ✅ 多维度健康检查 |
| **故障转移** | ✅ 自动 failover | ✅ 自动 failover + 手动切换 |

### 13.5 安全对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **Digest 认证** | ✅ 完整 | ✅ 完整 |
| **IP ACL** | ✅ 白名单+黑名单 | ✅ 完整 |
| **令牌桶限速** | ✅ 实现 | ✅ 多维度限速 |
| **用户并发控制** | ✅ 实现 | ✅ 完整 |
| **反欺诈** | ✅ 已实现 (黑名单/白名单/被叫拦截，CRUD API + invite-handler 执行) | ✅ 完整 (异常检测/黑名单) |
| **TLS** | ✅ 支持 | ✅ 支持 |
| **SRTP** | ⚠️ 部分 (SDES-SRTP 加解密模块已实现，媒体层集成中) | ✅ 支持 |
| **防暴力破解** | ⚠️ nonce 动态验证 + replay cache (DashMap，缺 IP 锁定机制) | ✅ 完整 (锁定/IP封禁) |

### 13.6 管理界面对比

| 特性 | VOS-RS | 昆石 VOS |
|------|--------|----------|
| **仪表板** | ✅ 统计+趋势图 | ✅ 完整 |
| **CDR 查询** | ✅ 分页+过滤+导出 | ✅ 完整 |
| **录音播放** | ✅ Web 播放 | ✅ Web 播放+下载 |
| **报表** | ✅ 7天/30天/自定义 | ✅ 完整 (多维度) |
| **实时监控** | ✅ 活跃呼叫 | ✅ 完整 |
| **多租户** | ⚠️ 字段已定义 | ✅ 完整 |
| **权限控制** | ❌ 未实现 | ✅ RBAC |

---

## 14. 不合理项与优化建议

### 14.1 严重问题 (已修复/已完成)

| # | 问题 | 影响 | 建议 |
|---|------|------|------|
| 1 | **main.rs 超大单文件** | 可维护性极差，难以定位和修改 | **已修复**：核心逻辑已完成向子模块拆分和重构。 |
| 2 | **注册存储纯内存** | 重启丢失所有注册，无法集群 | **已修复**：引入 Redis 共享存储持久化，支持集群同步与 TTL 自动失效。 |
| 3 | **CDR 双写无事务** | 可能重复或丢失 | 统一写入路径，移除 NATS→cdr-worker 链路 |
| 4 | **SRTP 未完整集成** | 媒体流明文传输 | **已修复**：SDES-SRTP 加解密与 SDP offer/answer 完整协商已打通。 |
| 5 | **无 ACL 动态更新** | 需重启才能更新白名单/黑名单 | API 热更新 SBC 规则 |

### 14.2 功能缺失 (已修复/已完成)

| # | 功能 | 优先级 | 说明 |
|---|------|--------|------|
| 1 | **编解码转码** | 已完成 | **已修复**：支持挂断后通过 ffmpeg 异步将 WAV 转码为 Opus/AMR 压缩格式。 |
| 2 | **会议桥** | 已完成 | **已修复**：实现基于单腿 UAS 呼入集成的会议混音器，提供 Mix-Minus 音频混音转发与自动拆线清理。 |
| 3 | **SRTP 完整集成** | 已完成 | **已修复**：打通 SDP offer/answer 完整流程。 |
| 4 | **PUBLISH** | 低 | SIP PUBLISH 方法，用于状态发布 (Presence) |
| 5 | **BLF/SUBSCRIBE** | 低 | 完整的 SUBSCRIBE 处理器，支持忙灯 (BLF) 订阅 |
| 6 | **权限控制 (RBAC)** | 高 | 管理界面多用户权限，目前无任何鉴权 |
| 7 | **多租户隔离** | 已完成 | **已修复**：tenant.rs 域隔离与资源配额控制已集成就绪。 |
| 8 | **防暴力破解 IP 锁定** | 中 | 已有 nonce replay cache，缺 IP 自动封禁与失败计数 |


### 14.3 设计不合理项

| # | 问题 | 昆石 VOS 做法 | 建议 |
|---|------|---------------|------|
| 1 | **Call-ID 翻译用 MD5** | 用随机 UUID | MD5 有碰撞风险，改用 UUID v4 |
| 2 | **CDR 双写路径** | 单一写入 | 统一 PostgreSQL 直写，移除 NATS 链路 |
| 3 | **录音同步写入** | 异步写入+缓冲 | 高并发下同步 I/O 可能阻塞，改为 async write |
| 4 | **SBC 限速用 Mutex** | 无锁数据结构 | **已完成**：已改用 DashMap 并发分段锁，彻底消除限速瓶颈 |
| 5 | **用户并发数遍历 DashMap** | 维护计数器 | **已完成**：已改用 O(1) 的 per-user 并发计数器，避免全量遍历事务 |
| 6 | **认证 secret_key 启动时随机生成** | 配置固定密钥 | **已完成**：已支持配置文件固定配置，确保集群各节点一致 |
| 7 | **路由表全量加载到内存** | 数据库查询+缓存 | 路由变更需重启，应支持热加载 |
| 8 | **CDR 索引不足** | 完整索引 | 缺少 caller/callee/direction 索引，大表查询慢 |
| 9 | **前端无状态管理** | Redux/Vuex | 组件间状态传递依赖 props drilling |
| 10 | **无 WebSocket 实时推送** | 长轮询/WebSocket | 活跃呼叫、注册状态需实时更新 |

### 14.4 性能与媒体优化建议

| # | 优化点 | 预期收益 | 实现状态 |
|---|--------|----------|----------|
| 1 | **UDP 发送/接收缓冲区** | 降低大并发包丢失率 | 已优化 (4MB 缓存) |
| 2 | **DashMap 分片锁** | 极小化全局状态读写竞争 | 已全面落地 |
| 3 | **录音异步写入** | 避免同步 I/O 阻塞信令线程 | 已使用基于 tokio channel 的后台 Task 缓冲写入 |
| 4 | **媒体端口分配无锁化** | 避免 `UdpSocket::bind` 抢占全局互斥锁 | 已实现 Lock-free 端口分配 |
| 5 | **可编程媒体动态降采样** | 兼容多码率 (16k/44.1k/48k) 音频源接入 | 已实现高精度线性重采样 |
| 6 | **流平滑切换重写** | 规避 Exclusive 播放后流的爆音和丢包 | 已实现 SSRC 序列号/时间戳连续重写 |
| 7 | **CDR 批量写入** | 减少 PostgreSQL 频繁写开销 | 已通过 NATS JetStream 异步批量消费落地 |
| 8 | **数据库连接池** | Postgres 连接复用 | 已使用 sqlx Pool 复用连接 |

### 14.5 代码质量建议

| # | 问题 | 建议 |
|---|------|------|
| 1 | **无文档** | 为每个 crate 添加 README，为关键函数添加 doc comment |
| 2 | **测试覆盖** | 184 个测试，但缺少集成测试 |
| 3 | **错误处理** | 统一使用 thiserror，减少 Box<dyn Error> |
| 4 | **日志规范** | 统一使用 tracing，结构化日志 |
| 5 | **配置管理** | 从环境变量迁移到配置文件 (toml/yaml) |
| 6 | **CI/CD** | 添加 GitHub Actions (lint + test + build) |

---

## 附录 A: 坐席呼出主叫号码处理

### 问题描述

当坐席（分机用户）发起呼出时，CDR 中的 caller 字段应显示坐席分配的号码，而非 SIP From header 中的硬编码值。

### 当前实现

```rust
// call.rs:118-121
caller: request
    .headers
    .get("from")
    .map(|value| value.as_str().to_string()),
```

### 解决方案

1. **号码库存表**: `number_inventory` + `gateway_number_assignments` 已就绪
2. **Caller ID 模式**: `caller_id_mode` (passthrough/virtual/random) 已配置
3. **修复点**: 在 `handle_inbound_invite` 中，根据 `caller_id_mode` 从号码库存表查询实际主叫号码

```rust
// 伪代码
let caller = match gateway.caller_id_mode {
    "passthrough" => request.from_header(),
    "virtual" => gateway.virtual_caller,
    "random" => db.query_random_number(gateway_id),
};
```

---

## 附录 B: 统一配置文件与配置字段清单

详见 [`docs/development/ENV_VARS.md`](file:///docs/development/ENV_VARS.md)。

本项目已废弃所有零散的环境变量，仅保留 `VOS_RS_CONFIG_FILE` 变量用于引导 `config.yaml`。所有关于基础设施的连接设置（PostgreSQL, Redis, NATS, S3）以及信令/媒体相关的设置均已整理并收拢到单一主配置文件及高动态数据库表 `system_configs` 中。

---

*文档结束*
