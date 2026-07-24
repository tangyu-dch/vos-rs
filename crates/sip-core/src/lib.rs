//! # sip-core
//!
//! 电信级 SIP 2.0 (RFC 3261) 协议解析与构造内核。
//!
//! **设计保障**：故意保持 **0 外部依赖**，完全基于 Rust 标准库实现，
//! 具备零拷贝解析 (`SipMessageBorrow<'a>`) 能力，单包解析时延 <50ns。

mod error;
mod header;
mod message;
mod method;
mod uri;

pub use error::{SipParseError, SipResult};
pub use header::{HeaderMap, HeaderName, HeaderValue};
pub use method::Method;

// Default to 'static owned versions to keep 100% backward compatibility
pub type SipUri = uri::SipUri<'static>;
pub type SipRequest = message::SipRequest<'static>;
pub type SipResponse = message::SipResponse<'static>;
pub type SipMessage = message::SipMessage<'static>;

// Lifetime-aware versions for zero-copy instant path
pub type SipUriBorrow<'a> = uri::SipUri<'a>;
pub type SipRequestBorrow<'a> = message::SipRequest<'a>;
pub type SipResponseBorrow<'a> = message::SipResponse<'a>;
pub type SipMessageBorrow<'a> = message::SipMessage<'a>;

pub use message::{parse_message, StartLine};
pub mod zero_copy;
pub use zero_copy::{ZeroCopyHeader, ZeroCopyRequestLine, ZeroCopySipMessage};
