# SIPp 中继与号码业务验证

这套用例验证的是数据库业务配置经过 SIP Edge 后形成的真实 SIP 信令，不只检查接口是否返回成功。执行器会从落地侧信令日志断言最终 `From` 主叫和 Request-URI 被叫，分机呼入会先完成 Digest REGISTER，再验证 DID 投递。

## 场景矩阵

| 场景 | 呼叫来源 | 主叫策略 | 预期落地 | 核心断言 |
|---|---|---|---|---|
| `passthrough` | `sipp-access-pass` | 严格透传 | `sipp-egress` | 授权号码 `861380020002` 原样进入落地 `From` |
| `fixed` | `sipp-access-fixed` | 固定号码 | `sipp-egress` | 任意来话主叫被改为 `861380020001` |
| `pool` | `sipp-access-pool` | 号码池 | 第二成员的 owner | 第一成员容量已满，改选 `861380020102` 并进入它的归属中继 |
| `extension-out` | 分机 `2001` | 固定号码 | `sipp-egress` | Digest 注册成功，呼出主叫改为 `4008002001` |
| `extension-in` | `sipp-egress` | DID 入站 | 分机 `2001` | 中继拥有 DID，DID 映射到已注册 Contact 并完成通话 |
| `owner-failure` | `sipp-access-fail` | 固定号码 | 号码 owner 返回 `503` | 上游收到终局 `503`，且其他中继端口没有收到 INVITE |

`owner-failure` 是安全用例，不是跨中继成功切换。真实号码一旦确定 owner，就不能因为 owner 故障而从其他运营商中继冒用该号码。跨 owner 的可控切换只能在主叫号码尚未最终选定时发生，例如号码池成员容量回退。

## 测试号码与端口

| 资源 | 地址或号码 | 用途 |
|---|---|---|
| SIP Edge | `127.0.0.1:5160` | 独立业务测试入口 |
| 接入透传 | `127.0.0.1:5164` | IP + 端口白名单接入 |
| 接入固定 | `127.0.0.1:5165` | 固定主叫策略 |
| 接入号码池 | `127.0.0.1:5166` | 容量回退策略 |
| 接入失败 | `127.0.0.1:5167` | owner 故障安全验证 |
| 主落地 | `127.0.0.1:5190` | `sipp-egress` |
| 备用归属落地 | `127.0.0.1:5191` | `sipp-egress-fail`，在 pool 场景正常应答 |
| 落地呼入源 | `127.0.0.1:5190` | 模拟主落地中继发起 DID 入站 |
| 分机 Contact | `127.0.0.1:5180` | 分机 `2001` 注册位置 |
| 固定主叫/DID | `4008002001` | 分机主叫及落地呼入 DID |
| 透传号码 | `861380020002` | 仅授权给透传接入中继 |
| 固定号码 | `861380020001` | 固定策略号码，owner 为主落地 |
| 号码池 | `861380020101/861380020102` | 首成员满载，第二成员被选择 |
| 测试被叫 | `9000000001` | 匹配前缀 `9` 的 6 秒/0.05 元测试费率 |

所有模拟端点都使用 `127.0.0.1`，通过唯一源端口区分中继。这会直接验证平台的 `IP:端口` 精确识别；同 IP 只有一个中继时仍兼容 IP-only 识别，同 IP 多中继且端口不匹配时必须拒绝歧义。

## 执行

依赖 PostgreSQL、Redis、NATS、`psql`、`curl` 和 SIPp 3.7+。默认执行全部场景：

```bash
tools/sipp/run_business_flows.sh all
```

也可以单独执行：

```bash
tools/sipp/run_business_flows.sh passthrough
tools/sipp/run_business_flows.sh fixed
tools/sipp/run_business_flows.sh pool
tools/sipp/run_business_flows.sh extension-out
tools/sipp/run_business_flows.sh extension-in
tools/sipp/run_business_flows.sh owner-failure
```

执行器会：

1. 通过 `business_seed.sql` 写入 `sipp-*` 隔离数据。
2. 检查并结束占用 `5160/5180/5182/5183/5190/5191` 的残留测试进程。
3. 使用 `business_flow.yaml` 启动独立 SIP Edge。
4. 逐场启动对应 UAC/UAS，并校验两侧 SIP 报文。
5. 结束后保留幂等的 `sipp-*` 隔离数据，便于在前端查看中继、号码和 CDR。

完整信令和错误日志位于 `target/sipp-business/`，带 RTP 的录音位于 `target/recordings/sipp-business/`。调试已有独立节点时，可设置 `BUSINESS_SKIP_BUILD=1` 和 `BUSINESS_SKIP_SEED=1`。

## 结果判定

- UAC 成功不等于业务成功，执行器还会核对落地侧实际收到的主叫、被叫和目标端口。
- `pool` 必须由 `127.0.0.1:5191` 收到主叫 `861380020102`，证明容量回退同时迁移号码 owner。
- `pool` 开始前会在 Redis 写入一个 300 秒测试租约占满 `861380020101`，结束时只清理该隔离租约。
- `owner-failure` 使用 UDP sentinel 监听非 owner 中继；任何转发都会令用例失败。
- `extension-in` 必须先返回 REGISTER `200`，随后同一 Contact 收到入站 INVITE。
- 接入场景发送 1 秒真实 RTP；CDR 录音路径必须对应可读取的 PCM WAV，录音 API 应返回 `audio/wav`。
- 费率为每 6 秒 0.05 元，不足 6 秒按一个脉冲计费；数据库金额保留三位展示精度。

## 仍需扩展的生产场景

- TCP/TLS/WSS 接入及 Digest 中继注册续期。
- CANCEL、早期媒体 `183`、无应答超时和主被叫任一侧 BYE。
- 双向 RTP、DTMF、传真和编解码转换；当前套件已覆盖单向 RTP 录音和 API 播放。
- 余额不足、并发抢占、Redis 租约续期及节点故障恢复。
- 同号码高并发和多进程 SIPp 压测；业务正确性套件默认串行，避免测试互相污染。
