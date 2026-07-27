# B2BUA Session ID 主键模型

> 本文档描述 vos-rs 中 B2BUA（Back-to-Back User Agent）的核心会话模型：
> **A/B-leg Call-ID → session_id → media_session** 的三级索引架构。

---

## 一、设计动机

### 1.1 传统 B2BUA 的痛点

在传统 B2BUA 实现中，媒体层通常直接使用 SIP Call-ID 作为会话标识。但这会带来以下问题：

| 问题 | 描述 |
|------|------|
| **Call-ID 漂移** | B2BUA 在出站 INVITE 时会改写 Call-ID（拓扑隐藏），导致 A-leg 与 B-leg 的 Call-ID 不同，媒体层无法用单一 Call-ID 关联两侧 RTP |
| **REFER 转接的第三腿** | Blind Transfer 会创建新的 transfer leg，其 Call-ID 与原 call/gateway leg 都不同，但媒体需要复用原 RTP 端口 |
| **Forking 的多腿问题** | 同一入站 INVITE 可能 fork 到多个网关，每个 fork 有独立的 Call-ID，但都共享同一个 caller leg 的媒体上下文 |
| **Call-ID 复用** | SIP 规范允许同一 Call-ID 在不同 dialog 中复用（不同的 From/To tag），不能作为唯一键 |

### 1.2 解决方案：session_id 主键

vos-rs 引入 **session_id** 作为 B2BUA 会话的唯一内部标识，所有媒体资源（RTP 端口、录音、DTMF 状态）均以 session_id 为键：

```text
A-leg Call-ID ─┐
               ├─> session_id ─> media_session
B-leg Call-ID ─┘
```

- **session_id**：在入站 INVITE 进入 `handle_invite_request` 时由 B2BUA 生成的 UUID（`uuid::Uuid::new_v4().simple()`）
- **A-leg Call-ID**：来自呼叫方的原始 INVITE Call-ID
- **B-leg Call-ID**：B2BUA 出站 INVITE 时生成的 Call-ID（拓扑隐藏）
- **media_session**：媒体层（`MediaRelayState`）维护的 RTP 中继/录音/DTMF 状态

---

## 二、核心数据结构

### 2.1 CallSessionStore（会话存储）

[`CallSessionStore`](../../services/sip-edge/src/edge_state/models.rs) 是 B2BUA 的核心会话表，使用 **双索引** 结构：

```rust
pub(crate) struct CallSessionStore {
    /// 主表：session_id → InboundTransaction
    sessions: DashMap<String, InboundTransaction>,
    /// 反向索引：Call-ID → session_id（支持 A/B/transfer/fork 任一 leg 查询）
    dialog_index: DashMap<String, String>,
}
```

**查询路径**：
1. 任何 SIP 消息进入 B2BUA 时，先提取 Call-ID
2. 通过 `session_id_for_dialog(call_id)` 查 `dialog_index` 得到 `session_id`
3. 再用 `session_id` 查 `sessions` 得到完整的 `InboundTransaction`

### 2.2 InboundTransaction（会话状态）

```rust
pub(crate) struct InboundTransaction {
    /// 内部会话唯一标识（UUID），媒体层主键
    pub(crate) session_id: String,
    /// B2BUA 双腿对话状态（caller = A-leg，gateway = B-leg）
    pub(crate) dialogs: B2buaDialogPair,
    /// REFER 转接创建的第三腿（可选）
    pub(crate) transfer_dialog: Option<TransferDialogState>,
    /// Fork 出的多个 B-leg（可选）
    pub(crate) fork_dialogs: HashMap<String, ForkDialogState>,
    /// 媒体端点（caller/gateway 两侧 RTP）
    pub(crate) caller_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_rtp: Option<RtpEndpoint>,
    pub(crate) caller_relay_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_relay_rtp: Option<RtpEndpoint>,
    // ... 其他状态：Session Timer / PRACK / 录音 / 计费
}
```

### 2.3 B2buaDialogPair（双腿对话）

```rust
pub(crate) struct B2buaDialogPair {
    /// A-leg：呼叫方（UAC）的对话状态
    pub(crate) caller: DialogLegState,
    /// B-leg：网关（UAS）的对话状态
    pub(crate) gateway: DialogLegState,
}
```

每个 `DialogLegState` 完整记录 RFC 4235 对话状态：Call-ID / From-Tag / To-Tag / CSeq / Route-Set / Remote-Target / peer 地址。

---

## 三、session_id 生命周期

### 3.1 会话诞生（INVITE 入站）

```text
UAC ── INVITE (Call-ID: A) ──> sip-edge
                                  │
                                  ▼
                    handle_invite_request()
                                  │
                    1. session_id = uuid::new_v4()
                    2. remember_inbound_invite(session_id, ...)
                       - dialogs.caller.call_id = A
                       - dialogs.gateway.call_id = "" (待绑定)
                       - dialog_index[A] = session_id
                    3. 媒体资源预占（caller RTP 端口）
```

**源文件**：[`services/sip-edge/src/edge_state/session.rs`](../../services/sip-edge/src/edge_state/session.rs) → `remember_inbound_invite()`

