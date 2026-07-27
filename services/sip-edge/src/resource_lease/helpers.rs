use call_core::CallId;

use crate::edge_state::EdgeState;
use crate::resource_lease::error::CallResources;
use crate::resource_lease::scripts::{CALLS_KEY, CALL_EXPIRY_KEY, RENEW_SCRIPT};

pub(super) fn call_resources(edge_state: &EdgeState, call_id: &CallId) -> Option<CallResources> {
    selected_call_resources(edge_state, call_id).filter(|resources| {
        !resources.gateway_id.is_empty()
            && (resources.number_max_concurrent > 0 || resources.max_concurrent > 0)
    })
}

pub(super) fn selected_call_resources(
    edge_state: &EdgeState,
    call_id: &CallId,
) -> Option<CallResources> {
    let call = edge_state.call_manager.get(call_id)?;
    let candidate = call.candidates.get(call.current_candidate_index)?;
    Some(CallResources {
        caller_number: call
            .caller_identity
            .as_ref()
            .map(|identity| identity.presented_number.clone())
            .unwrap_or_default(),
        number_max_concurrent: call
            .caller_identity
            .as_ref()
            .map(|identity| identity.max_concurrent)
            .unwrap_or(0),
        gateway_id: candidate.target.gateway_id.as_str().to_string(),
        max_concurrent: candidate.target.max_concurrent.unwrap_or(0),
    })
    .filter(|resources| !resources.gateway_id.is_empty())
}

pub(super) fn gateway_key(gateway_id: &str) -> String {
    format!("vos_rs:{{resource-leases}}:trunk:{gateway_id}")
}

pub(super) fn number_key(number: &str) -> String {
    format!("vos_rs:{{resource-leases}}:number:{number}")
}

pub(super) fn parse_lease_value(value: &str) -> Option<(String, String)> {
    let (number, gateway) = value.split_once('\u{1f}')?;
    (!gateway.is_empty()).then(|| (number.to_string(), gateway.to_string()))
}

pub(super) async fn invoke_renewal<C>(
    connection: &mut C,
    call_id: &str,
    caller_number: &str,
    gateway_id: &str,
    ttl_secs: u64,
) -> Result<i64, redis::RedisError>
where
    C: redis::aio::ConnectionLike + Send,
{
    redis::Script::new(RENEW_SCRIPT)
        .key(CALLS_KEY)
        .key(CALL_EXPIRY_KEY)
        .key(number_key(caller_number))
        .key(gateway_key(gateway_id))
        .arg(ttl_secs)
        .arg(call_id)
        .arg(caller_number)
        .arg(gateway_id)
        .invoke_async(connection)
        .await
}
