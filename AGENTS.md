# AGENTS.md
# ===== AI Agent 项目指南 =====
# 本文件供 AI 编程助手（Cursor / Claude / Copilot / MiMo 等）阅读，
# 用于理解项目架构、约定和开发规范。AI 在参与本项目开发前应完整阅读。

---

## 1. 项目概述

- **项目名称**：vos-rs
- **项目类型**：后端服务（电信级 VoIP 软交换平台）
- **一句话描述**：用 Rust 编写的电信级软交换平台，对标商业 VOS-3000，目标单机 1700+ 并发通话 / 1000+ CPS
- **核心目标用户**：电信运营商、VoIP 服务商、企业内部通信系统
- **项目状态**：开发中（已有基础功能，实测 < 200 CPS，需系统性重构）
- **仓库地址**：https://github.com/your-org/vos-rs

---

## 2. 技术栈

### 2.1 语言与运行时

| 层级 | 技术 | 版本要求 |
|------|------|---------|
| 主语言 | Rust | >= 1.89 (edition 2021) |
| 异步运行时 | tokio (multi_thread) | =1.x |
| 前端 | TypeScript + React (Vite) | >= 5.3 |

### 2.2 核心依赖

| 依赖 | 用途 | 版本约束 |
|------|------|---------|
| tokio | 异步运行时 | =1.x |
| axum + tower-http | HTTP REST API | =0.7.x |
| sqlx (postgres) | 数据库访问 | =0.7.x |
| serde + serde_json | 序列化/反序列化 | =1.x |
| tracing + tracing-subscriber | 结构化日志 | =0.1.x |
| dashmap | 高性能并发 HashMap | =6.x |
| async-nats | NATS JetStream 消息队列 | 最新 |
| prometheus-client | 指标暴露 | 最新 |
| tokio-rustls + rustls | TLS 支持 | 最新 |
| stun | STUN 协议 NAT 穿透 | 最新 |
| thiserror | 库错误类型 | =1.x |
| anyhow | 应用错误处理 | =1.x |

### 2.3 基础设施

| 组件 | 技术 |
|------|------|
| 数据库 | PostgreSQL (主数据 + CDR) |
| 消息队列 | NATS JetStream (CDR 事件流) |
| 录音存储 | 本地 FS / 阿里云 OSS (双写) |
| 容器化 | Docker + Docker Compose |
| 前端 | React (Vite + nginx) |

---

## 3. 项目结构

```
vos-rs/
├── AGENTS.md                  # 本文件，AI 指南
├── Cargo.toml                 # Workspace 根 (11 members)
├── README.md                  # 项目说明
├── DEPLOY.md                  # 部署指南
├── WEB_GUIDE.md               # Web 管理界面指南
├── Makefile                   # 常用命令
├── Dockerfile
├── docker-compose.yml
├── .env / .env.production
│
├── crates/                    # 7 个协议/业务库 crate
│   ├── sip-core/              # SIP 消息解析 (零外部依赖)
│   │   └── src/               #   error, header, message, method, uri
│   ├── rtp-core/              # RTP/RTCP 协议 (轻量依赖：SRTP 加密原语)
│   │   └── src/               #   error, packet, payload, rtcp, telephone_event, srtp
│   ├── sdp-core/              # SDP 解析 (零外部依赖)
│   │   └── src/               #   error, session
│   ├── call-core/             # 呼叫控制 + 路由 + CDR 生成
│   │   └── src/               #   call, cdr, error, manager, routing
│   ├── cdr-core/              # CDR 存储 (PostgreSQL) + 数据模型
│   │   └── src/               #   models, schema, store, termination_models, termination_schema, utils
│   ├── storage-core/          # 录音存储抽象 (Local/OSS/Dual)
│   │   └── src/               #   config, local, oss
│   └── media-core/            # 媒体处理抽象层 (conference/recording/metrics/g711/dtmf)
│       └── src/               #   conference, recording, metrics, g711, dtmf
│
  ├── services/                  # 5 个服务二进制
  │   ├── sip-edge/              # SIP B2BUA 核心 (已拆分重构)
  │   │   └── src/               #   main.rs (入口，已重构瘦身), cdr, routing, rules, utils,
  │   │                          #   media, auth, dialog, transaction, outbound, registrar,
  │   │                          #   transport, sbc, anti_fraud, fork, manage, multimedia,
  │   │                          #   nats_cdr, stun_client, topology, transcode, turn,
  │   │                          #   upnp, tenant, subscribe
│   ├── api-server/            # REST API (Axum, 30+ 端点)
│   │   └── src/               #   main, recording, report, billing, calls, numbers,
│   │                          #   anti_fraud, metrics, import, llm_configs, copilot,
│   │                          #   resources/{ivr_menus,prompts,routes,numbers,users,...}
│   ├── cdr-worker/            # NATS CDR 消费者 (批量写 PostgreSQL)
│   │   └── src/               #   main.rs (单文件 392 行)
│   ├── media-edge/            # 独立媒体节点 (WebRTC/转码/eBPF)
│   └── sip-router/            # 分布式信令路由代理
│
├── web/                       # React 管理界面 (Vite + TypeScript, 14 页面)
├── docs/                      # 文档目录
│   ├── README.md              # 文档索引
│   ├── architecture/          # 架构与对比设计
│   │   ├── ARCHITECTURE.md
│   │   ├── B2BUA_SESSION_MODEL.md          # B2BUA session_id 主键模型
│   │   ├── VOS_RS_ARCHITECTURE_ANALYSIS.md
│   │   ├── rtp-sip-completeness.md
│   │   ├── NATS_VCI_COMMAND_DESIGN.md      # VCI 2.0 NATS 命令规范
│   │   ├── TRUNK_CALLER_TERMINATION_DESIGN.md
│   │   ├── TRUNK_FLOWCHART.md
│   │   └── VOS_RS_BUSINESS_GAPS_REQUIREMENTS.md
│   ├── deployment/            # 部署指南
│   │   ├── DEPLOY.md
│   │   ├── CLUSTER_DEPLOYMENT.md
│   │   └── OS_KERNEL_TUNING.md
│   ├── development/           # 开发与环境配置
│   │   ├── ENV_VARS.md
│   │   ├── AI_PLUGIN_INTEGRATION_GUIDE.md
│   │   ├── FRONTEND_OPTIMIZATION.md
│   │   ├── PERFORMANCE_BENCHMARK.md
│   │   ├── SIPP_BUSINESS_SCENARIOS.md
│   │   └── WEBHOOKS.md
│   └── user-guide/            # 用户操作指南
│       ├── WEB_GUIDE.md
│       └── ROUTING_TRUNK_GUIDE.md
├── scripts/                   # SQL 迁移 + 开发脚本
├── tools/                     # SIPp 测试工具
├── deploy/                    # 部署配置
├── recordings/                # 录音文件输出
└── logs/                      # 日志输出
```

