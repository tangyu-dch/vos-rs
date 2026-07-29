# RWI 实时控制台设计

本规范定义 VOS-RS 中的 RWI（Real-Time WebSocket Interface）实时控制台协议与链路设计。RWI 是面向运维与管理员的实时双工通道，提供 SIP 通话全生命周期事件推送以及强插、播报、监听、转接、挂断等控制指令能力，与基于 HTTP 的 Webhook 体系完全解耦。

---

## 一、设计目标

| 目标 | 说明 |
| :--- | :--- |
| 实时通话事件推送 | 覆盖 SIP 通话全生命周期（发起、振铃、接通、结束、媒体事件），毫秒级抵达浏览器 |
| 双工控制指令 | 支持强插（BargeIn）、播报（Speak）、监听（Listen）、转接（Transfer）、挂断（Hangup）五类下行指令 |
| 浏览器兼容 | 浏览器原生 WebSocket API 无法设置自定义 Header，需提供 query 参数回退认证方案 |
| 与 Webhook 体系解耦 | Webhook 未启用时仍能独立工作，事件广播链路具备独立运行模式 |

---

## 二、整体架构

RWI 链路分为三层，分别承担事件生产、事件转发、终端呈现职责，通过 NATS 主题与 WebSocket 通道串联。

```
┌─────────────────────┐       NATS 核心 / JetStream        ┌─────────────────────┐      WebSocket      ┌─────────────────────┐
│                     │  vos_rs.call.{event_type} (核心)   │                     │  /rwi/v1/ws 双工    │                     │
│   sip-edge (B2BUA)  │ ─────────────────────────────────▶ │   api-server (网关)  │ ◀──────────────────▶│  前端实时控制台      │
│                     │  vos_rs.call.commands  (核心)      │                     │  RwiMessage 信封    │  (React + TS)       │
│  - 通话事件生产      │ ◀───────────────────────────────── │  - 订阅 vos_rs.call.>│                     │  - 通话列表/详情    │
│  - 管理端点执行      │  JetStream 持久化主题 (Webhook)    │  - HTTP 转发管理端点 │                     │  - KPI/操作弹窗     │
└─────────────────────┘ ─────────────────────────────────▶ └─────────────────────┘                     └─────────────────────┘
        第一层：热路径生产                第二层：订阅与转发                       第三层：终端呈现
```

三层职责划分：

1. **sip-edge 热路径 → NATS 核心**：`CallManager` 在通话状态机变迁时写入有界内存队列，独立任务将事件以 `vos_rs.call.{event_type}` 主题发布到 NATS 核心。
2. **api-server WebSocket 网关 ← NATS 订阅**：订阅 `vos_rs.call.>` 通配主题，将 `WebhookEvent` 转换为 `RwiEvent`，封装为 `RwiMessage` 后通过 `/rwi/v1/ws` 推送前端。
3. **前端实时控制台 ← WebSocket 双工通道**：`useRwiWebSocket` 维护连接、心跳、重连，接收事件渲染列表，通过同一通道下发 `RwiCommand`。

---

## 三、协议定义

协议层定义位于 `call-core`，与传输层（NATS / WebSocket）完全解耦。传输层只负责搬运字节，协议层负责语义。

代码位置：`crates/call-core/src/rwi.rs`

### 3.1 RwiMessage 信封

所有上行事件与下行指令都封装在统一的 `RwiMessage` 信封中，通过 `payload` 字段的 `untagged` 枚举区分事件与指令。

```rust
pub struct RwiMessage {
    pub id: String,           // 消息 UUID
    pub version: String,      // 协议版本，当前为 "1.0"
    pub payload: RwiPayload,  // 消息载荷（untagged 枚举，序列化时扁平化）
}

pub enum RwiPayload {
    Event(RwiEvent),      // 上行事件
    Command(RwiCommand),  // 下行指令
}
```

序列化时 `payload` 通过 `#[serde(flatten)]` 扁平化到顶层，因此 `RwiEvent` 与 `RwiCommand` 的标签字段（`event` / `command`）会出现在 `RwiMessage` 的顶层 JSON 中。

### 3.2 RwiEvent 上行事件

`RwiEvent` 通过 `#[serde(tag = "event", content = "data")]` 进行内部标签序列化，`rename_all = "snake_case"`。

