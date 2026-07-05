# vos-rs 环境变量与数据库配置参考

> 本文档覆盖 sip-edge、api-server、cdr-worker 三个服务的所有环境变量，
> 以及 PostgreSQL 数据库配置表的 schema 和 fallback 行为。

---

## 1. sip-edge 服务

### 1.1 SIP 核心配置

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SIP_UDP_BIND` | `0.0.0.0:5060` | SIP UDP 监听地址 |
| `VOS_RS_SIP_ADVERTISED_ADDR` | `127.0.0.1:5060` | SIP Via/Contact 中通告的外部地址 |
| `VOS_RS_SIP_DEFAULT_GATEWAY` | _(空)_ | 默认出站网关 URI（如 `sip:1.2.3.4:5060`），空则无路由 |

**数据库优先**：若配置了 `VOS_RS_DATABASE_URL` 且 `sip_gateways`/`sip_routes` 表有数据，则 DB 路由覆盖此环境变量。首次启动时，DB 为空则从此变量种子写入 DB。

### 1.2 认证

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SIP_AUTH_USERS` | _(空，禁用认证)_ | 用户凭证，格式 `user1:pass1,user2:pass2` 或 `user1=pass1,user2=pass2` |
| `VOS_RS_SIP_AUTH_REALM` | `vos-rs` | Digest 认证 Realm |
| `VOS_RS_SIP_AUTH_NONCE` | `vos-rs-dev-nonce` | 静态 nonce；保留默认值时自动生成动态 nonce |

**数据库优先**：认证验证时先查 `sip_users` 表（`VOS_RS_DATABASE_URL` 设置时），找不到再回退到环境变量。首次启动 DB 为空时，从此变量种子写入 `sip_users`。

### 1.3 SBC（会话边界控制器）

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SBC_ALLOW` | _(空，全部放行)_ | IP 白名单（CIDR 逗号分隔） |
| `VOS_RS_SBC_BLOCK` | _(空，无封禁)_ | IP 黑名单（CIDR 逗号分隔） |
| `VOS_RS_SBC_LIMIT_CAPACITY` | `100.0` | 每 IP 令牌桶容量 |
| `VOS_RS_SBC_LIMIT_FILL_RATE` | `10.0` | 每 IP 令牌补充速率（tokens/s） |
| `VOS_RS_SBC_MAX_CONCURRENCY` | `10` | 每用户最大并发 SIP 事务数 |

### 1.4 会话定时器

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SESSION_EXPIRES_GATEWAY` | `600` | 网关侧 Session-Expires（秒） |
| `VOS_RS_SESSION_EXPIRES_CALLER` | `1800` | 呼叫方侧 Session-Expires（秒） |

### 1.5 TLS

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SIP_TLS_BIND` | _(从 UDP bind 推导，端口 5061)_ | TLS 监听地址 |
| `VOS_RS_SIP_TLS_CERT_PATH` | _(空，TLS 禁用)_ | TLS 证书文件路径 |
| `VOS_RS_SIP_TLS_KEY_PATH` | _(空)_ | TLS 私钥文件路径 |
| `VOS_RS_SIP_TLS_ALLOW_TEST_CERT` | `false` | 允许自签名/测试证书 |
| `VOS_RS_SIP_TLS_CA_PATH` | _(空)_ | 出站 TLS 的 CA 证书路径 |
| `VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY` | `false` | 跳过出站 TLS 证书验证 |
| `VOS_RS_SIP_TLS_SERVER_NAME` | _(空)_ | 出站 TLS 的 SNI 覆盖 |

### 1.6 WebSocket

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_SIP_WS_BIND` | _(从 UDP bind 推导，端口 5062)_ | WebSocket 监听地址 |

