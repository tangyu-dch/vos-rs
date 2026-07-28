//! # SIP SUBSCRIBE/NOTIFY 订阅状态管理
//!
//! 本模块实现 RFC 6665 SUBSCRIBE/NOTIFY 订阅状态机的基础框架：
//!
//! - **订阅存储**：按 `(event_package, aor)` 索引活跃订阅
//! - **生命周期**：创建 / 刷新 / 终止（Expires=0）
//! - **TTL 清理**：调用方定期触发 `prune_expired`
//! - **NOTIFY 触发**：状态变更后向所有订阅者发送初始/更新通知
//!
//! ## 支持的事件包
//!
//! | 事件包 | RFC | 用途 |
//! |--------|-----|------|
//! | `presence` | RFC 3856 | 在线状态（BLF） |
//! | `dialog` | RFC 4235 | 通话状态（BLF） |
//! | `message-summary` | RFC 3842 | 留言计数（MWI） |
//!
//! 模块仅维护订阅本身；具体公告体由调用方按事件包构造后通过
//! [`SubscriptionStore::notify_all`] 发送。

mod store;
mod types;

pub(crate) use store::{parse_subscribe_request, SubscriptionStore, SubscriptionStoreError};
#[allow(unused_imports)]
pub(crate) use types::{
    default_expires_seconds, expires_at_from, normalize_expires, EventPackage, Subscription,
    SubscriptionId, SubscriptionState,
};
