//! 多租户运行时策略与隔离。
//!
//! 本模块提供基于 tenant_id 的运行时上下文：
//! - [`TenantContext`]：从 SIP 请求中解析得到的租户上下文，贯穿呼叫生命周期
//! - [`TenantPolicy`]：租户级运行时策略（并发上限、CPS 限额、跨域策略）
//! - [`TenantRegistry`]：内存中的租户注册表，周期从 PostgreSQL 同步
//!
//! ## 设计原则
//!
//! 1. **域 → tenant_id 映射**：从 SIP From 头解析 domain，查表得到 tenant_id
//! 2. **tenant_id 传播**：在 CallSession / CDR / 计费链路中传递 tenant_id
//! 3. **运行时策略隔离**：并发上限、CPS 限额、跨域策略按 tenant 维度独立配置
//! 4. **零侵入**：现有调用方代码仅在需要 tenant 隔离的位置增量引入
//!
//! ## 与现有"域隔离"的关系
//!
//! 现有的 `check_cross_tenant`（基于 From 域名）是本模块的简化形式。
//! 本模块将其升级为可配置的 per-tenant 策略，并扩展至并发/CPS/路由维度。

pub(crate) mod context;
pub(crate) mod policy;
pub(crate) mod registry;
pub(crate) mod store;

pub(crate) use context::TenantContext;
pub(crate) use registry::TenantRegistry;
pub(crate) use store::TenantStore;
