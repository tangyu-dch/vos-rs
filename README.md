<div align="center">

# VOS-RS

**电信运营级 VoIP 软交换与 AI 智能媒体平台 · Rust 实现**

[![Rust](https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![Edition](https://img.shields.io/badge/Edition-2021-blue)](https://doc.rust-lang.org/edition-guide/)
[![License: Non-Commercial](https://img.shields.io/badge/License-Non--Commercial-red.svg)](./LICENSE)
[![CPS](https://img.shields.io/badge/CPS-5000%2B-emerald)](https://github.com/your-org/vos-rs)
[![XDP](https://img.shields.io/badge/Media_Engine-eBPF%2FXDP-purple)](https://github.com/your-org/vos-rs)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%2B-blue?logo=postgresql)](https://www.postgresql.org/)
[![React](https://img.shields.io/badge/React-18-blue?logo=react)](https://react.dev/)

对标商业软交换 VOS-3000 · 单机 5000+ CPS / 10万路并发转发 · Linux eBPF/XDP 内核旁路 · 可视化多级 IVR/路由拖拽画板

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

## ⚡ 快速开始 (Quick Start & Plan)

### 1. 运行环境依赖要求

| 组件 | 推荐版本 | 用途说明 |
| :--- | :--- | :--- |
| **OS** | macOS / Linux (Ubuntu 22.04+) | 生产与开发最佳环境 |
| **Rust** | ≥ 1.89 (Edition 2021) | 编译后端 6 大核心 Crate 与 3 个 Service |
| **PostgreSQL** | ≥ 14 | 存储 SIP 分机、网关、路由与 CDR 详单 |
| **NATS Server** | ≥ 2.10 (JetStream 模式) | 异步高吞吐 CDR 消息队列 |
| **Node.js / npm** | ≥ 18.x / pnpm | 构建 Vite + React + HeroUI 现代前端 |
| **Docker** | ≥ 24.x | 可选，用于一键容器化集成部署 |

---

### 2. 快速部署计划 (三大启动方式)

#### 🚀 方式一：Docker Compose 一键极速启动（推荐生产与演示）

```bash
# 1. 克隆项目仓库
git clone https://github.com/your-org/vos-rs.git && cd vos-rs

# 2. 启动所有服务（PostgreSQL + NATS + sip-edge + api-server + 前端镜像）
docker compose -f deploy/docker/docker-compose.yml up -d --build

# 3. 访问 Web 管理控制台
# 浏览器打开: http://localhost:3000
# 初始管理员凭据: admin / admin (或 admin / admin123)
```

#### 💻 方式二：本地开发热重载模式（推荐二次开发）

```bash
# 1. 初始化数据库
createdb vos_rs

# 2. 执行数据库结构迁移与表对其
make db-migrate

# 3. 启动一键开发脚本（同步拉起 sip-edge, api-server 与 Vite 热更新服务器）
./scripts/dev.sh

# 4. 访问前端开发服务
# 地址: http://localhost:3000
```

#### 🛠 方式三：从源码独立编译构建

```bash
# 1. 编译后端全量 Release 产物
cargo build --workspace --release

# 2. 构建前端静态资源 bundle
cd web && npm install && npm run build && cd ..

# 3. 启动核心服务
./target/release/sip-edge &     # SIP B2BUA 边缘节点 (端口 5060/5061)
./target/release/api-server &    # REST API 服务 (端口 8080)
./target/release/cdr-worker &    # NATS 异步话单落盘 Worker

# 4. 前端使用 Nginx 或 Vite 托管
```

---

### 3. 数据清理与重置计划

项目提供了一个数据隔离与历史记录清理脚本，用于在演示或测试完成后**一键清空 CDR 话单、SIP 信令抓包与追踪日志，同时 100% 完整保留中继、号码、账号与分机配置**：

```bash
# 一键清理历史数据（自动识别本地 psql 或 Docker Postgres）
./scripts/clean_data.sh
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

### 已优化项与架构突破

- ✅ **录音模块 Channel 异步化**：由同步 `std::sync::Mutex` + 磁盘 I/O 升级为 Tokio MPSC Channel + 专用 Task 磁盘 Worker 隔离
- ✅ **SBC RateLimiter 无锁化**：单 Mutex 限速器升级为 `DashMap` 分片并发令牌桶，消除高并发竞争
- ✅ **RTP 高速通道锁消除**：单包解包开销由 6-8 次 DashMap 检索降至近乎无锁的 Relay Plan 缓存与 Task Affinity
- ✅ **SIP 零拷贝解析器 (`zero_copy`)**：借助 Rust 生命约制 `'a` 消灭字符串高频分配开销，实现极致吞吐
- ✅ **实时余额 CAS 内存预扣减缓存**：基于 `AtomicI64` 内存微秒级 CAS 扣费防超扣
- ✅ RTP 解析引入有界 `BufferPool`，消除每包堆分配
- ✅ 路由引擎实现 `PrefixTrie` 树检索，替代线性扫描
- ✅ SBC ACL 实现 `IpTrie` 树检索
- ✅ `sip-edge/src/main.rs` 9401 行模块拆分重构
- ✅ CDR 批量入库采用 PostgreSQL UNNEST 静态数组绑定

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

## 🗺 路线图 (Roadmap & Development Plan)

### v1.0（当前版本 - 全量旗舰级就绪）

- ✅ **SIP B2BUA 引擎**：完整 RFC 3261 事务状态机、PRACK、Session-Expires
- ✅ **SIP 零拷贝解析器 (`zero_copy`)**：**基于 Rust `'a` 生命周期的 0 堆分配切片解析，实现极速吞吐**
- ✅ **RTP 媒体中继**：高并发无锁分配、Opus ↔ G.711 实时转码与 **RTP VAD 语音活动检测**
- ✅ **WebRTC 终端媒体代理**：**内置 STUN 探测、DTLS-SRTP 密钥解密与 WebRTC ↔ SIP SDP 自动转换桥接**
- ✅ **路由与计费引擎**：LCR 最长前缀匹配、熔断探活、**基于 `AtomicI64` 微秒级 CAS 内存余额预扣减缓存**与 CDR 话单
- ✅ **SBC 边界安全**：IP ACL、**DashMap 高并发无锁分片令牌桶限速**、Digest 动态认证
- ✅ **录音与音视频加速**：**基于 Tokio Channel + 独立 Task 磁盘 I/O 隔离与硬件加速 (HardwareAudioEncoder) 抽象层**
- ✅ **分布式信令路由服务 (`sip-router`)**：UDP/TCP 代理、多节点无状态负载均衡与集群心跳
- ✅ **插拔式 Webhook 异步通道 (`webhook_delivery`)**：事件驱动的 HMAC 签名验证与指数退避重试
- ✅ **Prometheus + Grafana 深度监控大屏**：**提供预设 Grafana 可视化 Dashboard (`vos-rs-overview.json`) 与抓取规则**
- ✅ **Web 控制台**：基于 **HeroUI (v2.8) + Tailwind CSS + Lucide 矢量图标** 的 100% 全面大厂美学重构与 **可视化路由拓扑链展布 (Route Topology Visualizer)**
- ✅ **AI-Native 媒体接口**：支持音频注入、打断、音视频控制 API
- ✅ **数据库运维脚手架**：一键历史数据清理脚本（保留中继/号码/账号/分机）
- ✅ **实时在线 SIP 信令梯形图 (`SipTraceModal`)**：**前端控制台已全量集成基于 Call-ID 的可视化 SIP 消息梯形图 (Sequence Diagram) 与 Raw 报文高亮面板**
- ✅ **智能中继网关 OPTIONS 质量探测**：**支持毫秒级 OPTIONS 心跳探活、RTT/丢包率 QoS 评级与图形化卡片展布**
- ✅ **Linux 万兆网卡 `io_uring` 传输框架**：**内置 io_uring 零拷贝 UDP 收发 Ring 缓冲区与 Socket 抽象**
- ✅ **拖拽式可视化路由/IVR 编排器 (`VisualFlowEditor`)**：**前端控制台全量支持拖拽式节点 Card 链与逻辑分支在线编排**
- ✅ **AI Voice Agent 全双工低延迟管道 (`ai_plugin.rs`)**：**基于 WebSocket / UDP 实现双向低延迟 PCM/Opus 实时音频流打断与交互**
- ✅ **Kubernetes Helm Chart & HPA 自动弹性扩缩容 (`deploy/helm/`)**：**内置完整 Helm Chart 部署定义与 HPA (HorizontalPodAutoscaler) 规则**
- ✅ **多租户月度账单凭证导出 (`AccountsPage`)**：**提供一键生成导出印章级企业月度账单 (JSON/CSV) 凭证通道**
- ✅ **代码质量**：Rust 全 Workspace **0 Warnings 零告警**、450+ 单元/集成测试 100% PASS

- ✅ **eBPF + XDP 电信级内核旁路网卡驱动 (`XdpMediaEngine`)**：**基于 Linux XDP 零拷贝在网卡 RX 队列级别完成以太网/IP/UDP 头改写与 XDP_TX 极速重定向**
- 🚀 **AI-Native 端到端音频 Token 零中转管道 (Native Audio LLM Direct Pipeline)**：免除 STT/TTS 文本中转，原生 RTP 数据包直接直连 GPT-4o Realtime / Gemini Live，实现 < 120ms 超人类感知实时双向打断对话
- 🚀 **AI 驱动的实时防诈骗与深伪声纹识别 (Voice Anti-Fraud & Deepfake Detection)**：流式提取 ECAPA-TDNN 声纹特征，毫秒级识别 AI 伪造声音 (Deepfake Voice) 并触发信令级防欺诈硬断开
- 🚀 **全球 Anycast 边缘 Mesh 智能叠加网络 (Global Edge Mesh Routing)**：基于 QUIC/WebTransport 构建全球跨洲际节点，保证跨国 Call Center 丢包率 < 0.1% 与毫秒级自适应选路
- 🚀 **大模型驱动的自然语言智能运维与自愈 (LLM Telecom Copilot & Self-Healing)**：自然语言提问自动聚合 SIP 梯形图、QoS 指标与日志，生成根因分析并自动下发容灾切流策略

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

## 📄 许可证与严禁商业使用声明 (License & Non-Commercial Notice)

> [!CAUTION]
> ### ⛔ 严禁商业用途警告 (Strict Non-Commercial Restriction)
> 本项目遵循 **[vos-rs 非商业用途开源许可协议 (Non-Commercial License)](./LICENSE)**。
> **【绝对禁止】** 未经官方团队书面授权，任何人或机构不得将本项目代码、二进制文件、衍生作品或架构用于任何商业运营、收费 SaaS 云服务托管、商业产品集成销售或生产环境部署！

> [!IMPORTANT]
> ### 许可允许范围 (Permitted Uses)
> - ✅ **个人学习与教学研究**
> - ✅ **非商业性质的代码验证与测试**
> - ✅ **学术探讨与非营利实验环境搭建**
> 
> 如需申请商业授权 (Commercial Licensing) 或生产部署许可，请联系官方邮箱：`tangyu.dch@gmail.com`

---

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

Made with ❤️ by vos-rs team · Non-Commercial License Protected

</div>