---

## 4. 架构设计

### 4.1 分层架构（进程内模块划分）

```
┌─────────────────────────────────────────────────────────────┐
│                        vos-server (单进程)                    │
│                   Tokio Runtime (多线程异步)                   │
│                                                              │
│  ┌───────────┐ ┌────────────┐┌────────────┐ ┌────────────┐  │
│  │ SIP Layer │ │ Routing    ││ Billing    │ │ Media      │  │
│  │ (B2BUA)   │ │ Engine     ││ Engine     │ │ Controller │  │
│  │ UDP/TCP/  │ │ Prefix     ││ Realtime   │ │ RTP Relay  │  │
│  │ TLS/WS    │ │ Match/LCR  ││ Balance    │ │ Recording  │  │
│  │ Parser    │ │ Failover   ││ CDR Writer │ │ Transcode  │  │
│  │ Dialog    │ │ Rewrite    ││ Rating     │ │ DTMF       │  │
│  │ Transact  │ │            ││            │ │            │  │
│  └───────────┘ └────────────┘└────────────┘ └────────────┘  │
│                                                              │
│  ┌───────────┐ ┌────────────┐┌────────────┐ ┌────────────┐  │
│  │ Security  │ │ Trunk Mgr  ││ API Server │ │ Admin API  │  │
│  │ SBC/ACL   │ │ Health Chk ││ (Axum)     │ │ (Internal) │  │
│  │ Auth      │ │ Registratn ││ REST/WS    │ │            │  │
│  │ Anti-Fraud│ │            ││            │ │            │  │
│  └───────────┘ └────────────┘└────────────┘ └────────────┘  │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Shared State Layer                        │   │
│  │  DashMap (本地并发缓存) │ NATS (分布式) │ sqlx Pool  │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 4.2 信令与媒体分离原则

信令节点（轻量级，CPU 密集）与媒体节点（重量级，I/O + CPU 密集）应可独立扩展。

```
信令节点职责：              媒体节点职责：
  - SIP 解析/生成            - RTP/RTCP 收发
  - SIP 事务状态机           - NAT 穿透 (Symmetric RTP)
  - 路由引擎调用             - Codec 转码（可选）
  - 计费引擎调用             - DTMF 检测/转换
  - 媒体节点分配             - 录音（异步）
  - SDP 改写                 - 媒体质量统计
  - CDR 生成                 - Jitter Buffer
```

### 4.3 核心设计模式

- **依赖注入**：通过构造函数注入依赖，方便测试
- **Result 类型错误处理**：所有可恢复错误返回 Result，不使用 panic
- **Builder 模式**：复杂对象使用 Builder 构建
- **Repository 模式**：数据访问抽象化（cdr-core 提供 trait）

### 4.4 关键设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 数据库 ORM | sqlx（编译期检查） | 类型安全、性能好 |
| 错误处理 | thiserror (库) + anyhow (应用) | 分层错误处理 |
| 序列化 | serde + serde_json | Rust 生态标准 |
| 日志 | tracing + tracing-subscriber | 结构化、异步友好 |
| 并发数据结构 | DashMap (分片锁) | 高并发读多写少场景 |
| 消息队列 | NATS JetStream | CDR 事件流，批量写入 |
| SIP 解析 | 自研 (sip-core, 零外部依赖) | 轻量、可控 |

### 4.5 并发状态管理

| 层级 | 数据结构 | 锁类型 | 说明 |
|------|---------|--------|------|
| SIP Dialogs | CallSessionStore (DashMap<session_id, InboundTransaction> + DashMap<Call-ID, session_id>) | DashMap 分段锁 | 双索引：session_id 主键 + Call-ID 反向索引（A/B/transfer/fork 多腿） |
| RTP Sessions | DashMap<u16, SocketAddr> 等 6 个 DashMap | DashMap 分段锁 | per-port 状态 |
| Media Inner | Arc<Mutex<MediaRelayStateInner>> | std::sync::Mutex | 端口分配 |
| Trunk 状态 | Arc<Vec<Arc<TrunkState>>> | AtomicBool/AtomicU32 | 读多写少 |
| 路由表 | Arc<RwLock<RouteTable>> | std::sync::RwLock | 极少写 |
| CDR 缓存 | Mutex<Vec<CallCdr>> | std::sync::Mutex | 低频写 |
| SBC RateLimiter | DashMap<IpAddr, TokenBucket> | DashMap 分片锁 | 已改为 DashMap 分片并发令牌桶 |
| 录音写入 | tokio::sync::mpsc::Sender<RecordingChunk> | Tokio MPSC Channel | 已重构为 Tokio MPSC Channel + 独立 Task 磁盘 I/O 隔离 |
| Registration | tokio::sync::Mutex<RegistrationStore> | tokio::sync::Mutex | 异步感知 |

---

## 5. 编码规范

### 5.1 通用规则

1. **所有代码必须通过 linter 检查**，无 warning (`cargo clippy`)
2. **公共 API 必须有文档注释** (`///`)
3. **禁止 unwrap() 出现在生产代码中**（测试代码除外）
4. **函数长度不超过 50 行**，超出应拆分
5. **文件长度不超过 500 行**，超出应拆模块
6. **命名清晰，避免缩写**（`user` 不写成 `usr`，`response` 不写成 `resp`）
7. **禁止用 `#[allow(...)]` 属性压制警告**（详见 5.2 节）

### 5.2 `#[allow(...)]` 属性使用规范

本项目坚持"零警告"原则，**禁止使用 `#[allow(dead_code)]` / `#[allow(unused_*]` 等属性压制编译器/clippy 警告**。出现警告必须从根源解决，而非掩盖。

