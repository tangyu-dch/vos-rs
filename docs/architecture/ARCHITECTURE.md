# VOS-RS 项目逻辑全梳理与 VOS 对比分析

> 本文档从架构、呼叫流程、媒体、CDR、计费、安全、前端等维度全面梳理 VOS-RS 项目逻辑，
> 并与商用 VOS（昆石/华为 VOS）进行逐项对比，标注不合理之处和优化建议。

---

## 一、项目架构总览

```
                          ┌──────────────┐
                          │  Web 前端     │  React 18 + Arco Design + ECharts
                          │  (端口 3000)  │
                          └──────┬───────┘
                                 │ HTTP
                          ┌──────▼───────┐
                          │  api-server  │  Axum REST (端口 8080)
                          │  CDR查询/管理 │  30+ REST 端点
                          └──────┬───────┘
                                 │ SQLx
                          ┌──────▼───────┐
                          │  PostgreSQL  │  CDR/用户/网关/路由/计费/号码
                          └──────▲───────┘
                                 │ SQLx/NATS
    ┌──────────────┐     ┌──────┴───────┐     ┌──────────────┐
    │  呼叫方(UAC)  │────▶│  sip-edge    │────▶│  网关(UAS)    │
    │  (坐席/分机)  │◀────│  B2BUA       │◀────│  (运营商)     │
    └──────────────┘     │  RTP中继+录音 │     └──────────────┘
                         └──────┬───────┘
                                │ NATS JetStream
                         ┌──────▼───────┐
                         │ cdr-worker   │  异步CDR写入
                         └──────────────┘
```

### 代码量统计

| 模块 | 行数 | 语言 |
|------|------|------|
| sip-edge | ~26,000 | Rust |
| call-core | ~350 | Rust |
| cdr-core | ~1,700 | Rust |
| storage-core | ~400 | Rust |
| api-server | ~500 | Rust |
| cdr-worker | ~100 | Rust |
| 前端 | ~8,000 | TypeScript/React |
| **合计** | **~37,000** | |

---

## 二、呼叫流程（核心逻辑）

### 2.1 坐席呼出流程（出站呼叫）

```
坐席(UAC)                 sip-edge(B2BUA)              网关(UAS)
    │                          │                           │
    │── INVITE ───────────────▶│                           │
    │   (From: 1001)           │                           │
    │                          │── 100 Trying ────────────▶│
    │◀── 100 Trying ──────────│                           │
    │                          │── INVITE ────────────────▶│  (拓扑隐藏：修改Call-ID)
    │                          │                           │
    │◀── 180 Ringing ─────────│◀── 180 Ringing ──────────│
    │                          │                           │
    │◀── 200 OK (SDP) ────────│◀── 200 OK (SDP) ─────────│  (媒体协商 + 录音开始)
    │── ACK ──────────────────▶│── ACK ──────────────────▶│
    │                          │                           │
    │◀═══ RTP (音频) ════════▶│◀═══ RTP (音频) ═════════▶│  (RTP中继 + 录音)
    │                          │                           │
    │── BYE ─────────────────▶│── BYE ──────────────────▶│
    │◀── 200 OK ──────────────│◀── 200 OK ───────────────│  (CDR生成)
```

**关键代码路径**：
- INVITE 入站处理：`services/sip-edge/src/sip/handlers/invite.rs` → 路由选择 → 拓扑隐藏映射登记 → 转发到网关
- 路由选择：`crates/call-core/src/routing/table.rs` → `PrefixTrie` 前缀匹配树 → LCR + 加权负载均衡
- 拓扑隐藏：`services/sip-edge/src/sip/outbound.rs` (出站 Call-ID 覆写) & `services/sip-edge/src/sip/response.rs` (入站响应还原)
- 200 OK 处理：`services/sip-edge/src/sip/handlers/in_dialog.rs` → SDP 重写 → RTP 中继建立 → 录音开始
- BYE 处理：`services/sip-edge/src/sip/handlers/in_dialog.rs` → CDR 生成 → 录音停止


### 2.2 来电流程（入站呼叫）

```
外部呼叫方                sip-edge(B2BUA)              坐席(UAS)
    │                          │                           │
    │── INVITE ───────────────▶│                           │
    │   (From: 外部号码)       │                           │
    │                          │── 100 Trying ────────────▶│
    │                          │── INVITE ────────────────▶│  (路由到注册坐席)
    │◀── 180 Ringing ─────────│◀── 180 Ringing ──────────│
    │◀── 200 OK ──────────────│◀── 200 OK ───────────────│
    │◀═══ RTP ═══════════════▶│◀═══ RTP ═══════════════▶│
```

### 2.3 CPS 测试中的呼入呼出区分

