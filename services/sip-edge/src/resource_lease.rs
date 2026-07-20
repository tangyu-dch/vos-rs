use std::{sync::Arc, time::Duration};

use call_core::CallId;

use crate::edge_state::EdgeState;

const RENEWAL_INTERVAL_SECS: u64 = 60;
const RENEWAL_TTL_SECS: u64 = 300;

const ACQUIRE_SCRIPT: &str = r#"
local redis_time = redis.call('TIME')
local now = tonumber(redis_time[1])
local expires = now + tonumber(ARGV[1])
local call_id = ARGV[2]
local caller_number = ARGV[3]
local gateway_id = ARGV[4]
local number_max_concurrent = tonumber(ARGV[5])
local trunk_max_concurrent = tonumber(ARGV[6])
local lease_value = caller_number .. '\31' .. gateway_id

local expired_calls = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', now)
for _, expired_call in ipairs(expired_calls) do
  redis.call('HDEL', KEYS[1], expired_call)
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)

redis.call('ZREMRANGEBYSCORE', KEYS[3], '-inf', now)
redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', now)

local existing = redis.call('HGET', KEYS[1], call_id)
if existing then
  if existing ~= lease_value then
    return -3
  end
  local call_expiry = tonumber(redis.call('ZSCORE', KEYS[2], call_id)) or 0
  local number_expiry = caller_number == '' and call_expiry or tonumber(redis.call('ZSCORE', KEYS[3], call_id)) or 0
  local trunk_expiry = tonumber(redis.call('ZSCORE', KEYS[4], call_id)) or 0
  expires = math.max(expires, call_expiry, number_expiry, trunk_expiry)
else
  if caller_number ~= '' and number_max_concurrent > 0 and redis.call('ZCARD', KEYS[3]) >= number_max_concurrent then
    return -1
  end
  if trunk_max_concurrent > 0 and redis.call('ZCARD', KEYS[4]) >= trunk_max_concurrent then
    return -2
  end
end

redis.call('HSET', KEYS[1], call_id, lease_value)
redis.call('ZADD', KEYS[2], expires, call_id)
if caller_number ~= '' then
  redis.call('ZADD', KEYS[3], expires, call_id)
end
redis.call('ZADD', KEYS[4], expires, call_id)
return 1
"#;

const RELEASE_SCRIPT: &str = r#"
local call_id = ARGV[1]
local caller_number = ARGV[2]
local gateway_id = ARGV[3]
local lease_value = caller_number .. '\31' .. gateway_id
if redis.call('HGET', KEYS[1], call_id) ~= lease_value then
  return 0
end
redis.call('HDEL', KEYS[1], call_id)
redis.call('ZREM', KEYS[2], call_id)
if caller_number ~= '' then
  redis.call('ZREM', KEYS[3], call_id)
end
redis.call('ZREM', KEYS[4], call_id)
return 1
"#;

const RENEW_SCRIPT: &str = r#"
local redis_time = redis.call('TIME')
local now = tonumber(redis_time[1])
local expires = now + tonumber(ARGV[1])
local call_id = ARGV[2]
local caller_number = ARGV[3]
local gateway_id = ARGV[4]
local lease_value = caller_number .. '\31' .. gateway_id

local existing = redis.call('HGET', KEYS[1], call_id)
if not existing then
  return 0
end
if existing ~= lease_value then
  return -3
end

local call_expiry = tonumber(redis.call('ZSCORE', KEYS[2], call_id))
local number_expiry = caller_number == '' and call_expiry or tonumber(redis.call('ZSCORE', KEYS[3], call_id))
local trunk_expiry = tonumber(redis.call('ZSCORE', KEYS[4], call_id))
if not call_expiry or not number_expiry or not trunk_expiry or call_expiry <= now or number_expiry <= now or trunk_expiry <= now then
  redis.call('HDEL', KEYS[1], call_id)
  redis.call('ZREM', KEYS[2], call_id)
  if caller_number ~= '' then
    redis.call('ZREM', KEYS[3], call_id)
  end
  redis.call('ZREM', KEYS[4], call_id)
  return -4
end

local renewal_expiry = math.max(expires, call_expiry, number_expiry, trunk_expiry)
redis.call('ZADD', KEYS[2], renewal_expiry, call_id)
if caller_number ~= '' then
  redis.call('ZADD', KEYS[3], renewal_expiry, call_id)
