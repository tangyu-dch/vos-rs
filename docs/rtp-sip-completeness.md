# RTP 和 SIP 完成度

本文档追踪了当前项目对照 VOS 级 VoIP 平台的 SIP/RTP 覆盖范围。当前代码可以运行一个完整的本地 UDP SIP 呼叫，支持 SDP 重写、PCMU/PCMA RTP 中继、BYE 和 CDR 持久化，但它目前仍是一个 MVP，而非生产环境的运营商级软交换。

## 当前评估结论

- SIP 控制面覆盖度：足以在 UDP 上运行基本的 B2BUA 呼叫流程。
- RTP 媒体面覆盖度：足以通过 G.711 PCMU 和 PCMA 进行普通 RTP 中继。
- SDP 音频媒体段 SDES-SRTP `a=crypto` 属性解析已加入 `sdp-core`，可提取 crypto tag、suite、key parameters 和 session parameters；`rtp-core` 已支持将 `inline:` key parameters 解码为 `SrtpConfig`。
- 最主要的遗留差距：完整的 SIP 服务端事务/高级对话行为、高级 NAT 穿透、高级 RTCP 通话质量报告、SRTP、生产环境路由策略、集群状态以及运维可观测性。

## 当前本机性能基线

最近一次本机 SIPp + RTP 压测日期：2026-07-09。测试环境为本地 loopback，`sip-edge` release 构建，PostgreSQL/NATS 关闭，RTP 端口范围 `40000-65000`，媒体由 `tools/sipp/rtp_range_sender.py` 生成。

| 场景 | 条件 | 当前结果 |
|------|------|----------|
| SIP + RTP relay，录音关闭 | 20k RTP pps，2048 个 relay RTP 端口扫发 | 400 target CPS / 2200 calls / 0 failed 通过；450 target CPS / 2400 calls 出现失败 |
| SIP + RTP relay + 录音开启 | 1000 RTP pps，1024 个 relay RTP 端口扫发，`VOS_RS_RECORDING_WORKERS=4` | 300 target CPS / 1800 calls / 0 failed 通过 |
| SIP + RTP relay + 录音开启 | 5k RTP pps，512 个 relay RTP 端口扫发，`VOS_RS_RECORDING_WORKERS=4` | 80 target CPS / 500 calls / 0 failed 通过；100 target CPS / 600 calls 出现 5 failed |
| SIP + RTP relay + 录音开启 | 5k RTP pps，512 个 relay RTP 端口扫发，`VOS_RS_RECORDING_WORKERS=8` | 100 target CPS / 600 calls / 0 failed；录音丢包 207 |
| SIP + RTP relay + 录音开启 | 5k RTP pps，512 个 relay RTP 端口扫发，`VOS_RS_RECORDING_WORKERS=16` | 100 target CPS / 600 calls / 5 failed；录音丢包 1，不建议无 CPU 余量时继续增加 worker |

已做的相关优化：

- `tools/sipp/run_bench_media.sh` 现在使用长通话 SIPp 场景、SIPp timeout、真实 RTP 发送器和 edge 侧媒体计数器，避免旧脚本把“无媒体呼叫”误当成媒体 CPS。
- `tools/sipp/rtp_range_sender.py` 会按配置的 RTP 端口范围发送 PCMU RTP，并在收到 TERM/INT 时输出最终发包统计。
- 录音启动改为懒加载：接通路径只登记录音 session，首次收到 RTP 时才创建 WAV、写 JSON 元数据并启动录音 worker。
- 录音从“每通一个 OS thread”改为固定 worker pool，同一通话固定分配到同一 worker，避免高 CPS 下线程爆炸。
- WAV 录音不再每个 RTP 包随机 seek/read/write 和刷新 header，改为内存双声道混音缓冲、周期性顺序写入，并在录音结束时最终刷新 header。
- sip-edge 管理 API 新增 `/manage/media-metrics`，api-server 新增 `/api/media/metrics`，并将媒体聚合指标接入 `/metrics` Prometheus 文本输出，可直接观察录音队列深度、队列容量、worker 数、录音队列丢包与录音错误计数。

仍需优化的性能风险：

