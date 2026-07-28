# 多租户架构设计

> 本文档描述 vos-rs 的多租户（Multi-Tenant）隔离模型：
> **SIP 域 → tenant_id → TenantPolicy 快照** 的运行时映射架构，
> 以及 **商户关联费率、分机关联商户、商户统一计费账户** 的业务模型。
> 设计以"零侵入降级"为核心约束：当 `tenant.enabled=false` 时，
> 整套多租户能力完全不注入运行时，行为与旧部署完全一致。

---

## 一、设计目标

### 1.1 业务模型

| 关系 | 含义 | 外键 |
|------|------|------|
| 分机 → 商户 | 一个 SIP 分机（sip_users）归属于一个商户（tenant） | `sip_users.tenant_id → tenants.id` |
| 商户 → 费率 | 商户绑定专属费率表；未绑定则回退全局费率 | `billing_rates.tenant_id → tenants.id`（NULL = 全局） |
| 商户 → 计费账户 | 商户统一关联一个计费账户，覆盖分机/中继级配置 | `tenants.billing_account_id → billing_accounts.id` |

### 1.2 核心设计原则

| 原则 | 说明 |
|------|------|
| 零侵入降级 | `tenant.enabled=false` 时 `TenantRegistry` 不注入 `EdgeState`，所有呼叫走默认策略，完全兼容旧部署 |
| 运行时策略快照 | INVITE 入站时一次性加载 `TenantPolicy`，贯穿 CallSession / CDR / 计费链路，避免热路径查表 |
| 计费账户统一关联 | 通过 `billing_account_id` 外键统一商户计费入口，优先级高于 per-user / per-trunk 配置 |
| 域映射驱动 | 以 SIP From 头解析的域作为租户解析入口，兼容多种 SIP URI 格式 |
| 仅加载启用租户 | 内存注册表只载入 `enabled=TRUE` 的记录，禁用即等价于不存在 |

---

## 二、数据模型

### 2.1 tenants 表 schema

```sql
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT UNIQUE NOT NULL,
    max_concurrent_calls INTEGER NOT NULL DEFAULT 0,
    max_cps INTEGER NOT NULL DEFAULT 0,
    cross_tenant_policy TEXT NOT NULL DEFAULT 'allow_if_same_domain',
    recording_enabled BOOLEAN,
    allowed_gateway_ids JSONB,
    billing_account_id BIGINT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | TEXT (UUID v4 字符串) | 租户唯一标识，可由调用方指定或服务端生成 |
| `name` | TEXT | 租户显示名（用于日志/CDR） |
| `domain` | TEXT (UNIQUE) | SIP From 头解析的域，租户解析入口 |
| `max_concurrent_calls` | INTEGER | 最大并发通话数（0 = 不限制） |
| `max_cps` | INTEGER | 最大每秒呼叫数（0 = 不限制） |
| `cross_tenant_policy` | TEXT | 跨租户策略：`allow` / `deny` / `allow_if_same_domain`（默认） |
| `recording_enabled` | BOOLEAN | 录音覆盖开关（NULL = 沿用全局） |
| `allowed_gateway_ids` | JSONB | 允许的网关 ID 列表（NULL = 全部允许） |
| `billing_account_id` | BIGINT | 统一计费账户外键 |
| `enabled` | BOOLEAN | 是否启用（FALSE 不加载到内存） |
| `created_at` / `updated_at` | TIMESTAMPTZ | 审计时间戳 |

**索引**：
- `idx_tenants_domain`：按 `domain` 索引，且仅索引 `enabled = TRUE` 的行
- `idx_tenants_enabled`：按 `enabled` 索引，便于运维查询

**源文件**：[`crates/cdr-core/src/schema/sip_tables.rs`](../../crates/cdr-core/src/schema/sip_tables.rs)（约 144-165 行）

### 2.2 关联关系

```text
┌──────────────┐   tenant_id    ┌──────────────┐  billing_account_id  ┌──────────────────┐
│  sip_users   │ ─────────────> │   tenants    │ ───────────────────> │ billing_accounts  │
│  (SIP 分机)   │                │  (商户/租户)  │                       │  (计费账户)       │
└──────────────┘                └──────┬───────┘                       └──────────────────┘
                                       │
                                       │ tenant_id (NULL = 全局)
                                       ▼
                               ┌──────────────────┐
                               │  billing_rates   │
                               │  (费率表)         │
                               └──────────────────┘
