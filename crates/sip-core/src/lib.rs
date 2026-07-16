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
