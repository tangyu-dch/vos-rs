//! Webhook HTTP 签名投递与 Redis 记录。

use crate::config::WebhookConfig;
use call_core::WebhookEvent;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::warn;

type AnyError = Box<dyn std::error::Error + Send + Sync>;
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize)]
struct DeliveryRecord<'a> {
    event_id: &'a str,
    call_id: &'a str,
    event_type: &'a str,
    endpoint_url: &'a str,
    attempt: u64,
    status: &'a str,
    http_status: Option<u16>,
    error: Option<&'a str>,
    updated_at_ms: i64,
    event: &'a WebhookEvent,
}

pub(crate) struct DeliveryOutcome {
    pub(crate) success: bool,
    pub(crate) retryable: bool,
    pub(crate) http_status: Option<u16>,
    pub(crate) error: Option<String>,
}

pub(crate) async fn deliver_event(
    client: &reqwest::Client,
    config: &WebhookConfig,
    event: &WebhookEvent,
) -> DeliveryOutcome {
    let body = match serde_json::to_vec(event) {
        Ok(body) => body,
        Err(error) => return DeliveryOutcome::error(error.to_string()),
    };
    let timestamp = unix_seconds().to_string();
    let signature = match sign_payload(&config.signing_secret, &timestamp, &body) {
        Ok(signature) => signature,
        Err(error) => return DeliveryOutcome::error(error.to_string()),
    };
    match client
        .post(&config.endpoint_url)
        .header("content-type", "application/json")
        .header("x-vos-webhook-id", &event.event_id)
        .header("x-vos-webhook-timestamp", timestamp)
        .header("x-vos-webhook-signature", format!("v1={signature}"))
        .body(body)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => DeliveryOutcome {
            success: true,
            retryable: false,
            http_status: Some(response.status().as_u16()),
            error: None,
        },
        Ok(response) => {
            let status = response.status();
            DeliveryOutcome {
                success: false,
                retryable: status.as_u16() == 408
                    || status.as_u16() == 429
                    || status.is_server_error(),
                http_status: Some(status.as_u16()),
                error: Some(format!("HTTP {status}")),
            }
        }
        Err(error) => DeliveryOutcome::error(error.to_string()),
    }
}

impl DeliveryOutcome {
    fn error(error: String) -> Self {
        Self {
            success: false,
            retryable: true,
            http_status: None,
            error: Some(error),
        }
    }
}

pub(crate) fn sign_payload(
    secret: &str,
    timestamp: &str,
    payload: &[u8],
) -> Result<String, AnyError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(payload);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub(crate) async fn record_delivery(
    connection: &mut redis::aio::MultiplexedConnection,
    config: &WebhookConfig,
    event: &WebhookEvent,
    attempt: u64,
    status: &str,
    outcome: &DeliveryOutcome,
) -> bool {
    let updated_at_ms = unix_millis();
    let record = DeliveryRecord {
        event_id: &event.event_id,
        call_id: &event.call_id,
        event_type: event_type(&event.event),
        endpoint_url: &config.endpoint_url,
        attempt,
        status,
        http_status: outcome.http_status,
        error: outcome.error.as_deref(),
        updated_at_ms,
        event,
    };
    let json = match serde_json::to_string(&record) {
        Ok(json) => json,
        Err(error) => {
            warn!(%error, "Webhook 投递记录序列化失败");
            return false;
        }
    };
    let key = format!("vos_rs:webhooks:delivery:{}", event.event_id);
    let ttl = i64::try_from(config.delivery_record_ttl_secs).unwrap_or(i64::MAX);
    let expired_before = updated_at_ms.saturating_sub(ttl.saturating_mul(1000));
    let result: Result<(), redis::RedisError> = redis::pipe()
        .atomic()
        .set(&key, json)
        .ignore()
        .expire(&key, ttl)
        .ignore()
        .zadd("vos_rs:webhooks:deliveries", &event.event_id, updated_at_ms)
        .ignore()
        .zrembyscore("vos_rs:webhooks:deliveries", 0, expired_before)
        .ignore()
        .query_async(connection)
        .await;
    if let Err(error) = result {
        warn!(event_id = %event.event_id, %error, "Webhook 投递记录写入 Redis 失败");
        return false;
    }
    true
}

pub(crate) fn retry_delay(base_ms: u64, attempt: u64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(6) as u32;
    Duration::from_millis(base_ms.saturating_mul(2_u64.pow(exponent)))
}

fn event_type(event: &call_core::CallEvent) -> &'static str {
    match event {
        call_core::CallEvent::CallInitiated { .. } => "call_initiated",
        call_core::CallEvent::CallOriginated { .. } => "call_originated",
        call_core::CallEvent::CallRinging { .. } => "call_ringing",
        call_core::CallEvent::CallAnswered { .. } => "call_answered",
        call_core::CallEvent::CallBridged { .. } => "call_bridged",
        call_core::CallEvent::DtmfReceived { .. } => "dtmf_received",
        call_core::CallEvent::CallFinished { .. } => "call_finished",
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_payload_is_stable() {
        let first =
            sign_payload("0123456789abcdef", "1720000000", b"payload").expect("HMAC 签名应成功");
        let second =
            sign_payload("0123456789abcdef", "1720000000", b"payload").expect("相同载荷签名应稳定");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn test_retry_delay_uses_capped_exponential_backoff() {
        assert_eq!(retry_delay(1000, 1), Duration::from_secs(1));
        assert_eq!(retry_delay(1000, 3), Duration::from_secs(4));
        assert_eq!(retry_delay(1000, 20), Duration::from_secs(64));
    }
}
