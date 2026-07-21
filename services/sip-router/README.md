# sip-router — SIP 集群路由器

> **vos-rs 的集群 SIP 代理** — 无状态 SIP 路由分发 + 对话路由 + 节点发现

## 这是什么？

`sip-router` 是 vos-rs 平台的 **集群前置 SIP 代理**。在多节点 `sip-edge` 集群部署时，`sip-router` 作为统一入口：
- 接收所有入站 SIP 流量
- 按 CallID 哈希分发到后端 `sip-edge` 节点（保证同一通话走同一节点）
- 维护对话路由表，in-dialog 请求路由到正确的后端节点
- 自动发现健康的 `sip-edge` 节点

适用场景：
- 多节点 `sip-edge` 集群部署
- 高可用（单节点宕机不影响服务）
- 横向扩展（按需增加 `sip-edge` 节点）

## 核心能力

| 能力 | 说明 |
| :--- | :--- |
| **无状态代理** | UDP + TCP 同时支持，不维护事务状态 |
| **CallID 哈希分发** | 同一通话路由到同一后端，避免状态不一致 |
| **对话路由** | in-dialog 请求（ACK/BYE/Re-INVITE）路由到初始节点 |
| **节点发现** | 通过 Redis 自动发现健康 `sip-edge` 节点 |
| **健康检查** | 定期 OPTIONS 探测，故障节点自动摘除 |
| **TLS 终结** | 可选 TLS 监听，解密后明文转发后端 |
| **TCP 连接池** | 复用 TCP 连接，降低握手开销 |
| **管理 API** | HTTP 端点查询路由表/节点状态 |

## 在项目中的位置

```text
用户软电话 ──SIP──→ sip-router ──CallID 哈希──→ sip-edge 节点 A
                  (无状态代理)         ├─→ sip-edge 节点 B
                                       └─→ sip-edge 节点 C
```

`sip-router` 是集群部署的可选组件，单机部署不需要它。

## 模块结构

| 模块 | 职责 |
| :--- | :--- |
| `proxy/` | SIP 代理核心（UDP 透传 + 事务转发） |
| `tcp/` | TCP 连接池 + 分帧 |
| `discovery.rs` | 通过 Redis 发现后端节点 |
| `routes.rs` | 对话路由表（Redis 存储） |
| `security.rs` | 安全防护（限速/ACL） |
| `http.rs` | 管理 HTTP API |
| `metrics.rs` | Prometheus 指标 |
| `config.rs` | 配置加载 |

## 与 Kamailio 的关系

`sip-router` 是 vos-rs 自研的轻量 SIP 代理，定位类似 Kamailio 但更聚焦：
- Kamailio：通用 SIP 服务器，功能全但配置复杂
- `sip-router`：专为 vos-rs 集群设计，零配置自动发现

生产环境也可用 Kamailio 替代，配置文件见 [../../deploy/kamailio/](../../deploy/kamailio/)。

## 运行

### 本地开发

```bash
cargo run -p sip-router --release
```

### 配置

`config.yaml` 关键项：

```yaml
router:
  udp_bind: 0.0.0.0:5060
  tcp_bind: 0.0.0.0:5060
  manage_bind: 0.0.0.0:8082
  redis_url: redis://127.0.0.1:6379
  dialog_route_ttl_secs: 3600
```

### Docker

```bash
docker run -d --name sip-router \
  -p 5060:5060/udp -p 5060:5060/tcp -p 8082:8082 \
  -e VOS_RS_REDIS_URL=redis://redis:6379 \
  vos-rs:sip-router
```

## 管理 API

| 路径 | 方法 | 说明 |
| :--- | :--- | :--- |
| `/router/nodes` | GET | 当前健康的后端节点列表 |
| `/router/dialogs` | GET | 活跃对话路由表 |
| `/metrics` | GET | Prometheus 指标 |

## 性能

- **无状态代理**：单机 10000+ CPS
- **TCP 连接池**：复用连接，避免握手开销
- **Redis 路由表**：对话路由 O(1) 查找

## 相关文档

- 服务总览：[../README.md](./README.md)
- 集群部署：[../../docs/deployment/CLUSTER_DEPLOYMENT.md](../../docs/deployment/CLUSTER_DEPLOYMENT.md)
- Kamailio 替代方案：[../../deploy/kamailio/README.md](../../deploy/kamailio/README.md)