| 事件类型 | event 标签 | 字段 | 说明 |
| :--- | :--- | :--- | :--- |
| 呼叫发起 | `call_started` | `call_id: String`<br>`caller: String`<br>`callee: String`<br>`direction: String`<br>`timestamp_ms: i64` | B2BUA 收到 INVITE 并通过 ACL 校验后触发，direction 取值 `inbound` / `outbound` |
| 呼叫振铃 | `call_ringing` | `call_id: String`<br>`sip_status: u16`<br>`leg: String`<br>`timestamp_ms: i64` | 收到 180/183 临时响应时触发，leg 标识振铃腿（通常为 `b_leg`） |
| 呼叫接通 | `call_answered` | `call_id: String`<br>`sip_status: u16`<br>`leg: String`<br>`timestamp_ms: i64` | 对端应答 200 OK 时触发，sip_status 通常为 200 |
| 呼叫结束 | `call_ended` | `call_id: String`<br>`duration_secs: u64`<br>`reason: String`<br>`sip_status: Option<u16>`<br>`timestamp_ms: i64` | 通话释放或建立失败时触发，未接通时 duration_secs 为 0 |
| 媒体事件 | `media_event` | `call_id: String`<br>`event_type: String`<br>`payload: String`<br>`timestamp_ms: i64` | DTMF 按键、音频状态变动、ASR/TTS 状态等，payload 为 JSON 字符串 |

### 3.3 RwiCommand 下行指令

`RwiCommand` 同样使用 `#[serde(tag = "command", content = "data")]` 内部标签序列化。

| 指令类型 | command 标签 | 字段 | 说明 |
| :--- | :--- | :--- | :--- |
| 强插 | `barge_in` | `call_id: String`<br>`mode: String`<br>`target_leg: Option<String>` | mode 取值 `listen` / `whisper` / `listen_and_speak`，target_leg 指定强插腿 |
| 播报 | `speak` | `call_id: String`<br>`text: String`<br>`voice: Option<String>`<br>`speed: Option<f32>` | 调用 TTS 引擎向通道注入语音，speed 控制语速 |
| 监听 | `listen` | `call_id: String`<br>`stream_url: String`<br>`format: Option<String>` | 将通话媒体流转发到指定 WebSocket，format 通常为 `pcmu` / `pcm16` |
| 转接 | `transfer` | `call_id: String`<br>`target: String`<br>`transfer_type: Option<String>` | transfer_type 取值 `blind` / `attended`，target 为 SIP URI |
| 挂断 | `hangup` | `call_id: String`<br>`reason_code: Option<u8>` | reason_code 为 Q.850 原因值，通常为 16（正常挂机） |

---

## 四、事件广播链路

### 4.1 sip-edge 发布侧

`sip-edge` 在通话状态机变迁时调用 `CallManager::new_with_event_sink` 将 `WebhookEvent` 写入 `tokio::sync::mpsc::channel`，避免阻塞 SIP 热路径。后台任务从 channel 消费事件并发布到 NATS。

`spawn_publisher`（`pipeline.rs:121-151`）实现双主题发布：

1. **JetStream 持久化主题**：写入 WorkQueue 流，供 HTTP Webhook 投递消费者拉取并执行签名投递与失败重试。
2. **普通核心主题 `vos_rs.call.{event_type}`**：发布到 NATS 核心，供 `api-server` 的 RWI WebSocket 网关实时订阅。

```rust
// pipeline.rs:121-151 关键逻辑
fn spawn_publisher(
    jetstream: jetstream::Context,
    nats_client: async_nats::Client,
    config: WebhookConfig,
    mut receiver: mpsc::Receiver<WebhookEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let rwi_subject = format!("{RWI_EVENT_SUBJECT_PREFIX}{}", event_type(&event.event));
            let payload = serde_json::to_vec(&event)?;

            // 1. 持久化到 JetStream（用于 HTTP Webhook 投递与重试）
            publish_with_retry(&jetstream, &config, payload.clone()).await?;

            // 2. 同时广播到普通 NATS 主题，供 RWI WebSocket 网关实时订阅
            nats_client.publish(rwi_subject, payload.into()).await?;
        }
    });
}
```

