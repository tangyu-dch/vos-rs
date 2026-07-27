use std::time::Duration;

use super::helpers::{gateway_key, invoke_renewal, number_key, parse_lease_value};
use super::scripts::*;

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
    let renewal_result = invoke_renewal(&mut redis, &call_id, &number, &gateway, RENEWAL_TTL_SECS)
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
