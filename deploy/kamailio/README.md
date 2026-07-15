# Kamailio 外部 SIP 入口模板

该模板为 `router_mode: external` 提供 UDP/TCP 入口、OPTIONS 健康检查、Call-ID
亲和、Record-Route 和初始事务故障切换。修改 `dispatcher.list` 中的地址，使其与各
`sip_edge.cluster.advertised_addr` 一致，然后挂载到 Kamailio 的 `/etc/kamailio/`。

```yaml
sip_edge:
  cluster:
    enabled: true
    router_mode: external
    node_id: sip-edge-1
    advertised_addr: 10.0.0.11:5060
```

模板中的 `htable` 会把 Call-ID 归属保留 24 小时，防止健康节点集合变化时把已建立
对话静默迁移到另一台 sip-edge。故障路由只为尚未建立的事务选择下一个节点；已建立
通话的信令恢复仍需上层重试或重新建联。

如果部署多台 Kamailio，入口负载均衡器必须保持五元组连接亲和，并启用 Kamailio
DMQ/共享 htable 复制对话归属；否则不同 Kamailio 实例可能得到不同的故障状态。
不希望维护这套外部状态时，可使用 VOS-RS `sip-router`，它通过 Redis 共享对话归属。

WebSocket 建议直接终止在 sip-edge。注册连接归属写入 Redis，跨 sip-edge 的出站消息
通过 NATS 回到持有 WebSocket/TCP 连接的节点。