- **坐席呼出**（outbound）：`From: "1001" <sip:1001@...>`，主叫号码是坐席分机号
- **外部来电**（inbound）：`From: "ext-2001" <sip:2001@...>`，主叫是外部号码

SIP 头部 `X-Call-Direction` 标记呼叫方向，sip-edge 读取后写入 CDR。

---

## 三、各模块详细逻辑

### 3.1 SIP 协议处理（sip-core + sip-edge）

| 功能 | 实现位置 | VOS 对比 |
|------|----------|----------|
| SIP 消息解析 | `sip-core` — 自研解析器 | VOS 用 OSIP/sofia-sip |
| UDP/TCP/TLS/WebSocket | `transport.rs` + `main.rs` UDP 循环 | VOS 支持更多传输 |
| SIP Digest 认证 | `auth.rs` + 数据库 fallback | VOS 用 LDAP/数据库 |
| 事务管理 | `transaction.rs` — RFC 3261 定时器 | VOS 同 |
| 重传处理 | `main.rs` — 500ms T1 定时器 | VOS 同 |

**不合理之处**：
- ⚠️ 单线程 UDP 事件循环是性能瓶颈（~20 CPS 上限）
- VOS 用多线程 + epoll，可达 200+ CPS

### 3.2 路由引擎（call-core/routing.rs）

**路由选择算法**：
1. 从 `sip_routes` 表加载路由规则（含 weight 字段）
2. 按被叫号码最长前缀匹配（prefix length DESC）
3. 同前缀按优先级排序（priority DESC，数字越大越优先）
4. 同优先级按成本升序（cost ASC，LCR 最低成本路由）
5. 同等条件下按权重加权随机（weight DESC/random）
6. 检查时间窗口（time_start/time_end）
7. 检查网关健康状态（GatewayHealthTracker circuit breaker）
8. 检查并发容量（max_capacity / current_concurrent）
9. 只对最终选中的网关执行 acquire（HalfOpen probe 保护）

**前缀规则**：`abc:def`（替换）、`:def`（添加前缀）、`abc:`（剥离前缀），逗号分隔多条。

**VOS 对比**：
- VOS 有完整的 LCR + 时间路由 + 负载均衡 + 故障转移
- VOS-RS 已实现：权重负载均衡、故障转移（failover 408/5xx）、网关 circuit breaker
- VOS-RS 缺少：基于 ACD/ASR 的动态路由

### 3.3 媒体处理与可编程控制（media/）

| 功能 | 实现 | VOS 对比 |
|------|------|----------|
| RTP 中继 | DashMap 端口映射 + tokio UDP | VOS 用专用媒体服务器 |
| 录音 | 双声道 WAV 8kHz/16bit | VOS 支持多格式 + 云存储 |
| DTMF | RFC 2833 检测 + SIP-INFO | VOS 同 + 中继 |
| 端口分配 | **无锁 (Lock-free) 位图分配** (基于有界原子位图 `PortLeaseMap` 无锁分配，偶数优先奇数配对，规避锁竞争) | VOS 用端口池 |
| 媒体控制 API | 支持实时 `/play`、`/stop-play`、`/mute`、`/unmute` 与 `/status` 状态轮询 | VOS 用外置 IVR 服务器 |
| 重采样引擎 | 解码时利用线性插值自动将 16kHz/44.1kHz 等 WAV 音频降采样至 8000Hz PCM | VOS 不直接支持多格式重采样 |
| 终端连续性同步 | 首包 Marker Bit 重置 + Exclusive 播放结束后 SSRC 序列号与时间戳平滑重写 (Smooth Transition) | VOS 依赖外置 SBC 或流媒体转接 |

**已优化与设计要点**：
- **无锁端口分配**：消除了高并发下 `std::net::UdpSocket::bind` 系统调用对全局 Mutex 锁的长时间占用，实测并发吞吐显著提升。
- **可编程媒体控制**：为 AI Agent 实时语音注入提供了底层电信级流平滑过渡保证，终端切音无底噪和丢包。
- ⚠️ 录音文件直接写本地磁盘，无压缩/转码。

### 3.4 CDR 系统

**CDR 数据流**：
```
sip-edge → CallManager.completed_cdrs → flush_completed_cdrs → PostgreSQL
         → 或 → NatsCdrPublisher → NATS JetStream → cdr-worker → PostgreSQL
```

**CDR 字段**：call_id, caller, callee, started_at, answered_at, ended_at, duration, status, recording_path, direction, RTCP 指标, MOS, DTMF

**VOS 对比**：
- VOS CDR 支持更丰富的字段：ANI/DNIS、转发号码、ACD 信息
- VOS-RS 缺少：呼叫转移详情、会议桥信息、ACD 排队信息

### 3.5 计费系统