### 1.7 媒体/RTP

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_RTP_ADVERTISED_ADDR` | `127.0.0.1` | SDP 中通告的 RTP 公网 IP |
| `VOS_RS_RTP_PORT_MIN` | `40000` | RTP 中继端口范围最小值 |
| `VOS_RS_RTP_PORT_MAX` | `40100` | RTP 中继端口范围最大值 |
| `VOS_RS_RTP_SYMMETRIC_LEARNING` | `true` | 启用对称 RTP 学习（从首包学习真实远端地址） |

### 1.8 录制

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_RECORDING_ENABLED` | `false` | 启用呼叫录音 |
| `VOS_RS_RECORDING_DIR` | `target/recordings` | 录音文件存储目录 |

### 1.9 NAT 穿越

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_STUN_SERVER` | _(空，禁用)_ | STUN 服务器（逗号分隔多个，如 `stun.l.google.com:19302,stun1.l.google.com:19302`） |
| `VOS_RS_UPNP_ENABLED` | `false` | 启用 UPnP 端口映射（自动发现路由器，映射 SIP+RTP 端口） |

### 1.10 CDR/存储

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_DATABASE_URL` | _(空，禁用 PostgreSQL)_ | PostgreSQL 连接字符串 |
| `VOS_RS_NATS_URL` | _(空，禁用 NATS)_ | NATS JetStream 连接地址 |
| `VOS_RS_NATS_CDR_STREAM` | `VOS_RS_CDRS` | JetStream 流名称 |
| `VOS_RS_NATS_CDR_SUBJECT` | `vos-rs.cdrs` | CDR 事件的 NATS subject |

### 1.11 管理 API

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_MANAGE_BIND` | `127.0.0.1:8082` | 管理 API 监听地址（查询活跃呼叫、强制拆线） |

### 1.12 日志

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `RUST_LOG` | `sip_edge=info` | 日志级别过滤器（`debug` 可看到 SIP 信令、媒体 relay 详情） |

---

## 2. api-server 服务

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `DATABASE_URL` | `postgres://localhost/vos_rs` | PostgreSQL 连接字符串（注意：**不是** `VOS_RS_DATABASE_URL`） |
| `API_PORT` | `8080` | HTTP 监听端口 |
| `VOS_RS_RECORDING_DIR` | `target/recordings` | 录音文件目录（用于下载接口） |
| `VOS_RS_MANAGE_BASE` | `http://127.0.0.1:8082` | sip-edge 管理 API 基础 URL |
| `RUST_LOG` | `api_server=debug,tower_http=debug,info` | 日志级别 |

