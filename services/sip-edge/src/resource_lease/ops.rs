use std::{sync::Arc, time::Duration};

use call_core::CallId;

use crate::edge_state::EdgeState;
use crate::resource_lease::error::{CallResources, LeaseError};
use crate::resource_lease::helpers::{
    call_resources, gateway_key, invoke_renewal, number_key, parse_lease_value,
};
use crate::resource_lease::scripts::{
    ACQUIRE_SCRIPT, CALLS_KEY, CALL_EXPIRY_KEY, RELEASE_SCRIPT, RENEWAL_INTERVAL_SECS,
    RENEWAL_TTL_SECS,
};

/// Atomically leases the selected managed caller number and egress capacity.
pub(crate) async fn acquire(
    edge_state: &EdgeState,
    call_id: &CallId,
    _max_duration_secs: Option<u32>,
) -> Result<(), LeaseError> {
    let Some(resources) = call_resources(edge_state, call_id) else {
        return Ok(());
    };
    let Some(mut connection) = edge_state.redis_connection() else {
        return Err(LeaseError::InfrastructureUnavailable);
    };
    let result: i64 = redis::Script::new(ACQUIRE_SCRIPT)
        .key(CALLS_KEY)
        .key(CALL_EXPIRY_KEY)
        .key(number_key(&resources.caller_number))
        .key(gateway_key(&resources.gateway_id))
        .arg(RENEWAL_TTL_SECS)
        .arg(call_id.as_str())
        .arg(&resources.caller_number)
        .arg(&resources.gateway_id)
        .arg(resources.number_max_concurrent)
        .arg(resources.max_concurrent)
        .invoke_async(&mut connection)
        .await?;
    match result {
        1 => Ok(()),
        -1 => Err(LeaseError::NumberBusy),
        -2 => Err(LeaseError::TrunkAtCapacity),
        -3 => Err(LeaseError::CallConflict),
        _ => Err(LeaseError::Redis(redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "unexpected resource lease response",
        )))),
    }
}

/// Managed capacity can only be represented by a single resource snapshot per Call-ID.
pub(crate) fn requires_single_leg(edge_state: &EdgeState, call_id: &CallId) -> bool {
    edge_state.call_manager.get(call_id).is_some_and(|call| {
        call.caller_identity
            .as_ref()
            .is_some_and(|identity| identity.max_concurrent > 0)
            || call
                .candidates
                .iter()
                .any(|candidate| candidate.target.max_concurrent.unwrap_or(0) > 0)
    })
}

/// Renews a live lease only when its Call-ID still owns the same resource snapshot.
async fn renew_with_connection<C>(
    connection: &mut C,
    call_id: &CallId,
    resources: &CallResources,
) -> Result<bool, LeaseError>
where
    C: redis::aio::ConnectionLike + Send,
{
    let result = invoke_renewal(
        connection,
        call_id.as_str(),
        &resources.caller_number,
        &resources.gateway_id,
        RENEWAL_TTL_SECS,
    )
    .await?;
    match result {
        1 => Ok(true),
        0 | -4 => Ok(false),
        -3 => Err(LeaseError::CallConflict),
        _ => Err(LeaseError::Redis(redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "unexpected resource lease renewal response",
        )))),
    }
}

/// Keeps resource capacity reserved for calls that outlive their initial lease TTL.
pub(crate) fn spawn_renewal_loop(edge_state: Arc<EdgeState>) {
    let interval_duration = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(RENEWAL_INTERVAL_SECS)
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(interval_duration);
        interval.tick().await;
        loop {
            interval.tick().await;
            let Some(mut connection) = edge_state.redis_connection() else {
                tracing::warn!("resource lease renewal skipped because Redis is unavailable");
                continue;
            };
            let call_ids = edge_state
                .inbound_transactions
                .iter()
                .map(|entry| CallId::new(entry.key().clone()))
                .collect::<Vec<_>>();
            for call_id in call_ids {
                let Some(resources) = call_resources(&edge_state, &call_id) else {
                    continue;
                };
                if let Err(error) =
                    renew_with_connection(&mut connection, &call_id, &resources).await
                {
                    tracing::warn!(call_id = %call_id.as_str(), %error, "failed to renew call resource lease");
                }
            }
        }
    });
}

/// Releases only resources still owned by this Call-ID. Repeated release is harmless.
pub(crate) fn release(edge_state: &EdgeState, call_id: &CallId) {
    let Some(mut connection) = edge_state.redis_connection() else {
        return;
    };
    let call_id = call_id.as_str().to_string();
    tokio::spawn(async move {
        let lease_value: Result<Option<String>, redis::RedisError> = redis::cmd("HGET")
            .arg(CALLS_KEY)
            .arg(&call_id)
            .query_async(&mut connection)
            .await;
        let lease_value = match lease_value {
            Ok(Some(value)) => value,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%call_id, %error, "failed to load call resource lease");
                return;
            }
        };
        let Some((caller_number, gateway_id)) = parse_lease_value(&lease_value) else {
            tracing::warn!(%call_id, "invalid call resource lease snapshot");
            return;
        };
        let result: Result<i64, redis::RedisError> = redis::Script::new(RELEASE_SCRIPT)
            .key(CALLS_KEY)
            .key(CALL_EXPIRY_KEY)
            .key(number_key(&caller_number))
            .key(gateway_key(&gateway_id))
            .arg(&call_id)
            .arg(&caller_number)
            .arg(&gateway_id)
            .invoke_async(&mut connection)
            .await;
        if let Err(error) = result {
            tracing::warn!(%call_id, %error, "failed to release call resource lease");
        }
    });
}
