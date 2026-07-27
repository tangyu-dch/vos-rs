#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CallResources {
    pub(super) caller_number: String,
    pub(super) number_max_concurrent: u32,
    pub(super) gateway_id: String,
    pub(super) max_concurrent: u32,
}

#[derive(Debug)]
pub(crate) enum LeaseError {
    NumberBusy,
    TrunkAtCapacity,
    CallConflict,
    InfrastructureUnavailable,
    Redis(redis::RedisError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NumberBusy => formatter.write_str("主叫号码正在被其他通话占用"),
            Self::TrunkAtCapacity => formatter.write_str("落地中继并发已满"),
            Self::CallConflict => formatter.write_str("Call-ID 已绑定到其他资源"),
            Self::InfrastructureUnavailable => formatter.write_str("资源租约服务不可用"),
            Self::Redis(error) => write!(formatter, "资源租约服务不可用: {error}"),
        }
    }
}

impl From<redis::RedisError> for LeaseError {
    fn from(error: redis::RedisError) -> Self {
        Self::Redis(error)
    }
}