```

| 外键路径 | 含义 |
|---------|------|
| `sip_users.tenant_id → tenants.id` | 分机关联商户（一个分机归属一个商户） |
| `billing_rates.tenant_id → tenants.id` | 商户关联费率（NULL 表示全局费率，所有租户共享） |
| `tenants.billing_account_id → billing_accounts.id` | 商户统一计费账户（覆盖 per-user/per-trunk） |

### 2.3 TenantRecord 与 TenantPolicy

#### TenantRecord（cdr-core 层，对齐 DB）

```rust
#[derive(Debug, Clone)]
pub struct TenantRecord {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub policy: TenantPolicy,
    /// 是否启用（仅当为 true 时才会被加载到内存注册表）。
    pub enabled: bool,
}
```

**源文件**：[`services/sip-edge/src/tenant/store.rs`](../../services/sip-edge/src/tenant/store.rs)

#### TenantPolicy（sip-edge 运行时快照）

在 INVITE 入站时从 `TenantRegistry` 加载，与 `TenantContext` 一起贯穿呼叫生命周期。所有 `0` 值表示"无限制"。

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct TenantPolicy {
    pub max_concurrent_calls: u32,    // 0 = 不限制
    pub max_cps: u32,                 // 0 = 不限制
    pub cross_tenant_policy: CrossTenantPolicy,
    pub recording_enabled: Option<bool>,        // None = 沿用全局
    pub allowed_gateway_ids: Option<Vec<String>>, // None = 全部允许
    pub billing_account_id: Option<i64>,         // 覆盖 per-user 计费账户
}
```

**源文件**：[`services/sip-edge/src/tenant/policy.rs`](../../services/sip-edge/src/tenant/policy.rs)

#### CrossTenantPolicy 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossTenantPolicy {
    Allow,                // 允许跨租户呼叫（开放模式）
    Deny,                 // 拒绝所有跨租户呼叫（严格隔离）
    AllowIfSameDomain,    // 仅允许同域内呼叫（默认，向后兼容）
}

impl Default for CrossTenantPolicy {
    fn default() -> Self { Self::AllowIfSameDomain }
}
```

默认值为 `AllowIfSameDomain`，与旧版 `check_cross_tenant` 的"同域 + 注册命中"启发式判断行为一致，保证向后兼容。

#### TenantContext（单次呼叫的租户上下文）

```rust
#[derive(Debug, Clone, Default)]
pub struct TenantContext {
    pub tenant_id: Option<String>,    // None = 默认租户
    pub tenant_name: Option<String>,   // 用于日志/CDR
    pub domain: Option<String>,       // SIP From 头解析的域
    pub policy: TenantPolicy,         // 运行时策略快照
}
```

降级构造方法：

| 方法 | 场景 | 结果 |
|------|------|------|
| `TenantContext::anonymous()` | `TenantRegistry` 未注入 / From 头解析失败 | 所有字段为 None，policy 为默认值（无限制） |
| `TenantContext::from_domain(domain)` | 域未在注册表中命中 / 租户被禁用 | 仅记录 domain，不关联 tenant_id，policy 为默认值 |

**源文件**：[`services/sip-edge/src/tenant/context.rs`](../../services/sip-edge/src/tenant/context.rs)

---

## 三、运行时架构

### 3.1 TenantRegistry 内存注册表

```rust
#[derive(Debug, Clone)]
pub struct TenantRegistry {
    inner: Arc<RwLock<HashMap<String /* domain */, TenantRecord>>>,
    store: TenantStore,
}
```

| 特性 | 说明 |
|------|------|
| 数据结构 | `Arc<RwLock<HashMap<String, TenantRecord>>>`，按域（小写）索引 |
| 刷新周期 | 默认 60 秒，可通过 `tenant.refresh_interval_secs` 配置；`0` 禁用自动刷新 |
| 加载范围 | 仅加载 `enabled = TRUE` 的租户（`SELECT ... WHERE enabled = TRUE`） |
| 首次刷新 | 后台任务启动时立即执行一次，随后按周期 tick |
| 查询入口 | `context_for_domain(domain)` / `context_for_from_header(from_header)` |
| 降级策略 | DB 查询失败时返回空 HashMap，所有呼叫走默认策略并记录 warn 日志 |

**查询路径**：

```text
SIP From 头 ─> extract_domain_from_from() ─> domain(小写)
                                              │
                                              ▼
                              registry.inner.read().await
                                              │
                              命中且 enabled ─> 构造 TenantContext (tenant_id + policy)
                              未命中 / 禁用  ─> TenantContext::from_domain(domain) 降级
