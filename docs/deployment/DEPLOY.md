# VOS-RS 部署与运维指南

本文档介绍 `vos-rs` 电信级软交换平台的本地开发部署、Docker Compose 生产环境编排、数据库初始化以及可编程媒体管理接口的集成方法。

## 一、系统架构与组件

VOS-RS 包含以下核心可运行组件及基础设施：

| 组件/服务 | 作用 | 默认监听端口 | 核心依赖 |
| :--- | :--- | :--- | :--- |
| `sip-edge` | SIP 边缘网关 (B2BUA、对称 RTP 中继、录音写盘、SBC 防御) | `5060` (SIP UDP/TCP), `8082` (管理 API) | Postgres, NATS |
| `api-server` | REST API 服务 (Axum 后端，供 Web 管理控制台调用) | `8080` (HTTP) | Postgres, NATS |
| `cdr-worker` | NATS 话单写入消费者，批量将内存/队列话单刷入 PG | — (后台守护进程) | Postgres, NATS |
| `web` | Web 管理控制台前端 (React 18 + TS + Vite + Nginx) | `3000` (Nginx) | api-server |
| `PostgreSQL` | CDR 话单、路由表、费率、账户等主数据存储 | `5432` | — |
| `NATS Server` | 高性能流式消息队列 (JetStream 模式)，缓存与投递 CDR 话单 | `4222` / `4223` | — |

---

## 二、本地开发环境启动

### 1. 前置依赖安装
- **Rust**: `>= 1.89` (Edition 2021)
- **Node.js**: `>= 18.0` (npm / yarn)
- **PostgreSQL**: `>= 14.0` (推荐在本地安装并开启服务)
- **NATS Server**: 支持 JetStream（可通过本地 docker 快速拉起）

### 2. 数据库与消息队列准备
在本地 PostgreSQL 中创建 `vos_rs` 数据库。开发环境下，推荐连接字符串为 `postgres://tangyu@127.0.0.1:5432/vos_rs`。

通过 Docker 拉起 NATS 消息队列：
```bash
# 启动开启了 JetStream 的本地 NATS Server，映射 4222 端口
docker run -d --name nats-dev -p 4222:4222 -p 8222:8222 nats:latest -js
```

### 3. 一键开发脚本启动
我们提供了一键启动开发链的脚本，它会自动读取 `config.yaml` 配置并跑起所有服务：
```bash
# 执行一键开发脚本 (自动编译并拉起前端、后端、以及 sip-edge)
./scripts/dev.sh
```
*   **管理后台**: http://localhost:3000
*   **REST API**: http://localhost:8080
*   **sip-edge 控制接口**: http://localhost:8082

### 4. 手动独立终端调试
如果您希望对单独组件进行打断点调试或单独查看日志，可以使用以下命令行分别在不同终端启动：

```bash
# 终端 1: 启动信令/媒体网关 sip-edge
VOS_RS_CONFIG_FILE=config.yaml cargo run -p sip-edge

# 终端 2: 启动 API 控制台后端 api-server
VOS_RS_CONFIG_FILE=config.yaml cargo run -p api-server

# 终端 3: 启动异步 CDR 话单入库组件 cdr-worker
VOS_RS_CONFIG_FILE=config.yaml cargo run -p cdr-worker

# 终端 4: 启动前端 Web 控制台
cd web
npm run dev
```

---

## 三、Docker Compose 生产环境部署

生产环境下推荐使用 Docker 容器化编排，各容器的编排配置文件与 `Dockerfile` 集中管理在 `deploy/docker/` 目录下：

```bash
# 1. 编译并启动所有服务 (以 daemon 模式后台运行)
docker compose -f deploy/docker/docker-compose.yml up -d --build

# 2. 检查各容器健康状态
docker compose -f deploy/docker/docker-compose.yml ps

# 3. 监控特定服务的日志 (如 sip-edge)
docker compose -f deploy/docker/docker-compose.yml logs -f sip-edge
```

容器化编排中的核心端口分布及调整点：
*   **SIP 信令端口**: 默认宿主机映射 UDP/TCP `5060` 端口。如果在公网或云端部署，需要在 `config.yaml` 中配置 `sip_edge.network.advertised_addr` 为公网 IP。
*   **管理后台端口**: http://localhost:3000。
*   **RTP 中继端口范围**: 默认范围为 `40000` 到 `40100`。生产环境大量通话时，请在 `docker-compose.yml` 中适当放开范围（例如 `40000-45000`），并确保云防火墙同时放行该范围的 UDP 流量。

---

## 四、可编程媒体管理 API 接口

`sip-edge` 暴露了高吞吐、毫秒级响应的实时媒体控制接口，用于 AI 音频注入或坐席控制：

### 1. 向指定呼叫注入音频播放 (Play)
```http
POST /manage/calls/{call_id}/play
Content-Type: application/json
Authorization: Bearer <internal_auth_token>

{
  "leg": "caller",
  "file_path": "/var/media/welcome.wav",
  "mode": "exclusive",
  "loop_playback": false
}
```
*   `leg`: `caller`（主叫听到），`callee`（被叫听到），`both`（双方都听到）。
*   `mode`: `exclusive`（独占播放，阻断对方传来的声音），`background`（背景混音）。
*   **注**: WAV 格式可为任意采样率，系统自带的重采样引擎会自动以线性插值对齐至 8000Hz PCM 发送。

### 2. 停止音频播放 (Stop Play)
```http
POST /manage/calls/{call_id}/stop-play
Content-Type: application/json

{
  "leg": "caller"
}
```

### 3. 通话静音 (Mute) & 取消静音 (Unmute)
```http
POST /manage/calls/{call_id}/mute
Content-Type: application/json

{
  "leg": "callee"
}
```
*   将该分支设为静音后，来自此 Leg 的 RTP 包在转发时会被丢弃，实现降噪或阻断效果。

### 4. 获取通话媒体详细状态 (Status)
```http
GET /manage/calls/{call_id}/status
```
*   返回当前主被叫的静音状态、是否在播放音频、播音文件路径及高精度的播音进度比例。

---

## 五、数据库与 Schema 初始化

服务启动时，`sqlx-migrate` 会自动对绑定的 PostgreSQL 数据库执行初始化变更。核心表如下：
- `call_cdrs`: 呼叫记录表。
- `sip_registrations`: 分机注册绑定关系。
- `sip_users`: 分机账密。
- `sip_gateways`: 出站中继网关。
- `sip_routes`: 选路表，支持基于 `time_start`/`time_end` 字段的时间路由过滤。
- `billing_accounts`: 计费账户余额。
- `billing_rates`: 针对字头匹配的计费费率表。
- `dtmf_events`: 存储检测到的带内带外 DTMF 拨号历史。

---

## 六、生产部署安全防范要点

1.  **管理端口隔离**：`sip-edge` 的管理端口 `8082` 以及 `api-server` 的后台控制接口**绝对不能对公网直接开放**，必须通过安全组或 VPC 隔离，仅允许受信任的业务服务访问（或内网通过 Nginx 反代鉴权）。
2.  **动态 Nonce 开启**：在系统生产配置中，建议不要设置静态认证 Nonce。留空后系统会自动为每个 REGISTER / INVITE 请求生成动态 Nonce，并启动周期过期与重放攻击拦截。
3.  **录音路径可用空间保护**：在数据库参数 `recording_min_free_bytes` 中设置保护阈值（默认 512MB）。当挂载录音的本地磁盘空间低于该阈值时，网关将自动停止新录音写入并触发告警，保护主磁盘不被撑爆崩溃。