### 3.2 B-leg 绑定（出站 INVITE 响应）

```text
sip-edge ── INVITE (Call-ID: B) ──> Gateway
                                       │
sip-edge <── 200 OK ──────────────── Gateway
                  │
                  ▼
    bind_gateway_dialog(session_id, B, gateway_tag)
                  │
    1. dialogs.gateway.call_id = B
    2. dialogs.gateway.remote_tag = gateway_tag
    3. dialog_index[B] = session_id
    4. 记录网关媒体端点，配对 RTP 端口
```

**源文件**：[`services/sip-edge/src/edge_state/session.rs`](../../services/sip-edge/src/edge_state/session.rs) → `bind_gateway_dialog()`

### 3.3 媒体绑定（200 OK 处理）

```text
Gateway ── 200 OK (SDP) ──> sip-edge ── 200 OK (SDP) ──> UAC
                                │
                                ▼
              remember_gateway_media(call_id, ...)
                                │
              1. 通过 call_id 查 dialog_index → session_id
              2. media_relay.pair_ports(caller_port, gateway_port)
              3. media_relay.start_call_recording(session_id, ...)
                 ↑ 媒体层只识别 session_id，与 wire Call-ID 解耦
              4. call_manager.set_recording_path(CallId(A), path)
                 ↑ CDR 层仍用 A-leg Call-ID 对外暴露
```

**源文件**：[`services/sip-edge/src/edge_state/media.rs`](../../services/sip-edge/src/edge_state/media.rs) → `remember_gateway_media()`

### 3.4 REFER 转接（第三腿）

```text
原通话：A (caller) ←─ session_id ─→ B (gateway)
                                        │
                                        ▼ REFER (refer-to: C)
                              transfer_dialog.dialog.call_id = C
                              dialog_index[C] = session_id
                                        │
                                        ▼
                    新通话：A (caller) ←─ session_id ─→ C (transfer)
                    原通话：B (gateway) 被 BYE
```

**源文件**：[`services/sip-edge/src/sip/handlers/in_dialog/refer.rs`](../../services/sip-edge/src/sip/handlers/in_dialog/refer.rs)

### 3.5 Forking（多腿出站）

```text
UAC ── INVITE (Call-ID: A) ──> sip-edge
                                  │
                    ┌─────────────┼─────────────┐
                    ▼             ▼             ▼
              INVITE (B1)   INVITE (B2)   INVITE (B3)
                    │             │             │
                    └─ fork_dialogs[B1] ───────┘
                    └─ fork_dialogs[B2] ───────┘
                    └─ fork_dialogs[B3] ───────┘
                                  │
                  dialog_index[B1] = session_id
                  dialog_index[B2] = session_id
                  dialog_index[B3] = session_id
                  （任一 fork 响应都能定位到同一 session）
```

**源文件**：[`services/sip-edge/src/sip/handlers/invite/outbound/dispatch.rs`](../../services/sip-edge/src/sip/handlers/invite/outbound/dispatch.rs)

### 3.6 会话终结（BYE/CANCEL）

```text
任一 leg ── BYE ──> sip-edge
                       │
                       ▼
         teardown_call_transaction(call_id)
                       │
         1. 通过 call_id 查 dialog_index → session_id
         2. 移除 sessions[session_id]
         3. 清理 dialog_index 中所有指向该 session_id 的条目
         4. 释放媒体资源（RTP 端口、停止录音）
         5. 生成 CDR（使用 A-leg Call-ID 作为主键）
         6. 递减 caller 并发计数
```

**源文件**：[`services/sip-edge/src/edge_state/session.rs`](../../services/sip-edge/src/edge_state/session.rs) → `teardown_call_transaction()`

---

## 四、设计原则

### 4.1 媒体层与信令层解耦

| 层级 | 标识 | 说明 |
|------|------|------|
| SIP 信令层 | Call-ID | 对外暴露，用于 SIP 消息路由和 CDR |
| B2BUA 会话层 | session_id | 内部唯一键，关联 A/B/transfer/fork 所有腿 |
| 媒体层 | session_id | RTP 端口分配、录音文件命名、DTMF 状态均使用 session_id |

**好处**：媒体层不关心 Call-ID 的变化（拓扑隐藏、REFER 转接、Forking），只要 session_id 不变，媒体上下文就保持连续。

### 4.2 多态查询接口

`CallSessionStore` 提供 O(1) 的多 leg 查询：

```rust
// 通过任一 Call-ID（A/B/transfer/fork）获取会话
let txn = store.get(call_id)?;

// 自动解析：call_id → dialog_index → session_id → sessions
```

所有调用方无需关心自己处理的是哪条腿，统一通过 Call-ID 查询。

### 4.3 CDR 层兼容

CDR（话单）层仍使用 **A-leg Call-ID** 作为对外暴露的主键，与 SIP RFC 和运维习惯一致：

- `CallManager` 的 `CallId` 类型封装的是 A-leg Call-ID
- 录音文件路径通过 `call_manager.set_recording_path(CallId(A), path)` 反向通知
- CDR 表的 `call_id` 字段存储 A-leg Call-ID

