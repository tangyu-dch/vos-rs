# api-server — REST API 服务

> **vos-rs 的 Web 控制台后端** — 提供 30+ 业务端点给前端和外部系统集成

## 这是什么？

`api-server` 是 vos-rs 平台的 **REST API 服务**。它是一个基于 Axum 的 HTTP 服务，对外暴露所有业务能力的 RESTful 接口：
- 前端 Web 控制台通过它做所有 CRUD 操作
- 外部 CRM/ERP/工单系统通过它查询话单、管理用户
- Prometheus 通过 `/metrics` 拉取监控指标

打个比方：如果 `sip-edge` 是电话交换机的「内核」，那 `api-server` 就是「管理后台」——你通过浏览器看到的所有数据，都是它返回的。

## 核心能力

| 能力 | 说明 |
| :--- | :--- |
| **30+ 业务端点** | 覆盖用户/网关/路由/号码/费率/计费/录音/反欺诈全部业务 |
| **统一响应格式** | `{code, message, data, timestamp, request_id}` |
| **分页查询** | 所有列表接口支持 `page` + `page_size` + `sort_by` + `order` |
| **JWT 鉴权** | 所有业务接口需携带 Bearer Token |
| **Prometheus 指标** | `/metrics` 端点暴露运行时指标 |
| **热点缓存** | 高频查询走 Redis 缓存，降低 PG 压力 |
| **集群管理** | 查询和管理 sip-edge / media-edge 集群节点 |

## API 端点一览

| 模块 | 路径前缀 | 说明 |
| :--- | :--- | :--- |
| CDR 话单 | `/api/v1/cdr` | 通话记录查询 |
| 仪表盘 | `/api/v1/dashboard/stats` | 运营数据汇总 |
| 活跃通话 | `/api/v1/active-calls` | 当前通话列表 |
| SIP 用户 | `/api/v1/users` | 分机管理 CRUD |
| 网关 | `/api/v1/gateways` | 中继管理 CRUD |
| 路由 | `/api/v1/routing/rules` | 路由规则 CRUD（含 topology JSONB） |
| 号码 | `/api/v1/numbers` | 号码库存管理 |
| 费率 | `/api/v1/rates` | 计费费率 CRUD |
| 计费账户 | `/api/v1/billing/accounts` | 预付费账户 |
| 录音 | `/api/v1/recordings` | 录音查询与播放 |
| 注册状态 | `/api/v1/registrations` | SIP 在线注册 |
| 反欺诈 | `/api/v1/anti-fraud/rules` | 反欺诈规则 CRUD |
| IVR 菜单 | `/api/v1/ivr/menus` | IVR 流程管理（含 nodes/edges JSONB） |
| 呼叫中心 | `/api/v1/call-center/*` | 座席/队列管理 |
| 终结域 | `/api/v1/termination/*` | 中继/IP规则/出局端点/号码池/DID 目的地 |
| 系统 | `/api/v1/system/*` | 系统设置 |
| 指标 | `/metrics` | Prometheus 拉取 |

## 在项目中的位置

```text
web/ (前端) ──HTTP──→ api-server ──SQL──→ PostgreSQL
                          │
                          ├──→ Redis (热点缓存)
                          ├──→ sip-edge (管理 API)
                          └──→ media-edge (管理 API)
```

`api-server` 主要依赖 `cdr-core` 做数据持久化，是 Web 控制台和外部系统的统一入口。

## 模块结构

| 模块 | 职责 |
| :--- | :--- |
| `v1.rs` | 路由注册总入口 |
| `auth.rs` | JWT 鉴权中间件 |
| `users.rs` | SIP 用户 CRUD |
| `gateways.rs` | 网关 CRUD |
| `routes.rs` | 路由规则 CRUD（含 topology） |
| `numbers.rs` | 号码库存 |
| `billing.rs` | 计费账户 + 流水 |
| `cdr.rs` | CDR 查询 |
| `recording.rs` | 录音查询 |
| `ivr_menus.rs` | IVR 菜单（含 nodes/edges JSONB） |
| `call_center.rs` | 座席 + 队列 |
| `anti_fraud.rs` | 反欺诈规则 |
| `registrations.rs` | 注册状态 |
| `dashboard.rs` | 仪表盘统计 |
| `report.rs` | 报表导出 |
| `termination/` | 终结域（中继/IP规则/出局端点/号码池/DID） |
| `hot_cache.rs` | Redis 热点缓存 |
| `sip_cluster.rs` | sip-edge 集群管理 |
| `media_cluster.rs` | media-edge 集群管理 |
| `metrics.rs` | Prometheus 指标 |
| `audit.rs` | 审计日志 |
| `system.rs` | 系统设置 |

## 统一响应格式

### 成功响应

```json
{
  "code": 0,
  "message": "success",
  "data": { },
  "timestamp": 1720000000,
  "request_id": "req_xxxxxxxxxxxx"
}
```

### 错误响应

```json
{
  "code": 40001,
  "message": "用户不存在",
  "details": "User with id 42 not found",
  "timestamp": 1720000000,
  "request_id": "req_xxxxxxxxxxxx"
}
```

### 分页响应

```json
{
  "data": [...],
  "pagination": {
    "page": 1,
    "page_size": 20,
    "total": 156,
    "total_pages": 8
  }
}
```

## 运行

### 本地开发

```bash
cargo run -p api-server --release
# 默认监听 0.0.0.0:8081
```

### Docker

```bash
docker run -d --name api-server \
  -p 8081:8081 \
  -e VOS_RS_DATABASE_URL=postgres://user:pass@db:5432/vosrs \
  vos-rs:api-server
```

### systemd

```bash
sudo systemctl enable --now api-server
```

## 鉴权

除 `/metrics` 和健康检查外，所有接口需携带 JWT：

```bash
curl -H "Authorization: Bearer <token>" http://localhost:8081/api/v1/users
```

Token 通过登录接口获取：

```bash
curl -X POST http://localhost:8081/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"xxx"}'
```

## 测试

### 单元测试

```bash
cargo test -p api-server
```

### 集成测试

集成测试在 `web/src/test/` 下，通过 Vitest 运行：

```bash
cd web && npm test
```

## 相关文档

- 服务总览：[../README.md](./README.md)
- API 设计规范：[../../AGENTS.md#8-api-设计规范](../../AGENTS.md)
- 环境变量：[../../docs/development/ENV_VARS.md](../../docs/development/ENV_VARS.md)
- Web 界面指南：[../../docs/user-guide/WEB_GUIDE.md](../../docs/user-guide/WEB_GUIDE.md)
