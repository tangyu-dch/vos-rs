# vos-rs 配置文件与连接参数参考指南

本文档整理并归纳了 `vos-rs` 软交换平台中 `sip-edge`、`api-server` 以及 `cdr-worker` 服务的统一配置文件架构、数据库与 Redis 模块的连接池调优以及对象存储配置。

---

## 0. 配置架构设计：单一引导文件 + 数据库高动态覆盖

为了实现电信级的动态调优和单点集群管理，`vos-rs` 删除了所有散落在各服务中的环境变量和独立的配置文件，统一通过唯一环境变量 `VOS_RS_CONFIG_FILE` 指定引导配置文件（默认路径为 `config.yaml`）。

```
  ┌─────────────────┐       ┌─────────────────┐       ┌─────────────────┐
  │   config.yaml   │ ───>  │  PostgreSQL     │ ───>  │  Redis 缓存 Hash │
  │  Bootstrap 引导  │       │  system_configs │       │  (vos_rs:...)   │
  └─────────────────┘       └─────────────────┘       └─────────────────┘
```

1. **统一引导（Bootstrap）**：在系统启动时，各服务通过读取 `config.yaml` 获取底层基础设施连接（PostgreSQL、Redis、NATS、S3）与网络侦听端口。
2. **连接池与性能调优**：支持在数据库与 Redis 模块中单独配置最大连接数（`max_connections`），提升高 CPS 吞吐下的并发处理能力。
3. **缓存双写与高可用容灾**：
   * 页面进行参数更新时，配置项会**双写**更新至 PostgreSQL 并以 `HSET` 命令同步至 Redis 中（键名为 `vos_rs:system_configs`）。
   * `sip-edge` 边缘网关启动时，**优先读取 Redis** 获取高并发实时参数配置覆盖；当 Redis 不可用或发生局部网络故障时，网关将**自动降级 Fallback 直读 PostgreSQL** 获取数据，保障电信级的运行稳定性。

---

## 1. 唯一环境变量

在启动系统时，仅保留以下环境变量作为配置文件引导入口：

| 环境变量名称 | 强制性 | 默认值 | 作用描述 |
| :--- | :--- | :--- | :--- |
| `VOS_RS_CONFIG_FILE` | *可选* | `config.yaml` | 指定系统统一的主配置文件路径 |

---

## 2. 统一配置文件 `config.yaml` 架构

在项目根目录下或通过 `VOS_RS_CONFIG_FILE` 指定的路径中，`config.yaml` 必须包含以下嵌套的多级功能分级结构：

```yaml
# ==========================================
# 1. 基础设施连接配置 (Connections)
# ==========================================
connections:
  database:
    host: "127.0.0.1"
    port: 5432
    username: "tangyu"
    password: ""
    database: "vos_rs"
    max_connections: 50      # PostgreSQL 最大连接池大小
  redis:
    host: "127.0.0.1"
    port: 6379
    password: ""
    database: 0
    max_connections: 50      # Redis 客户端最大连接数
  nats:
    url: "nats://127.0.0.1:4222"
    cdr_stream: "VOS_RS_CDR"
    cdr_subject: "vos_rs.cdr"
  s3:
    backend: "s3"            # 存储类型: s3 / local
    local_dir: "recordings"  # 本地缓存录音目录
    endpoint: "https://s3.amazonaws.com"
    bucket: "vos-rs-recordings"
    access_key: "my-access-key"
    secret_key: "my-secret-key"
    region: "us-east-1"
    key_prefix: "recordings/"

# ==========================================
# 2. sip-edge 信令边缘网关服务配置
# ==========================================
sip_edge:
  network:
    sip_udp_bind: "0.0.0.0:5060"
    advertised_addr: "127.0.0.1:5060"
    manage_bind: "127.0.0.1:8082"
  routing:
    default_gateway: ""
  nat_traversal:
    stun_server: ""
    upnp_enabled: false

# ==========================================
# 3. api-server 管理控制台后端服务配置
# ==========================================
api_server:
  network:
    port: 8081
    allowed_origins: ""
  security:
    jwt_secret: "secret"
    internal_secret: "internal-dev-secret"
  admin_credentials:
    admin_password: "admin"
    operator_password: "operator"
    financier_password: "financier"

# ==========================================
# 4. cdr-worker 话单入库消费者服务配置
# ==========================================
cdr_worker:
  queue:
    nats_cdr_consumer: "vos_rs_cdr_worker"
    nats_cdr_dlq_subject: "vos_rs.cdr.dlq"
    nats_cdr_dlq_stream: "VOS_RS_CDR_DLQ"
  batch_settings:
    max_batch_size: 100
    batch_timeout_ms: 1000
    max_deliveries: 3
    nak_delay_ms: 5000
    db_retry_attempts: 5
```

---

## 3. PostgreSQL 中高动态配置项对照表 (`system_configs`)

所有的**媒体服务**、**SBC安全**、以及**录音选项**均持久化在 `system_configs` 数据库表中。前端通过 `/api/system/configs` 接口进行配置修改时，配置修改动作会被双写刷新至 Redis：

### 3.1 SIP 与会话设置

| 配置项 Key | 典型数值 | 配置描述 |
| :--- | :--- | :--- |
| `session_expires_gateway` | `600` | 出站网关 SIP 会话过期定时器（秒） |
| `session_expires_caller` | `1800` | 普通呼叫方会话过期定时器（秒） |

### 3.2 RTP 与媒体 Relay 规则

| 配置项 Key | 典型数值 | 配置描述 |
| :--- | :--- | :--- |
| `rtp_advertised_addr` | `127.0.0.1` | RTP 媒体包流在 SDP 改写中通告的外部 IP 地址 |
| `rtp_port_min` | `40000` | 中继 RTP 通道动态分配的起始 UDP 端口 |
| `rtp_port_max` | `40100` | 中继 RTP 通道动态分配的结束 UDP 端口 |
| `rtp_symmetric_learning` | `true` | 是否对对称 NAT 下的 RTP 音频包源地址进行自动学习改写 |
| `rtp_anti_spoofing` | `true` | 是否启用 RTP 防欺诈攻击过滤 |
| `rtp_source_relearn_secs` | `30` | 发生媒体切换时，RTP 重学习的锁定期限（秒） |

### 3.3 SBC 与限流防欺诈控制

| 配置项 Key | 典型数值 | 配置描述 |
| :--- | :--- | :--- |
| `sbc_rate_limit_capacity` | `2000.0` | 令牌桶并发限流容量大小 |
| `sbc_rate_limit_fill_rate` | `500.0` | 令牌桶每秒充填速率 |
| `sbc_max_concurrency` | `2000` | 网关单机最大并发通话数（CPS限制） |