**API 端点**：

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/health` | 健康检查 |
| **CDR** | | |
| GET | `/api/cdrs` | 分页查询 CDR（支持 status/caller/callee/时间范围过滤） |
| GET | `/api/cdrs/:call_id` | 单条 CDR 详情 |
| GET | `/api/cdrs/:call_id/dtmf` | DTMF 事件审计 |
| **仪表盘** | | |
| GET | `/api/dashboard/stats` | 今日汇总（总量/接通率/MOS/丢包/注册用户） |
| GET | `/api/dashboard/trend` | 今日每小时趋势 |
| **报表** | | |
| GET | `/api/reports/summary` | 报表（按状态/按日聚合） |
| GET | `/api/reports/export` | CSV 导出 CDR |
| **用户管理** | | |
| GET/POST | `/api/users` | 列表 / 创建 SIP 用户 |
| PUT/DELETE | `/api/users/:username` | 更新 / 删除用户 |
| **网关管理** | | |
| GET/POST | `/api/gateways` | 列表 / 创建网关 |
| PUT/DELETE | `/api/gateways/:id` | 更新 / 删除网关 |
| **路由管理** | | |
| GET/POST | `/api/routes` | 列表 / 创建路由 |
| PUT/DELETE | `/api/routes/:id` | 更新 / 删除路由 |
| GET | `/api/route-preview` | 预览路由决策 |
| **注册** | | |
| GET | `/api/registrations` | 列出 SIP 注册 |
| **录音** | | |
| GET | `/api/recordings` | 列出录音文件 |
| GET | `/api/recordings/:call_id/audio` | 下载录音音频 |
| **账单** | | |
| GET/POST | `/api/rates` | 列表 / 创建费率 |
| PUT/DELETE | `/api/rates/:id` | 更新 / 删除费率 |
| GET | `/api/accounts` | 列出账户 |
| POST | `/api/accounts/:username/credit` | 充值 |
| GET | `/api/ledger` | 列出账单流水 |
| POST | `/api/billing/reconcile` | 触发账单对账 |
| **号码** | | |
| GET/POST | `/api/numbers` | 列表 / 创建号码 |
| PUT/DELETE | `/api/numbers/:number` | 更新 / 删除号码 |
| **呼叫控制** | | |
| GET | `/api/calls/active` | 列出活跃呼叫 |
| POST | `/api/calls/:call_id/terminate` | 强制结束呼叫 |

---

## 3. cdr-worker 服务

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `VOS_RS_DATABASE_URL` | **（必填）** | PostgreSQL 连接字符串 |
| `VOS_RS_NATS_URL` | `nats://127.0.0.1:4222` | NATS 连接地址 |
| `VOS_RS_NATS_CDR_STREAM` | `VOS_RS_CDRS` | 主 CDR 流名称 |
| `VOS_RS_NATS_CDR_SUBJECT` | `vos-rs.cdrs` | CDR subject |
| `VOS_RS_NATS_CDR_CONSUMER` | `vos-rs-cdr-worker` | 持久化消费者名称 |
| `VOS_RS_NATS_CDR_DLQ_SUBJECT` | `vos-rs.cdrs.dlq` | 死信队列 subject |
| `VOS_RS_NATS_CDR_DLQ_STREAM` | `VOS_RS_CDR_DLQ` | 死信队列流名称 |
| `VOS_RS_CDR_BATCH_SIZE` | `50` | 批量写入大小 |
| `VOS_RS_CDR_BATCH_TIMEOUT_MS` | `100` | 批量超时（毫秒） |
| `VOS_RS_CDR_MAX_DELIVERIES` | `5` | NATS 最大投递次数 |
| `VOS_RS_CDR_NAK_DELAY_MS` | `1000` | NAK 后重投延迟 |
| `VOS_RS_CDR_DB_RETRY_ATTEMPTS` | `3` | DB 批量插入重试次数 |
| `RUST_LOG` | `cdr_worker=info` | 日志级别 |

---

## 4. 数据库表 Schema

所有表在 `PostgresCdrStore::connect()` 时自动创建/迁移。

### 4.1 sip_users — SIP 认证凭证

| 列 | 类型 | 约束 |
|----|------|------|
| `username` | TEXT | PRIMARY KEY |
| `password` | TEXT | NOT NULL |
| `created_at` | TIMESTAMPTZ | DEFAULT now() |

**Fallback**：DB 存在此表时先查 DB；找不到或 DB 不可用时回退到 `VOS_RS_SIP_AUTH_USERS` 环境变量。首次启动 DB 为空时自动从环境变量种子写入。

### 4.2 sip_gateways — 出站网关

| 列 | 类型 | 约束 |
|----|------|------|
| `id` | TEXT | PRIMARY KEY |
| `host` | TEXT | NOT NULL |
| `port` | INTEGER | nullable |
| `transport` | TEXT | DEFAULT 'udp' |
| `max_capacity` | INTEGER | nullable |
| `created_at` | TIMESTAMPTZ | DEFAULT now() |

**Fallback**：DB 有网关数据时从 DB 加载；DB 为空时从 `VOS_RS_SIP_DEFAULT_GATEWAY` 种子写入。

### 4.3 sip_routes — 路由规则

| 列 | 类型 | 约束 |
|----|------|------|
| `id` | TEXT | PRIMARY KEY |
| `prefix` | TEXT | NOT NULL |
| `priority` | INTEGER | NOT NULL, DEFAULT 100 |
| `gateway_id` | TEXT | NOT NULL, FK → sip_gateways.id ON DELETE CASCADE |
| `cost` | DOUBLE | NOT NULL, DEFAULT 0.0（支持最低成本路由） |
| `created_at` | TIMESTAMPTZ | NOT NULL, DEFAULT now() |
| `time_start` | TEXT | nullable（HH:MM UTC 格式，时间窗口起点，ALTER TABLE 迁移添加） |
| `time_end` | TEXT | nullable（时间窗口终点，ALTER TABLE 迁移添加） |

