# VOS-RS (VoIP Softswitch in Rust)

`vos-rs` 是用 Rust 语言编写的**电信运营级 VoIP 软交换与媒体转发平台**。项目对标商业软交换平台 VOS-3000，旨在单机环境下实现 5000+ 并发通话和 1000+ CPS (Calls Per Second) 的超高性能要求。

平台采用**信令与媒体分离**的设计原则，集成了信令路由、对称 RTP 媒体中继、SBC 安全防护、实时计费、录音存储、以及 API/Web 控制台于一体。同时，项目提供了一套**AI-Native 的可编程媒体控制接口**，可极其方便地对接 AI Voice Agent（智能语音机器人）、TTS/ASR 系统及现代呼叫中心。

---

## 🚀 核心设计与高性能架构

- **异步运行时**：基于 `Tokio`（多线程模式）构建高并发 I/O 环，线程亲和度（Affinity）绑定 CPU 物理核心。
- **零拷贝 (Zero-Copy) SIP 解析**：自研 `sip-core`、`rtp-core`、`sdp-core` 消息解析器。SIP 头部、URI、方法、SipMessage 等使用携带生命周期参数的借用类型直接引用原始接收缓冲区切片，消除了大并发信令下高频产生的堆内存分配与垃圾回收压力。
- **高并发零锁媒体中继**：无锁式 UDP 端口分配，绑定套接字前不占用任何全局互斥锁；转码器 `LiveTranscoder` 上下文直接作为协程局部变量生存于网络转发循环，彻底规避全局锁争抢。
- **G.711 PCMA/PCMU 查表法加速**：基于 `OnceLock` 预先缓存 static LUT 查找表，将 G.711 原地原地转码（In-place）的位移分支计算退化为 $O(1)$ 级 L1-Cache 缓存寻址，实现原地零内存分配转码。
- **WAV 放音异步化与隔离**：使用 `spawn_blocking` 隔离 WAV 文件磁盘读取，彻底解决磁盘 I/O 带来的网络协程停顿。
- **PostgreSQL UNNEST 批量写入**：CDR 批量入库抛弃了常规 QueryBuilder 动态拼接 SQL，重构为 **静态 UNNEST 数组绑定模式**，降低客户端序列化成本和 PostgreSQL 语法解析器负载达 90% 以上。
- **NATS JetStream 话单事件流**：异步批量缓冲与流式入库，保障信令与媒体节点不受外部数据库 I/O 阻塞影响。
- **Opus ↔ G.711 实时高性能转码**：集成了 Rust 社区顶尖的 `opus`（支持 `bundled` 静态编译以完全免除系统 FFI 开发包依赖）与 `rubato`（高性能重采样引擎），内置 `fifo` 环形缓冲区对抗网络包抖动。

---

## 📂 项目模块结构

```text
vos-rs/
├── crates/                    # 核心协议与业务模块 (零拷贝解析)
│   ├── sip-core/              # SIP 信令语法树与解析器 (RFC 3261)
│   ├── rtp-core/              # RTP/RTCP 封包解析与 SRTP 加密通道
│   ├── sdp-core/              # SDP 媒体协商解析与重写工具
│   ├── call-core/             # 呼叫状态机、路由匹配与 CDR 生成器
│   ├── cdr-core/              # 话单数据模型与 PostgreSQL 操作库
│   └── storage-core/          # 录音存储抽象层（本地磁盘与 OSS 双写）
│
├── services/                  # 独立二进制服务
│   ├── sip-edge/              # 边缘信令与媒体代理 (B2BUA + RTP Relay + 录音 + API)
│   ├── api-server/            # REST API 后端服务 (Axum 框架，支持 30+ 业务端点)
│   └── cdr-worker/            # NATS 异步话单消费者，批量写入数据库
│
├── web/                       # 前端管理界面 (React + TypeScript + Vite)
├── tools/                     # 集成测试与 SIPp 性能压测工具
└── scripts/                   # SQL 迁移与一键开发辅助脚本
```

---

## ⚡ 核心能力一览

| 模块维度 | 技术指标与覆盖功能 |
| :--- | :--- |
| **SIP 信令控制** | 支持 UDP / TCP / TLS / WebSocket 传输；REGISTER 注册挑战认证；完整实现 INVITE / BYE / REFER 分支控制；事务状态机 (RFC 3261)、PRACK 可靠临时响应 (RFC 3262)、Session-Expires 会话定时器 (RFC 4028)、3xx 路由重定向。 |
| **媒体通道处理** | 高性能对称 RTP / RTCP 中继转发；支持 **Opus (48kHz) ↔ G.711 PCMA/PCMU (8kHz) 实时双向热转码**；实时 SDP 改写；DTMF 检测（支持 SIP INFO 与 RFC 2833 带内按键）；WAV 呼叫双向/单向录音。 |
| **路由与计费系统** | 路由前缀最长匹配 (LCR) + 优先级备用；网关健康主动探测与自动熔断/恢复；呼叫容量与并发速率控制；支持时间窗路由与防黑名单欺诈；实时的余额预扣减与限时拆线。 |
| **SBC 安全防御** | 基于 IP ACL 的网段黑白名单过滤；针对单 IP / 全局的 Token Bucket (令牌桶) CPS 限速；租户间域名与号段强物理隔离；Digest 动态认证 Nonce 防重放。 |
| **NAT 穿越与映射** | 动态 STUN 公网映射地址发现（支持多服务器 Fallback）；UPnP 自动网关端口映射；NAT 探测与 keepalive 心跳保活。 |
| **API 与 Web** | 提供 30+ REST 端点（实时话单查询、仪表盘统计、用户/网关配置、账单对账、通话拆线、路由预览）；可视化 React 管理控制台。 |