#### 5.2.1 禁止使用的 allow 属性

| 属性 | 禁止原因 | 正确处理方式 |
|------|---------|-------------|
| `#[allow(dead_code)]` | 掩盖未使用代码，让死代码长期堆积 | 删除代码，或接入调用路径，或标注 `#[cfg(test)]` |
| `#[allow(unused_variables)]` | 掩盖未使用变量，通常意味着逻辑遗漏 | 使用变量（加 `_` 前缀仅限有意忽略），或删除变量 |
| `#[allow(unused_imports)]` | 掩盖未使用导入，污染命名空间 | 删除未使用的 `use` 语句 |
| `#[allow(unused_mut)]` | 掩盖不必要的可变性 | 移除 `mut` 关键字 |
| `#[allow(unused_must_use)]` | 掩盖被忽略的 `Result`/`#[must_use]` | 处理返回值（`?` 传播或 `let _ = ...` 显式忽略并注释原因） |
| `#[allow(clippy::all)]` | 一刀切禁用 clippy | 针对性解决具体 lint |

#### 5.2.2 唯一允许的例外场景

仅以下两种场景可以使用 `#[allow(...)]`，且必须在属性上方添加注释说明理由：

```rust
// 例外一：测试专用辅助方法，仅在 #[cfg(test)] 构建中调用
#[cfg(test)]
pub(crate) fn record_rtcp_reports_for_test(&self, port: u16, snapshot: RtcpQualitySnapshot) {
    // ...
}

// 例外二：第三方依赖生成的代码或 procedural macro 展开产物，且无法修改
// 此场景在本项目中极为罕见，需在 PR 中明确说明
```

> **注意**：`#[cfg(test)]` 不是 `#[allow]`，它是条件编译门控，是推荐做法。

#### 5.2.3 处理流程

当 `cargo clippy` 或 `cargo check` 报告 unused/dead_code 警告时，按以下顺序处理：

1. **优先删除**：确认代码确实无用途 → 直接删除（不要保留"以后可能用到"的代码）
2. **接入调用路径**：代码有业务价值但未接入 → 实现 HTTP 端点 / 模块导出 / 测试用例，使其被引用
3. **测试门控**：方法仅服务于单元测试 → 添加 `#[cfg(test)]` 限定，并在测试中实际调用
4. **有意忽略**：变量确实不需要使用（如实现 trait 的签名要求）→ 用 `_` 前缀命名（`_unused`），并添加行内注释

```rust
// ✅ 正确：trait 实现签名要求该参数，但本实现不需要
fn handle(&self, _context: &Context) -> Result<(), Error> {
    // context 在本实现中未使用，但 trait 签名要求保留
    Ok(())
}

// ❌ 错误：用 allow 掩盖
#[allow(unused_variables)]
fn handle(&self, context: &Context) -> Result<(), Error> {
    Ok(())
}
```

#### 5.2.4 前端（TypeScript/React）等价规范

前端同样禁止用 `// @ts-ignore` / `// eslint-disable-*` / `/* eslint-disable */` 压制警告：

- `@ts-ignore` → 修正类型定义或使用类型断言（`as`）并注释原因
- `eslint-disable` → 重构代码满足 lint 规则
- `no-explicit-any` → 定义具体类型，禁止 `any`（必要时用 `unknown` + 类型守卫）

唯一例外：第三方库类型定义缺陷，且已在注释中说明库版本与缺陷来源。

#### 5.2.5 自检清单（提交前必查）

- [ ] `cargo clippy --workspace -- -D warnings` 零警告
- [ ] `cargo check --workspace` 零警告
- [ ] 全局搜索 `#[allow(` 确认无新增违规（`grep -rn "#\[allow(" --include="*.rs" .`）
- [ ] 前端 `tsc --noEmit` 零警告
- [ ] 前端无 `@ts-ignore` / `eslint-disable` 新增

### 5.3 命名约定

| 元素 | 风格 | 示例 |
|------|------|------|
| 变量/函数 | snake_case | `get_user_by_id` |
| 类型/结构体 | PascalCase | `UserService` |
| 常量 | SCREAMING_SNAKE | `MAX_RETRY_COUNT` |
| 文件名 | snake_case | `user_repository.rs` |
| 模块名 | snake_case | `user_service` |
| 数据库表名 | snake_case, 复数 | `sip_users` |
| API 路径 | kebab-case | `/api/user-accounts` |
| 环境变量 | SCREAMING_SNAKE, `VOS_RS_` 前缀 | `VOS_RS_DATABASE_URL` |
| SIP 常量 | SCREAMING_SNAKE | `DEFAULT_RTP_PORT_MIN` |

### 5.4 错误处理规范

```rust
// ✅ 正确：使用自定义错误类型
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("用户不存在: {0}")]
    UserNotFound(i64),
    #[error("数据库错误: {0}")]
    Database(#[from] sqlx::Error),
    #[error("参数无效: {0}")]
    Validation(String),
}

// ✅ 正确：使用 ? 传播错误
async fn get_user(id: i64) -> Result<User, AppError> {
    let user = repo.find_by_id(id).await?;
    user.ok_or(AppError::UserNotFound(id))
}

// ❌ 错误：使用 unwrap 或 expect
async fn bad_example(id: i64) -> User {
    repo.find_by_id(id).await.unwrap()  // 禁止！
}
```

### 5.5 异步编程规范

```rust
// ✅ 正确：异步函数中避免持有锁跨越 .await
async fn process(&self) -> Result<(), Error> {
    let data = {
        let guard = self.state.lock().await;
        guard.get_data().clone()  // 先克隆数据，释放锁
    };  // 锁在此释放
    self.do_async_work(data).await
}

// ❌ 错误：持锁跨 await（死锁风险）
async fn bad_process(&self) -> Result<(), Error> {
    let mut guard = self.state.lock().await;
    self.do_async_work(&guard).await;  // 危险！
    Ok(())
}
```

**特别注意**：录音模块 (`media.rs`) 已重构为 Tokio MPSC Channel + 独立 Task 磁盘 I/O 隔离方案，不再使用 `std::sync::Mutex` + 同步 I/O。

### 5.6 注释规范