**匹配逻辑**：最长前缀匹配 → 同前缀按优先级排序 → 同优先级按 cost 升序（LCR）→ 检查时间窗口 → 检查网关健康状态。

### 4.4 sip_registrations — 注册绑定

| 列 | 类型 | 约束 |
|----|------|------|
| `aor` | TEXT | PK |
| `contact_uri` | TEXT | PK |
| `received_from` | TEXT | NOT NULL |
| `expires_at` | TIMESTAMPTZ | NOT NULL |
| `updated_at` | TIMESTAMPTZ | DEFAULT now() |
| `path` | TEXT | nullable |

**Fallback**：无 DB 时仅存内存；有 DB 时持久化到 PostgreSQL，重启不丢失。

### 4.5 call_cdrs — 呼叫详单

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | BIGSERIAL | PRIMARY KEY |
| `call_id` | TEXT NOT NULL | 呼叫 ID |
| `caller` / `callee` | TEXT | 主被叫 |
| `started_at` / `answered_at` / `ended_at` | TIMESTAMPTZ | 时间戳 |
| `duration_ms` / `billable_duration_ms` | BIGINT NOT NULL | 时长（毫秒） |
| `status` | TEXT NOT NULL | answered / canceled / failed |
| `failure_status_code` | INTEGER | SIP 失败状态码 |
| `failure_reason` | TEXT | 失败原因 |
| `caller_rtcp_loss_rate` / `caller_rtcp_jitter_ms` / `caller_rtcp_rtt_ms` | DOUBLE/INT | 呼叫方 RTCP 质量指标 |
| `gateway_rtcp_loss_rate` / `gateway_rtcp_jitter_ms` / `gateway_rtcp_rtt_ms` | DOUBLE/INT | 网关侧 RTCP 质量指标 |
| `mos` | DOUBLE | MOS 评分 |
| `dtmf_digits` | TEXT | DTMF 按键序列 |
| `inserted_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

### 4.6 dtmf_events — DTMF 审计

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | BIGSERIAL | PRIMARY KEY |
| `call_id` | TEXT NOT NULL | 呼叫 ID |
| `digit` | TEXT NOT NULL | DTMF 按键 |
| `source` | TEXT NOT NULL | RTP / SIP-INFO |
| `timestamp_ms` | BIGINT NOT NULL | 时间戳 |
| `rtp_timestamp` | BIGINT | RTP 时间戳 |
| `duration_ms` | INTEGER | 持续时间 |
| `volume` | INTEGER | 音量 |
| `inserted_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

### 4.7 billing_rates — 费率表

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | TEXT | PRIMARY KEY |
| `prefix` | TEXT NOT NULL | 被叫号码前缀（最长匹配） |
| `rate_per_minute` | DOUBLE NOT NULL | 每分钟费率 |
| `description` | TEXT | 描述 |
| `created_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

### 4.8 billing_accounts — 账户余额

| 列 | 类型 | 说明 |
|----|------|------|
| `username` | TEXT | 用户名（PK） |
| `balance` | DOUBLE | 余额，默认 0 |
| `currency` | TEXT | 币种，默认 CNY |
| `created_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