```

`extract_domain_from_from` 支持的 SIP URI 格式：

- `<sip:user@domain:port>`
- `<sip:user@domain>`
- `"Display" <sip:user@domain>`
- `sip:user@domain`
- `sips:user@domain:port`（TLS）
- 带 URI 参数：`<sip:user@domain:5060;user=phone>`

**源文件**：[`services/sip-edge/src/tenant/registry.rs`](../../services/sip-edge/src/tenant/registry.rs)、[`services/sip-edge/src/tenant/store.rs`](../../services/sip-edge/src/tenant/store.rs)

### 3.2 呼叫生命周期集成

```text
UAC ── INVITE (From: <sip:1001@acme.com>) ──> sip-edge
                                                 │
                                                 ▼
                          ┌──────────────────────────────────────────┐
                          │ 1. resolve_tenant_context_pub()          │
                          │    - 解析 From 头域 → acme.com            │
                          │    - 查 TenantRegistry → TenantContext   │
                          │    - 一次性加载 TenantPolicy 快照         │
                          └──────────────────────────────────────────┘
                                                 │
                                                 ▼
                          ┌──────────────────────────────────────────┐
                          │ 2. check_tenant_limits()                 │
                          │    - 并发计数：increment                  │
                          │      （在 remember_inbound_invite 之后）  │
                          │    - CPS 滑动窗口：保留最近 1 秒时间戳    │
                          │    - 超限 → 返回 503 Service Unavailable  │
                          └──────────────────────────────────────────┘
                                                 │
                                                 ▼
                          ┌──────────────────────────────────────────┐
                          │ 3. check_cross_tenant()                  │
                          │    - 优先使用 TenantPolicy.cross_tenant  │
                          │    - 未注入 registry 回退旧启发式         │
                          │    - 违反策略 → 返回 403 Forbidden       │
                          └──────────────────────────────────────────┘
                                                 │
                                                 ▼
                          ┌──────────────────────────────────────────┐
                          │ 4. TenantContext 贯穿呼叫链路            │
                          │    - CallSession（绑定 tenant_id）        │
                          │    - CDR（写入 tenant_id 字段）          │
                          │    - 计费（解析 billing_account_id）     │
                          │    - 录音（应用 recording_enabled 覆盖）  │
                          └──────────────────────────────────────────┘
                                                 │
                                                 ▼
                  任一 leg ── BYE ──> sip-edge
                                            │
                                            ▼
                          ┌──────────────────────────────────────────┐
                          │ 5. teardown_call_transaction()           │
                          │    - 在清理会话前 decrement 并发计数       │
                          │    - 释放媒体资源、生成 CDR               │
                          └──────────────────────────────────────────┘