```rust
/// 从数据库中查找用户
///
/// # Arguments
/// * `id` - 用户唯一标识
///
/// # Returns
/// * `Ok(Some(user))` - 找到用户
/// * `Ok(None)` - 用户不存在
pub async fn find_user(&self, id: i64) -> Result<Option<User>, AppError> {
    // ...
}
```

- **公共 API**：必须有文档注释（`///`）
- **复杂逻辑**：必须有行内注释说明 why，而非 what
- **简单代码**：不需要注释，代码应自解释
- **TODO/FIXME**：格式为 `// TODO(author): 描述，包含关联的 issue 编号`

### 5.7 大文件拆分规范

#### 触发条件（满足任一即应拆分）

- 单文件超过 300 行（含空行和注释）
- 单个函数超过 50 行
- 单文件包含 3 个以上不相关的职责
- 单个 `mod` 内子模块超过 5 个，且层级清晰可分组

#### 拆分前的分析步骤

在动手拆分之前，AI 必须先完成以下分析，并将方案告知用户确认后再执行：

1. **梳理依赖关系**：画出文件内各类型 / 函数之间的调用图，识别强耦合簇
2. **识别职责边界**：按"这个函数 / 类型属于哪个领域概念"归类，而非按"功能相似"归类
3. **确定拆分粒度**：每个新模块应有且只有一个明确职责（单一职责原则）
4. **评估影响范围**：列出所有会受影响的 `use` 语句、测试、和外部调用点
5. **给出拆分方案**：以列表形式呈现"从哪拆、拆到哪、拆后结构是什么"

#### 拆分方案模板

AI 在执行拆分前，必须先输出以下格式的方案：

```
## 拆分方案：[文件路径]

### 当前状态
- 文件总行数：XXX 行
- 包含的类型/函数/impl 块：列举

### 拆分后结构
src/domain/
├── mod.rs                    # 重新导出，XX 行
├── user.rs                   # User + UserError，XX 行
├── user/
│   ├── mod.rs
│   ├── model.rs              # User 结构体 + 简单方法，XX 行
│   ├── service.rs            # UserService 业务逻辑，XX 行
│   └── repository.rs         # UserRepository trait + impl，XX 行
├── billing.rs                # Billing 相关，XX 行
└── ...

### 迁移清单
| 原位置                          | 迁移目标                      | 类型/函数名         |
|-------------------------------|------------------------------|---------------------|
| domain/mod.rs L45-120         | domain/user/model.rs         | struct User         |
| domain/mod.rs L121-200        | domain/user/service.rs       | impl UserService    |
| ...                           | ...                          | ...                 |

### 影响范围
- 需更新的 use 语句：[文件列表]
- 需更新的测试：[文件列表]
- 外部 crate 的 pub 接口是否变化：是/否，说明

### 风险点
- [例如：User 和 Invoice 之间有循环引用，需要提取公共类型到 shared.rs]
```

**必须等用户确认方案后，再执行拆分操作。**

#### Rust 拆分策略

**策略一：扁平拆分**（模块职责 < 5 个相关类型）

```
拆分前：src/service.rs (400 行，包含 UserService + EmailService + CacheService)

拆分后：
src/service/
├── mod.rs       # pub use 重新导出
├── user.rs      # UserService
├── email.rs     # EmailService
└── cache.rs     # CacheService
```

**策略二：按层级拆分**（类型定义 + 逻辑 + 持久化分离）

```
拆分前：src/domain/user.rs (600 行)

拆分后：
src/domain/user/
├── mod.rs           # 重新导出
├── model.rs         # struct User, UserStatus
├── service.rs       # UserService（业务逻辑）
├── repository.rs    # UserRepository trait + 实现
├── error.rs         # UserError 枚举
└── tests.rs         # 所有测试集中存放
```

**策略三：提取公共类型**（解决循环依赖）

```
问题：user.rs 引用 billing.rs，billing.rs 也引用 user.rs

解决：
src/domain/
├── shared.rs    # 提取公共类型（UserId, Money, Timestamp 等值对象）
├── user.rs      # use shared::UserId;
├── billing.rs   # use shared::{UserId, Money};
```

#### 拆分执行规则

1. **一次只拆一个文件**：拆分完成后确认编译通过 + 测试全绿，再拆下一个
2. **保持公共接口不变**：通过 `pub use` 重新导出，确保外部 `use` 语句不需要修改
3. **不修改任何业务逻辑**：拆分是纯重构行为，禁止"顺便改个 bug""顺手优化一下算法"
4. **每个步骤后验证**：`cargo check` → `cargo test` → `cargo clippy`
5. **保持 git 历史可追溯**：如果可能，使用 `git mv` 而非删除重建

#### 拆分后的文件行数参考

| 文件类型 | 建议行数上限 | 说明 |
|---------|------------|------|
| 纯数据模型（struct + 简单 impl） | 200 行 | 超出通常意味着职责过多 |
| 业务逻辑（Service） | 300 行 | 超出应检查是否混入了多个领域概念 |
| Repository / 持久化 | 250 行 | 每张主要表的方法通常 50-80 行 |
| 错误定义 | 100 行 | 超出说明错误类型可能需要按模块拆分 |
| 测试文件 | 无硬限制 | 但单个测试函数不超过 30 行 |
| 工具函数 | 150 行 | 超出通常意味着混入了不相关的工具 |

#### 禁止事项

- 禁止一次性拆分多个文件
- 禁止拆分过程中修改业务逻辑或重构算法
- 禁止创建超过 4 层的模块嵌套（`a::b::c::d` 是上限）
- 禁止仅因文件行数多就拆分——行数多只是信号，真正原因是职责不单一
- 禁止创建空的"桶文件"（mod.rs 只有 `pub use` 但子模块只有一个）
- 禁止拆分后丢失文档注释或 TODO 标记

### 5.8 前端页面统一规范

所有 Web 管理界面页面必须遵循以下统一规范，确保视觉一致、交互统一、全中文呈现。

#### 5.8.1 语言规范（全中文）

