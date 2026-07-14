//! # SIP 信令处理层
//!
//! 本模块实现了 SIP 信令的核心处理逻辑，包括：
//!
//! - **事务管理**：SIP 事务状态机（INVITE/Non-INVITE 客户端/服务端事务）
//! - **对话管理**：SIP 对话（Dialog）的创建、维护和终止
//! - **认证**：SIP Digest Auth 认证和 nonce 防重放
//! - **路由**：入站 INVITE 路由选择和出站 INVITE 构建
//! - **注册**：SIP REGISTER 处理和 Contact 绑定
//! - **转发**：BYE/CANCEL/INFO/REFER 等消息转发
//!
//! ## 处理流程
//!
//! ```text
//! UDP 接收 → handle_datagram → 事务匹配 → 对话匹配 → handlers 处理 → 响应/转发
//! ```
//!
//! ## 模块职责
//!
//! | 模块 | 职责 |
//! |------|------|
//! | `dispatcher` | 入口：UDP 数据报分发 |
//! | `handlers` | 业务逻辑：INVITE/BYE/CANCEL/INFO/REFER 处理 |
//! | `outbound` | 出站消息构建：INVITE/BYE/OPTIONS/NOTIFY |
//! | `response` | 响应构建：100/180/200/4xx/5xx |
//! | `transaction` | 事务状态机和重传 |
//! | `dialog` | 对话管理和验证 |
//! | `auth` | Digest Auth 认证 |
//! | `registrar` | REGISTER 注册处理 |

pub(crate) mod auth;
pub(crate) mod dialog;
pub(crate) mod dispatcher;
pub(crate) mod handlers;
pub(crate) mod outbound;
pub(crate) mod registrar;
pub(crate) mod response;
pub(crate) mod transaction;

pub(crate) use auth::{AuthConfig, AuthDecision};
pub(crate) use dialog::DialogValidationError;
pub(crate) use dispatcher::handle_datagram;
pub(crate) use transaction::{ClientTransactionKey, InviteAckKey, RequestTransactionKey};
