# SIP 协议覆盖范围

本文档追踪了 `vos-rs` 项目对照 VOS 级 VoIP 运营平台的 SIP 相关功能范围。当前的实现是一个可运行的 MVP，而非完整的运营商级软交换。

## 已实现

- UDP SIP 边缘监听器和数据报解析器。
- `sip-core` 中的 SIP 请求与响应解析。
- 紧凑标头（compact headers）、重复标头（repeated headers）、折行标头（folded headers）以及基于 Content-Length 的消息体处理。
- `sip-core` 识别的方法：REGISTER、INVITE、ACK、BYE、CANCEL、OPTIONS、INFO、UPDATE、REFER、SUBSCRIBE、NOTIFY、MESSAGE、PRACK 以及自定义扩展方法。
- 带有 Allow 标头的 OPTIONS 保活响应。
- 具有 Contact 存储、查询、Expires 过期处理、通配符注销及内存查找功能的 REGISTER 注册中心。
- 针对 REGISTER 的可选 SIP Digest 认证。
- 通过静态默认网关路由的 INVITE 路由选择。
- 针对本地用户间呼叫，将 INVITE 路由到已注册的 Contact 绑定。
- B2BUA 呼叫状态迁移，支持初始 INVITE、振铃、已接通、已失败、已取消、已终止，以及已完成 CDR 的生成。
- 针对已建立/被跟踪呼叫的对话内 ACK、BYE、CANCEL 和 INFO 转发。
- 基础的对话内校验，包括源对端地址、From tag、协商后的 To tag 以及单调递增的 CSeq。
- 针对重传的非 ACK 请求，带有 TTL 和容量限制的 UDP 请求事务响应缓存。
- 临时/最终的出局响应向入局对端的转发。
- SDP 音频端点解析以及用于媒体中继的 SDP 重写。
- RTP 和 RTCP 中继，具有可配置的通告地址、RTP 端口范围、活动端口租用分配、端口耗尽处理、对称源学习、数据包计数器以及 RTCP 丢包/抖动/RTT 质量快照。
- 用于 SDP Offer 和 Answer 的 G.711 PCMU 与 PCMA 编解码器协商，并在存在兼容语音负载时保留 `telephone-event/8000` 动态 DTMF 负载。
- SIP INFO DTMF 与 RFC 2833/4733 RTP telephone-event DTMF 的按键提取、去重累计和 CDR `dtmf_digits` 写入。
- 可选的 PCMU/PCMA 呼叫录音，生成双声道 WAV 文件（带 JSON 元数据）。
- 针对已完成呼叫的 PostgreSQL CDR 持久化。
- 用于高并发队列的可选 NATS JetStream CDR 事件发布。
- 具有持久化 JetStream 拉取消费者和 PostgreSQL 写入功能的 CDR 工作服务（cdr-worker）。
- 包含 REGISTER、网关呼叫、已注册用户呼叫、RTP 中继、BYE 及 CDR 验证的 SIPp 冒烟流程与本地全流程测试。
- 显式 Dialog 会话跟踪与 Record-Route / Route 路由集重写、Contact 目标刷新与松散路由支持。
- 符合 RFC 3261 的出局请求客户端事务定时重传（Timer A/E）与超时（32s）Failover 自动合成 503 触发。
- 符合 RFC 3261 的服务端事务状态机（Invite / Non-Invite Server Transaction），支持重传机制、定时器（Timer G/H/I/J）及重复请求吞噬与过滤。
- REGISTER 和 INVITE 呼入 Digest 认证，包含安全动态 Nonce 生命周期管理（5分钟失效）与 Nonce/NC 滑动窗口防重放攻击保护，防止 toll fraud 盗拨。
- 完整的 REFER 呼叫转接：支持 B2BUA 盲转（Blind Transfer），转接成功后自动双向桥接媒体并发送 terminated NOTIFY 结束订阅，转接失败后自动执行媒体会话回滚并恢复原 referrer 与 transferee 链路。
- 符合 RFC 4028 的 Session-Expires 与 Min-SE 会话保活机制，支持 UAC/UAS 刷新角色协商、半生命周期主动发送 UPDATE 刷新请求，以及对端/本地自生成刷新响应拦截 and 超时 BYE 释放通道。
- 完整的 SIP over TCP & TLS 传输层支持：支持接收和发送 TCP/TLS 信令、多连接活动连接池（Connection Pool）管理、按 Via 协议头自动复用或建连路由、以及基于 Content-Length 的流式分帧与半包粘包重组。
- 完整的 3xx 重定向递归支持（3xx Redirect Recursion）：当出局 INVITE 收到 `300..399` (如 `302 Moved Temporarily`) 响应时，B2BUA 自动提取 Contact 标头中的重定向目标，动态将其作为新路由候选，无缝发起下一轮出局拨号并重用媒体中继端口，对呼入侧完全透明。
- 完整的出局与会话内即时消息（MESSAGE, RFC 3428）路由引擎：支持对会话外（Pager/SMS）和会话内（Instant Messaging）消息基于注册中心 Contact 绑定与网关路由规则的自动路由、匹配 Via 临时映射、并对响应生命周期进行垃圾回收清理。
- 完整的 NAT 穿透与后台保活策略（RFC 3581 / RFC 5626）：支持自动检测并改写注册联系人及呼叫对话目标的公网映射（received_from 与 outbound_peer），支持对话外/内信令直接路由公网对端，并开启后台 Nat Keepalive 轮询以定期发送双 CRLF 或单 CRLF 探测包保持防火墙端口开放。
- 完整的 SIP over WebSocket 传输层实现（RFC 7118）：支持 `ws` / `wss` 连接、注册中心绑定、多传输协议动态检测路由并自动复用活动的 WebSocket 连接信道。
- 完整的呼叫保持（Call Hold）与媒体重新协商（Re-INVITE）：支持在 SDP offer/answer 重新协商时识别 `0.0.0.0` 及端口 0，自动安全丢弃媒体数据报以防止 socket 发送错误，同时在 SDP 改写中 100% 保留 `sendonly/recvonly/inactive` 等方向属性。
- 完整的可靠临时响应（PRACK，RFC 3262）支持：包含对呼入 PRACK 请求的 CSeq / RAck 标头（`<rseq> <cseq> <method>`）的校验与解析，对非法 PRACK 响应 `400 Bad Request`，对合法 PRACK 响应 `200 OK` 确认。
- 完整的 Path 与 Service-Route 路由支持（RFC 3327 / RFC 3608）：实现 REGISTER 请求中 Path 路由节点的提取与持久化，并在出局拨号时自动前置为 Route 标头，同时响应 Service-Route 给注册客户端。
- 完整的会话边界控制（SBC）安全防御：支持基于 CIDR/网段的 IP 访问控制列表（IP ACL）、基于令牌桶（Token Bucket）的 CPS 呼叫速率限制、以及基于注册账户/用户名的活跃呼叫并发限制（Call Concurrency Limits）。
- 完整的数据库驱动路由策略增强：基于 PostgreSQL 持久化路由表与网关表，支持最长前缀匹配、优先级排序、最低成本路由（LCR，按 `cost` 升序），并在网关表新增 `max_capacity` 容量上限字段用于活跃呼叫并发控制。
- 网关健康度检测与熔断器（Circuit Breaker）：`GatewayHealthTracker` 跟踪每个网关的成功/失败计数、连续失败数与活跃呼叫数，当连续失败超过阈值或采样成功率低于阈值时自动熔断（open），在 `recovery_interval` 半开探测恢复；路由候选阶段自动过滤不可用/超容量的网关，全部不可用时回退到完整候选集以避免黑洞。
- 异步持久化增强：`cdr-worker` 支持批量消费（batching）、DLQ 死信流（独立 `VOS_RS_CDR_DLQ` stream + `Limits` 保留策略）、基于 NATS 投递计数（`Info.delivered`）的有毒消息策略（未超 `max_deliveries` 时 `AckKind::Nak(Some(delay))` 重投，超限时 `AckKind::Term` 终止并落 DLQ）、以及 DB 写入失败时的进程内指数退避重试。
- DTMF 审计明细表：新增 `dtmf_events` 表与 `DtmfEventRecord`，按事件级别记录来源（`rtp` / `sip-info`）、digit、毫秒时间戳、RTP timestamp、duration 与音量；`media_relay` 在每个 RTP/SIP INFO DTMF 事件触发时即时采集，并在呼叫拆线时通过 `insert_dtmf_events_batch` 批量落库。