`start_rwi_broadcast`（`pipeline.rs:50-73`）提供独立运行模式：当 Webhook 未启用但需要 RWI 实时控制台时，仅消费 channel 并发布到普通核心主题，跳过 JetStream 持久化，避免引入 Webhook 配置依赖。

代码位置：`services/sip-edge/src/webhooks/pipeline.rs`

### 4.2 api-server 订阅侧

`subscribe_and_forward_events`（`rwi_ws.rs:63-86`）订阅通配主题 `vos_rs.call.>`，消费到事件后调用 `parse_rwi_event` 解析。

由于 `sip-edge` 发布的是 `WebhookEvent` 结构，而 RWI 协议使用 `RwiEvent`，因此 `parse_rwi_event_from_value`（`rwi_ws.rs:111-180`）承担类型转换映射：

| WebhookEvent.event_type | RwiEvent 变体 | 字段映射说明 |
| :--- | :--- | :--- |
| `call_started` / `call_initiated` | `CallStarted` | 从 `data.caller` / `data.callee` / `data.direction` 提取，direction 缺省为 `inbound` |
| `call_ringing` | `CallRinging` | 从 `data.sip_status` 提取，缺省 180；从 `data.leg` 提取，缺省 `b_leg` |
| `call_answered` | `CallAnswered` | 从 `data.sip_status` 提取，缺省 200；从 `data.leg` 提取，缺省 `b_leg` |
| `call_ended` / `call_finished` | `CallEnded` | 从 `data.duration_secs` / `data.reason` / `data.sip_status` 提取，sip_status 可选 |
| `media_event` / `dtmf_received` | `MediaEvent` | event_type 透传，data 节点整体序列化为 payload 字符串 |

转换完成后封装为 `RwiMessage`（随机生成 `id`、固定 `version: "1.0"`），序列化为 JSON 文本帧通过 WebSocket 推送前端。

代码位置：`services/api-server/src/rwi_ws.rs`

### 4.3 事件链路序列图

```
前端                api-server            NATS              sip-edge            SIP 终端
 │                      │                  │                   │                   │
 │                      │                  │                   │  收到 INVITE       │
 │                      │                  │                   │ ◀─────────────────│
 │                      │                  │  publish          │                   │
 │                      │                  │  vos_rs.call.     │                   │
 │                      │                  │  call_started     │                   │
 │                      │                  │ ◀─────────────────│                   │
 │                      │  subscribe       │                   │                   │
 │                      │  vos_rs.call.>   │                   │                   │
 │                      │ ◀────────────────│                   │                   │
 │                      │                  │                   │  180 Ringing      │
 │                      │                  │                   │ ◀─────────────────│
 │  WS Text (RwiMessage │                  │  publish          │                   │
 │  CallStarted)        │                  │  call_ringing     │                   │
 │ ◀────────────────────│                  │ ◀─────────────────│                   │
 │                      │ ◀────────────────│                   │                   │
 │  WS Text (CallRinging│                  │                   │  200 OK           │
 │ ◀────────────────────│                  │                   │ ◀─────────────────│
 │                      │                  │  publish          │                   │
 │                      │                  │  call_answered    │                   │
 │                      │                  │ ◀─────────────────│                   │
 │  WS Text (CallAnswered                    │                  │                   │
 │ ◀────────────────────│                  │                   │  BYE              │
 │                      │                  │                   │ ─────────────────▶│
 │                      │                  │  publish          │                   │
 │                      │                  │  call_ended       │                   │
 │                      │                  │ ◀─────────────────│                   │
 │  WS Text (CallEnded) │                  │                   │                   │
 │ ◀────────────────────│                  │                   │                   │
```

---

## 五、控制指令链路

### 5.1 前端发送

前端通过 `useRwiWebSocket` Hook 维护 WebSocket 连接生命周期。`sendCommand` 方法将 `RwiCommand` 包装为 `RwiMessage` 后通过 WebSocket 文本帧发送。

连接保活与容错策略：

