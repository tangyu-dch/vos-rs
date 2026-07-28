use call_core::CallerIdentity;
use sip_core::SipUri;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHandling {
    pub response: Vec<u8>,
    pub outbound_invite: Option<OutboundInvitePlan>,
}

impl RequestHandling {
    fn response(response: Vec<u8>) -> Self {
        Self {
            response,
            outbound_invite: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundInvitePlan {
    pub outbound_uri: SipUri,
    pub target_override_addr: Option<String>,
    pub gateway_id: String,
    pub caller_identity: Option<CallerIdentity>,
}

impl From<Vec<u8>> for RequestHandling {
    fn from(response: Vec<u8>) -> Self {
        Self::response(response)
    }
}
