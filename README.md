# VOS-RS

VOS-RS 是一个干净的（从零开始实现的）Rust 语言载波级 VoIP 运营平台，灵感来源于常见的 VOS 级别软交换能力。

## 📁 项目结构

```
vos-rs/
├── crates/             # 核心库
│   ├── sip-core/      # SIP 协议实现
│   ├── rtp-core/      # RTP/RTCP 协议实现
│   ├── sdp-core/      # SDP 协议实现
│   ├── call-core/     # 呼叫控制和 CDR 生成
│   └── cdr-core/      # CDR 存储和 API 数据模型
├── services/          # 服务
│   ├── sip-edge/      # SIP 边缘服务
│   ├── cdr-worker/    # CDR 工作服务
│   └── api-server/    # REST API 服务 (新增)
├── web/               # Web 管理控制台 (新增)
├── tools/             # 工具
├── docs/              # 文档
└── README.md
```

## 🧩 功能模块

**Web 管理控制台**（`web/`，Arco Design React）：
- 仪表盘 — 实时呼叫统计、质量指标、趋势图、状态分布
- 活跃呼叫 — 实时通话监控、强制拆线
- 呼叫记录 — CDR 查询、详情（含录音回放、DTMF、媒体质量）
- 录音 — 在线试听与下载
- 报表 — 时段统计、CSV 导出
- SIP 用户 / 网关 / 路由（含时间路由、选路试算）/ 注册信息 / 号码库存
- 费率 / 账户（余额、充值、离线对账扣费明细）

**后端服务**：
- `sip-edge` — SIP 边缘 + B2BUA + RTP 中继 + 录音 + 管理 API（活跃呼叫/拆线/选路试算）
- `api-server` — REST API（管理控制台后端）
- `cdr-worker` — NATS CDR 消费者

## 🚀 快速开始

### 方式一：Docker Compose 一键启动（推荐）

```bash
docker compose up -d --build
```

访问 http://localhost:3000 即可。完整部署说明见 [DEPLOY.md](DEPLOY.md)。

### 方式二：本地开发

前置：Rust 1.89+、PostgreSQL 14+、Node.js 18+

```bash
createdb vos_rs
./scripts/dev.sh   # 一键启动 sip-edge + api-server + 前端
```

或分终端手动启动（见 [DEPLOY.md](DEPLOY.md)）。

### 手动启动各服务

#### 1. 设置数据库

```bash
# 创建数据库
createdb vos_rs

# 设置环境变量
export DATABASE_URL=postgres://user:password@localhost/vos_rs
```

#### 2. 启动 API 服务

```bash
cargo run -p api-server
```

API 服务将在 `http://localhost:8080` 启动。

#### 3. 启动 SIP 边缘服务 (可选)

```bash
# 使用默认配置
cargo run -p sip-edge

# 或使用自定义配置
DATABASE_URL=postgres://... cargo run -p sip-edge
```

#### 4. 启动 Web 管理控制台

```bash
cd web

# 安装依赖
npm install

# 启动开发服务器
npm run dev
```

访问 `http://localhost:3000` 查看管理控制台。

更多详细信息请参阅 [web/README.md](web/README.md)。

第一个里程碑版本被有意控制在较小规模：

1. 解析 SIP 请求和响应。
2. 围绕 UDP/TCP/TLS 传输层构建 SIP 边缘服务（sip-edge）。
3. 添加 B2BUA（背靠背用户代理）呼叫控制。
4. 添加路由选择、实时计费、CDR（呼叫详细记录）生成和计费账本。

当前的 Crate 结构：

- `crates/sip-core`: SIP 核心数据类型及解析器。
- `crates/rtp-core`: RTP 数据包解析与编码基础组件。
- `crates/sdp-core`: SDP 媒体端点解析与 Offer/Answer 重写辅助工具。
- `crates/call-core`: B2BUA 呼叫状态、路由和呼叫管理器基础组件。
- `crates/cdr-core`: CDR 事件模式与 PostgreSQL 持久化。
- `services/sip-edge`: UDP SIP 边缘服务，支持 INVITE 路由、CDR 持久化、SDP 重写以及 RTP 中继监听器。
- `services/cdr-worker`: NATS JetStream CDR 消费者，负责将完成的 CDR 写入 PostgreSQL。

SIP 协议的实现状态在 [`docs/sip-coverage.md`](docs/sip-coverage.md) 中跟踪。
更广泛的 SIP/RTP 完成度在 [`docs/rtp-sip-completeness.md`](docs/rtp-sip-completeness.md) 中跟踪。

运行检查：

```sh
cargo test
cargo fmt --check
```

开发快捷方式可通过 `make` 访问，并从 `.env` 读取本地默认配置：

```sh
make help
make quick
make smoke
make full-flow
make verify
```

运行 SIPp 冒烟测试：

```sh
tools/sipp/run_smoke.sh
```

该冒烟测试会在本地回环端口上启动 `sip-edge`、一个 SIPp 网关 UAS 以及一个 SIPp 呼叫方 UAC。
日志和 SIP 消息跟踪记录将写入 `target/sipp/`。

运行本地完整呼叫流程：

```sh
tools/full-flow/run_full_flow.sh
```

该 full-flow 脚本会构建 `sip-edge`，创建一个临时 PostgreSQL 数据库，通过 SIP Digest 认证注册本地主叫/被叫 Contact，通过 UDP SIP run 一次网关呼叫和一次已注册用户呼叫，使用真实 RTP 数据包验证双向 RTP 中继，发送 BYE，检查已持久化的接通 CDR，并将产物写入 `target/full-flow/`。