- 当前录音已经使用固定 worker pool，但仍和 `sip-edge` 同进程竞争 CPU/磁盘；生产目标建议拆成专用媒体录音进程或独立媒体节点。
- WAV 混音已改为内存缓冲和顺序写，录音队列容量可通过 `VOS_RS_RECORDING_QUEUE_CAPACITY` 调整，但高 pps 下仍受本地磁盘、worker 数、channel backpressure 和 SIPp 本机 UDP buffer 影响。
- 高 CPS 失败档会出现 INVITE/BYE client transaction timeout，需继续优化 SIP 事务调度、UDP socket buffer、SIPp 多实例压测方式和本机内核 UDP buffer。

## SIP 协议覆盖范围

已实现：

- UDP SIP 监听器与解析器集成。
- 支持紧凑标头（compact headers）、重复标头（repeated headers）、折行标头（folded headers）以及基于 Content-Length 的消息体处理的 SIP 请求/响应解析。
- 支持的方法识别：REGISTER、INVITE、ACK、BYE、CANCEL、OPTIONS、INFO、UPDATE、REFER、SUBSCRIBE、NOTIFY、MESSAGE、PRACK 以及自定义扩展方法。
- 具有内存缓存与 PostgreSQL 数据库持久化的 REGISTER 注册机，支持 Contact 绑定、Expires 过期处理、通配符注销及可选 Digest 认证。
- 通过 PostgreSQL 数据库驱动的多网关自动故障转移（Gateway Failover）与最长前缀+优先级匹配的动态路由选择。
- 数据库驱动路由策略增强：路由表新增 `cost` 字段支持最低成本路由（LCR，按 `prefix 长度 desc → priority desc → cost asc` 排序），网关表新增 `max_capacity` 字段用于按网关活跃呼叫并发上限的容量控制。
- 网关健康度熔断器（Circuit Breaker）：`GatewayHealthTracker` 跟踪每网关成功/失败计数、连续失败数与活跃呼叫数，连续失败超阈值或采样成功率低于阈值时熔断，`recovery_interval` 后半开探测恢复；路由候选自动过滤不可用/超容量网关，全部不可用时回退到完整候选集。
- 针对振铃、接通、失败、取消、终止状态的 B2BUA 呼叫状态迁移，以及 CDR 的生成。
- 对话内 ACK、BYE、CANCEL 和 INFO 消息的转发。
- 支持 SIP INFO DTMF（`application/dtmf-relay` 与 `application/dtmf`）按键提取，并在呼叫结束时写入 CDR。
- DTMF 事件审计明细表（`dtmf_events`）：按事件级别记录来源（`rtp` / `sip-info`）、digit、毫秒时间戳、RTP timestamp、duration 与音量，`media_relay` 在每个 DTMF 事件触发时即时采集，呼叫拆线时批量落库。
- 异步持久化增强：`cdr-worker` 支持批量消费（batching）、DLQ 死信流（独立 `VOS_RS_CDR_DLQ` stream）、基于 NATS 投递计数的有毒消息策略（`AckKind::Nak(Some(delay))` 重投 vs `AckKind::Term` 终止落 DLQ）以及 DB 写入失败的进程内指数退避重试。
- caller 与 gateway 两侧对话内请求基础双向处理：对话内 BYE/INFO/UPDATE/REFER 可按来源 leg 转发到对端 leg；REFER 可回复 `202 Accepted`，将 REFER 转发到对端 leg，并向转接发起方发送 `Event: refer` 的 NOTIFY sipfrag `100 Trying` 进度通知。
- 针对对端地址、From tag、协商后 To tag 以及 CSeq 顺序的基准对话校验。
- 针对非 ACK 重传的 UDP 重复请求响应缓存。
- 临时响应和最终响应从出局段（outbound leg）向入局段（inbound leg）的转发。
- 通过 PostgreSQL 或 NATS JetStream 配合 `cdr-worker` 进行 CDR 持久化。
- 启动时的数据库自动种子填充（Seeding）以解析和加载环境变量配置。
- SIPp 冒烟流程与本地完整流测试。
- 显式建模 Dialog（会话），支持 Record-Route / Route 路由集传递与 Contact 刷新，松散路由（Loose Routing）支持。
- 符合 RFC 3261 的客户端出局请求事务定时重传（Timer A/E）与超时失败管理（Timer B/F），并在超时无响应时本地合成 503 触发网关自动 Failover。
- 完整的服务端事务状态机（Invite / Non-Invite Server Transaction），支持重传与 Timer G/H/I/J。
- SIP over TCP, TLS, and WebSockets (RFC 7118) 传输层支持与活动连接复用池。
- 符合 RFC 3262 的可靠临时响应（PRACK）支持，包含 RAck 标头校验。
- 呼叫保持（Call Hold）、媒体重新协商以及 Session-Expires (RFC 4028) 会话刷新。
- 完整的 REFER 呼叫转接（盲转、桥接、通知、失败回滚）与 MESSAGE 即时消息路由引擎。
- 呼入 Digest 认证与安全动态 Nonce 生命周期管理及 Nc 防重放滑动窗口保护。
- SBC 安全防御控制（IP ACL、令牌桶 CPS 速率限制、活跃呼叫并发限制）。
- Path 与 Service-Route 路由机制支持。