| 元素 | 规则 | 示例 |
|------|------|------|
| 页面标题 | 纯中文，4 个字 | `实时控制`、`运行总览`、`租户管理` |
| 侧边栏菜单 | 纯中文，4 个字 | `号码分机`、`系统安全`、`计费账务` |
| 分组标签 | 纯中文，4 个字 | `运行中心`、`号码分机`、`系统安全` |
| 按钮/操作 | 纯中文，使用图标代替文字 | `<Edit />` 图标，而非"编辑"文字 |
| 状态标签 | 纯中文 | `已接通`、`响铃中`、`已结束` |
| KPI 指标 | 纯中文标签 + 中文单位 | `并发通话 12 路`、`链路延迟 45 毫秒` |
| 技术术语 | 翻译为中文，不在界面显示英文原名 | `实时双工通道`（非 `WebSocket`）、`语音合成`（非 `TTS`） |

**禁止在用户可见界面出现以下英文**：
- 产品/功能英文原名：`RWI`、`WebSocket`、`TTS`、`BargeIn`、`Transfer`、`Hangup`、`Live`、`Max`
- 技术缩写：`WS`、`CPS`（改为"每秒呼叫"）、`PSTN`（改为"公用电话网"）
- 版本标识：`v2.5 - Live`（改为中文"实时"）

**允许保留英文的场景**（仅限代码内部，不显示给用户）：
- 变量名、函数名、类型名
- 代码注释
- 日志输出

#### 5.8.2 视觉规范（HeroUI 主题）

| 元素 | 规范 |
|------|------|
| 主题色 | HeroUI primary（蓝色），禁止使用紫色主题 |
| 语义类 | 使用 `text-primary`、`bg-content1`、`text-foreground` 等语义类，禁止硬编码 hex 值 |
| 状态色 | `success`（绿色/正常）、`warning`（黄色/警告）、`danger`（红色/危险）、`primary`（蓝色/主操作） |
| 卡片 | `bg-content1 border border-default-200 rounded-2xl shadow-sm` |
| 按钮图标 | 表格内操作按钮使用 `isIconOnly` + 图标，不显示中文字符 |

#### 5.8.3 页面结构规范

每个资源管理页面必须使用统一的 `ResourceWorkspace` 组件框架：

```
页面顶部：标题 + 描述 + 新建按钮
页面中部：数据表格（支持搜索、筛选、分页）
页面详情：多标签页（基本配置 / 关联关系 / 状态信息）
```

特殊页面（如实时控制台）允许自定义布局，但必须：
1. 顶部状态栏：标题 + 实时连接状态 + KPI 统计
2. 主工作区：左栏列表 + 右栏详情（12 栅格布局）
3. 底部弹窗：操作确认/输入弹窗

#### 5.8.4 自检清单（提交前必查）

- [ ] 所有用户可见文本均为中文，无英文缩写或原名
- [ ] 菜单项和分组标签均为 4 个字
- [ ] 表格操作按钮使用图标，无中文字符
- [ ] 颜色使用语义类，无硬编码 hex 值
- [ ] 使用 HeroUI primary 蓝色主题，无紫色
- [ ] 主题设置可跨会话持久化
- [ ] `tsc --noEmit` 零警告
- [ ] `npm run lint` 零警告
- [ ] 无 `@ts-ignore` / `eslint-disable` 新增

---

## 6. 测试规范

### 6.1 测试金字塔

```
         ╱╲
        ╱  ╲        E2E 测试（少量，SIPp 场景）
       ╱────╲
      ╱      ╲      集成测试（适量，模块协作）
     ╱────────╲
    ╱          ╲    单元测试（大量，核心逻辑）
   ╱────────────╲
```

### 6.2 测试命名与组织

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 测试函数命名：test_[行为]_[条件]_[期望结果]
    #[tokio::test]
    async fn test_create_user_with_valid_input_returns_user() {
        // Arrange（准备）
        // Act（执行）
        // Assert（断言）
    }
}
```

### 6.3 测试要求

| 类别 | 要求 |
|------|------|
| 协议解析 | sip-core / rtp-core / sdp-core 覆盖率 >= 90% |
| 业务逻辑 | call-core 路由 + 状态机覆盖率 >= 90% |
| API 层 | 每个 endpoint 至少 1 个集成测试 |
| 边界条件 | 空值、超长输入、并发、超时 |
| 性能 | call-core 已有 criterion bench (`benches/concurrency.rs`) |
| SIPp | `tools/sipp/` 下有端到端 SIP 场景测试 |

### 6.4 Mock 规范

- 外部依赖通过 trait 抽象，测试中用 mock 实现
- 不 mock 内部实现细节，只 mock 外部边界（数据库、HTTP 调用）
- 集成测试使用 docker-compose 启动真实依赖

---

## 7. Git 工作流

### 7.1 分支策略

```
main ───────────────────────────────────── (生产)
  │
  ├── develop ──────────────────────────── (开发主线)
  │     │
  │     ├── feat/user-auth ─────────────── (功能分支)
  │     ├── feat/payment ───────────────── (功能分支)
  │     └── fix/login-timeout ──────────── (修复分支)
  │
  └── release/v1.2.0 ──────────────────── (发布分支)
```

### 7.2 Commit 规范

格式：`<type>(<scope>): <description>`

```
feat(auth): 添加 JWT 刷新令牌机制
fix(billing): 修复并发余额扣减竞态条件
refactor(rtp): 提取 RTP 解析为独立模块
perf(rtp): RTP 收发引入 buffer pool
refactor(sip-edge): main.rs 拆分为多个子模块
fix(media): 录音 sync I/O 改为 async channel-based
```

**type 类型**：`feat` / `fix` / `refactor` / `docs` / `test` / `perf` / `chore` / `ci`

**scope 范围**：`sip-core` / `rtp-core` / `sdp-core` / `call-core` / `cdr-core` / `sip-edge` / `api-server` / `cdr-worker` / `media` / `routing` / `billing` / `auth` / `sbc`

### 7.3 Pull Request 规则

- 标题与 commit 格式一致
- 必须关联 issue（`Closes #123`）
- 必须通过 CI（`cargo clippy` + `cargo test` + `cargo build`）
- 至少 1 人 review
- 不超过 500 行变更（大 PR 应拆分）

---

## 8. API 设计规范

### 8.1 RESTful 约定

```
GET    /api/v1/users          # 列表（支持分页）
POST   /api/v1/users          # 创建
GET    /api/v1/users/:id      # 详情
PUT    /api/v1/users/:id      # 全量更新
PATCH  /api/v1/users/:id      # 部分更新
DELETE /api/v1/users/:id      # 删除
```

