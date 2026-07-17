# VOS-RS NATS 会话控制协议与命令设计规范 (NATS VCI & Command Design)

本规范定义了 VOS-RS 中基于 NATS 消息队列的呼叫控制协议（VOS Call Instruction - VCI 2.0）。系统支持两种层面的数据通信：
1. **呼叫生命周期事件 (Call Lifecycle Events)**：由 `sip-edge` 主动投递给控制器的事件，用于报告呼叫进度（如振铃、接通、按键、挂断）。
2. **呼叫控制指令 (Call Control Instructions / Commands)**：由控制器下发给 `sip-edge` 的指令，用于驱动媒体和信令行为（如放音、收键、外呼、挂断）。

系统支持 **交互式控制循环 (Request-Reply 模式)** 以及 **带外异步指令 (Pub/Sub 模式)**，两者完全共用相同的消息数据结构。

---

## 1. NATS 主题设计 (NATS Subjects)

| 配置项 (config.yaml) | 默认主题 | 模式 | 传输方向 | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| `control_incoming_subject` | `vos_rs.call.incoming` | Request-Reply | `sip-edge` ➔ 第三方控制器 | 投递“呼叫生命周期事件”，并同步等待控制器回复“呼叫控制指令”。 |
| `control_command_subject` | `vos_rs.call.commands` | Pub/Sub | 第三方控制器 ➔ `sip-edge` | 接收带外控制指令信封，进行异步的命令控制。 |
| `control_dtmf_subject` | `vos_rs.call.dtmf` | Pub/Sub | `sip-edge` ➔ 第三方控制器 | 当激活实时按键监听时，异步推送检测到的 DTMF 信号。 |

---

## 2. 第一部分：呼叫生命周期事件 (Call Lifecycle Events)

所有由 `sip-edge` 上报给控制器的事件都包装在统一的 `WebhookEvent` 结构体中，并通过 `event_type` 字段进行类型标识。

每一条事件消息的 `data` 节点均包含 `leg` 字段，以表明触发该事件的具体呼叫腿（Call Leg）：
- `"a_leg"`：表示主叫侧，通常对应入局端（Inbound Leg）。
- `"b_leg"`：表示被叫侧，通常对应出局端（Outbound Leg）。

### 2.1 事件统一信封格式 (`WebhookEvent`)
```json
{
  "event_id": "a576b4a3-76f8-45a9-bc84-9844ee48d1e2",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 1,
  "occurred_at_ms": 1720000000123,
  "event_type": "事件类型",
  "data": {
    "leg": "a_leg",
    "事件特有参数"
  }
}
```

### 2.2 事件类型矩阵与 JSON 示例

#### 1. 呼叫发起 (`call_initiated`)
*   **说明**：B2BUA 收到呼入的 `INVITE` 请求并经过基本 ACL 校验后触发。
*   **事件特有数据 (`data`)**：
    *   `caller` (字符串): 主叫标识，通常来自 SIP `From` 头部中的用户名。
    *   `callee` (字符串): 被叫号码，来自 `Request-URI`。
    *   `direction` (字符串): 呼叫方向：`"inbound"`（呼入）或 `"outbound"`（外呼）。
    *   `leg` (字符串): 腿标识，此处固定为 `"a_leg"`。
*   **JSON 示例**：
```json
{
  "event_id": "a576b4a3-76f8-45a9-bc84-9844ee48d1e2",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 1,
  "occurred_at_ms": 1720000000123,
  "event_type": "call_initiated",
  "data": {
    "caller": "1001",
    "callee": "1002",
    "direction": "inbound",
    "leg": "a_leg"
  }
}
```

#### 2. 主动发起外呼 (`call_originated`)
*   **说明**：通过平台 `originate` 指令向特定网关或被叫号码发起呼叫时触发。
*   **事件特有数据 (`data`)**：
    *   `target_uri` (字符串): 呼叫的目标 SIP URI。
    *   `caller_id` (字符串): 改写的主叫标识。
    *   `leg` (字符串): 腿标识，此处固定为 `"b_leg"`。