end
redis.call('ZADD', KEYS[4], renewal_expiry, call_id)
return 1
"#;

const MIGRATE_SCRIPT: &str = r#"
local redis_time = redis.call('TIME')
local now = tonumber(redis_time[1])
local call_id = ARGV[1]
local old_caller_number = ARGV[2]
local old_gateway_id = ARGV[3]
local new_caller_number = ARGV[4]
local new_gateway_id = ARGV[5]
local number_max_concurrent = tonumber(ARGV[6])
local trunk_max_concurrent = tonumber(ARGV[7])
local old_lease_value = old_caller_number .. '\31' .. old_gateway_id
local new_lease_value = new_caller_number .. '\31' .. new_gateway_id

if redis.call('HGET', KEYS[1], call_id) ~= old_lease_value then
  return -3
end

local call_expiry = tonumber(redis.call('ZSCORE', KEYS[2], call_id))
if not call_expiry or call_expiry <= now then
  redis.call('HDEL', KEYS[1], call_id)
  redis.call('ZREM', KEYS[2], call_id)
  if old_caller_number ~= '' then
    redis.call('ZREM', KEYS[3], call_id)
  end
  redis.call('ZREM', KEYS[4], call_id)
  return -4
end

redis.call('ZREMRANGEBYSCORE', KEYS[5], '-inf', now)
redis.call('ZREMRANGEBYSCORE', KEYS[6], '-inf', now)

if new_caller_number ~= '' and number_max_concurrent > 0 then
  local number_count = redis.call('ZCARD', KEYS[5])
  if redis.call('ZSCORE', KEYS[5], call_id) then
    number_count = number_count - 1
  end
  if number_count >= number_max_concurrent then
    return -1
  end
end
if trunk_max_concurrent > 0 then
  local trunk_count = redis.call('ZCARD', KEYS[6])
  if redis.call('ZSCORE', KEYS[6], call_id) then
    trunk_count = trunk_count - 1
  end
  if trunk_count >= trunk_max_concurrent then
    return -2
  end
end

if old_caller_number ~= '' then
  redis.call('ZREM', KEYS[3], call_id)
end
redis.call('ZREM', KEYS[4], call_id)
redis.call('HSET', KEYS[1], call_id, new_lease_value)
if new_caller_number ~= '' then
  redis.call('ZADD', KEYS[5], call_expiry, call_id)
end
redis.call('ZADD', KEYS[6], call_expiry, call_id)
return 1
"#;

