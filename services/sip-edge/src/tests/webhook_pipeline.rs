use super::*;
use crate::webhook_delivery::sign_payload;
use axum::{body::Bytes, http::HeaderMap, routing::post, Router};
use call_core::{CallEvent, WEBHOOK_SCHEMA_VERSION};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
#[ignore = "需要本机运行 NATS 4222 和 Redis 6379"]
async fn test_pipeline_delivers_signed_event_and_records_result() {
    let (request_tx, mut request_rx) = mpsc::channel(1);
    let app = Router::new().route(
        "/calls",
        post(move |headers: HeaderMap, body: Bytes| {
            let request_tx = request_tx.clone();
            async move {
                let _ = request_tx.send((headers, body)).await;
                axum::http::StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试 HTTP 监听应成功");
    let address = listener.local_addr().expect("测试监听地址应存在");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let secret = "0123456789abcdef";
    let config = WebhookConfig {
        enabled: true,
        endpoint_url: format!("http://{address}/calls"),
        signing_secret: secret.to_string(),
        stream: format!("WEBHOOK_TEST_{suffix}"),
        subject: format!("vos_rs.webhooks.test.{suffix}"),
        consumer: format!("webhook_test_{suffix}"),
        ..WebhookConfig::default()
    };
    let redis_client =
        redis::Client::open("redis://127.0.0.1:6379").expect("测试 Redis URL 应有效");
    let (event_tx, event_rx) = mpsc::channel(4);
    start_pipeline(
        config.clone(),
        "nats://127.0.0.1:4222",
        redis_client.clone(),
        event_rx,
    )
    .await
    .expect("Webhook 流水线应启动");

    let event = WebhookEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
        call_id: "pipeline-test-call".to_string(),
        sequence: 1,
        occurred_at_ms: 1,
        event: CallEvent::CallAnswered { sip_status: 200, leg: "b_leg".to_string() },
    };
    event_tx.send(event.clone()).await.expect("事件应进入队列");
    let (headers, body) = tokio::time::timeout(Duration::from_secs(5), request_rx.recv())
        .await
        .expect("HTTP 投递不应超时")
        .expect("应收到 HTTP 请求");
    verify_signed_request(secret, &event, &headers, &body);

    let key = format!("vos_rs:webhooks:delivery:{}", event.event_id);
    let record = wait_for_delivery_record(&redis_client, &key, "delivered").await;
    assert!(record.contains("\"status\":\"delivered\""));
    cleanup(&config, &redis_client, &event.event_id, key).await;
}

#[tokio::test]
#[ignore = "需要本机运行 NATS 4222 和 Redis 6379"]
async fn test_pipeline_retries_server_error_until_delivery_succeeds() {
    let (endpoint_url, attempts) = spawn_retry_receiver().await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let config = WebhookConfig {
        enabled: true,
        endpoint_url,
        signing_secret: "0123456789abcdef".to_string(),
        stream: format!("WEBHOOK_TEST_{suffix}"),
        subject: format!("vos_rs.webhooks.test.{suffix}"),
        consumer: format!("webhook_test_{suffix}"),
        retry_delay_ms: 20,
        max_deliveries: 3,
        ..WebhookConfig::default()
    };
    let redis_client =
        redis::Client::open("redis://127.0.0.1:6379").expect("测试 Redis URL 应有效");
    let (event_tx, event_rx) = mpsc::channel(4);
    start_pipeline(
        config.clone(),
        "nats://127.0.0.1:4222",
        redis_client.clone(),
        event_rx,
    )
    .await
    .expect("Webhook 流水线应启动");
    let event = test_event("pipeline-retry-call");
    event_tx.send(event.clone()).await.expect("事件应进入队列");
    let key = format!("vos_rs:webhooks:delivery:{}", event.event_id);
    let record = wait_for_delivery_record(&redis_client, &key, "delivered").await;
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert!(record.contains("\"attempt\":3"));
    cleanup(&config, &redis_client, &event.event_id, key).await;
}

fn verify_signed_request(secret: &str, event: &WebhookEvent, headers: &HeaderMap, body: &[u8]) {
    let timestamp = headers["x-vos-webhook-timestamp"]
        .to_str()
        .expect("时间戳应为文本");
    let expected = format!(
        "v1={}",
        sign_payload(secret, timestamp, body).expect("测试签名应成功")
    );
    assert_eq!(headers["x-vos-webhook-signature"], expected);
    assert_eq!(
        serde_json::from_slice::<WebhookEvent>(body).ok(),
        Some(event.clone())
    );
}

async fn wait_for_delivery_record(client: &redis::Client, key: &str, status: &str) -> String {
    let mut connection = client
        .get_multiplexed_tokio_connection()
        .await
        .expect("测试 Redis 应可连接");
    for _ in 0..100 {
        let record: Option<String> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut connection)
            .await
            .expect("读取投递记录应成功");
        if let Some(record) = record {
            if record.contains(&format!("\"status\":\"{status}\"")) {
                return record;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("未找到 Webhook 投递记录");
}

async fn spawn_retry_receiver() -> (String, Arc<AtomicUsize>) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let handler_attempts = Arc::clone(&attempts);
    let app = Router::new().route(
        "/calls",
        post(move || {
            let current = handler_attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if current < 2 {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    axum::http::StatusCode::NO_CONTENT
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试 HTTP 监听应成功");
    let address = listener.local_addr().expect("测试监听地址应存在");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}/calls"), attempts)
}

fn test_event(call_id: &str) -> WebhookEvent {
    WebhookEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
        call_id: call_id.to_string(),
        sequence: 1,
        occurred_at_ms: 1,
        event: CallEvent::CallAnswered { sip_status: 200, leg: "b_leg".to_string() },
    }
}

async fn cleanup(
    config: &WebhookConfig,
    redis_client: &redis::Client,
    event_id: &str,
    key: String,
) {
    let nats = async_nats::connect("nats://127.0.0.1:4222")
        .await
        .expect("测试 NATS 应可连接");
    let _ = jetstream::new(nats)
        .delete_stream(config.stream.clone())
        .await;
    let mut redis = redis_client
        .get_multiplexed_tokio_connection()
        .await
        .expect("测试 Redis 应可连接");
    let _: Result<(), redis::RedisError> = redis::pipe()
        .del(key)
        .ignore()
        .zrem("vos_rs:webhooks:deliveries", event_id)
        .ignore()
        .query_async(&mut redis)
        .await;
}