*   **JSON 示例**：
```json
{
  "event_id": "ee9a8f22-124b-4f99-bfb6-54a8cf66219f",
  "schema_version": "1.0",
  "call_id": "originate-b-leg-123",
  "sequence": 1,
  "occurred_at_ms": 1720000000500,
  "event_type": "call_originated",
  "data": {
    "target_uri": "sip:15300002222@192.168.1.100:5060",
    "caller_id": "10001",
    "leg": "b_leg"
  }
}
```

#### 3. 被叫振铃 (`call_ringing`)
*   **说明**：B2BUA 收到下游网关/终端返回 of `180 Ringing` 或 `183 Session Progress` 临时响应时触发。
*   **事件特有数据 (`data`)**：
    *   `sip_status` (整数): 触发该事件的 SIP 状态码（如 `180` 或 `183`）。
    *   `leg` (字符串): 腿标识，表示正在振铃的通话腿（如 `"b_leg"`）。
*   **JSON 示例**：
```json
{
  "event_id": "b118b4a3-76f8-45a9-bc84-9844ee48d442",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 2,
  "occurred_at_ms": 1720000002450,
  "event_type": "call_ringing",
  "data": {
    "sip_status": 180,
    "leg": "b_leg"
  }
}
```

#### 4. 呼叫接通 (`call_answered`)
*   **说明**：对应 Leg 应答接通（如被叫应答 `200 OK`，或主叫侧在本地应答）时触发。
*   **事件特有数据 (`data`)**：
    *   `sip_status` (整数): 触发应答的 SIP 状态码，通常为 `200`。
    *   `leg` (字符串): 腿标识，表示接通的通话腿（如 `"a_leg"` 或 `"b_leg"`）。
*   **JSON 示例**：
```json
{
  "event_id": "c928b4a3-76f8-45a9-bc84-9844ee48d610",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 3,
  "occurred_at_ms": 1720000005120,
  "event_type": "call_answered",
  "data": {
    "sip_status": 200,
    "leg": "b_leg"
  }
}
```

#### 5. 两路通话桥接成功 (`call_bridged`)
*   **说明**：当控制器下发 `bridge` 命令，将原本独立的两路呼叫在媒体层成功打通时触发。
*   **事件特有数据 (`data`)**：
    *   `call_id_a` (字符串): 参与桥接的 A-leg (Call-ID A)。
    *   `call_id_b` (字符串): 参与桥接的 B-leg (Call-ID B)。
*   **JSON 示例**：
```json
{
  "event_id": "f516a88b-11c9-4a92-ad55-78e81c0022fa",
  "schema_version": "1.0",
  "call_id": "call-bridge-group-1",
  "sequence": 4,
  "occurred_at_ms": 1720000005600,
  "event_type": "call_bridged",
  "data": {
    "call_id_a": "call_leg_a_12345",
    "call_id_b": "call_leg_b_12345"
  }
}
```

#### 6. 接收到按键 (`dtmf_received`)
*   **说明**：在执行 `gather` 指令收键成功，或在通话中通过 RFC 2833 / SIP INFO 收到 DTMF 信号时触发。
*   **事件特有数据 (`data`)**：
    *   `digits` (字符串): 接收到的按键字符序列（例如 `"1"`，`"123#"`）。
    *   `leg` (字符串): 腿标识，表明按键来自于哪一端。
*   **JSON 示例**：
```json
{
  "event_id": "d881b4a3-76f8-45a9-bc84-9844ee48d799",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 5,
  "occurred_at_ms": 1720000010500,
  "event_type": "dtmf_received",
  "data": {
    "digits": "1",
    "leg": "a_leg"
  }
}
```

#### 7. 呼叫结束 (`call_finished`)
*   **说明**：呼叫被挂机释放或呼叫建立失败时触发。
*   **事件特有数据 (`data`)**：
    *   `duration_secs` (整数): 通话的计费接通时长（秒）。若未接通，则该值为 `0`。
    *   `sip_status` (整数, 可选): 导致呼叫释放的 SIP 状态码。正常挂断时为 `200`。
    *   `q850_cause` (整数, 可选): ITU-T Q.850 挂机原因码（如 `16` 代表正常挂机）。
    *   `reason` (字符串): 可读的呼叫结束描述原因。
    *   `leg` (字符串): 挂断侧的腿标识。
