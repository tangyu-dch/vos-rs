# VOS-RS

Rust 语言载波级 VoIP 运营平台，灵感来源于 VOS 级软交换能力。

## 项目结构

```
vos-rs/
├── crates/             # 核心库
│   ├── sip-core/      # SIP 协议解析
│   ├── rtp-core/      # RTP/RTCP 协议
│   ├── sdp-core/      # SDP 解析与重写
│   ├── call-core/     # 呼叫控制与 CDR 生成
│   └── cdr-core/      # CDR 存储与 API 模型
├── services/          # 运行服务
│   ├── sip-edge/      # SIP 边缘代理 (B2BUA + RTP 中继 + 录音)
│   ├── api-server/    # REST API 服务
│   └── cdr-worker/    # NATS CDR 消费者
├── web/               # Web 管理控制台 (React)
├── tools/             # SIPp 测试工具
└── docs/              # 文档
```

## 快速开始

### Docker Compose（推荐）

```bash
docker compose up -d --build
# 访问 http://localhost:3000
```

### 本地开发

前置：Rust 1.89+、PostgreSQL 14+、Node.js 18+

```bash
createdb vos_rs
./scripts/dev.sh   # 一键启动 sip-edge + api-server + 前端
```

### 构建与测试

```bash
make help          # 查看所有 make 目标
make quick         # 格式检查 + 单元测试
make test          # 全量测试（189 单元 + 53 集成）
make smoke         # SIPp 呼叫冒烟测试
make full-flow     # 完整呼叫流程（SIP + RTP + 录音）
make verify        # quick + smoke + full-flow
make perf          # SIPp 并发性能测试
```

## 核心能力

| 模块 | 能力 |
|------|------|
| SIP 信令 | UDP/TCP/TLS/WebSocket 传输，REGISTER 认证，INVITE/BYE/REFER，事务状态机(RFC 3261)，PRACK(RFC 3262)，Session-Expires(RFC 4028)，3xx 重定向 |
| 媒体处理 | RTP/RTCP 中继，PCMU/PCMA 协商，SDP 重写，对称 RTP 学习，SIP INFO + RFC 2833 DTMF，基础录音 |
| 路由 | 最长前缀匹配 + 优先级 + LCR，网关健康熔断器，容量控制，时间窗口路由 |
| 安全 | Digest 认证（动态 Nonce + 防重放），SBC（IP ACL + CPS 限速 + 并发限制） |
| NAT 穿越 | STUN 公网地址发现（多服务器 fallback + 重试），UPnP 端口映射 |
| 存储 | PostgreSQL CDR 持久化，NATS JetStream 队列 + cdr-worker，WAV 录音 |
| API | 30+ REST 端点（CDR/仪表盘/用户/网关/路由/账单/号码/录音/活跃呼叫） |
| Web 控制台 | 仪表盘、CDR 查询、录音回放、用户/网关/路由管理、账单对账 |

## 文档

- [`docs/ENV_VARS.md`](docs/ENV_VARS.md) — 环境变量与数据库配置参考
- [`docs/rtp-sip-completeness.md`](docs/rtp-sip-completeness.md) — SIP/RTP 功能覆盖范围与演进路线图
- [`DEPLOY.md`](DEPLOY.md) — 部署指南（Docker Compose + 手动部署）
- [`WEB_GUIDE.md`](WEB_GUIDE.md) — Web 管理控制台使用指南
- [`docs/ENV_VARS.md`](docs/ENV_VARS.md) — 完整 API 端点列表见此文档 §2