```

**并发计数生命周期**：

| 时机 | 操作 | 位置 |
|------|------|------|
| 入站 INVITE 通过限额检查后 | `increment` | `remember_inbound_invite` 之后 |
| 会话终结（BYE/CANCEL）前 | `decrement` | `teardown_call_transaction` 之前 |

**CPS 滑动窗口**：

- 保留最近 `CPS_WINDOW_SECS = 1` 秒内的呼叫时间戳
- 新呼叫到达时清理过期时间戳，剩余数量 ≥ `max_cps` 则拒绝并返回 503
- 仅当 `TenantPolicy.has_cps_limit()` 为真时启用

**源文件**：[`services/sip-edge/src/sip/handlers/invite/resolution.rs`](../../services/sip-edge/src/sip/handlers/invite/resolution.rs)（约 61、80-85、390-415 行）

### 2.3 跨租户策略

| 策略 | 行为 | 适用场景 |
|------|------|---------|
| `Allow` | 允许跨租户呼叫（任何目标域） | 单一运营商、内部互联 |
| `Deny` | 禁止所有跨租户呼叫 | 严格隔离的多商户环境 |
| `AllowIfSameDomain` | 仅允许同域内呼叫（默认值） | 向后兼容的默认模式 |

策略判断通过 `TenantContext::allows_cross_tenant_call_to(target_domain)` 实现：

```rust
pub fn allows_cross_tenant_call_to(&self, target_domain: &str) -> bool {
    match self.policy.cross_tenant_policy {
        CrossTenantPolicy::Allow => true,
        CrossTenantPolicy::Deny => false,
        CrossTenantPolicy::AllowIfSameDomain => self
            .domain
            .as_deref()
            .map(|d| d.eq_ignore_ascii_case(target_domain))
            .unwrap_or(false),
    }
}
```

当 `TenantRegistry` 未注入时，`check_cross_tenant` 回退到旧的"同域 + 注册命中"启发式判断，保持向后兼容。

### 3.4 录音覆盖优先级

`TenantPolicy.recording_enabled` 是三态开关，覆盖全局录音配置：

| `recording_enabled` | 行为 |
|---------------------|------|
| `Some(true)` | 强制开启录音（覆盖全局关闭） |
| `Some(false)` | 强制关闭录音（覆盖全局开启） |
| `None` | 沿用全局录音开关（默认） |

### 3.5 计费账户优先级

计费账户解析按以下优先级从高到低：

| 优先级 | 来源 | 字段 |
|--------|------|------|
| 1 | 租户策略 | `TenantPolicy.billing_account_id` |
| 2 | 分机/中继级配置 | per-user / per-trunk 计费账户 |

即：租户绑定的 `billing_account_id` 覆盖分机或中继级别的计费账户配置，确保商户统一计费入口。

---

## 四、配置项

```yaml
tenant:
  enabled: false              # 默认 false，所有呼叫走默认策略（向后兼容）
  refresh_interval_secs: 60   # 从 PostgreSQL 重新加载 tenants 表的周期（秒），0 = 禁用自动刷新
```

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | `false` | 多租户隔离总开关；false 时 `TenantRegistry` 不注入 `EdgeState` |
| `refresh_interval_secs` | u64 | `60` | 注册表刷新周期；`0` 禁用自动刷新（仅启动时加载一次） |

**结构体定义**：

```rust
#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct TenantSection {
    /// 多租户隔离是否启用（默认 false，即所有呼叫走默认策略）。
    pub(super) enabled: Option<bool>,
    /// 从 PostgreSQL 重新加载 tenants 表的周期（秒）。0 = 禁用自动刷新。
    pub(super) refresh_interval_secs: Option<u64>,
}
```

**源文件**：
- 结构体：[`services/sip-edge/src/config/sections.rs`](../../services/sip-edge/src/config/sections.rs)（约 59-65 行）
- 加载：[`services/sip-edge/src/config/loader.rs`](../../services/sip-edge/src/config/loader.rs)（约 247-248 行）

---

## 五、API 端点

### 5.1 对外 REST API（api-server，JWT 鉴权）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/tenants` | 租户列表（支持分页、关键词过滤、enabled 过滤、CSV 导出） |
| POST | `/api/v1/tenants` | 创建租户（`id` 可由调用方指定或服务端生成 UUID v4） |
| GET | `/api/v1/tenants/:id` | 获取租户详情 |
| PUT | `/api/v1/tenants/:id` | 全量更新租户 |
| DELETE | `/api/v1/tenants/:id` | 删除租户 |
| POST | `/api/v1/tenants/:id/enabled` | 切换启用状态 |
| PUT | `/api/v1/tenants/:id/billing-account` | 关联计费账户（设置 `billing_account_id`） |

**源文件**：[`services/api-server/src/resources/tenants.rs`](../../services/api-server/src/resources/tenants.rs)

### 5.2 内部管理 API（sip-edge，X-VOS-Token 鉴权）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/manage/tenants` | 列出内存注册表中已加载的租户简要信息 |
| GET | `/manage/tenants/count` | 返回当前内存注册表中的租户数量 |