---

## 五、模块文件结构

### 5.1 会话状态层（edge_state）

```text
services/sip-edge/src/edge_state/
├── mod.rs                  # EdgeState 结构体定义与模块导出
├── models.rs               # CallSessionStore / InboundTransaction / B2buaDialogPair
├── session.rs              # remember_inbound_invite / bind_gateway_dialog / teardown
├── media.rs                # remember_gateway_media / clear_media_targets
├── inbound_dialog.rs       # InboundTransaction 对话校验 impl
└── ...                     # 其他子模块（auth/billing/concurrency 等）
```

### 5.2 SIP 处理层（sip/handlers）

```text
services/sip-edge/src/sip/handlers/
├── invite/                 # INVITE 入站处理（session_id 生成点）
│   ├── mod.rs              #   主编排器 handle_invite_request
│   ├── conference.rs       #   会议 INVITE
│   ├── resolution.rs       #   呼叫源解析
│   ├── routing.rs          #   DID 路由 / 分机组分发
│   └── outbound/           #   出站 INVITE 构建（B-leg 创建）
│       ├── mod.rs          #     出站上下文
│       ├── balance.rs      #     余额校验
│       ├── lease.rs        #     资源租约
│       └── dispatch.rs     #     forking / 单腿分发
├── in_dialog/              # in-dialog 请求处理（BYE/CANCEL/INFO/REFER）
│   ├── mod.rs              #   主编排器
│   ├── bye.rs              #   BYE/CANCEL（会话终结）
│   ├── info.rs             #   INFO/PRACK
│   ├── refer.rs            #   REFER（第三腿创建）
│   └── reinvite.rs         #   re-INVITE/UPDATE
└── interactive_control/    # VCI 指令执行（使用 session_id 索引媒体层）
    ├── mod.rs              #   入口与派发器
    ├── webhook.rs          #   Webhook 拦截/事件投递
    └── instructions/       #   VCI 指令实现
        ├── mod.rs          #     Dial/Hangup/Redirect/Pause
        ├── media.rs        #     Play/Gather/Stream/Record/Say
        └── queue.rs        #     Queue/Conference
```

### 5.3 媒体层（media）

```text
services/sip-edge/src/media/
├── relay/                  # RTP 中继
│   ├── mod.rs              #   MediaRelayState（按 session_id 索引）
│   └── listener.rs         #   UDP 收发循环
├── recording.rs            # 录音（按 session_id 写入）
└── dtmf.rs                 # DTMF 状态（按 session_id 索引）
```

---

## 六、与 VOS-3000 的对比

| 维度 | VOS-3000 | vos-rs |
|------|----------|--------|
| 会话主键 | 内部 call_id（数字自增） | session_id（UUID） |
| Call-ID 处理 | 改写为内部 call_id | 保留原 Call-ID，通过 dialog_index 映射 |
| 媒体层标识 | 内部 call_id | session_id |
| REFER 转接 | 新建 call_id，迁移媒体 | 同一 session_id，新增 transfer_dialog |
| Forking | 独立 call_id | 同一 session_id，fork_dialogs HashMap |
| CDR 主键 | 内部 call_id | A-leg Call-ID（与 SIP RFC 一致） |

**vos-rs 的优势**：
- CDR 主键与 SIP RFC 一致，便于与第三方系统对接
- REFER/Forking 不重建会话，媒体无缝迁移
- session_id 全局唯一，便于分布式追踪

---

## 七、相关源文件索引

| 功能 | 源文件 |
|------|--------|
| 会话存储 | [`edge_state/models.rs`](../../services/sip-edge/src/edge_state/models.rs) |
| 会话创建/绑定/终结 | [`edge_state/session.rs`](../../services/sip-edge/src/edge_state/session.rs) |
| 媒体资源绑定 | [`edge_state/media.rs`](../../services/sip-edge/src/edge_state/media.rs) |
| INVITE 入站处理 | [`sip/handlers/invite/mod.rs`](../../services/sip-edge/src/sip/handlers/invite/mod.rs) |
| 出站 INVITE 分发 | [`sip/handlers/invite/outbound/dispatch.rs`](../../services/sip-edge/src/sip/handlers/invite/outbound/dispatch.rs) |
| BYE/CANCEL 处理 | [`sip/handlers/in_dialog/bye.rs`](../../services/sip-edge/src/sip/handlers/in_dialog/bye.rs) |
| REFER 转接 | [`sip/handlers/in_dialog/refer.rs`](../../services/sip-edge/src/sip/handlers/in_dialog/refer.rs) |
| 响应分发 | [`sip/dispatcher/response/`](../../services/sip-edge/src/sip/dispatcher/response/) |
| VCI 指令执行 | [`sip/handlers/interactive_control/`](../../services/sip-edge/src/sip/handlers/interactive_control/) |
| 媒体中继 | [`media/relay/`](../../services/sip-edge/src/media/relay/) |

---

*最后更新：2026-07-27（B2BUA 重构后）*
