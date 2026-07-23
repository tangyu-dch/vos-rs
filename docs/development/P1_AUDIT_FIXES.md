# P1 级系统安全与可靠性漏洞修复指南

针对代码审计中发现的 12 项影响生产通话稳定性、数据一致性及安全的 P1 级缺陷，我们已完成了全面的修复和系统级重构。

---

## 修复缺陷列表与实现详情

### 1. SIP 信令与路由

#### 1.1 故障切换缺少非 2xx ACK
- **缺陷表现**：当 `sip-edge` 尝试向某一网关发起呼叫收到 408/5xx 最终故障响应时，会直接推进至下一条备用路由/网关，但没有向上一跳网关回复 ACK。导致旧网关持续重传最终响应，可能多次触发 `sip-edge` 的重选路由逻辑。
- **修复方案**：在 [outbound.rs](../../services/sip-edge/src/sip/outbound.rs) 中实现了 `build_non_2xx_response_ack`，严格按照 RFC 3261 构造逐跳 ACK（复用 Via 的 branch，不追加 Route-Set）。在 [response.rs](../../services/sip-edge/src/sip/dispatcher/response.rs) 的故障切换路径上实时构建并发送此 ACK 报文。

#### 1.2 业务 4xx 错误误计为中继故障
- **缺陷表现**：当主叫中继或被叫中继返回 401 Unauthorized, 403 Forbidden, 404 Not Found, 486 Busy 等业务错误时，系统将其全部计为网关物理故障，进而容易触发健康中继熔断。
- **修复方案**：调整 [response.rs](../../services/sip-edge/src/sip/dispatcher/response.rs) 中的分类逻辑，仅将 408 (超时) 和 5xx (服务器内部错误) 计入中继失败次数，业务常规响应不会触发健康扣分。

---

### 2. 媒体处理与并发控制

#### 2.1 SDP 媒体级 `c=` 连接行覆盖
- **缺陷表现**：RFC 4566 规定 `m=audio` 等媒体描述部分的 `c=` 连接地址应覆盖会话（Session）级别的连接地址。原快速改写逻辑未实现该覆盖，导致在复杂的 NAT 拓扑下改写后的媒体流无法送达正确的地址。
- **修复方案**：修改 [sdp.rs](../../crates/media-core/src/sdp.rs) 的快速重写状态机，解析到 `m=audio` 段内的 `c=` 行时直接更新 `original_addr`。

#### 2.2 会议混音锁跨 `.await` 导致死锁与静音丢包
- **缺陷表现**：在混音和向所有参与者发送 RTP 数据包时，`mix_and_send()` 持有了 `tokio::sync::Mutex` 并跨越了 UDP 发送的 `.await` 边界。由于网络发送可能耗时，这导致严重的锁竞争，使 `handle_rtp_packet` 无法获取到锁而直接丢弃音频数据包，形成通话瞬时静音。
- **修复方案**：将 `mix_and_send()` 重构为同步的 `mix_and_prepare()`。只在临界区内组装待发送数据包并返回 `Vec<(Arc<UdpSocket>, SocketAddr, Vec<u8>)>`，随后在锁外非阻塞地执行 UDP 发送。

#### 2.3 远程 `media-edge` DTMF 无法追踪
- **缺陷表现**：当通话媒体端口分配给远程媒体节点时，SIP 信令端的 `MediaRelayState` 仍在尝试写入本地状态来跟踪 DTMF 信号，导致远程节点的 RTP 业务无法向上层提供带内 DTMF 信号感知。
- **修复方案**：引入 `RemoteControlTarget` 路由寻址，如果分配到远程节点，则将 DTMF 操作（`/register_dtmf_tracking`, `/get_dtmf_digits` 等）透明路由/委托给对应的远程 `media-edge`。

---

### 3. 数据一致性与系统健壮性

#### 3.1 磁盘录音写失败静默通过
- **缺陷表现**：当磁盘空间耗尽或文件系统权限异常导致录音写入失败时，写盘后台 worker 没有将错误状态反馈给主会话，使得 API 仍返回录音成功，存在严重的金融和合规归档隐患。
- **修复方案**：在 `RecordingSessionInfo` 引入 `has_error` 原子变量。worker 写失败时将其置为 `true`。主会话在调用 `try_record` 时会进行阻断检查，向控制面抛出明确错误。

#### 3.2 CDR Spool 坏数据隔离
- **缺陷表现**：若由于系统断电导致 spool 文件末尾被非法截断或 JSON 数据损坏，解析器在加载该 spool 文件时会整体抛错，导致该文件中剩余的所有正常 CDR 数据无法正常入库并卡在队列中。
- **修复方案**：在 [cdr_spool.rs](../../services/sip-edge/src/cdr_spool.rs) 中改为逐行流式解析，遇到损坏行时将其转储至对应的 `.corrupt` 文件，同时保证健康行正常解析入库。

#### 3.3 IVR 及呼叫队列保存非原子事务
- **缺陷表现**：保存 IVR 或呼叫队列时，分为主表插入和子表（如 `ivr_actions`，`queue_agents`）的删除与重新写入。先前版本的子表操作未使用事务包裹，且使用了 `.ok()` 吞掉错误。如果中途发生断电或数据库故障，将产生不完整的孤儿配置。
- **修复方案**：将保存逻辑移入事务（`pool.begin()`），并在任何步骤出错时直接回滚事务，将具体的 API 错误透传给前端展示。

#### 3.4 数据库迁移并发死锁
- **缺陷表现**：当高并发启动多个服务容器实例时，每个实例会并行运行 Schema DDL 迁移，导致表级排他锁产生死锁。
- **修复方案**：在迁移开始时获取 PostgreSQL 事务级咨询锁 `SELECT pg_advisory_xact_lock(75812903)`，以串行化并发的迁移操作。

#### 3.5 部署环境的 NATS 健康检查
- **缺陷表现**：NATS 容器镜像极为精简，缺少 `nats-server` 等 CLI 命令，健康检查总是报告失败。
- **修复方案**：修改 [docker-compose.yml](../../deploy/docker/docker-compose.yml) 中的检查命令为直接发起 HTTP GET 探测：`["CMD", "wget", "-qO-", "http://127.0.0.1:8222/healthz"]`。

---

### 4. 计费系统金额定点化

- **缺陷表现**：计费、费率、透支额度等金额使用 `f64` 类型，在数据库中使用 `DOUBLE PRECISION` 强制类型转换，可能引入浮点精度截断问题，且每次累加均存在舍入误差。
- **修复方案**：全部改为使用符合金融要求的 `rust_decimal::Decimal`，并且在 SQL 中移除所有的 float 强制转换，保证高精度与账单数据对齐。

---

### 5. API 强凭证安全限制

- **缺陷表现**：API 允许在不设置 `VOS_RS_ENV=production` 的情况下默认绑定在 `0.0.0.0` 上公开暴露，并使用弱 JWT 密钥和默认账号密码启动，极易遭到黑客入侵。
- **修复方案**：修改 [main.rs](../../services/api-server/src/main.rs)，API 默认绑定到 `127.0.0.1`。若显式绑定到非回环地址（如 `0.0.0.0`），则强制激活生产环境凭证安全检验，拒绝使用默认弱密钥启动。