| 机制 | 参数 | 说明 |
| :--- | :--- | :--- |
| 心跳发送 | 30 秒间隔 | 发送 `{type:'ping', ts: <Date.now()>}`，携带时间戳用于计算链路延迟 |
| 心跳超时 | 10 秒未收到 Pong | 主动关闭连接并触发重连（`PONG_TIMEOUT_MS = 10_000`） |
| 自动重连 | 指数退避 | 起始 1 秒，每次失败翻倍，上限 30 秒（`RECONNECT_BASE_MS=1_000`，`RECONNECT_MAX_MS=30_000`） |
| 链路延迟 | 前端计算 | 收到 Pong 时 `Date.now() - parsed.ts`，单位毫秒，展示为顶部 KPI |

代码位置：`web/src/pages/operations/rwi-console/use-rwi-websocket.ts`

### 5.2 api-server 转发

`process_ws_message`（`rwi_ws.rs:185-223`）接收前端 WebSocket 文本帧，反序列化为 `RwiMessage`，若 `payload` 为 `Command` 则调用 `execute_rwi_command`，同时回送 ACK（原指令回传）。

`execute_rwi_command`（`rwi_ws.rs:226-233`）实现双路下发：

1. **NATS 异步通道**：发布到 `vos_rs.call.commands` 主题，供 `sip-edge` 或其他订阅方异步消费，实现解耦。
2. **HTTP 同步执行**：调用 `relay_rwi_command`，将指令映射为 `sip-edge` 管理 API 端点，通过 HTTP POST 同步执行，确保指令立即生效。

```rust
// rwi_ws.rs:226-233
pub async fn execute_rwi_command(state: &AppState, cmd: &RwiCommand) -> Result<(), String> {
    // 1. NATS 异步广播（解耦订阅方）
    if let Some(nats) = &state.nats_client {
        if let Ok(bytes) = serde_json::to_vec(cmd) {
            let _ = nats.publish("vos_rs.call.commands", bytes.into()).await;
        }
    }
    // 2. HTTP 同步转发到 sip-edge 管理 API
    relay_rwi_command(state, cmd).await
}
```

代码位置：`services/api-server/src/rwi_ws.rs`

### 5.3 sip-edge 执行

`sip-edge` 内置管理 API 服务（`X-VOS-Token` 头部鉴权），承担指令的实际执行。`relay_rwi_command` 根据指令类型映射到对应端点。

| 端点 | 方法 | 对应指令 | 说明 |
| :--- | :--- | :--- | :--- |
| `/manage/calls/:call_id/play` | POST | Speak（音频文件） | 在通道中播放指定 WAV 音频 |
| `/manage/calls/:call_id/stop-play` | POST | - | 停止当前播放 |
| `/manage/calls/:call_id/mute` | POST | - | 静音指定腿 |
| `/manage/calls/:call_id/unmute` | POST | - | 取消静音 |
| `/manage/calls/:call_id/barge-in` | POST | BargeIn | 强插/耳语/监听模式切入 |
| `/manage/calls/:call_id/stream` | POST | Listen | 媒体流转发到指定 WebSocket |
| `/manage/calls/:call_id/status` | GET | - | 查询通话状态 |
| `/manage/calls/:call_id/monitor` | POST | Listen | 启动监听（媒体旁路） |
| `/manage/calls/:call_id/stop-monitor` | POST | - | 停止监听 |
| `/manage/calls/:call_id/terminate` | POST | Hangup | 强制挂断 |
| `/manage/calls/:call_id/transfer` | POST | Transfer | SIP REFER 转接 |

代码位置：`services/sip-edge/src/manage/media_control.rs`、`services/sip-edge/src/manage/mod.rs:87-110`

---

## 六、WebSocket 认证

### 6.1 浏览器兼容方案

浏览器原生 WebSocket API 无法在升级请求中设置自定义 Header，因此 RWI 端点 `/rwi/v1/ws` 提供双路认证提取策略：

| 优先级 | 来源 | 适用场景 |
| :--- | :--- | :--- |
| 1 | `Authorization: Bearer <token>` Header | 非浏览器客户端（如自动化测试、服务端到服务端） |
| 2 | query 参数 `access_token` | 浏览器 WebSocket 升级请求 |

JWT 中间件在 `extract_bearer_token` 中优先尝试 Header，若失败且请求路径为 `/rwi/v1/ws`，则回退到 query 参数解析。由于 JWT base64 编码可能包含 `+` / `/` / `=` 字符，必须使用 `percent_decode_str` 对 query 值进行 URL 解码，否则会导致 token 截断或校验失败。

