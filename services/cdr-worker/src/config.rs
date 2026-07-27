use cdr_core::{DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use std::env;

pub const DEFAULT_NATS_URL: &str = "nats://127.0.0.1:4222";
pub const DEFAULT_CDR_CONSUMER: &str = "vos-rs-cdr-worker";
pub const DEFAULT_CDR_DLQ_SUBJECT: &str = "vos-rs.cdrs.dlq";
pub const DEFAULT_CDR_DLQ_STREAM: &str = "VOS_RS_CDR_DLQ";
pub const DEFAULT_BATCH_SIZE: usize = 50;
pub const DEFAULT_BATCH_TIMEOUT_MS: u64 = 100;
pub const DEFAULT_MAX_DELIVERIES: u32 = 5;
pub const DEFAULT_NAK_DELAY_MS: u64 = 1000;
pub const DEFAULT_DB_RETRY_ATTEMPTS: u32 = 3;

pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[derive(serde::Deserialize, Debug, Default)]
pub struct CdrWorkerConfig {
    pub connections: Option<ConnectionsSection>,
    pub cdr_worker: Option<CdrWorkerSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct ConnectionsSection {
    pub database: Option<DatabaseSection>,
    pub redis: Option<RedisSection>,
    pub nats: Option<NatsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct RedisSection {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
    pub database: Option<u16>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct DatabaseSection {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct NatsSection {
    pub url: Option<String>,
    pub cdr_stream: Option<String>,
    pub cdr_subject: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct CdrWorkerSection {
    pub queue: Option<QueueSection>,
    pub batch_settings: Option<BatchSettingsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct QueueSection {
    pub nats_cdr_consumer: Option<String>,
    pub nats_cdr_dlq_subject: Option<String>,
    pub nats_cdr_dlq_stream: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct BatchSettingsSection {
    pub max_batch_size: Option<usize>,
    pub batch_timeout_ms: Option<u64>,
    pub max_deliveries: Option<u32>,
    pub nak_delay_ms: Option<u64>,
    pub db_retry_attempts: Option<u32>,
}

pub struct ResolvedConfig {
    pub database_url: String,
    pub redis_url: String,
    pub nats_url: String,
    pub stream_name: String,
    pub subject: String,
    pub consumer_name: String,
    pub dlq_subject: String,
    pub dlq_stream_name: String,
    pub max_batch_size: usize,
    pub batch_timeout_ms: u64,
    pub max_deliveries: u32,
    pub nak_delay_ms: u64,
    pub db_retry_attempts: u32,
    pub max_connections: u32,
}

pub fn load_config() -> Result<ResolvedConfig, AnyError> {
    let config_file_path =
        env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    let config_content = std::fs::read_to_string(&config_file_path)
        .map_err(|error| format!("读取配置文件 {config_file_path} 失败: {error}"))?;
    let config: CdrWorkerConfig = serde_yaml::from_str(&config_content)
        .map_err(|error| format!("解析配置文件 {config_file_path} 失败: {error}"))?;

    let conn_section = config.connections.unwrap_or_default();
    let db_section = conn_section.database.unwrap_or_default();
    let nats_section = conn_section.nats.unwrap_or_default();
    let worker_section = config.cdr_worker.unwrap_or_default();
    let queue_section = worker_section.queue.unwrap_or_default();
    let batch_section = worker_section.batch_settings.unwrap_or_default();

    let database_url = build_database_url(&db_section)?;
    let redis_section = conn_section.redis.unwrap_or_default();
    let redis_url = build_redis_url(&redis_section);

    let nats_url = nats_section
        .url
        .unwrap_or_else(|| DEFAULT_NATS_URL.to_string());
    let stream_name = nats_section
        .cdr_stream
        .unwrap_or_else(|| DEFAULT_CDR_STREAM.to_string());
    let subject = nats_section
        .cdr_subject
        .unwrap_or_else(|| DEFAULT_CDR_SUBJECT.to_string());
    let consumer_name = queue_section
        .nats_cdr_consumer
        .unwrap_or_else(|| DEFAULT_CDR_CONSUMER.to_string());
    let dlq_subject = queue_section
        .nats_cdr_dlq_subject
        .unwrap_or_else(|| DEFAULT_CDR_DLQ_SUBJECT.to_string());
    let dlq_stream_name = queue_section
        .nats_cdr_dlq_stream
        .unwrap_or_else(|| DEFAULT_CDR_DLQ_STREAM.to_string());

    Ok(ResolvedConfig {
        database_url,
        redis_url,
        nats_url,
        stream_name,
        subject,
        consumer_name,
        dlq_subject,
        dlq_stream_name,
        max_batch_size: batch_section.max_batch_size.unwrap_or(DEFAULT_BATCH_SIZE),
        batch_timeout_ms: batch_section
            .batch_timeout_ms
            .unwrap_or(DEFAULT_BATCH_TIMEOUT_MS),
        max_deliveries: batch_section
            .max_deliveries
            .unwrap_or(DEFAULT_MAX_DELIVERIES),
        nak_delay_ms: batch_section.nak_delay_ms.unwrap_or(DEFAULT_NAK_DELAY_MS),
        db_retry_attempts: batch_section
            .db_retry_attempts
            .unwrap_or(DEFAULT_DB_RETRY_ATTEMPTS),
        max_connections: db_section.max_connections.unwrap_or(10),
    })
}

fn build_database_url(db: &DatabaseSection) -> Result<String, AnyError> {
    if let (Some(host), Some(port), Some(username), Some(database)) = (
        db.host.clone(),
        db.port,
        db.username.clone(),
        db.database.clone(),
    ) {
        let password = db.password.clone().unwrap_or_default();
        if password.is_empty() {
            Ok(format!("postgres://{username}@{host}:{port}/{database}"))
        } else {
            Ok(format!(
                "postgres://{username}:{password}@{host}:{port}/{database}"
            ))
        }
    } else {
        Err("PostgreSQL 数据库连接配置缺失，请检查 config.yaml".into())
    }
}

fn build_redis_url(redis: &RedisSection) -> String {
    if let (Some(host), Some(port)) = (redis.host.clone(), redis.port) {
        let password = redis.password.clone().unwrap_or_default();
        let db = redis.database.unwrap_or(0);
        if password.is_empty() {
            format!("redis://{host}:{port}/{db}")
        } else {
            format!("redis://:{password}@{host}:{port}/{db}")
        }
    } else {
        "redis://127.0.0.1:6379".to_string()
    }
}
