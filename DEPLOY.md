# VOS-RS 部署指南

VOS-RS 包含 4 个可运行组件：

| 组件 | 作用 | 端口 |
|---|---|---|
| `sip-edge` | SIP 边缘服务（B2BUA + RTP 中继 + 录音） | SIP UDP/TCP 5060，管理 API 8082 |
| `api-server` | REST API（管理控制台后端） | 8080 |
| `web` | 前端管理控制台（nginx 静态） | 3000 |
| `postgres` | 数据库（CDR/用户/网关/路由/计费/号码） | 5432 |

---

## 一、Docker Compose 一键部署（推荐）

前置：安装 Docker + Docker Compose v2。

```bash
# 构建并启动全部服务
docker compose up -d --build

# 查看状态
docker compose ps

# 查看日志
docker compose logs -f api-server
```

启动后：
- 前端：http://localhost:3000
- API：http://localhost:8080
- 数据库初始化由 sip-edge/api-server 启动时自动 migrate（建表）

停止：
```bash
docker compose down            # 停止（保留数据卷）
docker compose down -v         # 停止并清空数据库/录音
```

### 配置调整

修改 `docker-compose.yml` 中各服务的 `environment`：
- 数据库账密：`postgres.environment`
- SIP 端口：`sip-edge.ports` + `VOS_RS_SIP_UDP_BIND`
- 录音：`VOS_RS_RECORDING_ENABLED` / `VOS_RS_RECORDING_DIR`

---

## 二、本地开发启动（一键脚本）

前置：Rust 1.89+、Node.js 18+、PostgreSQL 14+。

```bash
# 1. 准备数据库
createdb vos_rs
# 或用 Docker 单独起 postgres:
# docker run -d --name vos-pg -e POSTGRES_USER=vos_rs -e POSTGRES_PASSWORD=vos_rs \
#   -e POSTGRES_DB=vos_rs -p 5432:5432 postgres:16

# 2. 一键启动 sip-edge + api-server + 前端
./scripts/dev.sh
```

启动后：
- 前端：http://localhost:3001
- API：http://localhost:8081
- sip-edge 管理：http://localhost:8082

`dev.sh` 用 `DATABASE_URL=postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs`，可用环境变量覆盖。Ctrl+C 停止全部。

### 手动分终端启动

```bash
# 终端1：sip-edge
VOS_RS_SIP_UDP_BIND=127.0.0.1:5090 \
VOS_RS_SIP_DEFAULT_GATEWAY=127.0.0.1:5070 \
VOS_RS_DATABASE_URL=postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs \
VOS_RS_MANAGE_BIND=127.0.0.1:8082 \
VOS_RS_RECORDING_ENABLED=true VOS_RS_RECORDING_DIR=target/recordings \
  cargo run -p sip-edge

# 终端2：api-server
API_PORT=8081 \
DATABASE_URL=postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs \
VOS_RS_RECORDING_DIR=target/recordings \
VOS_RS_MANAGE_BASE=http://127.0.0.1:8082 \
  cargo run -p api-server

# 终端3：前端
cd web && VITE_API_TARGET=http://localhost:8081 npm run dev
```

---

## 三、环境变量参考

### sip-edge
| 变量 | 说明 | 默认 |
|---|---|---|
| `VOS_RS_SIP_UDP_BIND` | SIP 监听地址 | 0.0.0.0:5060 |
| `VOS_RS_SIP_DEFAULT_GATEWAY` | 默认出局网关 | — |
| `VOS_RS_DATABASE_URL` | PostgreSQL 连接串 | — |
| `VOS_RS_MANAGE_BIND` | 管理 API 监听地址 | 127.0.0.1:8082 |
| `VOS_RS_RECORDING_ENABLED` | 启用录音 | false |
| `VOS_RS_RECORDING_DIR` | 录音目录 | target/recordings |
| `VOS_RS_SIP_AUTH_USERS` | SIP 用户凭证（user:pass,...） | — |

### api-server
| 变量 | 说明 | 默认 |
|---|---|---|
| `DATABASE_URL` | PostgreSQL 连接串 | postgres://localhost/vos_rs |
| `API_PORT` | 监听端口 | 8080 |
| `VOS_RS_RECORDING_DIR` | 录音目录（读录音文件） | target/recordings |
| `VOS_RS_MANAGE_BASE` | sip-edge 管理 API 地址 | http://127.0.0.1:8082 |

### 前端
| 变量 | 说明 | 默认 |
|---|---|---|
| `VITE_API_TARGET` | 开发模式 API 代理目标 | http://localhost:8080 |

---

## 四、造演示数据

### CDR（真实呼叫）
```bash
# 开录音重跑 full-flow，造真实 CDR + 录音到 vos_rs 库
VOS_RS_FULL_FLOW_DATABASE_URL=postgres://vos_rs:vos_rs@127.0.0.1:5432/vos_rs \
VOS_RS_FULL_FLOW_GATEWAY_PORT=5070 \
  python3 tools/full-flow/run_full_flow.py
```

### 失败/取消 CDR（手造，演示状态分布）
```bash
python3 tools/seed_extra_cdrs.py --clean
```

### 计费对账
在前端"账户"页充值后点"离线对账"，或：
```bash
curl -X POST "http://localhost:8081/api/billing/reconcile"
```

---

## 五、数据库表

sip-edge/api-server 启动时自动 migrate 建表：

- `call_cdrs` — 呼叫详细记录
- `sip_users` / `sip_gateways` / `sip_routes` / `sip_registrations`
- `dtmf_events` — DTMF 事件
- `billing_rates` / `billing_accounts` / `billing_ledger` — 计费
- `number_inventory` — 号码库存

`sip_routes` 含 `time_start`/`time_end`（时间路由，sip-edge 启动加载时按当前时间过滤）。

---

## 六、生产部署要点

- 前端 nginx 已配 `/api/` 反代到 `api-server`，生产同源访问。
- sip-edge 管理 API（8082）仅限内网，**不要暴露到公网**（可强制拆线）。
- 录音目录需持久化卷（sip-edge 写、api-server 读）。
- 数据库定期备份（`pg_dump`）。
- SIP UDP 在 NAT/容器环境需注意 `VOS_RS_SIP_ADVERTISED_ADDR` 与 RTP 端口映射。