*   **JSON 示例**：
```json
{
  "event_id": "f999b4a3-76f8-45a9-bc84-9844ee48d999",
  "schema_version": "1.0",
  "call_id": "invite-vci-123456@example.com",
  "sequence": 6,
  "occurred_at_ms": 1720000065200,
  "event_type": "call_finished",
  "data": {
    "duration_secs": 60,
    "sip_status": 200,
    "q850_cause": 16,
    "reason": "Normal clearing",
    "leg": "a_leg"
  }
}
```

---

## 3. 第二部分：呼叫控制指令 (Call Control Instructions / Commands)

控制器收到呼叫事件后，可以通过下发 `VciInstruction` 结构控制呼叫走向。带外指令通过 `vos_rs.call.commands` 发布时，同样需要使用 `CallCommand` 包裹控制指令。

### 3.1 控制指令类型矩阵与 JSON 示例

#### 1. 呼叫并发外呼/转接 (`dial`)
*   **说明**：将当前会话转接给一个或多个外部网关或分机，进行媒体桥接。
*   **参数说明**：
    *   `targets` (字符串数组): 目的 SIP URI 列表。如果配置多个，将根据 `sim_ring` 决定呼叫策略。
    *   `sim_ring` (布尔): 是否同时振铃，先应答者接通，其余支线释放。
    *   `caller_id` (字符串, 可选): 出局时改写的主叫号码。
    *   `timeout_secs` (整数, 可选): 并发外呼的振铃超时时间。
    *   `record_call` (布尔): 转接成功接通后，是否自动开启双向混音录音。
*   **JSON 示例**：
```json
{
  "action": "dial",
  "targets": ["sip:2001@192.168.1.100:5060"],
  "sim_ring": false,
  "caller_id": "88889999",
  "timeout_secs": 30,
  "record_call": true
}
```

#### 2. 播放提示音 (`play`)
*   **说明**：在通道中向主叫播放指定的 WAV 音频文件。
*   **参数说明**：
    *   `url` (字符串): 音频文件的绝对路径或网络可访问的 URL。
    *   `loop_count` (整数): 重复播放次数，`9999` 代表无限循环。
*   **JSON 示例**：
```json
{
  "action": "play",
  "url": "/opt/vos-rs/recordings/welcome.wav",
  "loop_count": 2
}
```

#### 3. 收集按键 (`gather`)
*   **说明**：在通道中向用户播放音频提示音，并收集用户按下的 DTMF 数字。
*   **参数说明**：
    *   `play_url` (字符串, 可选): 收键时的背景提示音。
    *   `max_digits` (整数): 最大收键位数。达到此位数自动截止上报。
    *   `timeout_ms` (整数): 等待收键的全局超时时间。
    *   `inter_digit_timeout_ms` (整数, 可选): 位间超时时间。
    *   `finish_on_key` (字符串, 可选): 收键的截止按键（如 `"#"`）。
    *   `barge_in` (布尔): 是否允许用户在提示音播放期间按键打断播放。
*   **JSON 示例**：
```json
{
  "action": "gather",
  "play_url": "/opt/vos-rs/recordings/menu.wav",
  "max_digits": 4,
  "timeout_ms": 10000,
  "inter_digit_timeout_ms": 3000,
  "finish_on_key": "#",
  "barge_in": true
}
```

#### 4. 挂断通话 (`hangup`)
*   **说明**：主动释放通话，清理所有信令和媒体端口。
*   **参数说明**：
    *   `reason_code` (整数): Q.850 挂断原因值。
    *   `sip_cause` (整数, 可选): SIP 挂断状态码（如 `486` Busy，`480` Temporarily Unavailable）。
*   **JSON 示例**：
```json
{
  "action": "hangup",
  "reason_code": 16,
  "sip_cause": 486
}
```

#### 5. 通话录音 (`record`)
*   **说明**：开启或停止对当前 Leg 通话的音频双向录制并输出为本地文件。
*   **参数说明**：
    *   `max_length_secs` (整数): 最大录制时长。
    *   `play_beep` (布尔): 开始录音前是否先在通话通道中播放一声 Beep 提示音。
*   **JSON 示例**：
```json
{
  "action": "record",
  "max_length_secs": 1800,
  "play_beep": true
}
```