const CALLS_KEY: &str = "vos_rs:{resource-leases}:calls";
const CALL_EXPIRY_KEY: &str = "vos_rs:{resource-leases}:call-expiry";
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallResources {
    caller_number: String,
    number_max_concurrent: u32,
    gateway_id: String,
    max_concurrent: u32,
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

/// Atomically moves an existing lease to the route currently selected by the call manager.
pub(crate) async fn migrate_to_current(
    edge_state: &EdgeState,
    call_id: &CallId,
    previous_gateway_id: &str,
) -> Result<(), LeaseError> {
    if !current_candidate_matches_caller_owner(edge_state, call_id) {
        return Err(LeaseError::CallConflict);
    }
    let Some(new_resources) = selected_call_resources(edge_state, call_id) else {
        return Ok(());
    };
    let previous_managed = gateway_was_managed(edge_state, call_id, previous_gateway_id);
    let new_managed = new_resources.number_max_concurrent > 0 || new_resources.max_concurrent > 0;
    if !migration_requires_redis(previous_managed, new_managed) {
        return Ok(());
    }
    let Some(mut connection) = edge_state.redis_connection() else {
        return Err(LeaseError::InfrastructureUnavailable);
    };
    let lease_value: Option<String> = redis::cmd("HGET")
        .arg(CALLS_KEY)
        .arg(call_id.as_str())
        .query_async(&mut connection)
        .await?;
    let Some((old_caller_number, old_gateway_id)) =
        lease_value.as_deref().and_then(parse_lease_value)
    else {
        drop(connection);
        return if new_managed && !previous_managed {
            acquire(edge_state, call_id, None).await
        } else {
            Err(LeaseError::CallConflict)
        };
    };
    if old_caller_number == new_resources.caller_number
        && old_gateway_id == new_resources.gateway_id
    {
        return Ok(());
    }

    let result: i64 = redis::Script::new(MIGRATE_SCRIPT)
        .key(CALLS_KEY)
        .key(CALL_EXPIRY_KEY)
        .key(number_key(&old_caller_number))
        .key(gateway_key(&old_gateway_id))
        .key(number_key(&new_resources.caller_number))
        .key(gateway_key(&new_resources.gateway_id))
        .arg(call_id.as_str())
        .arg(&old_caller_number)
        .arg(&old_gateway_id)
        .arg(&new_resources.caller_number)
        .arg(&new_resources.gateway_id)
        .arg(new_resources.number_max_concurrent)
        .arg(new_resources.max_concurrent)
        .invoke_async(&mut connection)
        .await?;
    match result {
        1 => Ok(()),
        -1 => Err(LeaseError::NumberBusy),
        -2 => Err(LeaseError::TrunkAtCapacity),
        -3 => Err(LeaseError::CallConflict),
        -4 => Err(LeaseError::InfrastructureUnavailable),
        _ => Err(LeaseError::Redis(redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "unexpected resource lease migration response",
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

fn call_resources(edge_state: &EdgeState, call_id: &CallId) -> Option<CallResources> {
    selected_call_resources(edge_state, call_id).filter(|resources| {
        !resources.gateway_id.is_empty()
            && (resources.number_max_concurrent > 0 || resources.max_concurrent > 0)
    })
}

fn selected_call_resources(edge_state: &EdgeState, call_id: &CallId) -> Option<CallResources> {
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

fn current_candidate_matches_caller_owner(edge_state: &EdgeState, call_id: &CallId) -> bool {
    edge_state.call_manager.get(call_id).is_some_and(|call| {
        let Some(identity) = call.caller_identity.as_ref() else {
            return true;
        };
        call.candidates
            .get(call.current_candidate_index)
            .is_some_and(|candidate| candidate.target.gateway_id == identity.owner_gateway_id)
    })
}

fn gateway_was_managed(edge_state: &EdgeState, call_id: &CallId, gateway_id: &str) -> bool {
    edge_state.call_manager.get(call_id).is_some_and(|call| {
        call.caller_identity
            .as_ref()
            .is_some_and(|identity| identity.max_concurrent > 0)
            || call.candidates.iter().any(|candidate| {
                candidate.target.gateway_id.as_str() == gateway_id
                    && candidate.target.max_concurrent.unwrap_or(0) > 0
            })
    })
}

fn migration_requires_redis(previous_managed: bool, new_managed: bool) -> bool {
    previous_managed || new_managed
}

fn gateway_key(gateway_id: &str) -> String {
    format!("vos_rs:{{resource-leases}}:trunk:{gateway_id}")
}

fn number_key(number: &str) -> String {
    format!("vos_rs:{{resource-leases}}:number:{number}")
}

fn parse_lease_value(value: &str) -> Option<(String, String)> {
    let (number, gateway) = value.split_once('\u{1f}')?;
    (!gateway.is_empty()).then(|| (number.to_string(), gateway.to_string()))
}

async fn invoke_renewal<C>(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_uses_short_renewable_ttl() {
        assert_eq!(RENEWAL_INTERVAL_SECS, 60);
        assert_eq!(RENEWAL_TTL_SECS, 300);
    }

    #[test]
    fn gateway_keys_share_the_cluster_hash_slot() {
        assert!(gateway_key("gw-a").contains("{resource-leases}"));
        assert!(gateway_key("gw-b").contains("{resource-leases}"));
        assert!(number_key("13800138000").contains("{resource-leases}"));
    }

    #[test]
    fn lease_value_preserves_empty_number_and_gateway() {
        assert_eq!(
            parse_lease_value("\u{1f}gw-a"),
            Some((String::new(), "gw-a".to_string()))
        );
        assert_eq!(parse_lease_value("number-only"), None);
    }

    #[test]
    fn unmanaged_failover_does_not_require_redis() {
        assert!(!migration_requires_redis(false, false));
        assert!(migration_requires_redis(true, false));
        assert!(migration_requires_redis(false, true));
    }

    #[tokio::test]
    async fn redis_lease_is_idempotent_capacity_bounded_and_releasable() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_one = format!("call-one-{suffix}");
        let call_two = format!("call-two-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_one, &number, &gateway, 1, 1, 30).await,
            1
        );
        let initial_expiry = expiry_score(&mut redis, &call_one).await;
        assert_eq!(
            invoke_acquire(&mut redis, &call_one, &number, &gateway, 1, 1, 1).await,
            1
        );
        assert_eq!(expiry_score(&mut redis, &call_one).await, initial_expiry);
        assert_eq!(
            invoke_acquire(&mut redis, &call_two, &number, &gateway, 1, 1, 30).await,
            -1
        );
        let other_number = format!("other-{number}");
        assert_eq!(
            invoke_acquire(&mut redis, &call_two, &other_number, &gateway, 0, 1, 30,).await,
            -2
        );
        assert_eq!(
            invoke_release(&mut redis, &call_one, &number, &gateway).await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &call_one, &number, &gateway).await,
            0
        );
        assert_eq!(
            invoke_acquire(&mut redis, &call_two, &number, &gateway, 1, 1, 30).await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &call_two, &number, &gateway).await,
            1
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_expiry_is_based_on_redis_time() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_id = format!("clock-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        let before = redis_epoch_secs(&mut redis).await;
        assert_eq!(
            invoke_acquire(
                &mut redis,
                &call_id,
                &number,
                &gateway,
                1,
                1,
                RENEWAL_TTL_SECS,
            )
            .await,
            1
        );
        let after = redis_epoch_secs(&mut redis).await;
        let expiry = expiry_score(&mut redis, &call_id)
            .await
            .expect("lease should have an expiry score");
        assert!(expiry >= before + RENEWAL_TTL_SECS);
        assert!(expiry <= after + RENEWAL_TTL_SECS);

        assert_eq!(
            invoke_release(&mut redis, &call_id, &number, &gateway).await,
            1
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_capacity_recovers_after_ttl() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_one = format!("ttl-one-{suffix}");
        let call_two = format!("ttl-two-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_one, &number, &gateway, 1, 1, 1).await,
            1
        );
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        assert_eq!(
            invoke_acquire(&mut redis, &call_two, &number, &gateway, 1, 1, 30).await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &call_two, &number, &gateway).await,
            1
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_renewal_requires_the_original_call_owner() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let owner = format!("owner-{suffix}");
        let other = format!("other-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &owner, &number, &gateway, 1, 1, 30).await,
            1
        );
        let initial_expiry = expiry_score(&mut redis, &owner).await;
        assert_eq!(
            invoke_renewal(&mut redis, &other, &number, &gateway, RENEWAL_TTL_SECS)
                .await
                .expect("renewal script should execute"),
            0
        );
        assert_eq!(expiry_score(&mut redis, &owner).await, initial_expiry);

        let wrong_gateway = format!("wrong-{gateway}");
        assert_eq!(
            invoke_renewal(
                &mut redis,
                &owner,
                &number,
                &wrong_gateway,
                RENEWAL_TTL_SECS,
            )
            .await
            .expect("renewal script should execute"),
            -3
        );
        assert_eq!(expiry_score(&mut redis, &owner).await, initial_expiry);
        assert_eq!(
            invoke_release(&mut redis, &owner, &number, &gateway).await,
            1
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_renewal_never_shortens_an_existing_lease() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_id = format!("long-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_id, &number, &gateway, 1, 1, 3_600).await,
            1
        );
        let initial_expiry = expiry_score(&mut redis, &call_id).await;
        assert_eq!(
            invoke_renewal(&mut redis, &call_id, &number, &gateway, RENEWAL_TTL_SECS,)
                .await
                .expect("renewal script should execute"),
            1
        );
        assert_eq!(expiry_score(&mut redis, &call_id).await, initial_expiry);

        assert_eq!(
            invoke_release(&mut redis, &call_id, &number, &gateway).await,
            1
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_renewal_does_not_revive_an_expired_or_released_lease() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_id = format!("expired-{suffix}");
        let number = format!("number-{suffix}");
        let gateway = format!("gateway-{suffix}");
        cleanup(&mut redis, &number, &gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_id, &number, &gateway, 1, 1, 1).await,
            1
        );
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        let renewal_result =
            invoke_renewal(&mut redis, &call_id, &number, &gateway, RENEWAL_TTL_SECS)
                .await
                .expect("renewal script should execute");
        assert!(matches!(renewal_result, 0 | -4));
        assert_eq!(expiry_score(&mut redis, &call_id).await, None);
        assert_eq!(
            invoke_release(&mut redis, &call_id, &number, &gateway).await,
            0
        );
        assert_eq!(
            invoke_renewal(&mut redis, &call_id, &number, &gateway, RENEWAL_TTL_SECS)
                .await
                .expect("renewal script should execute"),
            0
        );
        cleanup(&mut redis, &number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_counts_multiple_number_and_trunk_slots() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let gateway = format!("gateway-{suffix}");
        let calls = (1..=3)
            .map(|index| format!("call-{index}-{suffix}"))
            .collect::<Vec<_>>();
        let shared_number = format!("shared-{suffix}");
        cleanup(&mut redis, &shared_number, &gateway).await;

        for call in calls.iter().take(2) {
            assert_eq!(
                invoke_acquire(&mut redis, call, &shared_number, &gateway, 2, 0, 30).await,
                1
            );
        }
        assert_eq!(
            invoke_acquire(&mut redis, &calls[2], &shared_number, &gateway, 2, 0, 30,).await,
            -1
        );
        for call in calls.iter().take(2) {
            assert_eq!(
                invoke_release(&mut redis, call, &shared_number, &gateway).await,
                1
            );
        }

        let numbers = (1..=3)
            .map(|index| format!("number-{index}-{suffix}"))
            .collect::<Vec<_>>();
        for (call, number) in calls.iter().zip(numbers.iter()).take(2) {
            assert_eq!(
                invoke_acquire(&mut redis, call, number, &gateway, 0, 2, 30).await,
                1
            );
        }
        assert_eq!(
            invoke_acquire(&mut redis, &calls[2], &numbers[2], &gateway, 0, 2, 30,).await,
            -2
        );
        for (call, number) in calls.iter().zip(numbers.iter()).take(2) {
            assert_eq!(invoke_release(&mut redis, call, number, &gateway).await, 1);
            cleanup(&mut redis, number, &gateway).await;
        }
        cleanup(&mut redis, &shared_number, &gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_migration_moves_number_and_trunk_atomically() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_id = format!("migrate-{suffix}");
        let old_number = format!("old-number-{suffix}");
        let old_gateway = format!("old-gateway-{suffix}");
        let new_number = format!("new-number-{suffix}");
        let new_gateway = format!("new-gateway-{suffix}");
        cleanup(&mut redis, &old_number, &old_gateway).await;
        cleanup(&mut redis, &new_number, &new_gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_id, &old_number, &old_gateway, 1, 1, 30,).await,
            1
        );
        let initial_expiry = expiry_score(&mut redis, &call_id).await;
        assert_eq!(
            invoke_migration(
                &mut redis,
                &call_id,
                &old_number,
                &old_gateway,
                &new_number,
                &new_gateway,
                0,
                0,
            )
            .await,
            1
        );
        let snapshot: Option<String> = redis::cmd("HGET")
            .arg(CALLS_KEY)
            .arg(&call_id)
            .query_async(&mut redis)
            .await
            .expect("lease snapshot should be readable");
        assert_eq!(snapshot, Some(format!("{new_number}\u{1f}{new_gateway}")));
        assert_eq!(expiry_score(&mut redis, &call_id).await, initial_expiry);
        assert_eq!(
            invoke_release(&mut redis, &call_id, &old_number, &old_gateway).await,
            0
        );
        let old_slot_probe = format!("old-slot-probe-{suffix}");
        assert_eq!(
            invoke_acquire(
                &mut redis,
                &old_slot_probe,
                &old_number,
                &old_gateway,
                1,
                1,
                30,
            )
            .await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &old_slot_probe, &old_number, &old_gateway,).await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &call_id, &new_number, &new_gateway).await,
            1
        );
        cleanup(&mut redis, &old_number, &old_gateway).await;
        cleanup(&mut redis, &new_number, &new_gateway).await;
    }

    #[tokio::test]
    async fn redis_lease_migration_keeps_old_owner_when_new_number_is_full() {
        let Some(mut redis) = test_redis().await else {
            return;
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let call_id = format!("migrate-call-{suffix}");
        let blocker = format!("migrate-blocker-{suffix}");
        let old_number = format!("old-number-{suffix}");
        let old_gateway = format!("old-gateway-{suffix}");
        let new_number = format!("new-number-{suffix}");
        let new_gateway = format!("new-gateway-{suffix}");
        cleanup(&mut redis, &old_number, &old_gateway).await;
        cleanup(&mut redis, &new_number, &new_gateway).await;

        assert_eq!(
            invoke_acquire(&mut redis, &call_id, &old_number, &old_gateway, 1, 1, 30,).await,
            1
        );
        assert_eq!(
            invoke_acquire(&mut redis, &blocker, &new_number, &new_gateway, 1, 0, 30,).await,
            1
        );
        assert_eq!(
            invoke_migration(
                &mut redis,
                &call_id,
                &old_number,
                &old_gateway,
                &new_number,
                &new_gateway,
                1,
                1,
            )
            .await,
            -1
        );
        let snapshot: Option<String> = redis::cmd("HGET")
            .arg(CALLS_KEY)
            .arg(&call_id)
            .query_async(&mut redis)
            .await
            .expect("lease snapshot should be readable");
        assert_eq!(snapshot, Some(format!("{old_number}\u{1f}{old_gateway}")));
        assert_eq!(
            invoke_release(&mut redis, &call_id, &old_number, &old_gateway).await,
            1
        );
        assert_eq!(
            invoke_release(&mut redis, &blocker, &new_number, &new_gateway).await,
            1
        );
        cleanup(&mut redis, &old_number, &old_gateway).await;
        cleanup(&mut redis, &new_number, &new_gateway).await;
    }

    async fn test_redis() -> Option<redis::aio::ConnectionManager> {
        let client = redis::Client::open("redis://127.0.0.1:6379").ok()?;
        redis::aio::ConnectionManager::new(client).await.ok()
    }

    async fn invoke_acquire(
        redis: &mut redis::aio::ConnectionManager,
        call_id: &str,
        number: &str,
        gateway: &str,
        number_capacity: u32,
        trunk_capacity: u32,
        ttl_secs: u64,
    ) -> i64 {
        redis::Script::new(ACQUIRE_SCRIPT)
            .key(CALLS_KEY)
            .key(CALL_EXPIRY_KEY)
            .key(number_key(number))
            .key(gateway_key(gateway))
            .arg(ttl_secs)
            .arg(call_id)
            .arg(number)
            .arg(gateway)
            .arg(number_capacity)
            .arg(trunk_capacity)
            .invoke_async(redis)
            .await
            .expect("lease script should execute")
    }

    async fn invoke_release(
        redis: &mut redis::aio::ConnectionManager,
        call_id: &str,
        number: &str,
        gateway: &str,
    ) -> i64 {
        redis::Script::new(RELEASE_SCRIPT)
            .key(CALLS_KEY)
            .key(CALL_EXPIRY_KEY)
            .key(number_key(number))
            .key(gateway_key(gateway))
            .arg(call_id)
            .arg(number)
            .arg(gateway)
            .invoke_async(redis)
            .await
            .expect("release script should execute")
    }

    #[allow(clippy::too_many_arguments)]
    async fn invoke_migration(
        redis: &mut redis::aio::ConnectionManager,
        call_id: &str,
        old_number: &str,
        old_gateway: &str,
        new_number: &str,
        new_gateway: &str,
        number_capacity: u32,
        trunk_capacity: u32,
    ) -> i64 {
        redis::Script::new(MIGRATE_SCRIPT)
            .key(CALLS_KEY)
            .key(CALL_EXPIRY_KEY)
            .key(number_key(old_number))
            .key(gateway_key(old_gateway))
            .key(number_key(new_number))
            .key(gateway_key(new_gateway))
            .arg(call_id)
            .arg(old_number)
            .arg(old_gateway)
            .arg(new_number)
            .arg(new_gateway)
            .arg(number_capacity)
            .arg(trunk_capacity)
            .invoke_async(redis)
            .await
            .expect("migration script should execute")
    }

    async fn expiry_score(redis: &mut redis::aio::ConnectionManager, call_id: &str) -> Option<u64> {
        redis::cmd("ZSCORE")
            .arg(CALL_EXPIRY_KEY)
            .arg(call_id)
            .query_async(redis)
            .await
            .expect("expiry score should be readable")
    }

    async fn redis_epoch_secs(redis: &mut redis::aio::ConnectionManager) -> u64 {
        let time: Vec<String> = redis::cmd("TIME")
            .query_async(redis)
            .await
            .expect("Redis time should be readable");
        time.first()
            .and_then(|seconds| seconds.parse().ok())
            .expect("Redis time should contain epoch seconds")
    }

    async fn cleanup(redis: &mut redis::aio::ConnectionManager, number: &str, gateway: &str) {
        let _: Result<(), redis::RedisError> = redis::cmd("DEL")
            .arg(number_key(number))
            .arg(gateway_key(gateway))
            .query_async(redis)
            .await;
    }
}