---

## 🎵 软件定义媒体与 AI 通信接口 (可编程 API)

`vos-rs` 拥有极为灵活的软件定义媒体中继环，支持通过 `sip-edge` 的管理端口，在不影响信令连接的情况下对当前的 RTP 媒体流实施热插拔控制，是快速构建 **AI 语音 Agent (智能大模型机器人)** 和 **可编程 IVR** 的首选平台：

### 1. 媒体控制 API
*   **注入音频播放**：`POST /manage/calls/:call_id/play`
    *   **参数配置**：指定目标 Leg (`caller`、`callee` 或 `both` 双方)。
    *   **独占替换模式 (Exclusive)**：播放本地 WAV 时，拦截另一侧的常规中继流量，接收端仅能听到广播音频。
    *   **背景混音模式 (Background)**：本地音频流与另一侧的实时声音混合中继。
    *   **动态重采样 (Auto Resampling)**：解码器集成了线性插值重采样引擎，用户上传 16kHz、44.1kHz 或 48kHz 等非标准采样率 of WAV 文件时，系统会在载入时**自动无缝重采样**至 8000Hz 播放，避免音调音速失准，保障稳定性。
*   **停止音频播放**：`POST /manage/calls/:call_id/stop-play`
    *   实时中止指定 Leg 正在进行的音频投递，干净释放后台轮询协程。
*   **实时静音/取消静音**：`POST /manage/calls/:call_id/mute` 与 `POST /manage/calls/:call_id/unmute`
    *   在接收端直接拦截该 Leg 的输入数据包（不进行中继转发），可用于坐席消噪或三方通话控制。
*   **实时转码 (Dynamic Transcoding)**：
    *   当 SIP 协商结果为一侧 Opus（常见于 WebRTC/小程序/网页客户端）而另一侧为 PCMA/PCMU（传统运营商线路）时，系统自动在物理转发循环中启用 `LiveTranscoder`。重采样和编解码由底层经过极致汇编优化的 FFI 处理，实现零全局锁高吞吐。
*   **呼叫媒体状态监测**：`GET /manage/calls/:call_id/status`
    *   返回通话两端的静音状态、正在播音的本地文件路径、播放模式、以及高精确度的音频文件播放进度百分比。

### 2. 电信级终端兼容与连续性保障
*   **Marker Bit 首帧通知**：音频注入开始的第一帧包强制设置 Marker 标记位为 `true`，通知硬终端重置 Jitter Buffer 缓冲区，消除源切换时的杂音。
*   **平滑 SSRC 序列号/时间戳重写 (Smooth Transition)**：在 Exclusive（独占播放）结束恢复常规通话中继时，媒体转发环能够自动计算在播放期间错过的包数和采样步长偏差（Offsets），并在中继时重新改写 RTP 包头。使得硬终端（如实体话机）接收到的序列号与时间戳呈现绝对的数学连续性，消除切换瞬间可能出现的“咔哒”爆音与丢包统计。

---

## 🛠 快速开始

### 运行环境要求
- **OS**: Linux / macOS
- **Compiler**: Rust 1.89+ (Edition 2021)
- **Database**: PostgreSQL 14+
- **Message Queue**: NATS Server (JetStream 模式)
- **Frontend Build**: Node.js 18+ & npm

### 1. Docker Compose 一键拉起（推荐）
```bash
# 启动所有基础设施（Postgres, NATS, S3）以及 vos-rs 节点 (sip-edge, api-server, React 前端)
docker compose -f deploy/docker/docker-compose.yml up -d --build

# 访问管理后台
# 地址: http://localhost:3000
```

### 2. 本地开发调试
```bash
# 创建本地测试数据库
createdb vos_rs

# 启动依赖服务（NATS 4222 端口，PostgreSQL 5432 端口）
# 并配置本地配置文件（见 docs/development/ENV_VARS.md）

# 一键启动三端进程（sip-edge + api-server + 前端 Dev Server）
./scripts/dev.sh
```

---

## 🧪 验证与压力测试

`vos-rs` 拥有严苛的自动化回归验证链。我们提供了一组基于 `SIPp` 的集成压力测试，用来模拟真实环境下的高频呼叫与 RTP 编解码传输。

```bash
# 查看所有命令帮助
make help

# 1. 运行格式检查与快速单元测试
make quick

# 2. 运行工作区下的全量测试（包括 180+ 单元与集成测试）
make test

# 3. 运行 SIPp 呼叫流冒烟测试
make smoke

# 4. 运行全流程验证（信令代理 + RTP 对称转发 + 高并发无锁端口占用验证）
make verify

# 5. 并发压力性能测试
make perf
```

## 📖 相关文档

### 架构与设计
- [系统分层架构规范](docs/architecture/ARCHITECTURE.md)
- [SIP/RTP 协议覆盖指标一览](docs/architecture/rtp-sip-completeness.md)
- [Webhooks 全流程控制与 VCI 2.0 指令集设计对比](docs/architecture/WEBHOOKS_DESIGN_COMPARISON.md)
- [Webhooks 插拔式可扩展通道架构](docs/architecture/WEBHOOKS_EXTENSIBILITY_ARCHITECTURE.md)

### 开发与集成
- [AI 语音插件标准接入协议（UDS 二进制流 + OpenAI/Gemini 接入示例）](docs/development/AI_PLUGIN_INTEGRATION_GUIDE.md)
- [环境变量配置参考](docs/development/ENV_VARS.md)

### 部署与运维
- [部署与调优指南](docs/deployment/DEPLOY.md)
- [Web 后台管理系统指引](docs/user-guide/WEB_GUIDE.md)

> 完整文档索引见 [docs/README.md](docs/README.md)。