启用 REGISTER 的 SIP Digest 认证：

```sh
VOS_RS_SIP_AUTH_USERS=1001:secret,1002:secret
VOS_RS_SIP_AUTH_REALM=vos-rs
VOS_RS_SIP_AUTH_NONCE=change-me
```

当未设置 `VOS_RS_SIP_AUTH_USERS` 时，本地开发中将禁用 REGISTER 认证。

SIP TLS 配置：

```sh
VOS_RS_SIP_TLS_BIND=0.0.0.0:5061
VOS_RS_SIP_TLS_CERT_PATH=/etc/vos-rs/tls/fullchain.pem
VOS_RS_SIP_TLS_KEY_PATH=/etc/vos-rs/tls/privkey.pem
VOS_RS_SIP_TLS_CA_PATH=/etc/vos-rs/tls/gateway-ca.pem
VOS_RS_SIP_TLS_SERVER_NAME=gw1.example.com
VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY=false
```

入站 SIP TLS 默认关闭，只有同时配置 `VOS_RS_SIP_TLS_CERT_PATH` 与 `VOS_RS_SIP_TLS_KEY_PATH` 后才会监听 TLS 端口。出站 SIP TLS 默认使用系统 CA 或 `VOS_RS_SIP_TLS_CA_PATH` 校验证书；仅本地开发可显式设置 `VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY=true` 跳过校验。内置测试证书只在 `VOS_RS_SIP_TLS_ALLOW_TEST_CERT=true` 时启用，不能用于生产。

RTP 中继配置：

```sh
VOS_RS_RTP_ADVERTISED_ADDR=203.0.113.10
VOS_RS_RTP_PORT_MIN=40000
VOS_RS_RTP_PORT_MAX=40100
VOS_RS_RTP_SYMMETRIC_LEARNING=true
```

`sip-edge` 会绑定所配置范围内的偶数 RTP 端口及其相邻的 RTCP 端口，将 SDP 音频 `c=`/`m=` 行重写为所通告的中继地址，并在从 SDP 学习到的主叫和网关媒体端点之间转发经过验证的 RTP/RTCP 数据包。默认启用对称 RTP/RTCP 学习：一旦有效的媒体数据包到达某个中继端口，成对的相反方向中继端口会将其目标地址更新为该数据包的实际源地址，这有助于那些 SDP 地址不可达的 NAT 后的端点。活跃的 RTP 端口按媒体段（leg）进行租用，并在呼叫释放或出局呼叫失败时释放；如果配置的端口范围耗尽，带有 SDP 的新 INVITE 请求将收到 SIP `503 Service Unavailable`。

音频编解码器协商目前支持 G.711 PCMU 和 PCMA：

- PCMU: 静态 RTP 负载类型 `0`, `PCMU/8000`
- PCMA: 静态 RTP 负载类型 `8`, `PCMA/8000`

如果 SDP 中同时包含 `telephone-event/8000`，`sip-edge` 会保留该动态负载类型，用于 RFC 2833/4733 RTP DTMF 中继和按键重建；但 `telephone-event` 本身不能作为唯一音频能力。如果 INVITE Offer 中没有兼容的 PCMU/PCMA 负载，`sip-edge` 将返回 `488 Not Acceptable Here`。其他暂不支持的音频负载会从转发的 SDP Offer 和 Answer 中移除。

DTMF 支持：

- SIP INFO: 支持 `application/dtmf-relay` 和 `application/dtmf` 的按键提取，并写入 CDR 的 `dtmf_digits` 字段。
- RTP telephone-event: 支持从协商的动态 payload type 中识别 RFC 2833/4733 DTMF，按 RTP timestamp 去重后累计到 CDR，并在媒体指标中记录 `dtmf_events`。

启用基础呼叫录音：

```sh
VOS_RS_RECORDING_ENABLED=true
VOS_RS_RECORDING_DIR=target/recordings
```

启用后，`sip-edge` 会对中继的 G.711 PCMU/PCMA RTP 负载进行解码，并为每个已接通呼叫写入一个双声道 8 kHz/16-bit WAV 文件以及一个 JSON 元数据旁路文件。左声道为主叫中继段，右声道为网关段。

启用 PostgreSQL CDR 持久化：

```sh
VOS_RS_DATABASE_URL=postgres://user:password@localhost/vos_rs tools/sipp/run_smoke.sh
```

当设置了 `VOS_RS_DATABASE_URL` 时，`sip-edge` 会在需要时创建 `call_cdrs` 表并持久化已完成的呼叫记录。如果未设置，本地协议测试中将禁用 CDR 持久化。

启用 NATS CDR 队列化：

```sh
VOS_RS_NATS_URL=nats://127.0.0.1:4222
VOS_RS_NATS_CDR_STREAM=VOS_RS_CDRS
VOS_RS_NATS_CDR_SUBJECT=vos-rs.cdrs
```

当设置了 `VOS_RS_NATS_URL` 时，`sip-edge` 将确保 JetStream 流存在，并将已完成的 CDR 作为 JSON 消息发布（带有发布确认）。在此模式下，`sip-edge` 不会直接将 CDR 写入 PostgreSQL。

运行 CDR 工作服务：

```sh
VOS_RS_NATS_URL=nats://127.0.0.1:4222
VOS_RS_DATABASE_URL=postgres://user:password@localhost/vos_rs
cargo run -p cdr-worker
```

`cdr-worker` 使用持久化的 JetStream 拉取消费者，将每个 CDR 写入 PostgreSQL，且仅在数据库插入成功后才确认（ACK）消息。
