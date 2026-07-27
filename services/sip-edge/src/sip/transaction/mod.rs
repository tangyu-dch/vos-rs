//! # SIP 事务管理
//!
//! 本模块实现了 SIP 事务状态机，包括：
//!
//! - **INVITE 客户端事务**：出站 INVITE 的重传和超时处理
//! - **Non-INVITE 客户端事务**：出站 BYE/OPTIONS 等的重传
//! - **INVITE 服务端事务**：入站 INVITE 的响应处理
//! - **Non-INVITE 服务端事务**：入站 BYE/INFO 等的响应处理
//!
//! ## 事务状态机
//!
//! ```text
//! INVITE 客户端事务：
//!   调用 → Calling → Proceeding → Completed → Terminated
//!
//! Non-INVITE 客户端事务：
//!   调用 → Trying → Proceeding → Completed → Terminated
//! ```
//!
//! ## 重传机制
//!
//! - INVITE 事务使用 Timer A（初始重传间隔）和 Timer B（事务超时）
//! - 非 INVITE 事务使用 Timer F（事务超时）
//! - 重传间隔指数增长，最大不超过 Timer B/F
//!
//! ## 子模块
//!
//! | 模块 | 职责 |
//! |------|------|
//! | `keys` | 事务匹配键（`ClientTransactionKey` / `RequestTransactionKey` / `InviteAckKey`） |
//! | `event` | 服务端事务事件枚举 `ServerTransactionEvent` |
//! | `server` | 服务端事务 spawn 函数（INVITE / Non-INVITE 状态机） |

mod event;
mod keys;
mod server;
#[cfg(test)]
mod tests;

pub(crate) use event::ServerTransactionEvent;
pub(crate) use keys::{branch_param, ClientTransactionKey, InviteAckKey, RequestTransactionKey};
pub(crate) use server::{spawn_invite_server_transaction, spawn_non_invite_server_transaction};