### 8.2 统一响应格式

```json
{
  "code": 0,
  "message": "success",
  "data": { },
  "timestamp": 1720000000,
  "request_id": "req_xxxxxxxxxxxx"
}
```

错误响应：

```json
{
  "code": 40001,
  "message": "用户不存在",
  "details": "User with id 42 not found",
  "timestamp": 1720000000,
  "request_id": "req_xxxxxxxxxxxx"
}
```

### 8.3 分页约定

```
GET /api/v1/users?page=1&page_size=20&sort_by=created_at&order=desc

Response:
{
  "data": [...],
  "pagination": {
    "page": 1,
    "page_size": 20,
    "total": 156,
    "total_pages": 8
  }
}
```

### 8.4 现有 API 端点 (api-server)

| 路径 | 方法 | 说明 |
|------|------|------|
| `/api/v1/cdr` | GET | CDR 查询 |
| `/api/v1/dashboard/stats` | GET | 仪表盘统计 |
| `/api/v1/active-calls` | GET | 当前通话列表 |
| `/api/v1/users` | CRUD | SIP 用户管理 |
| `/api/v1/gateways` | CRUD | 网关管理 |
| `/api/v1/routes` | CRUD | 路由管理 |
| `/api/v1/numbers` | CRUD | 号码管理 |
| `/api/v1/rates` | CRUD | 费率管理 |
| `/api/v1/billing/accounts` | CRUD | 计费账户 |
| `/api/v1/recordings` | GET | 录音查询 |
| `/api/v1/registrations` | GET | 注册状态 |
| `/api/v1/anti-fraud/rules` | CRUD | 反欺诈规则 |
| `/metrics` | GET | Prometheus 指标 |

### 8.5 管理 API (sip-edge 内置)

| 路径 | 方法 | 说明 |
|------|------|------|
| `/manage/active-calls` | GET | 当前通话列表 |
| `/manage/calls/:call_id/terminate` | POST | 强制断开通话 |
| `/manage/route-preview` | GET | 路由试算 |

#### IVR 音频提示音 (api-server)

| 路径 | 方法 | 说明 |
|------|------|------|
| `/api/v1/ivr/prompts` | GET | 列出已上传音频文件 |
| `/api/v1/ivr/prompts/upload` | POST | 上传音频文件 (multipart, 字段 `file`) |
| `/api/v1/ivr/prompts/:filename` | GET | 下载/试听音频 (内联, 支持 `<audio>`) |
| `/api/v1/ivr/prompts/:filename` | DELETE | 删除指定音频文件 |

> 音频文件落盘到 `VOS_RS_PROMPTS_DIR` (默认 `./prompts`) 目录, 支持 wav/mp3/gsm/ogg, 单文件最大 50MB。

---

## 9. 安全规范

### 9.1 绝对禁止

- 硬编码密码、API Key、Token 等敏感信息到代码中
- 在日志中输出用户密码、Token、信用卡号等 PII
- 使用 `eval()` 或不安全的反序列化
- SQL 拼接（必须使用参数化查询，sqlx 已内置）
- 禁用 TLS 证书验证（除非 `VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY=true` 仅限测试）

### 9.2 必须执行

- 所有用户输入必须校验和清洗
- SIP Digest Auth 必须启用（`VOS_RS_AUTH_ENABLED=true`）
- API 接口必须有鉴权中间件
- 敏感操作必须有审计日志
- 依赖定期安全扫描（`cargo audit`）

### 9.3 本项目已有安全机制

| 机制 | 实现文件 | 说明 |
|------|---------|------|
| SIP Digest Auth | `sip-edge/src/auth.rs` | MD5 Digest + 动态 nonce + 重放防护 |
| IP ACL (SBC) | `sip-edge/src/sbc.rs` | Allowlist/Blocklist + CIDR |
| Token Bucket 限速 | `sip-edge/src/sbc.rs` | 每 IP 令牌桶 |
| 反欺诈引擎 | `sip-edge/src/anti_fraud.rs` | 并发限制、CPS 限制、号码黑白名单 |
| 拓扑隐藏 | `sip-edge/src/topology.rs` | Contact/Via 头改写 |
| TLS 支持 | `sip-edge/src/transport.rs` | tokio-rustls + 自定义证书验证 |

---

## 10. 性能规范

### 10.1 关键指标

| 指标 | 当前 | 目标 |
|------|------|------|
| CPS (calls per second) | < 200 | >= 1000 |
| 并发通话 | 未知 | >= 1700 |
| API P99 延迟 | 未知 | < 100ms |
| 数据库查询 P99 | 未知 | < 50ms |
| 内存使用 | 未知 | 稳态无泄漏 |
| 启动时间 | 未知 | < 5s |

### 10.2 性能红线

- **禁止在热路径上分配大对象或频繁堆分配**（RTP 收发 loop、SIP 解析）
- **禁止 N+1 查询**（使用 JOIN 或批量查询）
- **禁止在 async 上下文中使用 std::sync::Mutex + 同步 I/O**
- 数据库必须有索引覆盖常用查询
- 大批量操作必须分批处理（batch size <= 1000）
- 缓存必须设置 TTL，防止内存膨胀

### 10.3 当前性能瓶颈（已审计确认）

| 瓶颈 | 严重级别 | 位置 | 说明 |
|------|---------|------|------|
| 录音 sync I/O | [已优化] | `media.rs` | 已重构为 Tokio MPSC Channel + 独立 Task 磁盘 I/O 隔离 |
| SBC RateLimiter 单 Mutex | [已优化] | `sbc.rs` | 已改为 DashMap 分片并发令牌桶 |
| RTP 每包 6-8 次 DashMap 锁 | 🟡 中 | `media.rs:1403-1476` | 高 pps 下 cache line bouncing |
| RTP 解析每包 Vec alloc | [已优化] | `rtp-core/packet.rs:85,115` | 已下沉并引入有界 BufferPool 机制 |
| SIP 解析非零拷贝 | 🟡 中 | `sip-core/message.rs:62` | String::from_utf8_lossy + .to_string() |
| main.rs 9401 行 | [已完成] | `sip-edge/main.rs` | **已重构拆分**为 20+ 子模块（当前 540 行） |
| `api-server` 大文件 | [已完成] | `api-server/src/` | 拆分至 65 个 .rs 文件，其中 `system/system.rs` (560)、`copilot/mod.rs` (525)、`main.rs` (521) 略超 500 行，待持续优化 |