### 4.9 billing_ledger — 账单流水

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | BIGSERIAL | PRIMARY KEY |
| `call_id` | TEXT NOT NULL | UNIQUE，幂等键 |
| `username` | TEXT NOT NULL | 用户名 |
| `duration_ms` | BIGINT NOT NULL | 通话时长 |
| `rate_per_minute` | DOUBLE NOT NULL | 费率 |
| `amount` | DOUBLE NOT NULL | 扣费金额 |
| `balance_after` | DOUBLE NOT NULL | 扣费后余额 |
| `created_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

### 4.10 number_inventory — 号码库存

| 列 | 类型 | 说明 |
|----|------|------|
| `number` | TEXT | 号码（PK） |
| `username` | TEXT | 绑定用户 |
| `status` | TEXT NOT NULL | available / assigned / reserved |
| `created_at` | TIMESTAMPTZ NOT NULL | DEFAULT now() |

---

## 5. CDR 数据流路径

### 路径 A：PostgreSQL 直写（无 NATS）

```
sip-edge → flush_completed_cdrs() → PostgresCdrStore::insert_call_cdr()
                                          ↓
                                     PostgreSQL
                                          ↓
                              api-server (REST 查询)
```

**触发条件**：设置了 `VOS_RS_DATABASE_URL` 且 **未设置** `VOS_RS_NATS_URL`。

### 路径 B：NATS JetStream + cdr-worker

```
sip-edge → flush_completed_cdrs() → NatsCdrPublisher::publish_cdr()
                                          ↓
                                    NATS JetStream
                                          ↓
                                   cdr-worker (批量消费)
                                          ↓
                              PostgresCdrStore::insert_events_batch()
                                          ↓
                                     PostgreSQL
                                          ↓
                              api-server (REST 查询)
```

**触发条件**：同时设置了 `VOS_RS_NATS_URL` 和 `VOS_RS_DATABASE_URL`。

### 路径 C：无存储

```
sip-edge → flush_completed_cdrs() → take_completed_cdrs() → drop
```

**触发条件**：既未设置 `VOS_RS_DATABASE_URL` 也未设置 `VOS_RS_NATS_URL`。CDR 仅在内存中短暂存在后丢弃。

---

## 6. 典型部署配置示例

### 最小开发环境（纯内存）

```bash
VOS_RS_SIP_UDP_BIND=0.0.0.0:5060
VOS_RS_SIP_ADVERTISED_ADDR=192.168.1.100:5060
VOS_RS_SIP_DEFAULT_GATEWAY=sip:gw.example.com:5060
VOS_RS_SBC_ALLOW=0.0.0.0/0
```

### 生产环境（PostgreSQL + NATS + 认证）

```bash
# SIP
VOS_RS_SIP_UDP_BIND=0.0.0.0:5060
VOS_RS_SIP_ADVERTISED_ADDR=203.0.113.10:5060
VOS_RS_SIP_DEFAULT_GATEWAY=sip:gw.provider.com:5060
VOS_RS_SIP_TLS_CERT_PATH=/etc/ssl/sip.pem
VOS_RS_SIP_TLS_KEY_PATH=/etc/ssl/sip.key

# 认证
VOS_RS_SIP_AUTH_USERS=admin:secret123

# 媒体
VOS_RS_RTP_ADVERTISED_ADDR=203.0.113.10
VOS_RS_RTP_PORT_MIN=40000
VOS_RS_RTP_PORT_MAX=40100
VOS_RS_RECORDING_ENABLED=true
VOS_RS_RECORDING_DIR=/var/lib/vos-rs/recordings

# NAT 穿越
VOS_RS_STUN_SERVER=stun.l.google.com:19302,stun1.l.google.com:19302
VOS_RS_UPNP_ENABLED=true

# 存储
VOS_RS_DATABASE_URL=postgres://vos:pass@db:5432/vos_rs
VOS_RS_NATS_URL=nats://nats:4222

# SBC
VOS_RS_SBC_ALLOW=10.0.0.0/8,172.16.0.0/12,192.168.0.0/16
VOS_RS_SBC_MAX_CONCURRENCY=50
```

### 账单系统（在 PostgreSQL 基础上增加）

```sql
-- 插入费率
INSERT INTO billing_rates (id, prefix, rate_per_minute, description)
VALUES ('cn-mobile', '861', 0.10, '中国手机号');

-- 创建账户
INSERT INTO billing_accounts (username, balance, currency)
VALUES ('1001', 100.00, 'CNY');

-- 手动对账
-- POST /api/billing/reconcile
```