#### 6. AI 语音实时流 (`stream`)
*   **说明**：将当前 Leg 的双向 RTP 媒体进行重采样，以低时延 WebSocket 传输协议推送到外部大语言模型。
*   **参数说明**：
    *   `websocket_url` (字符串): 外部 AI 接收端的 WebSocket 物理服务地址。
    *   `format` (字符串): 音频编码载荷类型，通常为 `"pcm16"` 或 `"raw"`。
    *   `barge_in` (布尔): 是否支持打断控制。
*   **JSON 示例**：
```json
{
  "action": "stream",
  "websocket_url": "ws://192.168.1.50:9000/realtime-ai",
  "format": "pcm16",
  "barge_in": true
}
```

#### 7. 文本转语音播音 (`say`)
*   **说明**：通过系统对接的 TTS 引擎播报指定文字内容。
*   **参数说明**：
    *   `text` (字符串): 播报文本内容。
    *   `voice` (字符串): TTS 角色/发音人音色标识。
*   **JSON 示例**：
```json
{
  "action": "say",
  "text": "您的验证码是 5 8 9 2，请尽快使用。",
  "voice": "zh-CN-XiaoxiaoNeural"
}
```

#### 8. 进入排队队列 (`queue`)
*   **说明**：将通话置入指定的呼叫中心技能等待队列中，等待座席应答。
*   **参数说明**：
    *   `queue_id` (字符串): 通话中心配置的座席队列名称。
    *   `moh_url` (字符串): 用户在排队等待时收听的背景音乐文件路径。
*   **JSON 示例**：
```json
{
  "action": "queue",
  "queue_id": "vip_service",
  "moh_url": "/opt/vos-rs/recordings/moh.wav"
}
```

#### 9. 加入多方会议 (`conference`)
*   **说明**：将当前 Leg 桥接进入媒体层的混音网桥房间，实现多方语音互动。
*   **参数说明**：
    *   `room_id` (字符串): 会议房间标识。
    *   `start_muted` (布尔): 成员刚加入会议时是否默认为静音状态。
*   **JSON 示例**：
```json
{
  "action": "conference",
  "room_id": "conf_8888",
  "start_muted": false
}
```

#### 10. 控制链重定向 (`redirect`)
*   **说明**：将当前通话后续所有的控制权及状态事件重定向至另一个第三方 Webhook URL 接口或另一个 NATS 主题。
*   **参数说明**：
    *   `url` (字符串): 目标重定向地址。如果是 `http://` 样式，会切换为 HTTP Webhook 控制；如果是 `nats://` 或 `vos_rs.` 样式，会以新主题继续进行 NATS 控制。
*   **JSON 示例**：
```json
{
  "action": "redirect",
  "url": "vos_rs.call.incoming.sales"
}
```

#### 11. 静默等待 (`pause`)
*   **说明**：静默等待指定时长，不播放任何声音并保持呼叫在线。
*   **参数说明**：
    *   `duration_ms` (整数): 静默的时长（毫秒）。
*   **JSON 示例**：
```json
{
  "action": "pause",
  "duration_ms": 5000
}
```

#### 12. 播送 DTMF 信号 (`play_digits`)
*   **说明**：向通话的对端反向模拟发送一段 DTMF 信号（常见于外呼后自动拨打分机）。
*   **参数说明**：
    *   `digits` (字符串): DTMF 按键序列。
*   **JSON 示例**：
```json
{
  "action": "play_digits",
  "digits": "9#"
}
```

#### 13. 主动发起外呼 (`originate`)
*   **说明**：指示 B2BUA 主动外呼单个 SIP 目的地，建立独立的单腿呼叫会话。主要用于双向回拨/双向外呼的第一步。
*   **参数说明**：
    *   `target_uri` (字符串): 目标 SIP URI（如 `sip:10001@127.0.0.1:5060`）。
    *   `caller_id` (字符串): 呼叫出局时显示的主叫号码。
*   **JSON 示例**：
```json
{
  "action": "originate",
  "target_uri": "sip:10001@127.0.0.1:5060",
  "caller_id": "10001"
}
```

#### 14. 媒体桥接 (`bridge`)
*   **说明**：在媒体层面将两个原本独立的单腿会话（通常为 originate 发起的 A-leg 与 B-leg）进行配对桥接，打通双方音频。
*   **参数说明**：
    *   `call_id_a` (字符串): 第一路单腿通话的 `call_id`。
    *   `call_id_b` (字符串): 第二路单腿通话的 `call_id`。
