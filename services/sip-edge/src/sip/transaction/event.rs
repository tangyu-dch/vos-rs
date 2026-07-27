//! # 服务端事务事件
//!
//! [`ServerTransactionEvent`] 由 dispatcher 发送给服务端事务 task，
//! 用于驱动 INVITE / Non-INVITE 服务端事务状态机。

use sip_core::SipRequest;

/// 服务端事务事件。
///
/// 事件由事务使用者（dispatcher）发送，事务 task 在 `select!` 循环中消费。
#[derive(Debug, Clone)]
pub(crate) enum ServerTransactionEvent {
    /// 收到入站请求（可能是重传），需要按当前状态机决定是否重发最后响应。
    Request(SipRequest),
    /// 收到响应字节，由事务 task 发送给客户端。
    ///
    /// `send_immediately` 为 `true` 时立即发送；为 `false` 时仅缓存，用于重传。
    Response {
        bytes: Vec<u8>,
        send_immediately: bool,
    },
    /// 更新最近一次的临时响应（1xx），用于后续 INVITE 重传时回送。
    UpdateLastProvisional(Vec<u8>),
    /// 收到 ACK（仅 INVITE 事务使用），触发状态机进入 Confirmed/Terminated。
    Ack,
}

impl ServerTransactionEvent {
    /// 构造立即发送的响应事件。
    pub(crate) fn send_response(bytes: Vec<u8>) -> Self {
        Self::Response {
            bytes,
            send_immediately: true,
        }
    }

    /// 构造仅观察（不立即发送）的响应事件。
    pub(crate) fn observe_response(bytes: Vec<u8>) -> Self {
        Self::Response {
            bytes,
            send_immediately: false,
        }
    }
}
