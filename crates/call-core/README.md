# call-core

> **呼叫控制与路由引擎** — 决定一通电话「怎么走、走哪条线、扣多少钱」

## 这是什么？

`call-core` 是 vos-rs 平台的 **业务逻辑核心**。当一通电话进来，`call-core` 决定：
- 这通电话该转给谁？（路由）
- 走哪个中继线路？（LCR + 熔断）
- 通话过程中扣多少费用？（实时计费）
- 通话结束生成什么话单？（CDR）
- 客服队列怎么分配？（ACD）

## 核心能力

| 能力 | 说明 |
| :--- | :--- |
| **呼叫状态机** | INVITE → Ringing → Established → Terminated 完整生命周期 |
| **路由引擎** | 最长前缀匹配 + LCR 最低成本 + 加权负载均衡 + 时间窗口 |
| **网关熔断** | Circuit Breaker 模式，故障自动隔离 + 恢复探测 |
| **容量控制** | per-gateway 并发上限，防止过载 |
| **Failover** | 408/5xx 响应自动切换下一个候选网关 |
| **实时计费** | AtomicI64 CAS 余额扣减，通话中实时扣费 |
| **ACD 队列** | 呼叫中心座席分配（最长空闲 / 轮询 / 技能路由） |
| **CDR 生成** | 通话详单，含 MOS、RTCP 质量指标 |
| **主叫改写** | 严格透传 / 固定号码 / 虚拟号池三种模式 |

## 路由选择算法

```
1. 最长前缀匹配 (prefix length DESC)
2. 优先级排序 (priority DESC)
3. 最低成本 (cost ASC, LCR)
4. 同等条件加权随机 (weight DESC + random)
5. 健康状态过滤 (Circuit Breaker)
6. 容量检查 (max_capacity / max_concurrent)
7. 时间窗过滤 (time_start/time_end)
```

## 在项目中的位置

```
sip-core (信令解析) → sip-edge (B2BUA) → call-core (路由+计费+ACD) → cdr-core (落库)
```

`sip-edge` 接到 INVITE 后调用 `call-core` 做路由决策和计费扣费，通话结束生成 CDR 交给 `cdr-core` 持久化。

## 模块结构

| 模块 | 职责 |
| :--- | :--- |
| `call` | `Call` / `CallLeg` / `CallState` 状态机 |
| `manager` | `CallManager` 并发安全呼叫管理器（DashMap） |
| `routing` | 路由表 + 前缀匹配 + 健康追踪 + 容量控制 |
| `billing` | `AtomicBillingBucket` 实时计费桶 |
| `acd` | ACD 引擎、座席状态、分配策略 |
| `caller_identity` | 主叫号码改写策略 |
| `cdr` | CDR 数据结构 |
| `queue` | 呼叫队列 |
| `webhooks` | 事件 webhook 推送 |
| `outbound_policy` | 出局策略（直连 / 组路由） |

## 性能基准

```bash
cargo bench -p call-core
```

`benches/concurrency.rs` 含并发路由查找基准测试，目标 > 1000 CPS。

## 测试

```bash
cargo test -p call-core
```

含单元测试 + 集成测试（`tests/routing_manager.rs` / `tests/call_state.rs` / `tests/queue_tests.rs`）。
