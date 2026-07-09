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
├── Cargo.toml                 # Workspace 根 (8 members)
├── README.md                  # 项目说明
├── DEPLOY.md                  # 部署指南
├── WEB_GUIDE.md               # Web 管理界面指南
├── Makefile                   # 常用命令
├── Dockerfile
├── docker-compose.yml
├── .env / .env.production
│
├── crates/                    # 6 个协议/业务库 crate
│   ├── sip-core/              # SIP 消息解析 (零外部依赖)
│   │   └── src/               #   error, header, message, method, uri
│   ├── rtp-core/              # RTP/RTCP 协议 (零外部依赖)
│   │   └── src/               #   error, packet, payload, rtcp, telephone_event, srtp
│   ├── sdp-core/              # SDP 解析 (零外部依赖)
│   │   └── src/               #   error, session
│   ├── call-core/             # 呼叫控制 + 路由 + CDR 生成
│   │   └── src/               #   call, cdr, error, manager, routing
│   ├── cdr-core/              # CDR 存储 (PostgreSQL) + 数据模型
│   │   └── src/               #   lib.rs (单文件 1838 行)
│   └── storage-core/          # 录音存储抽象 (Local/OSS/Dual)
│       └── src/               #   config, local, oss
│
├── services/                  # 3 个服务二进制
│   ├── sip-edge/              # SIP B2BUA 核心 (最大服务)
│   │   └── src/               #   main.rs(9401行!), media, auth, dialog, transaction,
│   │                          #   outbound, registrar, transport, sbc, anti_fraud,
│   │                          #   fork, manage, multimedia, nats_cdr, stun_client,
│   │                          #   topology, transcode, turn, upnp, tenant, subscribe
│   ├── api-server/            # REST API (Axum, 30+ 端点)
│   │   └── src/               #   main, recording, report, billing, calls, numbers,
│   │                          #   anti_fraud, metrics
│   └── cdr-worker/            # NATS CDR 消费者 (批量写 PostgreSQL)
│       └── src/               #   main.rs (单文件 392 行)
│
├── web/                       # React 管理界面 (Vite + TypeScript, 14 页面)
├── docs/                      # 架构文档
│   ├── ARCHITECTURE.md
│   ├── ENV_VARS.md
│   ├── rtp-sip-completeness.md
│   └── VOS_RS_ARCHITECTURE_ANALYSIS.md
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
| SIP Dialogs | DashMap<CallId, Arc<InboundTransaction>> | DashMap 分段锁 | 读多写少 |
| RTP Sessions | DashMap<u16, SocketAddr> 等 6 个 DashMap | DashMap 分段锁 | per-port 状态 |
| Media Inner | Arc<Mutex<MediaRelayStateInner>> | std::sync::Mutex | 端口分配 |
| Trunk 状态 | Arc<Vec<Arc<TrunkState>>> | AtomicBool/AtomicU32 | 读多写少 |
| 路由表 | Arc<RwLock<RouteTable>> | std::sync::RwLock | 极少写 |
| CDR 缓存 | Mutex<Vec<CallCdr>> | std::sync::Mutex | 低频写 |
| SBC RateLimiter | Mutex<HashMap<IpAddr, TokenBucket>> | std::sync::Mutex | 需优化 |
| Registration | tokio::sync::Mutex<RegistrationStore> | tokio::sync::Mutex | 异步感知 |

---

## 5. 编码规范

### 5.1 通用规则

1. **所有代码必须通过 linter 检查**，无 warning (`cargo clippy`)
2. **公共 API 必须有文档注释** (`///`)
3. **禁止 unwrap() 出现在生产代码中**（测试代码除外）
4. **函数长度不超过 50 行**，超出应拆分
5. **文件长度不超过 500 行**，超出应拆模块（当前 main.rs 9401 行需重构！）
6. **命名清晰，避免缩写**（`user` 不写成 `usr`，`response` 不写成 `resp`）

### 5.2 命名约定

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

### 5.3 错误处理规范

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

### 5.4 异步编程规范

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