| 表 | 用途 |
|----|------|
| `billing_rates` | 费率表（prefix → rate_per_minute） |
| `billing_accounts` | 账户余额（username → balance） |
| `billing_ledger` | 扣费流水 |

**VOS 对比**：
- VOS 有完整的实时计费 + 预付费/后付费 + 信用控制
- VOS-RS 只有基础的费率查询和余额扣减，缺少实时余额检查

### 3.6 安全（SBC + 防盗打与爆破锁）

**SBC 功能**（`sbc.rs`）：
- **IP 白名单/黑名单**（CIDR 支持，已支持通过 API 热重载而无需重启）
- **令牌桶限速**（基于 DashMap 的高并发分片锁实现）
- **每 IP 并发限制**
- **防暴力破解 IP 锁定**（60 秒内连续 5 次鉴权失败自动锁定 IP 10 分钟）

**防盗打**（AntiFraud）：
- IP 黑/白名单
- 号码黑名单/白名单
- CPS 限速
- 每账户/IP 并发限制

**VOS 对比**：
- VOS 有更完善的 SBC：NAT 穿越、SIP 压缩、拓扑隐藏、协议转换
- VOS-RS 已实现基础 SBC ACL 规则热重载及爆破动态封锁；但缺少：SIP 压缩、协议转换。

### 3.7 存储抽象（storage-core）

支持三种后端：
- **Local**：本地文件系统
- **OSS**：阿里云 OSS / MinIO（S3 协议兼容）
- **Dual**：双写模式（主 OSS + 备本地）

**VOS 对比**：
- VOS 通常用 NFS 或 SAN 存储录音
- VOS-RS 的 OSS 支持更灵活，适合云部署

---

## 四、前端管理控制台（14 个页面）

| 页面 | 路由 | 功能 |
|------|------|------|
| 仪表盘 | `/dashboard` | 呼叫量趋势、接通率、MOS、注册用户 |
| 活跃呼叫 | `/active-calls` | 实时通话列表、强制挂断 |
| 呼叫记录 | `/cdr` | CDR 分页查询、录音播放、方向标识 |
| 报表 | `/reports` | 快捷时间范围、6 指标卡片、4 图表、状态明细 |
| SIP 用户 | `/users` | 用户 CRUD |
| 落地网关 | `/gateways` | 出站网关管理 |
| 对接网关 | `/peer-gateways` | 入站对接管理 |
| 路由管理 | `/routes` | 路由规则 CRUD |
| 注册信息 | `/registrations` | SIP 注册列表 |
| 号码库存 | `/numbers` | 号码分配管理 |
| 费率 | `/rates` | 费率 CRUD |
| 账户 | `/accounts` | 账户余额管理 |
| 录音 | CDR 详情 | 在呼叫记录详情中在线播放和下载 |
| 防盗打 | `/anti-fraud` | 防盗打规则管理 |

---

## 五、与 VOS 的逐项对比

### 5.1 已实现（与 VOS 相当）

| 功能 | VOS-RS | VOS |
|------|--------|-----|
| SIP B2BUA | ✅ UDP/TCP/TLS/WS | ✅ |
| 拓扑隐藏 | ✅ Call-ID 翻译 | ✅ |
| 路由引擎 | ✅ 最长前缀 + LCR + 时间窗口 | ✅ 更完善 |
| 呼叫录音 | ✅ 双声道 WAV | ✅ 多格式 |
| CDR | ✅ PostgreSQL | ✅ |
| SIP 认证 | ✅ Digest | ✅ LDAP/DB |
| SBC | ✅ IP ACL + 限速 | ✅ 更完善 |
| RTP 中继 | ✅ | ✅ |
| DTMF | ✅ RFC 2833 + SIP-INFO | ✅ |
| Web 管理 | ✅ 14 页面 | ✅ 通常无 Web UI |
| 录音存储 | ✅ 本地/OSS/双写 | ✅ NFS/SAN |

### 5.2 性能与高级功能状态 (与 VOS 差异项说明)

| 功能 | VOS | VOS-RS 实现状态 | 优先级 |
|------|-----|----------------|--------|
| **多线程 SIP 处理** | 多线程 epoll | ✅ 已完整支持 (基于 Call-ID 哈希的多线程 Tokio 并发调度，解决并发性能瓶颈) | 已完成 |
| **呼叫转移** | blind/attended transfer | ✅ 已完整支持 (盲转、桥接、通知、失败回滚) | 已完成 |
| **呼叫保持** | hold/resume + MOH | ✅ 已支持 (Re-INVITE 媒体重新协商与 RTP 挂起) | 已完成 |
| **实时计费** | 余额预检 + 实时扣费 | ✅ 已完整支持 (余额预检拒接、最大时长定时器看门狗及自动发送 BYE 报文硬拆线) | 已完成 |
| **录音转码** | GSM/AMR/Opus | ✅ 已支持 (挂断后通过 ffmpeg 异步转码为 Opus/AMR，并自动清理 WAV 录音) | 已完成 |
| **多租户** | 域隔离 + 资源配额 | ✅ 已支持基本的 Tenant 隔离与分流 | 已完成 |
| **集群部署** | 多节点 + 共享状态 | ✅ 已支持 (Redis 存储注册状态及全局同步，实现跨节点状态感知) | 已完成 |
| **会议桥** | 多方通话 | ✅ 已完整支持 (基于单腿 UAS 呼入集成，混音器周期 20ms Mix-Minus 算法并自动拆线清理) | 已完成 |