*   **JSON 示例**：
```json
{
  "action": "bridge",
  "call_id_a": "call_leg_a_12345",
  "call_id_b": "call_leg_b_12345"
}
```

---

## 4. 带外异步指令包装规范 (`CallCommand`)

如果外部控制器希望通过 `vos_rs.call.commands` 主题（Pub/Sub 模式）下发带外异步指令，数据格式必须将上述 `VciInstruction` 指令序列化，并外层封装 `call_id`。

**Pub/Sub 带外挂断指令示例**：
```json
{
  "call_id": "invite-vci-123456@example.com",
  "action": "hangup",
  "reason_code": 16,
  "sip_cause": 487
}
```

**Pub/Sub 带外实时放音指令示例**：
```json
{
  "call_id": "invite-vci-123456@example.com",
  "action": "play",
  "url": "/opt/vos-rs/announcements/emergency_broadcast.wav",
  "loop_count": 1
}
```

---

## 5. 双向回拨 (Click-to-Dial) 完整实现方案与时序

当控制器需要实现“让分机 10001 呼叫外线被叫 15300002222，并在此过程中替换早期媒体及进行双向立体声录音”时，完全可以通过 `originate`、`bridge` 指令以及 `leg` 事件属性实现。

### 5.1 详细指令流转时序

1. **第一步：平台呼叫分机 A-leg**
   - 控制器通过 NATS 主题 `vos_rs.call.commands` 发送 `originate` 异步命令呼叫分机 10001，指定唯一的 `call_id` 为 `call_leg_a_123`：
     ```json
     {
       "call_id": "call_leg_a_123",
       "action": "originate",
       "target_uri": "sip:10001@127.0.0.1:5060",
       "caller_id": "platform"
     }
     ```
   - `sip-edge` 接收到命令，主动向 10001 发送 `INVITE`，并投递 `call_originated` 事件。
   - 10001 接通后，`sip-edge` 触发上报 `call_answered` 事件，且带上 `"leg": "b_leg"`。

2. **第二步：平台呼叫被叫 B-leg**
   - 控制器收到 A-leg 接通事件后，通过 NATS 发送第二个 `originate` 异步命令呼叫外线被叫 15300002222，指定唯一的 `call_id` 为 `call_leg_b_123`：
     ```json
     {
       "call_id": "call_leg_b_123",
       "action": "originate",
       "target_uri": "sip:15300002222@gateway_ip:5060",
       "caller_id": "10001"
     }
     ```
   - **同时**，为了让分机 10001 听到替换后的早期媒体，控制器向 A-leg 发送 `play` 播音指令（如回铃等待音）：
     ```json
     {
       "call_id": "call_leg_a_123",
       "action": "play",
       "url": "/opt/vos-rs/recordings/custom_ringback.wav",
       "loop_count": 999
     }
     ```
   - **并且**，控制器向 A-leg 发送 `record` 指令开启早期媒体阶段的双轨混音录音（系统会自动将 A-leg 的主叫语音录入 WAV 的左声道，被叫 B-leg 接通后的应答/振铃音频录入 WAV 的右声道）：
     ```json
     {
       "call_id": "call_leg_a_123",
       "action": "record",
       "max_length_secs": 3600,
       "play_beep": false
     }
     ```

3. **第三步：被叫接通与媒体桥接**
   - B-leg (15300002222) 应答后，B2BUA 触发 `call_answered` 事件。
   - 控制器捕获到 B-leg 的接通事件，立刻发送 `bridge` 指令将 A-leg 和 B-leg 打通：
     ```json
     {
       "call_id": "call_bridge_action_123",
       "action": "bridge",
       "call_id_a": "call_leg_a_123",
       "call_id_b": "call_leg_b_123"
     }
     ```
   - 此时，`sip-edge` 停止 A-leg 的 `play` 自定义提示音，将两路单腿的媒体端点交叉配对。主被叫双方正常通话，同时由于 `record` 早已开启，整段通话的录音（包括替换的早期媒体阶段）都将被完美录入为左声道 A-leg，右声道 B-leg 的双轨音频文件中。