**特别注意**：录音模块 (`media.rs`) 当前使用 `std::sync::Mutex` + 同步文件 I/O，这违反了异步编程规范，必须重构为 async channel-based 方案。

### 5.5 注释规范

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

### 5.6 大文件拆分规范

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
- **禁止在 async 上下文中使用 std::sync::Mutex + 同步 I/O**（录音模块违规！）
- 数据库必须有索引覆盖常用查询
- 大批量操作必须分批处理（batch size <= 1000）
- 缓存必须设置 TTL，防止内存膨胀

### 10.3 当前性能瓶颈（已审计确认）

| 瓶颈 | 严重级别 | 位置 | 说明 |
|------|---------|------|------|
| 录音 sync I/O | 🔴 高 | `media.rs:629-639` | std::fs::File 在 Mutex 内同步写，阻塞 tokio runtime |
| SBC RateLimiter 单 Mutex | 🔴 高 | `sbc.rs:88,106` | 高 CPS 下所有 SIP 收包串行化 |
| RTP 每包 6-8 次 DashMap 锁 | 🟡 中 | `media.rs:1403-1476` | 高 pps 下 cache line bouncing |
| RTP 解析每包 Vec alloc | 🟡 中 | `rtp-core/packet.rs:85,115` | 无 buffer pool |
| SIP 解析非零拷贝 | 🟡 中 | `sip-core/message.rs:62` | String::from_utf8_lossy + .to_string() |
| main.rs 9401 行 | 🔴 高 | `sip-edge/main.rs` | 无法独立测试、维护困难 |

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

1. **入口点**：`services/sip-edge/src/main.rs`（最大文件，9401 行）
2. **协议解析层**：`crates/sip-core/`、`crates/rtp-core/`、`crates/sdp-core/`（零外部依赖）
3. **业务逻辑层**：`crates/call-core/`（呼叫状态机、路由、CDR）
4. **数据存储层**：`crates/cdr-core/`（PostgreSQL CRUD + 数据模型）
5. **媒体处理**：`services/sip-edge/src/media.rs`（RTP relay + 录音）
6. **安全模块**：`services/sip-edge/src/sbc.rs`、`auth.rs`、`anti_fraud.rs`
7. **环境变量**：所有配置通过 `VOS_RS_` 前缀环境变量，见 `.env.example`
8. **测试目录**：`crates/*/tests/`（集成测试）、各模块内 `#[cfg(test)]`（单元测试）
9. **SIPp 测试**：`tools/sipp/`（端到端 SIP 场景）
10. **架构文档**：`docs/VOS_RS_ARCHITECTURE_ANALYSIS.md`（详细架构分析）

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

| 文档 | 位置 | 说明 |
|------|------|------|
| 架构分析 | `docs/VOS_RS_ARCHITECTURE_ANALYSIS.md` | 完整架构分析 + VOS 对比 |
| 架构设计 | `docs/ARCHITECTURE.md` | 系统架构、模块关系 |
| 环境变量 | `docs/ENV_VARS.md` | 配置项参考 |
| SIP/RTP 完整性 | `docs/rtp-sip-completeness.md` | 功能覆盖度 |
| 部署指南 | `DEPLOY.md` | Docker Compose + 手动部署 |
| Web 界面指南 | `WEB_GUIDE.md` | 管理界面使用说明 |

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

1. `sip-edge/src/main.rs` 9401 行 → 需拆分为 10+ 子模块
2. `cdr-core/src/lib.rs` 1838 行 → 需拆分为 db、models、cdr 子模块
3. 录音模块使用 `std::sync::Mutex` + sync I/O → 需改为 async
4. SBC RateLimiter 使用单 Mutex → 需改为 DashMap 分片
5. RTP 解析无 buffer pool → 需引入 `bytes::Bytes` 池化
6. SIP 解析非零拷贝 → 需引入借用生命周期
7. 路由引擎使用 Vec 线性扫描 → 需引入 Trie
8. 缺少实时余额扣减 → 需引入 AtomicI64 CAS 缓存