---

## 11. AI 辅助开发规范

### 11.1 AI 生成代码的要求

- AI 生成的代码必须通过完整 CI 流水线（`cargo clippy` + `cargo test` + `cargo build`）
- AI 生成的代码必须有人工 review
- AI 不得引入未经审批的新依赖
- AI 生成的代码必须符合本文件中定义的所有编码规范

### 11.2 AI 提交代码前的自检清单

- [ ] 代码通过 `cargo clippy`，无 warning
- [ ] 公共函数有文档注释 (`///`)
- [ ] 有对应的单元测试
- [ ] 错误处理完整，无 `unwrap()` / `expect()`
- [ ] 没有硬编码的魔法数字（使用常量）
- [ ] 没有引入安全漏洞（SQL 注入、信息泄露等）
- [ ] 没有性能退化（N+1、锁竞争、内存泄漏）
- [ ] 符合项目的分层架构依赖规则
- [ ] 没有在 async 上下文中使用 `std::sync::Mutex` + 同步 I/O
- [ ] RTP 收发热路径上没有堆分配

### 11.3 AI 上下文说明

当 AI 阅读本项目代码时，请注意：

1. **入口点**：`services/sip-edge/src/main.rs`（已进行子模块拆分瘦身）
2. **协议解析层**：`crates/sip-core/`、`crates/sdp-core/`（零外部依赖）、`crates/rtp-core/`（轻量依赖：SRTP 加密原语）
3. **业务逻辑层**：`crates/call-core/`（呼叫状态机、路由、CDR）
4. **数据存储层**：`crates/cdr-core/`（PostgreSQL CRUD + 数据模型）
5. **媒体处理**：`services/sip-edge/src/media/`（RTP relay + 录音 + DTMF，按 session_id 索引）
6. **安全模块**：`services/sip-edge/src/sbc.rs`、`services/sip-edge/src/sip/auth.rs`、`services/sip-edge/src/security/`
7. **环境变量**：所有配置通过 `VOS_RS_` 前缀环境变量，见 `.env.example`
8. **测试目录**：`crates/*/tests/`（集成测试）、各模块内 `#[cfg(test)]`（单元测试）
9. **SIPp 测试**：`tools/sipp/`（端到端 SIP 场景）
10. **架构文档**：`docs/VOS_RS_ARCHITECTURE_ANALYSIS.md`（详细架构分析）
11. **api-server 拆分**：已全量拆分至 copilot/resources/billing/system/cluster/termination/v1 等子目录，65 个 .rs 文件，其中 `system/system.rs` (560)、`copilot/mod.rs` (525)、`main.rs` (521) 3 个文件略超 500 行，待持续优化

---

## 12. 常用命令

```bash
# === 开发 ===
make setup                  # 初始化开发环境
make dev                    # 启动开发服务器
make test                   # 运行所有测试
make test-unit              # 仅单元测试
make test-integration       # 仅集成测试

# === 代码质量 ===
cargo clippy --workspace -- -D warnings   # Lint 检查
cargo fmt --check                          # 格式化检查
cargo check --workspace                    # 类型检查

# === 构建 ===
cargo build --workspace                    # 开发构建
cargo build --workspace --release          # 生产构建
make docker-build                          # Docker 镜像构建

# === 测试 ===
cargo test --workspace                     # 全量测试
cargo bench -p call-core                   # 性能基准测试
cd tools/sipp && ./run_all.sh              # SIPp 端到端测试

# === 数据库 ===
make db-migrate                            # 执行数据库迁移
make db-rollback                           # 回滚上一次迁移
make db-seed                               # 填充测试数据

# === 其他 ===
cargo audit                                # 安全审计
cargo doc --workspace --open               # 生成文档
```

---

## 13. 环境变量

```bash
# === 必需 ===
VOS_RS_DATABASE_URL=postgres://user:pass@localhost:5432/vosrs
VOS_RS_NATS_URL=nats://localhost:4222

# === SIP 配置 ===
VOS_RS_SIP_BIND=0.0.0.0:5060              # SIP 监听地址
VOS_RS_SIP_ADVERTISED_ADDR=1.2.3.4:5060   # 对外通告地址
VOS_RS_SIP_DEFAULT_GATEWAY=10.0.0.1       # 默认网关
VOS_RS_SIP_TLS_BIND=0.0.0.0:5061          # TLS 监听 (可选)
VOS_RS_SIP_TLS_CERT_PATH=/path/cert.pem   # TLS 证书 (可选)
VOS_RS_SIP_TLS_KEY_PATH=/path/key.pem     # TLS 私钥 (可选)

# === RTP 媒体 ===
VOS_RS_RTP_ADVERTISED_ADDR=1.2.3.4        # RTP 对外地址
VOS_RS_RTP_PORT_MIN=40000                  # RTP 端口范围起始
VOS_RS_RTP_PORT_MAX=40100                  # RTP 端口范围结束
VOS_RS_RTP_SYMMETRIC_LEARNING=true         # 对称 RTP 学习

# === 录音 ===
VOS_RS_RECORDING_ENABLED=false             # 录音开关
VOS_RS_RECORDING_DIR=target/recordings     # 录音目录

# === 认证 ===
VOS_RS_AUTH_ENABLED=true                   # SIP Digest Auth
VOS_RS_AUTH_REALM=vos-rs                   # Digest Auth Realm

# === SBC 安全 ===
VOS_RS_SBC_ALLOW=192.168.1.0/24           # IP 白名单 (CIDR)
VOS_RS_SBC_BLOCK=                          # IP 黑名单 (CIDR)
VOS_RS_SBC_LIMIT_CAPACITY=100              # 令牌桶容量
VOS_RS_SBC_LIMIT_FILL_RATE=10              # 令牌填充速率

# === 日志 ===
RUST_LOG=info                              # 日志级别
# 或分模块: RUST_LOG=sip_edge=debug,media=trace

# === UDP Workers ===
VOS_RS_UDP_WORKERS=0                       # 0=auto (CPU核心数)
VOS_RS_UDP_WORKERS_AUTO=true               # 自适应 worker 数量
```