## 部分实现

- 对话处理：已实现完整的路由集（route set）、Record-Route / Route 传递与松散路由、Contact 目标改写刷新，但分叉的早期对话（forked early dialogs）尚未完成。
- 注册中心：内存绑定与数据库持久化可用，但多 Contact 策略与集群副本同步尚未完成。
- 媒体方面：RTP/RTCP 中继可处理基础的 RTP 数据包、复合 RTCP 校验、RTCP Sender/Receiver Report 字段解析、SDP 端点重写、对称源学习、活动端口租期、数据包计数、RTP DTMF 事件重建以及基础的 RTCP 质量快照。存在基础 PCMU/PCMA WAV 录音，但 SRTP、ICE、媒体转码、录音保留/存储策略、长窗口抖动/丢包/MOS 指标、防欺诈欺骗策略及通话质量报告导出尚未完成。
- INFO/DTMF：SIP INFO DTMF 与 RTP telephone-event DTMF 已能提取并汇总到 CDR，并已落库到 `dtmf_events` 审计明细表；但实时 DTMF 事件流、Prometheus 指标导出、按租户策略控制和告警尚未完成。

## 尚未实现

- SUBSCRIBE/NOTIFY 事件包。
- 拓扑隐藏与除 Path/Service-Route 以外的高级代理。
- 除 SDP 中继重写及 received/outbound_peer 覆盖以外的 NAT 穿透功能。
- 多租户域策略和每账户授权。
- 呼叫、注册关系、事务、RTP 中继和 CDR 的高可用（HA）集群。

