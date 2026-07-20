<div align="center">

# VOS-RS

**电信运营级 VoIP 软交换与媒体转发平台 · Rust 实现**

[![Rust](https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![Edition](https://img.shields.io/badge/Edition-2021-blue)](https://doc.rust-lang.org/edition-guide/)
[![License](https://img.shields.io/badge/License-Proprietary-red)](./Cargo.toml)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%2B-blue?logo=postgresql)](https://www.postgresql.org/)
[![NATS](https://img.shields.io/badge/NATS-JetStream-blue?logo=nats.io)](https://nats.io/)
[![React](https://img.shields.io/badge/React-18-blue?logo=react)](https://react.dev/)
[![TailwindCSS](https://img.shields.io/badge/TailwindCSS-v4-blue?logo=tailwindcss)](https://tailwindcss.com/)
[![HeroUI](https://img.shields.io/badge/HeroUI-v2.8-purple)](https://www.heroui.com/)

对标商业软交换 VOS-3000 · 单机 5000+ 并发通话 · 1000+ CPS · AI-Native 可编程媒体控制

</div>

---

## 📖 目录

- [✨ 项目简介](#-项目简介)
- [📸 界面预览](#-界面预览)
- [🚀 核心特性](#-核心特性)
- [🛠 技术栈](#-技术栈)
- [🏗 系统架构](#-系统架构)
- [📂 项目结构](#-项目结构)
- [⚡ 快速开始](#-快速开始)
- [⚙️ 配置说明](#️-配置说明)
- [🧪 测试与压测](#-测试与压测)
- [🚢 部署指南](#-部署指南)
- [📊 性能指标](#-性能指标)
- [🤖 AI 集成](#-ai-集成)
- [🗺 路线图](#-路线图)
- [❓ FAQ](#-faq)
- [🤝 贡献指南](#-贡献指南)
- [📄 许可证](#-许可证)
- [🙏 致谢](#-致谢)

---

## ✨ 项目简介

`vos-rs` 是用 Rust 编写的**电信运营级 VoIP 软交换与媒体转发平台**，对标商业软交换 VOS-3000，旨在单机环境下实现 **5000+ 并发通话** 和 **1000+ CPS** 的超高性能要求。

平台采用 **信令与媒体分离** 的设计原则，集成了：

- **SIP B2BUA 信令代理**：完整 RFC 3261 事务状态机、PRACK/Session-Expires、3xx 重定向
- **对称 RTP 媒体中继**：Opus ↔ G.711 实时热转码、DTMF 检测、WAV 录音
- **SBC 安全防御**：IP ACL、令牌桶限速、Digest 认证、租户隔离
- **路由与计费**：LCR 前缀匹配、网关健康探测、实时余额扣减、CDR 话单
- **NAT 穿透**：STUN 公网映射、UPnP 端口映射、对称 RTP 学习
- **REST API + Web 控制台**：30+ 业务端点 + React 可视化管理界面
- **AI-Native 可编程媒体接口**：热插拔媒体控制，对接 AI Voice Agent / IVR / TTS / ASR

---

## 📸 界面预览

> 全部截图取自实际运行的控制台（深色主题），完整图集见 [`docs/assets/`](./docs/assets/)。

### 登录页

![登录页](./docs/assets/login.png)

### 仪表盘（运营总览）

![仪表盘](./docs/assets/dashboard.png)

### 活跃通话监控

![活跃通话](./docs/assets/active-calls.png)

### 分机管理

![分机管理](./docs/assets/extensions.png)

### 中继管理

![中继管理](./docs/assets/trunks.png)

### 路由配置

![路由配置](./docs/assets/routing.png)

### 计费账户

![计费账户](./docs/assets/billing-accounts.png)

### 系统设置（响应式自适应）

桌面宽屏（3 列布局）：

![系统设置-桌面](./docs/assets/settings.png)

窄屏自适应（2 列布局）：

![系统设置-窄屏](./docs/assets/settings-narrow.png)

### 安全防护（SBC / TLS）

![安全防护](./docs/assets/security.png)

### 集群基础设施

![基础设施](./docs/assets/infrastructure.png)

---

## 🚀 核心特性

### 信令与媒体

| 能力 | 说明 |
| :--- | :--- |
| **多传输支持** | UDP / TCP / TLS / WebSocket |
| **完整事务状态机** | RFC 3261 INVITE / BYE / REFER / PRACK (RFC 3262) / Session-Expires (RFC 4028) / 3xx 重定向 |
| **零拷贝 SIP 解析** | 自研 `sip-core`，借用类型直接引用接收缓冲区，消除高频堆分配 |
| **对称 RTP 中继** | 高并发无锁端口分配，转码器上下文作为协程局部变量 |
| **Opus ↔ G.711 转码** | 基于 `opus` + `rubato` FFI，G.711 查表法 $O(1)$ 加速 |
| **DTMF 检测** | 同时支持 SIP INFO 与 RFC 2833 带内按键 |
| **WAV 录音** | 双向/单向录音，`spawn_blocking` 隔离磁盘 I/O |

### 路由与计费

| 能力 | 说明 |
| :--- | :--- |
| **LCR 路由** | 前缀最长匹配 + 优先级备用 + 时间窗路由 |
| **网关熔断** | 主动健康探测，故障自动隔离与恢复 |
| **实时计费** | 余额预扣减 + 限时拆线 + 计费结算 |
| **CDR 话单** | PostgreSQL UNNEST 批量写入，NATS JetStream 异步事件流 |
| **反欺诈** | 并发/CPS 限制、号码黑白名单 |

### SBC 安全

| 能力 | 说明 |
| :--- | :--- |
| **IP ACL** | CIDR 网段黑白名单 |
| **令牌桶限速** | 单 IP / 全局 CPS 控制 |
| **Digest 认证** | 动态 Nonce 防重放 |
| **租户隔离** | 域名与号段强物理隔离 |
| **TLS 加密** | 自定义证书验证 |

### NAT 穿透

| 能力 | 说明 |
| :--- | :--- |
| **STUN** | 多服务器 Fallback 公网映射发现 |
| **UPnP** | 自动网关端口映射 |
| **Symmetric RTP** | 首包源地址学习 + keepalive 保活 |

---

## 🛠 技术栈

### 后端

| 层级 | 技术 | 版本 |
| :--- | :--- | :--- |
| 主语言 | Rust | ≥ 1.89 (Edition 2021) |
| 异步运行时 | Tokio (multi_thread) | =1.x |
| HTTP REST | Axum + tower-http | =0.7.x |
| 数据库 | sqlx (PostgreSQL) | =0.7.x |
| 消息队列 | async-nats (JetStream) | 最新 |
| 并发数据结构 | DashMap | =6.x |
| TLS | tokio-rustls + rustls | 最新 |
| 日志 | tracing + tracing-subscriber | =0.1.x |
| 错误处理 | thiserror (库) + anyhow (应用) | =1.x |

### 前端

| 层级 | 技术 | 版本 |
| :--- | :--- | :--- |
| 框架 | React + TypeScript | 18 / 5.3+ |
| 构建工具 | Vite | 5.x |
| 组件库 | HeroUI | ^2.8.0 |
| 样式 | Tailwind CSS v4 | ^4.3.3 |
| 路由 | React Router | 6.x |
| Toast | sonner | ^1.7.4 |
| 图标 | lucide-react | 最新 |

### 基础设施

| 组件 | 技术 |
| :--- | :--- |
| 数据库 | PostgreSQL 14+ (主数据 + CDR) |
| 消息队列 | NATS JetStream |
| 录音存储 | 本地 FS / 阿里云 OSS (双写) |
| 容器化 | Docker + Docker Compose |

---

## 🏗 系统架构

### 分层架构

```mermaid
flowchart TB
    subgraph Client[客户端]
        UA[SIP 终端<br/>硬电话/软电话]
        WEB[WebRTC 客户端<br/>浏览器/小程序]
    end

    subgraph SipEdge[sip-edge 服务]
        SBC[SBC 安全层<br/>IP ACL / 令牌桶 / Digest 认证]
        B2BUA[SIP B2BUA<br/>事务状态机 / Dialog]
        ROUTE[路由引擎<br/>LCR / 熔断 / 时间窗]
        BILL[计费引擎<br/>实时扣费 / 结算]
        MEDIA[媒体控制器<br/>RTP Relay / 转码 / 录音]
        SBC --> B2BUA
        B2BUA --> ROUTE
        ROUTE --> BILL
        BILL --> MEDIA
    end

    subgraph ApiServer[api-server 服务]
        REST[REST API<br/>Axum 30+ 端点]
        WEBHOOK[Webhook<br/>事件推送]
    end

    subgraph CdrWorker[cdr-worker 服务]
        CONSUMER[NATS 消费者<br/>批量落库]
    end

    subgraph Storage[存储层]
        PG[(PostgreSQL<br/>主数据 + CDR)]
        NATS[(NATS JetStream<br/>事件流)]
        OSS[(OSS / 本地 FS<br/>录音存储)]
    end

    UA <-->|SIP/RTP| SBC
    WEB <-->|SIP/WS/RTP| SBC
    WEB <-->|HTTP| REST
    REST <-->|读取/配置| PG
    B2BUA -.->|CDR 事件| NATS
    CONSUMER <-->|批量写入| PG
    MEDIA -.->|录音文件| OSS
    BILL <-->|余额/话单| PG
```

### 信令与媒体分离

```mermaid
flowchart LR
    subgraph 信令节点 轻量 CPU 密集
        S1[SIP 解析/生成]
        S2[事务状态机]
        S3[路由引擎]
        S4[计费引擎]
        S5[SDP 改写]
    end

    subgraph 媒体节点 重量级 IO + CPU
        M1[RTP/RTCP 收发]
        M2[NAT 穿透]
        M3[Codec 转码]
        M4[DTMF 检测]
        M5[录音]
        M6[Jitter Buffer]
    end

    S1 --> S2 --> S3 --> S4 --> S5
    S5 -.->|分配| M1
    M1 --> M2 --> M3 --> M4
    M1 --> M5
    M1 --> M6
```

### 通话建立流程

```mermaid
sequenceDiagram
    participant UAC as 主叫 UAC
    participant Edge as sip-edge B2BUA
    participant Route as 路由引擎
    participant Bill as 计费引擎
    participant UAS as 被叫 UAS

    UAC->>Edge: INVITE (SDP A)
    Edge->>Bill: 余额预检 + 预扣减
    Bill-->>Edge: 通过
    Edge->>Route: 路由查询 (前缀匹配)
    Route-->>Edge: 选定落地中继
    Edge->>Edge: SDP 改写 (Topology Hiding)
    Edge->>UAS: INVITE (SDP A')
    UAS-->>Edge: 100 Trying
    UAS-->>Edge: 180 Ringing
    UAS-->>Edge: 200 OK (SDP B)
    Edge->>Edge: SDP 改写 + 媒体分配
    Edge-->>UAC: 200 OK (SDP B')
    UAC->>Edge: ACK
    Edge->>UAS: ACK
    Note over UAC,UAS: 媒体通道建立 (RTP 双向中继)
    Note over Edge,Bill: 通话中实时计费
    UAC->>Edge: BYE
    Edge->>UAS: BYE
    UAS-->>Edge: 200 OK
    Edge->>Bill: 结算 + 释放预扣
    Edge->>Edge: 生成 CDR → NATS
```

---

## 📂 项目结构

```text
vos-rs/
├── crates/                       # 核心协议与业务模块 (零拷贝解析)
│   ├── sip-core/                 # SIP 信令语法树与解析器 (RFC 3261)
│   ├── rtp-core/                 # RTP/RTCP 封包解析与 SRTP 加密通道
│   ├── sdp-core/                 # SDP 媒体协商解析与重写工具
│   ├── call-core/                # 呼叫状态机、路由匹配与 CDR 生成器
│   ├── cdr-core/                 # 话单数据模型与 PostgreSQL 操作库
│   └── storage-core/             # 录音存储抽象层（本地磁盘与 OSS 双写）
│
├── services/                     # 独立二进制服务
│   ├── sip-edge/                 # 边缘信令与媒体代理 (B2BUA + RTP Relay + 录音)
│   ├── api-server/               # REST API 后端服务 (Axum 30+ 端点)
│   ├── cdr-worker/               # NATS 异步话单消费者
│   ├── media-edge/               # 独立媒体节点 (WebRTC / 转码)
│   └── sip-router/               # 分布式路由服务
│
├── web/                          # 前端管理界面 (React 18 + HeroUI v2 + Tailwind v4)
│   └── src/
│       ├── pages/
│       │   ├── operations/       # 运营监控（仪表盘、活跃通话）
│       │   ├── numbers/          # 号码管理（分机、号码池、DID）
│       │   ├── trunks/           # 中继管理（接入/落地/分组）
│       │   ├── call-center/      # 呼叫中心（坐席、队列、IVR）
│       │   ├── billing/          # 计费（账户、费率、交易、话单）
│       │   ├── system/           # 系统配置（路由、安全、基础设施、设置）
│       │   └── shared/           # 跨页面共享层
│       ├── components/           # 通用组件（ConsoleShell、detail-shell 等）
│       └── services/             # API 客户端与资源服务
│
├── docs/                         # 文档目录
│   ├── architecture/             # 架构与设计
│   ├── deployment/               # 部署指南
│   ├── development/              # 开发与环境配置
│   ├── user-guide/               # 用户操作指南
│   └── assets/                   # 截图与图片资源
│
├── tools/                        # SIPp 测试工具与场景脚本
├── scripts/                      # SQL 迁移与开发辅助脚本
├── deploy/                       # Docker Compose 部署配置
├── Cargo.toml                    # Workspace 根 (11 members)
├── Makefile                      # 常用命令
├── config.yaml                   # 默认配置
├── AGENTS.md                     # AI 编程助手指南
└── README.md                     # 本文件
```

---

## ⚡ 快速开始

### 运行环境要求

| 组件 | 版本 | 说明 |
| :--- | :--- | :--- |
| OS | Linux / macOS | 暂不支持 Windows |
| Rust | 1.89+ | Edition 2021 |
| PostgreSQL | 14+ | 主数据 + CDR |
| NATS Server | 2.10+ | JetStream 模式 |
| Node.js | 18+ | 前端构建 |
| Docker | 24+ | 可选，容器化部署 |

### 方式一：Docker Compose 一键启动（推荐）

```bash
# 1. 克隆仓库
git clone <repo-url> vos-rs && cd vos-rs

# 2. 启动所有服务（Postgres + NATS + S3 + sip-edge + api-server + 前端）
docker compose -f deploy/docker/docker-compose.yml up -d --build

# 3. 访问管理后台
# 地址: http://localhost:3000
# 默认账号: admin / admin
```

### 方式二：本地开发调试

```bash
# 1. 创建数据库
createdb vos_rs

# 2. 启动依赖服务（PostgreSQL 5432, NATS 4222）
# 参考 docs/development/ENV_VARS.md 配置 .env

# 3. 执行数据库迁移
make db-migrate

# 4. 一键启动三端进程（sip-edge + api-server + 前端 Dev Server）
./scripts/dev.sh

# 5. 访问 http://localhost:3000
```

### 方式三：从源码构建

```bash
# 1. 构建后端 workspace
cargo build --workspace --release

# 2. 构建前端
cd web && npm install && npm run build

# 3. 启动 sip-edge
./target/release/sip-edge

# 4. 启动 api-server
./target/release/api-server

# 5. 部署前端（nginx 托管 web/dist）
```

---

## ⚙️ 配置说明

所有配置通过 `VOS_RS_` 前缀环境变量加载，完整列表见 [`docs/development/ENV_VARS.md`](./docs/development/ENV_VARS.md)。

### 核心配置示例

```bash
# === 数据库与消息队列 ===
VOS_RS_DATABASE_URL=postgres://user:pass@localhost:5432/vosrs
VOS_RS_NATS_URL=nats://localhost:4222

# === SIP 信令 ===
VOS_RS_SIP_BIND=0.0.0.0:5060                      # SIP 监听地址
VOS_RS_SIP_ADVERTISED_ADDR=1.2.3.4:5060           # 对外通告地址
VOS_RS_SIP_TLS_BIND=0.0.0.0:5061                  # TLS 监听 (可选)
VOS_RS_SIP_TLS_CERT_PATH=/path/cert.pem
VOS_RS_SIP_TLS_KEY_PATH=/path/key.pem

# === RTP 媒体 ===
VOS_RS_RTP_ADVERTISED_ADDR=1.2.3.4                # RTP 对外地址
VOS_RS_RTP_PORT_MIN=40000                          # RTP 端口范围起始
VOS_RS_RTP_PORT_MAX=40100                          # RTP 端口范围结束
VOS_RS_RTP_SYMMETRIC_LEARNING=true                # 对称 RTP 学习

# === 录音 ===
VOS_RS_RECORDING_ENABLED=false
VOS_RS_RECORDING_DIR=/var/lib/vos-rs/recordings

# === 认证 ===
VOS_RS_AUTH_ENABLED=true                           # SIP Digest Auth
VOS_RS_AUTH_REALM=vos-rs

# === SBC 安全 ===
VOS_RS_SBC_ALLOW=192.168.1.0/24                    # IP 白名单 (CIDR)
VOS_RS_SBC_BLOCK=                                  # IP 黑名单
VOS_RS_SBC_LIMIT_CAPACITY=100                      # 令牌桶容量
VOS_RS_SBC_LIMIT_FILL_RATE=10                      # 令牌填充速率

# === 日志 ===
RUST_LOG=info
# 或分模块: RUST_LOG=sip_edge=debug,media=trace

# === UDP Workers ===
VOS_RS_UDP_WORKERS=0                               # 0=auto (CPU 核心数)
```

---

## 🧪 测试与压测

### 测试金字塔

```mermaid
flowchart TB
    E2E[E2E 测试<br/>SIPp 端到端场景]
    INT[集成测试<br/>模块协作]
    UNIT[单元测试<br/>核心逻辑]

    E2E --> INT --> UNIT
```

### 测试命令

```bash
# === 代码质量 ===
cargo clippy --workspace -- -D warnings    # Lint 检查
cargo fmt --check                          # 格式化检查
cargo check --workspace                    # 类型检查

# === 测试 ===
cargo test --workspace                     # 全量测试 (180+ 用例)
make test-unit                             # 仅单元测试
make test-integration                      # 仅集成测试
cargo bench -p call-core                   # 性能基准测试

# === SIPp 端到端 ===
cd tools/sipp && ./run_all.sh              # SIPp 场景测试
./tools/sipp/run_business_flows.sh         # 业务流程场景
./tools/sipp/run_cps_rec.sh 100 10 10      # 100 通话 / 10 CPS / 10 秒

# === 安全审计 ===
cargo audit                                # 依赖安全扫描
```

### SIPp 业务场景

`tools/sipp/scenarios/` 下提供完整 SIPp 场景脚本：

| 场景 | 文件 | 说明 |
| :--- | :--- | :--- |
| 接入中继主叫 | `business_access_uac.xml` | 模拟运营商接入 |
| 接入拒绝 | `business_access_rejected_uac.xml` | 验证 403/404 拒绝 |
| 落地入局 | `business_egress_inbound_uac.xml` | 模拟呼入业务 |
| 分机主叫 | `business_extension_uac.xml` | 分机 → 中继 |
| 分机被叫 | `business_extension_uas.xml` | 中继 → 分机 |
| 分机注册 | `business_extension_register_uac.xml` | REGISTER 流程 |
| 网关正常 | `business_gateway_uas.xml` | 模拟落地网关 |
| 网关故障 | `business_gateway_fail_uas.xml` | 验证故障转移 |

---

## 🚢 部署指南

### Docker 部署

```bash
# 构建镜像
make docker-build

# 启动完整栈
docker compose -f deploy/docker/docker-compose.yml up -d

# 查看服务状态
docker compose -f deploy/docker/docker-compose.yml ps

# 查看日志
docker compose -f deploy/docker/docker-compose.yml logs -f sip-edge
```

### 生产环境检查清单

- [ ] 配置独立的 PostgreSQL 实例（建议 16C/32G+）
- [ ] 配置 NATS Cluster（3 节点）
- [ ] 设置强密码（数据库、NATS、API）
- [ ] 启用 TLS（SIP / API / NATS）
- [ ] 配置防火墙规则（仅开放必要端口）
- [ ] 设置 SBC IP 白名单
- [ ] 配置录音存储（OSS Bucket 或独立磁盘）
- [ ] 设置日志轮转与监控告警
- [ ] 配置数据库备份策略
- [ ] 设置系统级资源限制（ulimit）

完整部署文档见 [`docs/deployment/DEPLOY.md`](./docs/deployment/DEPLOY.md)。

---

## 📊 性能指标

### 目标性能

| 指标 | 目标 | 当前 |
| :--- | :--- | :--- |
| CPS (calls per second) | ≥ 1000 | < 200（优化中） |
| 并发通话 | ≥ 5000 | 测试中 |
| API P99 延迟 | < 100ms | 测试中 |
| 数据库查询 P99 | < 50ms | 测试中 |
| 启动时间 | < 5s | < 3s |
| 内存使用 | 稳态无泄漏 | 监控中 |

### 已优化项

- ✅ RTP 解析引入有界 `BufferPool`，消除每包堆分配
- ✅ 路由引擎实现 `PrefixTrie` 树检索，替代线性扫描
- ✅ SBC ACL 实现 `IpTrie` 树检索
- ✅ `sip-edge/src/main.rs` 从 9401 行拆分为多个子模块
- ✅ CDR 批量入库采用 PostgreSQL UNNEST 静态数组绑定

### 已知瓶颈（持续优化）

- 🔴 录音模块使用 `std::sync::Mutex` + 同步 I/O，需重构为 async channel-based
- 🔴 SBC RateLimiter 单 Mutex，需改为 DashMap 分片
- 🟡 RTP 每包 6-8 次 DashMap 锁，高 pps 下 cache line bouncing
- 🟡 SIP 解析非零拷贝，需引入借用生命周期

---

## 🤖 AI 集成

`vos-rs` 提供 **AI-Native 可编程媒体控制接口**，是构建 AI Voice Agent 的首选平台：

### 媒体控制 API

| 端点 | 方法 | 说明 |
| :--- | :--- | :--- |
| `/manage/calls/:call_id/play` | POST | 注入音频播放（独占/混音模式） |
| `/manage/calls/:call_id/stop-play` | POST | 停止音频播放 |
| `/manage/calls/:call_id/mute` | POST | 实时静音 |
| `/manage/calls/:call_id/unmute` | POST | 取消静音 |
| `/manage/calls/:call_id/status` | GET | 通话媒体状态 |

### 关键能力

- **动态转码**：WebRTC (Opus 48kHz) ↔ 运营商 (PCMA/PCMU 8kHz) 实时双向转码
- **音频注入**：支持 8kHz/16kHz/44.1kHz/48kHz WAV 自动重采样
- **平滑切换**：SSRC/序列号/时间戳连续性重写，消除切换爆音
- **Marker Bit**：首帧 Marker 标记，通知终端重置 Jitter Buffer

### 接入示例

```bash
# 向通话注入音频（独占模式，仅 caller 听到）
curl -X POST http://localhost:8080/manage/calls/<call_id>/play \
  -H "Content-Type: application/json" \
  -d '{"file":"/var/lib/vos-rs/prompts/welcome.wav","leg":"caller","mode":"exclusive"}'

# 查询通话媒体状态
curl http://localhost:8080/manage/calls/<call_id>/status
```

完整 AI 接入指南见 [`docs/development/AI_PLUGIN_INTEGRATION_GUIDE.md`](./docs/development/AI_PLUGIN_INTEGRATION_GUIDE.md)。

---

## 🗺 路线图

### v1.0（当前）

- ✅ SIP B2BUA 完整事务状态机
- ✅ RTP 中继 + Opus/G.711 转码
- ✅ 路由引擎 + LCR + 熔断
- ✅ 计费引擎 + CDR + 实时扣费
- ✅ SBC 安全 + IP ACL + 限速
- ✅ Web 控制台 + REST API
- ✅ AI-Native 媒体控制 API

### v1.1（计划中）

- ⏳ 录音模块 async 化重构
- ⏳ SBC RateLimiter DashMap 分片
- ⏳ SIP 解析零拷贝重构
- ⏳ 实时余额扣减 AtomicI64 CAS 缓存
- ⏳ WebRTC 完整支持（媒体节点）

### v1.2（规划中）

- ⏳ 分布式信令节点（sip-router）
- ⏳ 集群级媒体调度
- ⏳ Webhook 插拔式通道
- ⏳ 可视化路由拓扑编辑器
- ⏳ Prometheus + Grafana 监控栈

---

## ❓ FAQ

<details>
<summary><b>Q: 为什么不用 Asterisk / FreeSWITCH？</b></summary>

A: Asterisk 与 FreeSWITCH 是成熟的 VoIP 平台，但在电信级高并发场景下存在瓶颈：
- **Asterisk**：基于线程池模型，单机并发上限约 1000 通话
- **FreeSWITCH**：基于 APR 线程模型，单机并发可达 5000+，但 C 语言开发效率低
- **vos-rs**：基于 Tokio 异步运行时，零拷贝解析 + 无锁媒体中继，目标单机 5000+ 通话 / 1000+ CPS，且 Rust 内存安全

</details>

<details>
<summary><b>Q: 为什么选择 Rust 而不是 Go？</b></summary>

A: Rust 在以下方面优于 Go：
- **零成本抽象**：异步运行时无 GC 暂停，适合实时媒体处理
- **内存安全**：编译期保证无数据竞争，无悬垂指针
- **性能**：与 C/C++ 同级，Go 的 2-3 倍
- **生态**：Tokio 是业界顶级异步运行时，sqlx 提供编译期 SQL 检查

</details>

<details>
<summary><b>Q: 如何对接现有的 SIP 硬件设备？</b></summary>

A: vos-rs 完整实现 RFC 3261 SIP 协议，兼容所有标准 SIP 终端：
- **硬件话机**：Yealink / Grandstream / Cisco 等
- **软电话**：Zoiper / Linphone / MicroSIP
- **WebRTC 客户端**：浏览器 / 小程序（需启用 Opus 转码）
- **运营商中继**：IP 互联 / SIP Trunking

</details>

<details>
<summary><b>Q: 录音文件如何存储？</b></summary>

A: 支持三种存储模式：
- **本地磁盘**：默认，写入 `VOS_RS_RECORDING_DIR`
- **阿里云 OSS**：上传至 OSS Bucket
- **双写**：同时写入本地与 OSS（推荐生产环境）

录音文件命名格式：`{call_id}_{leg}_{timestamp}.wav`，8kHz/16-bit/PCM。

</details>

<details>
<summary><b>Q: 如何进行容量规划？</b></summary>

A: 单节点推荐配置：

| 并发通话 | CPU | 内存 | 带宽 (G.711) | 带宽 (Opus) |
| :--- | :--- | :--- | :--- | :--- |
| 500 | 4C | 4G | 50 Mbps | 15 Mbps |
| 1000 | 8C | 8G | 100 Mbps | 30 Mbps |
| 2000 | 16C | 16G | 200 Mbps | 60 Mbps |
| 5000 | 32C | 32G | 500 Mbps | 150 Mbps |

</details>

---

## 🤝 贡献指南

我们欢迎社区贡献！请遵循以下流程：

### 开发流程

1. **Fork 仓库** 并克隆到本地
2. **创建分支**：`git checkout -b feat/your-feature`
3. **编写代码**：遵循 [`AGENTS.md`](./AGENTS.md) 中的编码规范
4. **通过测试**：
   ```bash
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   cd web && npm test
   ```
5. **提交代码**：使用 Conventional Commits 规范
   ```
   feat(auth): 添加 JWT 刷新令牌机制
   fix(billing): 修复并发余额扣减竞态条件
   refactor(rtp): 提取 RTP 解析为独立模块
   ```
6. **发起 PR**：关联 issue，等待 review

### Commit 规范

格式：`<type>(<scope>): <description>`

| Type | 说明 |
| :--- | :--- |
| `feat` | 新功能 |
| `fix` | Bug 修复 |
| `refactor` | 重构（不改业务逻辑） |
| `perf` | 性能优化 |
| `docs` | 文档 |
| `test` | 测试 |
| `chore` | 杂项 |
| `ci` | CI 配置 |

**Scope** 范围：`sip-core` / `rtp-core` / `sdp-core` / `call-core` / `cdr-core` / `sip-edge` / `api-server` / `cdr-worker` / `media` / `routing` / `billing` / `auth` / `sbc` / `web`

### PR 规则

- 标题与 commit 格式一致
- 必须关联 issue（`Closes #123`）
- 必须通过 CI（`cargo clippy` + `cargo test` + `cargo build`）
- 单 PR 变更不超过 500 行（大 PR 应拆分）

---

## 📄 许可证

本项目采用 **专有许可证 (Proprietary)**，详见 [`Cargo.toml`](./Cargo.toml)。

未经授权，禁止复制、修改、分发或商业使用。如需商业授权，请联系项目维护者。

---

## 🙏 致谢

### 核心依赖

- [Tokio](https://tokio.rs/) — 异步运行时
- [Axum](https://github.com/tokio-rs/axum) — Web 框架
- [sqlx](https://github.com/launchbadge/sqlx) — 数据库访问
- [DashMap](https://github.com/xacrimon/dashmap) — 并发 HashMap
- [async-nats](https://github.com/nats-io/nats.rs) — NATS 客户端
- [HeroUI](https://www.heroui.com/) — React 组件库
- [Tailwind CSS](https://tailwindcss.com/) — 原子化 CSS 框架
- [React](https://react.dev/) — UI 框架

### 协议参考

- [RFC 3261](https://www.rfc-editor.org/rfc/rfc3261) — SIP: Session Initiation Protocol
- [RFC 3262](https://www.rfc-editor.org/rfc/rfc3262) — Reliability of Provisional Responses
- [RFC 3264](https://www.rfc-editor.org/rfc/rfc3264) — An Offer/Answer Model with SDP
- [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550) — RTP: A Transport Protocol for Real-time Applications
- [RFC 4028](https://www.rfc-editor.org/rfc/rfc4028) — Session Timers in the Session Initiation Protocol
- [RFC 4566](https://www.rfc-editor.org/rfc/rfc4566) — SDP: Session Description Protocol
- [RFC 2833](https://www.rfc-editor.org/rfc/rfc2833) — RTP Payload for DTMF Digits

### 灵感来源

- [VOS-3000](http://www.vos3000.com/) — 商业软交换平台（对标产品）
- [Kamailio](https://kamailio.org/) — 开源 SIP 服务器
- [OpenSIPS](https://opensips.org/) — 开源 SIP 服务器
- [FreeSWITCH](https://freeswitch.org/) — 开源软交换平台

---

<div align="center">

**[⬆ 回到顶部](#vos-rs)**

Made with ❤️ by vos-rs team

</div>