`/manage/tenants` 返回的 `TenantSummary` 结构：

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct TenantSummary {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub enabled: bool,
    pub max_concurrent_calls: u32,
    pub max_cps: u32,
    pub cross_tenant_policy: CrossTenantPolicy,
    pub recording_enabled: Option<bool>,
    pub billing_account_id: Option<i64>,
}
```

**源文件**：[`services/sip-edge/src/manage/tenants.rs`](../../services/sip-edge/src/manage/tenants.rs)

---

## 六、核心设计决策

| 编号 | 决策 | 理由 |
|------|------|------|
| 1 | 域 → tenant_id 映射 | 以 SIP From 头解析的域作为租户解析入口，兼容多种 SIP URI 格式（含 display name、端口、URI 参数） |
| 2 | 零侵入降级 | `tenant.enabled=false` 时 `TenantRegistry` 不注入 `EdgeState`，所有呼叫走默认策略，完全兼容旧部署 |
| 3 | 运行时策略快照 | INVITE 入站时一次性加载 `TenantPolicy`，避免热路径反复查 `TenantRegistry`/DB |
| 4 | CPS 滑动窗口 | 保留最近 1 秒（`CPS_WINDOW_SECS=1`）呼叫时间戳，超限返回 503，实现简单且无锁竞争 |
| 5 | 并发计数生命周期 | `increment` 在 `remember_inbound_invite` 之后，`decrement` 在 `teardown_call_transaction` 之前，确保计数准确 |
| 6 | 录音覆盖优先级 | `recording_enabled` 三态（Some/None）覆盖全局开关，兼顾灵活性与默认行为 |
| 7 | 计费账户优先级 | `TenantPolicy.billing_account_id` 覆盖 per-user/per-trunk 配置，确保商户统一计费入口 |
| 8 | 统一计费关联 | 通过 `billing_account_id` 外键统一商户计费入口，与计费账户体系解耦但可组合 |

---

## 七、Redis 缓存分桶

为避免多租户场景下费率数据互相污染，Redis 中的费率相关数据按 `tenant_id` 分桶存储：

| 数据类型 | Redis Key 模式 | 说明 |
|---------|----------------|------|
| 费率表 | `tenant_rates:{tenant_id}` | 租户专属费率配置 |
| 计费区间 | `tenant_intervals:{tenant_id}` | 租户专属计费区间 |
| 价格表 | `tenant_prices:{tenant_id}` | 租户专属价格表 |

**费率回退策略**：

```text
查询租户费率
    │
    ▼
tenant_rates:{tid} 是否存在？
    │
    ├─ 是 ─> 返回租户专属费率
    │
    └─ 否 ─> 回退全局费率（tenant_id = NULL 的 billing_rates 记录）
```

当 `billing_rates.tenant_id` 为 NULL 时表示全局费率，所有未配置专属费率的租户共享。该回退逻辑确保新增租户无需立即配置费率即可正常计费。

---

## 八、相关源文件索引

| 功能 | 源文件 |
|------|--------|
| tenants 表 schema | [`crates/cdr-core/src/schema/sip_tables.rs`](../../crates/cdr-core/src/schema/sip_tables.rs) |
| TenantPolicy / CrossTenantPolicy | [`services/sip-edge/src/tenant/policy.rs`](../../services/sip-edge/src/tenant/policy.rs) |
| TenantContext | [`services/sip-edge/src/tenant/context.rs`](../../services/sip-edge/src/tenant/context.rs) |
| TenantRegistry 内存注册表 | [`services/sip-edge/src/tenant/registry.rs`](../../services/sip-edge/src/tenant/registry.rs) |
| TenantStore 持久化 | [`services/sip-edge/src/tenant/store.rs`](../../services/sip-edge/src/tenant/store.rs) |
| 租户模块入口 | [`services/sip-edge/src/tenant/mod.rs`](../../services/sip-edge/src/tenant/mod.rs) |
| INVITE 入站租户解析 | [`services/sip-edge/src/sip/handlers/invite/resolution.rs`](../../services/sip-edge/src/sip/handlers/invite/resolution.rs) |
| TenantSection 配置 | [`services/sip-edge/src/config/sections.rs`](../../services/sip-edge/src/config/sections.rs) |
| 配置加载 | [`services/sip-edge/src/config/loader.rs`](../../services/sip-edge/src/config/loader.rs) |
| 对外 REST API | [`services/api-server/src/resources/tenants.rs`](../../services/api-server/src/resources/tenants.rs) |
| 内部管理 API | [`services/sip-edge/src/manage/tenants.rs`](../../services/sip-edge/src/manage/tenants.rs) |

---

*最后更新：2026-07-29（多租户架构设计）*