未完成：

- 对话的分叉（forking）以及早期对话管理。
- 基于 Redis 存储共享的分布式注册、对话、事务及速率限制计数状态（集群高可用）。
- 活动呼叫的 HA 副本同步与重启恢复。
- SUBSCRIBE/NOTIFY 事件包。
- 拓扑隐藏与除 Path/Service-Route 以外的高级代理。
- 多租户域策略和每账户授权。
- 呼叫、注册关系、事务、RTP 中继和 CDR 的高可用（HA）集群。

## RTP 和 SDP 协议覆盖范围

已实现：

- 具有 RTP 版本校验、CSRC 处理、标头扩展处理、填充校验及负载类型范围校验的 RTP 数据包解析与编码基础函数。
- 具有复合包校验、已知包类型映射、长度校验及填充校验的 RTCP 数据包解析与编码基础函数。
- RTCP 发生方报告（Sender Report）和接收方报告（Receiver Report）字段解析，包括丢包率（fraction lost）、累计丢包数（cumulative lost）、扩展最高序列号、抖动（jitter）、LSR 和 DLSR。
- 支持静态 G.711 负载：PCMU（负载类型 0）和 PCMA（负载类型 8）。
- 支持从会话级（session-level）或媒体级（media-level）连接行解析首个音频 RTP 端点。
- 将首个音频 RTP 的 `c=` 和 `m=` 行重写为中继端点。
- SDP 负载修剪，保留兼容的 PCMU/PCMA 负载和 `telephone-event/8000` DTMF 动态负载，并移除不支持的负载特定属性。
- 可通过 `VOS_RS_RTP_ADVERTISED_ADDR`、`VOS_RS_RTP_PORT_MIN` 和 `VOS_RS_RTP_PORT_MAX` 配置 RTP 通告地址和偶数 RTP 端口范围。
- RTP 端口租用分配，自动跳过活动中继端口，在配置的端口范围耗尽时返回 SIP 503，并在呼叫拆线或出局呼叫失败时释放租期。
- 绑定在所配偶数端口上的 UDP RTP 监听器，以及绑定在相邻奇数端口上的 RTCP 监听器。
- 支持在主叫和网关段上从 SDP 学习 RTP/RTCP 目标。
- 配对中继端口之间的对称 RTP/RTCP 源学习，默认通过 `VOS_RS_RTP_SYMMETRIC_LEARNING=true` 启用。
- 基础的单端口 RTP/RTCP 计数器，以及关于丢包、抖动和估算 RTT 的最新/最大 RTCP 质量数据快照。
- RFC 2833/4733 `telephone-event` RTP DTMF 中继、按 RTP timestamp 去重后的按键重建、CDR `dtmf_digits` 写入以及 `dtmf_events` 媒体指标计数。
- 可选的呼叫录音（通过 `VOS_RS_RECORDING_ENABLED=true`），将中继的 PCMU/PCMA RTP 解码为单呼叫双声道 WAV 文件（附带 JSON 元数据）。

