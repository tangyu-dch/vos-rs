use std::fmt;

pub type CallResult<T> = Result<T, CallError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    MissingRequiredHeader(&'static str),
    InvalidDestinationUri,
    InvalidTransition {
        from: &'static str,
        event: &'static str,
    },
    NoRouteForDestination(String),
    GatewayUnavailable(String),
    UnknownCall(String),
    OutboundLegAlreadyExists,
    MissingOutboundLeg,
    CallerIdentityUnavailable(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequiredHeader(header) => {
                write!(f, "missing required SIP header: {header}")
            }
            Self::InvalidDestinationUri => write!(f, "invalid or missing destination user in URI"),
            Self::InvalidTransition { from, event } => {
                write!(f, "invalid call state transition from {from} on {event}")
            }
            Self::NoRouteForDestination(destination) => {
                write!(f, "no route for destination: {destination}")
            }
            Self::GatewayUnavailable(destination) => {
                write!(f, "all gateways unavailable for destination: {destination}")
            }
            Self::UnknownCall(call_id) => write!(f, "unknown call: {call_id}"),
            Self::OutboundLegAlreadyExists => write!(f, "outbound leg already exists"),
            Self::MissingOutboundLeg => write!(f, "missing outbound leg"),
            Self::CallerIdentityUnavailable(reason) => {
                write!(f, "caller identity unavailable: {reason}")
            }
        }
    }
}

impl std::error::Error for CallError {}