## 下一步优先级与演进路线图

我们规划了如下的开发演进路线，将按顺序一个一个进行执行与完善：

1. **[已完成] 方案 1：PostgreSQL 动态持久化**：将静态环境配置（如认证用户、路由规则等）以及内存中的 dynamic REGISTER 注册 Contact 绑定持久化至 PostgreSQL。
2. **[已完成] 方案 2：显式建模 Dialog（会话）**：增加 Record-Route / Route 标头重写、Contact 目标刷新和早期对话管理，支持复杂代理链路与拓扑隐藏。
3. **[已完成] 方案 3：实现符合 RFC 3261 规范的 SIP 事务层**：实现出局客户端事务（Client Transaction）重传机制以及 Timer A/E 定时和 Timer B/F 超时 failover，强化弱网丢包环境下的呼叫稳定性。
4. **[已完成] 方案 4：通话媒体质量观测性（RTCP MOS 报告）**：计算丢包率、抖动与延时并生成音质 MOS 分数，记录到计费 CDR 中。
5. **[已完成] 方案 5：RFC 2833 带外 DTMF 中继与计费支持**：支持 telephone-event 负载转发、RTP DTMF 解析去重、SIP INFO DTMF 提取和 CDR 汇总。

---
*注：当前 SIP/RTP 主线已覆盖 UDP/TCP/TLS 传输层、WebSockets 传输、事务与服务端状态机重传、SDP 重写、RTP/RTCP 中继、PRACK 校验、呼叫保持、SBC 安全控制（ACL、并发、速率限制）、Path/Service-Route 路由、PCMU/PCMA、RTP/SIP INFO DTMF、DTMF 事件审计明细表、基础录音、PostgreSQL CDR、NATS 队列化（含 DLQ 与有毒消息策略）与 RTCP MOS 报告；路由侧已支持 LCR + 网关健康熔断 + 容量控制。*