未完成：

- 基于转发生成的 RTCP 报告生成、长窗口丢包统计、抖动趋势跟踪以及 MOS/R-factor 通话质量报告。
- SRTP/DTLS-SRTP 在 `rtp-core` 已有上下文和加解密基础；SDES `a=crypto` 已可解析，但尚未接入 SIP Edge 的 Offer/Answer 选择、RTP Relay 加解密和端到端媒体测试。
- 高级对称 RTP 策略、源绑定、基于超时的重新学习和反欺骗（anti-spoofing）。
- ICE/STUN/TURN 支持（注：基础 STUN 发现已实现，用于公网地址发现）。
- 更完整的 DTMF 实时事件流、按租户策略控制以及失败/异常事件告警（注：DTMF 事件审计明细表已落库至 `dtmf_events`，媒体聚合指标已可通过管理 API 与 Prometheus `/metrics` 查询）。
- 转码、编解码器首选策略、ptime/maxptime 协商、舒适噪音、静音抑制以及当前 PCMU/PCMA 路径之外的动态负载映射。
- 多个 `m=` 媒体部分、视频、T.38、RTP Bundle 及高级 Offer/Answer 行为。
- 更稳健的单呼叫媒体销毁保障及分布式媒体中继协同。
- 录音文件轮转、保留策略、合法拦截（lawful intercept）接口、QoS/DSCP 标记以及更细粒度的按租户/按网关 Prometheus 指标导出。

## 推荐的后续步骤与演进路线图

我们对未来的工作项规划了如下的高优先级演进路线，将按顺序一个一个进行执行与完善：

1. **[已完成] 方案 1：PostgreSQL 动态持久化（订阅者、网关、路由与注册关系）**
   - **核心目标**：将静态环境配置（如认证用户、网关路由等）以及内存中的动态 REGISTER 注册 Contact 绑定持久化至 PostgreSQL。
   - **意义**：实现动态路由变更免重启，并为多节点集群与高可用打下分布式状态共享基础。
2. **[已完成] 方案 2：显式建模 Dialog（会话）与 RFC 3261 路由集（Record-Route）**
   - **核心目标**：在 `call-core` 中增加完整的 `Dialog` 会话状态跟踪，支持 Record-Route / Route 标头重写、Contact 目标刷新和早期对话管理。
   - **意义**：支持复杂网络拓扑（多级 Proxy/SBC 穿透）与拓扑隐藏。
3. **[已完成] 方案 3：实现符合 RFC 3261 规范的 SIP 事务层（定时器与重传机制）**
   - **核心目标**：在 UDP 传输上实现完整的 Client/Server Transaction 状态机，以及 Timer A~K 的主动调度和响应重传。
   - **意义**：极大地提升弱网/丢包环境下呼叫的稳定性和抗网络抖动能力。
4. **[已完成] 方案 4：通话媒体质量观测性与 RTCP MOS/R-factor 通话质量报告**
   - **核心目标**：解析对端 RTCP 报告并动态计算丢包率、抖动与延时，生成 MOS（音质得分），并在 CDR 中落库。
   - **意义**：提供运营商级音质故障诊断与服务水平协议（SLA）指标分析。
5. **[已完成] 方案 5：RFC 2833 带外 DTMF 中继与计费支持**
   - **核心目标**：支持 `telephone-event` 负载解析与转发，并对 DTMF 按键事件进行记录和审计。

---
*注：当前主线已经覆盖 UDP/TCP/TLS/WS 传输、事务与服务端重传状态机、SDP 重写、RTP/RTCP 中继、PRACK 校验、呼叫保持、SBC 安全防御、Path/Service-Route 路由、PCMU/PCMA 协商、RTP/SIP INFO DTMF + `dtmf_events` 审计明细表、基础录音、CDR、PostgreSQL 与 NATS 队列化（含 DLQ 与有毒消息策略）及 RTCP MOS 报告；路由侧已补齐 LCR + 网关健康熔断 + 容量控制。下一阶段建议优先补齐 SRTP/DTLS-SRTP 或高可用/分布式存储。*