> 完整列表见 `.env` 和 `.env.production`

---

## 14. 文档

> 完整索引见 [`docs/README.md`](docs/README.md)

| 文档 | 位置 | 说明 |
|------|------|------|
| 文档索引 | `docs/README.md` | 全部文档导航 |
| 架构设计 | `docs/architecture/ARCHITECTURE.md` | 系统架构、模块关系、关键代码路径 |
| B2BUA 会话模型 | `docs/architecture/B2BUA_SESSION_MODEL.md` | session_id 主键模型（A/B-leg 统一索引） |
| 多租户架构 | `docs/architecture/MULTI_TENANT_DESIGN.md` | 商户关联费率、分机关联商户、域→租户映射 |
| RWI 实时控制台 | `docs/architecture/RWI_DESIGN.md` | WebSocket 双工通道、NATS 事件链路、媒体控制指令 |
| 架构分析 | `docs/architecture/VOS_RS_ARCHITECTURE_ANALYSIS.md` | 完整架构分析 + VOS 对比 |
| SIP/RTP 完整性 | `docs/architecture/rtp-sip-completeness.md` | 协议覆盖度 + 性能基线 |
| NATS VCI 设计 | `docs/architecture/NATS_VCI_COMMAND_DESIGN.md` | VCI 2.0 命令规范 |
| 中继设计 | `docs/architecture/TRUNK_CALLER_TERMINATION_DESIGN.md` | 接入认证、号码池、落地决策 |
| 中继流程图 | `docs/architecture/TRUNK_FLOWCHART.md` | 中继选路流程图 |
| 业务需求 PRD | `docs/architecture/VOS_RS_BUSINESS_GAPS_REQUIREMENTS.md` | 业务缺失与后续开发需求 |
| 配置参考 | `docs/development/ENV_VARS.md` | `config.yaml` 架构与环境变量 |
| AI 插件指南 | `docs/development/AI_PLUGIN_INTEGRATION_GUIDE.md` | AI 语音插件二进制流协议 |
| 前端优化 | `docs/development/FRONTEND_OPTIMIZATION.md` | HeroUI v2 + Tailwind v4 重构记录 |
| 性能压测 | `docs/development/PERFORMANCE_BENCHMARK.md` | SIP 信令与媒体压测报告 |
| SIPp 业务用例 | `docs/development/SIPP_BUSINESS_SCENARIOS.md` | 中继与号码业务验证 |
| Webhook 协议 | `docs/development/WEBHOOKS.md` | 呼叫事件 Webhook 协议 |
| 部署指南 | `docs/deployment/DEPLOY.md` | Docker Compose + 手动部署 |
| 集群部署 | `docs/deployment/CLUSTER_DEPLOYMENT.md` | 多节点 + 共享状态 |
| 内核调优 | `docs/deployment/OS_KERNEL_TUNING.md` | 操作系统与内核参数 |
| Web 界面指南 | `docs/user-guide/WEB_GUIDE.md` | 管理界面使用说明 |
| 路由中继指南 | `docs/user-guide/ROUTING_TRUNK_GUIDE.md` | 中继与路由管理配置 |


---

## 15. 注意事项

### 给 AI 的特别提醒

1. **修改代码前先理解上下文**：阅读相关模块的现有代码，理解设计意图后再动手
2. **不要引入不必要的抽象**：YAGNI 原则，只在确实需要时才添加新层级
3. **保持一致性**：跟随项目现有的代码风格和模式，即使你认为有更好的方式
4. **大改动先讨论**：如果重构涉及多个模块或超过 500 行变更，请先给出方案，确认后再实施
5. **测试必须能独立运行**：每个测试不依赖其他测试的执行顺序或外部状态
6. **中文注释可以接受**：本项目面向国内团队，注释和文档使用中英文均可

### 已知技术债务（需逐步解决）

1. [已完成] `sip-edge/src/main.rs` 9401 行 → **已拆分为多个子模块**（当前 540 行）
2. [已完成] `cdr-core/src/lib.rs` 1838 行 → **已拆分为 models/schema/store/termination_models/termination_schema/utils 子模块**
3. [已完成] 录音模块使用 `std::sync::Mutex` + sync I/O → **已改为 Tokio Channel + Task 隔离**
4. [已完成] SBC RateLimiter 使用单 Mutex → **已改为 DashMap 分片**
5. [已完成] RTP 解析无 buffer pool → **已下沉并引入有界 `BufferPool`**
6. [已完成] SIP 解析非零拷贝 → **已引入 zero_copy 模块（基于 Rust 生命周期借用）**
7. [已完成] 路由引擎与 SBC ACL 线性扫描 → **已实现 `PrefixTrie` 与 `IpTrie` 树检索**
8. [已完成] 缺少实时余额扣减 → **已引入 AtomicI64 CAS 内存预扣减缓存**
9. [已完成] `api-server` 全量拆分 → **65 个 .rs 文件，其中 `system/system.rs` (560)、`copilot/mod.rs` (525)、`main.rs` (521) 3 个文件略超 500 行，待持续优化**
10. [已完成] IVR 音频提示音文件上传能力 → **新增 `resources/prompts.rs` + 4 个端点**, 前端可通过 multipart 上传 wav/mp3 并通过 `/api/v1/ivr/prompts/:filename` 试听
11. [已完成] 多租户架构 → **新增 `tenant` 模块 + `TenantRegistry` 内存注册表 + 域→租户映射**，商户关联费率、分机关联商户，详见 `docs/architecture/MULTI_TENANT_DESIGN.md`
12. [已完成] RWI 实时控制台 → **新增 `rwi_ws` WebSocket 网关 + NATS 双主题事件链路 + 11 个媒体控制端点**，详见 `docs/architecture/RWI_DESIGN.md`
13. [已完成] SUBSCRIBE/NOTIFY 事件包框架 → **RFC 6665 基础框架**，支持 presence/dialog/message-summary 三种事件包，DashMap 双索引
14. [已完成] DSCP/QoS 标记配置 → **`performance.sip_dscp` / `performance.rtp_dscp`**，在 std socket 阶段设置 `IP_TOS`
15. [已完成] SDP ptime/maxptime 属性解析 → **`sdp-core` 新增 `first_audio_ptime` / `first_audio_maxptime` 方法**
