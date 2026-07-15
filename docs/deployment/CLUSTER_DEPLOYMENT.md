# VOS-RS SIP 与媒体集群部署指南

VOS-RS 支持两种生产入口，二者共用同一套 `sip-edge`、Redis、NATS 和
`media-edge` 集群协议：

1. **外部路由模式**：使用 Kamailio/OpenSIPS 提供公网 SIP 入口；
2. **原生路由模式**：使用项目内置的纯 Rust `sip-router`，不把系统可用性绑定在 Kamailio 上。

单节点仍可使用 `direct` 模式，不要求部署路由器。

## 集群职责

```text
SIP 终端/运营商
        │
        ├── Kamailio/OpenSIPS（external）
        │          或
        └── VOS-RS sip-router（native）
                    │
          ┌─────────┴─────────┐
       sip-edge-1          sip-edge-2
          │  └──── Redis/NATS ┘  │
     media-edge-1          media-edge-2
```

- 路由器负责 SIP 节点健康检查、初始请求分配、事务回程和对话粘性。
- `sip-edge` 负责 B2BUA、认证、路由、计费、CDR 和媒体节点分配。
- Redis 保存节点心跳、注册流归属、对话快照、Call-ID 映射和媒体租约。
- NATS 承担发往其他 SIP 节点本地 TCP/WebSocket 连接的消息。
- `media-edge` 处理 RTP/RTCP、DTLS-SRTP、录音和媒体质量统计；RTP 热路径不查询 Redis。

## SIP 节点配置

每台 `sip-edge` 使用独立的 `node_id` 和内网 `advertised_addr`，但连接相同的
PostgreSQL、Redis 和 NATS。

```yaml
connections:
  redis:
    host: "10.0.2.10"
    port: 6379
  nats:
    url: "nats://10.0.2.11:4222"

sip_edge:
  cluster:
    enabled: true
    node_id: "sip-edge-1"
    router_mode: "external" # 或 native
    advertised_addr: "10.0.0.11:5060"
    heartbeat_interval_secs: 3
    node_timeout_secs: 10
    dialog_ttl_secs: 86400
    nats_subject_prefix: "vos_rs.sip.node"
```

`node_timeout_secs` 必须大于心跳周期。集群启动时如果缺少 Redis 或 NATS，服务会
拒绝启动，防止节点在不共享状态的情况下静默分裂。

## 多媒体节点配置

```yaml
sip_edge:
  media:
    allocation_strategy: "weighted_round_robin"
    health_check_interval_secs: 3
    unhealthy_threshold: 3
    nodes:
      - id: "media-edge-1"
        type: "remote"
        control_url: "http://10.0.1.11:3030"
        advertised_addr: "203.0.113.11"
        port_min: 40000
        port_max: 40998
        weight: 2
        control_token: "replace-with-random-token-1"
      - id: "media-edge-2"
        type: "remote"
        control_url: "http://10.0.1.12:3030"
        advertised_addr: "203.0.113.12"
        port_min: 41000
        port_max: 41998
        weight: 1
        control_token: "replace-with-random-token-2"
```

分配策略：

- `weighted_round_robin`：按权重轮询，适合节点规格不同的常规部署；
- `least_sessions`：优先使用活跃会话最少的健康节点；
- `call_id_hash`：按 Call-ID 稳定分配，适合需要可预测归属的部署。

`nodes` 是唯一媒体服务配置入口，而且至少需要一个节点。`type: local` 表示由当前
`sip-edge` 进程内承载媒体，不配置 `control_url`；`type: remote` 表示独立
`media-edge`，必须提供 HTTP、HTTPS 或 UDS 控制地址。一个节点就是单媒体部署，多个
节点组成媒体池；同一池最多包含一个 local 节点，并可与多个 remote 节点混合调度。
所有节点的 RTP 端口段必须不重叠，以便仅凭 RTP 端口确定后续控制请求的节点归属。

纯本地部署也使用相同结构：

```yaml
sip_edge:
  media:
    nodes:
      - id: "local-media"
        type: "local"
        advertised_addr: "198.51.100.10"
        port_min: 40000
        port_max: 40998
        weight: 1
```

节点列表为空、配置多个 local 节点、远程节点缺少控制地址或端口段重叠时，
`sip-edge` 会在监听端口前拒绝启动。管理界面的“媒体节点集群”页面执行同样校验。

## 两种 SIP 入口模式

### external：Kamailio/OpenSIPS

入口必须开启 Record-Route，并以 Call-ID/对话标识做一致性分配。初始 INVITE、
REGISTER 和对话内请求需保持同一节点；健康检查失败后只把新会话分配给其他节点。
后端地址使用 `sip_edge.cluster.advertised_addr`，不要使用公网 NAT 地址。
仓库提供 `deploy/kamailio/kamailio.cfg`、`dispatcher.list` 和部署说明作为起始模板。
多台 Kamailio 需要通过 DMQ/共享 htable 复制 Call-ID 归属，或由入口负载均衡器保持
连接亲和，避免同一对话落到不同的路由器实例。

### native：VOS-RS sip-router

原生路由器与 external 模式使用相同的节点注册数据和对话归属规则。当前支持 UDP、
TCP 节点动态发现、Call-ID 稳定选路、事务响应回程和多路由器共享对话归属。WebSocket
连接终止在 `sip-edge`；Redis 记录连接归属，其他 SIP 节点通过 NATS 将出站消息投递给
持有该连接的节点。路由器不做 B2BUA、计费或媒体处理，可独立水平扩容。

```yaml
sip_router:
  udp_bind: "0.0.0.0:5060"
  tcp_bind: "0.0.0.0:5060"
  advertised_addr: "198.51.100.10:5060"
  node_key_prefix: "vos_rs:cluster:sip_nodes"
  discovery_interval_secs: 2
  transaction_ttl_secs: 64
  dialog_route_ttl_secs: 86400
```

## 故障语义

- SIP 节点故障：新呼叫切换至健康节点；已建立媒体仍由原 `media-edge` 转发。
- UDP 对话：可依据 Redis 对话快照恢复后续请求，但正在进行的 SIP 事务不迁移。
- TCP/WebSocket：节点故障后终端必须重连，注册流随新连接更新归属。
- 媒体节点故障：停止分配新呼叫；现有媒体迁移需要 re-INVITE 或 ICE restart，不能只改 Redis。

## 启动前校验

```bash
make cluster-check CONFIG_FILE=/etc/vos-rs/config.yaml
```

生产环境还应确认 Redis/NATS 为高可用部署、节点 ID 不重复、防火墙已放行对应 SIP/RTP
端口，并通过密钥管理系统提供 `control_token` 和基础设施凭据。

仓库内置以下媒体闭环测试：

- `make full-flow`：单 local 节点；
- `make full-flow-remote`：单 HTTP remote 节点；
- `make full-flow-uds`：单 UDS remote 节点；
- `make full-flow-cluster`：双 remote 节点；
- `make full-flow-hybrid`：local 与 remote 混合调度。
