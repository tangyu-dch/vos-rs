# Webhook 呼叫事件协议

当前实现提供监听型 Webhook 最小闭环：

```text
CallManager -> 有界内存队列 -> NATS JetStream -> HTTP Webhook
                                              -> Redis 投递记录
```

SIP 呼叫热路径只执行 `try_send`，不会等待 NATS、HTTP、Redis 或数据库。

## 配置

所有配置位于 `config.yaml` 的 `sip_edge.webhooks`，仅
`VOS_RS_CONFIG_FILE` 可用于指定配置文件位置。

启用前必须配置：

```yaml
sip_edge:
  webhooks:
    enabled: true
    endpoint_url: "https://example.com/webhooks/calls"
    signing_secret: "使用至少16字符的随机密钥"
```

## 事件信封

```json
{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "schema_version": "1.0",
  "call_id": "sip-call-id@example.com",
  "sequence": 42,
  "occurred_at_ms": 1720000000123,
  "event_type": "call_answered",
  "data": {
    "sip_status": 200
  }
}
```

当前生命周期事件：

- `call_initiated`
- `call_ringing`
- `call_answered`
- `call_finished`

同一个 `event_id` 可能因超时或服务端错误被重复投递，接收方必须按
`event_id` 实现幂等处理。

## 签名校验

请求包含以下 Header：

- `X-VOS-Webhook-Id`
- `X-VOS-Webhook-Timestamp`
- `X-VOS-Webhook-Signature: v1=<hex>`

签名原文为：

```text
<timestamp>.<raw_request_body>
```

使用 `signing_secret` 执行 HMAC-SHA256，并以小写十六进制输出。接收方应：

1. 检查时间戳与当前时间的差值，建议最多允许 5 分钟。
2. 使用收到的原始请求体计算签名，不能重新序列化 JSON。
3. 使用常量时间比较签名。
4. 按 `event_id` 去重。

## 重试规则

- 网络错误、HTTP 408、429 和 5xx：指数退避重试。
- 其他 4xx：视为永久失败，不重试。
- 达到 `max_deliveries` 后终止 JetStream 重投，并记录为 `failed`。
- HTTP 成功但 Redis 记录失败时不会 ACK，保证投递记录恢复后重新处理。

## Redis 投递记录

单条记录：

```text
GET vos_rs:webhooks:delivery:<event_id>
```

按更新时间排序的事件 ID：

```text
ZREVRANGE vos_rs:webhooks:deliveries 0 99
```

记录状态包括 `delivered`、`retrying`、`failed`，并带有尝试次数、HTTP
状态码和最后错误。记录按照 `delivery_record_ttl_secs` 自动清理。
