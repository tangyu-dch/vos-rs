# VOS-RS 业务缺失与后续开发需求文档 (PRD)

> 文档版本: v1.2  
> 目标: 填补 VOS-rs 与行业主流可编程通信平台 (如 RustPBX, Twilio, Plivo) 的功能差距，在保持电信级高吞吐性能的同时，引入现代化的智能控制与观测诊断能力。

---

## 目录
1. [第一阶段：真实 SIP 信令捕获与可视化诊断 (SipFlow) ✅ 已完成](#1-第一阶段真实-sip-信令捕获与可视化诊断-sipflow-已完成)
2. [第二阶段：流式可编程控制与 Webhook 引擎 (VCI) ✅ 已完成](#2-第二阶段流式可编程控制与-webhook-引擎-vci-已完成)
3. [第三阶段：WebRTC 媒体安全接入与 WSS 信令支持 (已实现 / 待完善)](#3-第三阶段webrtc-媒体安全接入与-wss-信令支持-已实现--待完善)
4. [第四阶段：轻量级呼叫中心业务 (Queues & Conference) ✅ 已完成](#4-第四阶段轻量级呼叫中心业务-queues--conference-已完成)

---

## 1. 第一阶段：真实 SIP 信令捕获与可视化诊断 (SipFlow) ✅ 已完成

### 1.1 业务背景
当前系统的呼叫流程图是基于 CDR 时间戳近似合成的。在复杂路由（多网关备用切换、防盗打拦截、注册过期、呼叫挂机失败）场景下，合成数据无法反映真实的网络信令面细节。系统需要能够存储真实的 SIP 报文头部或事件流，以便管理员诊断丢包、超时及错误代码。

### 1.2 需求描述
- **信令抓包归档**：在 `sip-edge` 的信令收发点（UDP/TCP/TLS 读写层），以非阻塞方式提取 SIP 报文的 Method、方向、源/目的地址、Call-ID、From/To-Tag 及原始首部信息。
- **存储优化**：
  - 仅抓取信令包，不抓取 RTP 媒体数据包。
  - 使用数据库分区表或按天归档，支持自动过期清理机制（如只保留 7 天）。
- **前端可视化**：
  - 还原完全真实的 SIP 信令时序图。
  - 点击流程图中的单条信令（如 `INVITE` 或 `401 Unauthorized`），可展开弹窗查看完整的原始 SIP 文本报文。

### 1.3 实现状态：✅ 已完成
- **抓包**：`services/sip-edge/src/sip/sip_flow.rs` — `SipFlowWriter` 后台任务批量 100 条/100ms 落库，按 `uac_to_b2bua / uas_to_b2bua / b2bua_to_uac / b2bua_to_uas` 四向分类
- **存储**：`crates/cdr-core/src/store/sip_flow.rs` — 按日月分区 `sip_flows_y{YYYY}m{MM}d{DD}`，`insert_sip_flows_batch / get_sip_flows / delete_expired_sip_flows`
- **API**：`services/api-server/src/cluster/calls/sipflow.rs` — `GET /api/v1/calls/:call_id/sipflow`，优先返回真实抓包数据，无抓包时按 CDR 合成时序图
- **Copilot 集成**：`services/api-server/src/copilot/tools/dashboard.rs` — `vos_get_sip_flows` 工具支持 SIP ladder 可视化

---

## 2. 第二阶段：流式可编程控制与 Webhook 引擎 (VCI) ✅ 已完成

### 2.1 业务背景
传统的软交换采用固定的静态路由和分机配置。为了对接现代化 AI 语音坐席、交互式语音导航 (IVR) 及灵活的企业转接系统，系统需要具备"可编程"能力。

### 2.2 需求描述
- **双向 Webhooks 会话控制**：
  - 允许在路由规则中配置"Webhook 目的地"。
  - 当 `sip-edge` 收到呼入时，暂停呼叫建立，向三方服务器发送 `HTTP POST (incoming_call)`。
  - 根据三方服务器返回 of JSON 指令序列驱动通话状态。
- **VOS 呼叫指令集 (VCI)**：
  实现类似 Twilio / Plivo 的一套基础动作：
  - `play`：向呼叫通道流式播音。
  - `gather`：播音并同时收集 DTMF 按键输入，将按键反馈发送回三方 Webhook 决定下一步动作。
  - `dial`：呼叫第三方目的地，将两个 Leg 建立媒体桥接（B2BUA Bridge）。
  - `hangup`：指定 SIP 状态码并主动拆线。
  - `stream`：将通道的双向 PCM 音频通过低延迟 WebSockets 流式传给外部大语言模型接口，支持被叫"打断（Barge-in）"。

### 2.3 实现状态：✅ 已完成
- **指令分发器**：`services/sip-edge/src/sip/handlers/interactive_control/mod.rs` — `execute_instruction` 主入口
- **12 个指令全部实现**：`instructions/mod.rs` 覆盖 Dial/Play/Gather/Hangup/Stream/Record/Say/Queue/Conference/Redirect/Pause/PlayDigits
- **Webhook 引擎**：`services/sip-edge/src/sip/handlers/interactive_control/webhook.rs` — 支持 HTTP 与 NATS 两种 control_mode，事件回投与下一指令拉取
- **媒体指令**：`instructions/media.rs` — Play/Gather/Stream/Record/Say/PlayDigits
- **队列/会议指令**：`instructions/queue.rs` — Queue/Conference
- **架构设计**：详见 [NATS_VCI_COMMAND_DESIGN.md](./NATS_VCI_COMMAND_DESIGN.md)

---

## 3. 第三阶段：WebRTC 媒体安全接入与 WSS 信令支持 (已实现 / 待完善)

### 3.1 现状核查 (已实现部分)
经代码库核查，VOS-RS 的 **media-edge 已经具备原生 WebRTC 媒体面处理能力**：
- **ICE-Lite/STUN 协商**：`media-edge/src/media/relay/webrtc/ice.rs` 实现了完整的 STUN 报文识别与 Binding Request 验证响应。
- **DTLS 安全握手**：`media-edge/src/media/relay/webrtc/dtls.rs` 基于 `webrtc_dtls` 库自动生成自签名证书并协商 SRTP 密钥材料。
- **SRTP 加密解密**：`media-edge/src/media/relay/webrtc/srtp.rs` 实现了 RTP/RTCP 音频包的原地加密与解密。
- **SDP 双向转译**：`sip-edge/src/media/sdp.rs` 提供了 `rewrite_webrtc_offer_for_legacy`（将 WebRTC 转换为传统 SIP 媒体）和 `build_webrtc_answer`（为浏览器构造 WebRTC SDP）的逻辑。

### 3.2 业务缺失 (待完善与优化点)
尽管 WebRTC 媒体层已经打通，但仍存在以下**信令面与配置上的缺失**需要解决：
- **WebSocket (WS/WSS) 传输层稳定性**：
  - 当前 `sip-edge` 启动时，WebSocket 监听容易因为未配置有效的 TLS 证书链而失败。
  - 需要在 `config.yaml` 中完善 TLS 私钥/证书证书路径配置支持，并实现无缝的 WSS 代理与自动重连。
- **ICE/DTLS 握手异常状态监控**：
  - 在前端或 API 层面，缺乏展示特定通话 WebRTC 握手状态（ICE 成功、DTLS 握手中、SRTP 已激活）的诊断指标。
  - 需要将媒体层状态（如 DTLS 失败原因、丢包率等）实时反馈给 `api-server` 并在“媒体指标”中展现。

---

## 4. 第四阶段：轻量级呼叫中心业务 (Queues & Conference) ✅ 已完成

### 4.1 业务背景
企业应用往往需要多人多设备协同，比如客服排队、多人电话会议。

### 4.2 需求描述
- **智能排队队列 (Queues / ACD)**：
  - 支持多座席（Agent）登录和就绪状态机维护（空闲、通话中、话后处理 ACW、示忙）。
  - 通话进入队列后自动播放等待背景音（Music on Hold）。
  - 支持多种分配策略：轮询分配、最长空闲时间优先、群振（同时呼叫多个坐席）。
- **混音会议网桥 (Conference)**：
  - `rtp-core` 具备简单的混音算法，能够将多个入站 RTP 通道（音频 PCM）实时混合后，分发回各参与者，实现低延迟的电话会议室。

### 4.3 实现状态：✅ 已完成
- **排队引擎**：`crates/call-core/src/queue.rs` — `CallQueue` 支持 4 种策略（RoundRobin / LeastActive / Random / Weighted），`AgentState` 5 态状态机（Offline/Idle/Ringing/Talking/Paused），`QueueMetrics`（ASA/ACD 计算）
- **ACD 引擎**：`crates/call-core/src/acd.rs` — `AcdEngine` 独立实现，LongestIdle / RoundRobin 策略，MoH 触发
- **数据库表**：`crates/cdr-core/src/termination_schema.rs` — `call_queues / call_agents / queue_agents` 3 张表（外键级联）
- **REST API**：`services/api-server/src/resources/call_center.rs` — 完整 CRUD（队列与座席管理，含 moh_file / max_wait_secs / extension / status）
- **前端 UI**：`web/src/pages/call-center/queues.tsx` — Web 管理界面
- **会议桥**：`services/sip-edge/src/media/conference.rs` — `ConferenceManager` + 20ms Mix-Minus 混音循环，`crates/media-core/src/conference.rs` 底层混音算法
- **SIP 会议路由**：`services/sip-edge/src/sip/handlers/invite/conference.rs` — conf_/room_ 前缀路由到会议室