### 5.3 已优化与修复的不合理项

| 问题 | 说明 | 修复与改进措施 |
|------|------|----------------|
| **端口分配 Mutex 锁** | 原顺序分配占用全局锁，bind 阻塞严重 | **已修复**：引入基于有界原子位图 (PortLeaseMap) 的无锁 (Lock-free) 偶奇优先端口分配机制 |
| **呼叫保持与协商** | 原重协商时流匹配混乱 | **已修复**：增加了显式的 Dialog 会话状态重谈判与 SSRC 追踪 |
| **呼叫转移 (REFER)** | 原转接易出现状态失步 | **已修复**：引入完整的 REFER 状态机与松散路由 (Loose Routing) 支撑 |
| **路由无权重** | 原仅按 Cost 最低成本路由排序 | **已修复**：已支持 weight 字段与加权随机负载均衡选路 |
| **录音同步 I/O 阻塞** | 原录音包同步写盘，高 CPS 下发生写阻塞 | **已修复**：引入懒加载、内存双通道缓冲及固定 Worker 线程池异步刷盘 |
| **录音文件名易冲突** | 原使用时间戳命名 | **已修复**：改用 UUID `call_id.wav` 结合分段轮转机制 |


---

## 六、数据模型（PostgreSQL）

### 核心表

| 表名 | 用途 | 关键字段 |
|------|------|----------|
| `call_cdrs` | 呼叫详单 | call_id, caller, callee, status, recording_path, direction |
| `sip_gateways` | 网关配置 | id, host, port, gateway_type, max_concurrent |
| `sip_routes` | 路由规则 | prefix, priority, gateway_id, cost, weight, time_start/end |
| `sip_users` | SIP 用户 | username, password |
| `sip_registrations` | 注册绑定 | aor, contact_uri, expires_at |
| `billing_rates` | 费率表 | prefix, rate_per_minute |
| `billing_accounts` | 账户余额 | id, username, balance |
| `billing_ledger` | 扣费流水 | call_id, username, amount |
| `number_inventory` | 号码库存 | number, status, direction |
| `dtmf_events` | DTMF 审计 | call_id, digit, source, timestamp_ms |

---

## 七、配置文件与引导规范

详见 [`docs/development/ENV_VARS.md`](file:///Users/tangyu/Projects/vos-rs/docs/development/ENV_VARS.md)。

系统已删除所有分散的环境变量，统一通过唯一环境变量 `VOS_RS_CONFIG_FILE` 指定引导配置文件（默认路径为 `config.yaml`），在其中配置连接池、数据库与 Redis 端口、S3 对象存储以及 sip-edge 网络和路由选项。

---

## 八、优化路线图

### P0 — 性能与架构（已完成）
1. **多线程 UDP 处理**：已优化为基于 Call-ID 哈希的多线程 Tokio Worker 调度，分流大并发。
2. **端口分配优化**：已引入基于原子位图 (PortLeaseMap) 的 Lock-free 偶奇优先端口分配机制，规避 bind 系统调用阻塞 Mutex。
3. **注册集群同步**：已将注册绑定同步写入 Redis 并设置 TTL，多节点运行可以相互路由。

### P1 — 功能补全（已完成）
1. **呼叫转移**：盲转（REFER）与 attended transfer 已完整支持。
2. **呼叫保持**：Re-INVITE 媒体重新协商与 RTP 挂起已完成。
3. **实时计费与拆线**：已支持呼叫前余额校验、最大允许时长监测以及超时强制 BYE 报文硬拆线。
4. **录音格式转码**：已实现挂断后通过 ffmpeg 异步将 WAV 转码为 Opus/AMR 压缩格式并清理原始 WAV 文件。
5. **会议桥**：已实现拨号直接加入会议室、UAS 协商、20ms 内 RTP 解码混音（Mix-Minus）与挂断时自动 leaves 资源清理。

### P2 — 进一步展望
1. **ACD 排队**：技能路由 + 排队音乐 + 溢出策略。
2. **LDAP 认证**：对接 Active Directory / LDAP 集中账户。
3. **CDR 增强**：主叫 ANI / DNIS 细化、转接链路标识。