代码位置：`services/api-server/src/middleware.rs:46-79`

### 6.2 前端配合

前端在建立 WebSocket 连接前，将当前 Bearer token 以 `access_token` query 参数形式追加到 URL，并对 token 执行 `encodeURIComponent` 编码以避免特殊字符破坏 URL。

```typescript
function appendTokenToUrl(url: string, token: string | null): string {
  if (!token) return url;
  const sep = url.includes('?') ? '&' : '?';
  return `${url}${sep}access_token=${encodeURIComponent(token)}`;
}
```

### 6.3 中间件链

RWI WebSocket 端点与普通 REST 端点共用同一套中间件链：

```
请求 → response_contract → jwt_auth → audit_log → handler
```

| 中间件 | 职责 |
| :--- | :--- |
| `response_contract` | 统一响应格式包装、错误码归一化 |
| `jwt_auth` | JWT 解码 + 数据库动态权限检查，WebSocket 端点额外支持 query 参数回退 |
| `audit_log` | 请求审计记录，自动脱敏 `access_token` / `token` / `authorization` 等敏感字段 |

---

## 七、前端实现

### 7.1 页面结构

实时控制台页面遵循 `AGENTS.md` 5.8 节前端页面统一规范，采用顶部状态栏 + 左右分栏的工作区布局。

| 区域 | 内容 | 数据来源 |
| :--- | :--- | :--- |
| 顶部 KPI | 并发通话 / 响铃中 / 已接通 / 链路延迟（毫秒） | `calls` state 聚合 + `pingMs` |
| 左侧通话列表 | 实时通话卡片，按开始时间倒序 | `useRwiWebSocket.calls` |
| 右侧详情面板 | 基本信息 / 实时事件 / 媒体质量三标签页 | 选中通话的 `LiveCallItem` |
| 操作弹窗 | 强插 / 播报 / 监听 / 转接 | `handleBargeIn` / `handleSpeakSubmit` / `handleToggleListen` / `handleTransferSubmit` |

代码位置：`web/src/pages/operations/rwi-console/`

### 7.2 前端规范

| 规范项 | 要求 |
| :--- | :--- |
| 语言 | 全中文呈现，遵循 `AGENTS.md` 5.8.1 节 |
| 术语翻译 | WebSocket 在界面文案中称为「实时双工通道」，TTS 称为「语音合成」，CPS 称为「每秒呼叫」 |
| 主题色 | HeroUI primary 蓝色，禁止紫色主题 |
| 状态色 | `已接通` = success（绿色），`响铃中` = warning（黄色），`已结束` = default（灰色） |
| 表格操作 | 使用 `isIconOnly` 图标按钮，不显示中文字符 |
| 主题持久化 | 主题设置跨会话持久化 |

---

## 八、核心设计决策

| 决策 | 选择 | 理由 |
| :--- | :--- | :--- |
| 协议层与传输层解耦 | `call-core` 定义 `RwiMessage`/`RwiEvent`/`RwiCommand`，`api-server` 负责 WebSocket 传输，`sip-edge` 负责 NATS 分发 | 协议可在不改动传输层的前提下扩展新事件/指令，便于多端复用 |
| NATS 双主题 | JetStream 持久化主题 + 普通核心实时主题 | 持久化主题支撑 Webhook 可靠投递与重试，核心主题支撑 RWI 低延迟广播，互不阻塞 |
| 双路指令下发 | HTTP 同步执行 + NATS 异步广播 | HTTP 同步确保指令立即生效并返回结果，NATS 异步解耦其他订阅方（如录音触发、计费联动） |
| 浏览器兼容认证 | `Authorization` Header 优先，`access_token` query 回退 | 兼顾非浏览器客户端的标准做法与浏览器 WebSocket 的限制，query 回退仅对 `/rwi/v1/ws` 路径生效，避免安全面扩大 |
| 独立 RWI 广播模式 | `start_rwi_broadcast` 在 Webhook 未启用时独立运行 | RWI 控制台不依赖 Webhook 配置，降低部署门槛，事件链路始终可用 |
| 前端中文化呈现 | 遵循 `AGENTS.md` 5.8 前端页面统一规范 | 面向国内运维团队，技术术语翻译为中文，避免界面出现英文缩写 |
