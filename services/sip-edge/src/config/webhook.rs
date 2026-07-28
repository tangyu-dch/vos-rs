/// Webhook 异步投递配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WebhookConfig {
    /// 是否启动 Webhook 事件流水线。
    pub enabled: bool,
    /// 接收呼叫事件的 HTTP 地址。
    pub endpoint_url: String,
    /// HMAC-SHA256 签名密钥。
    pub signing_secret: String,
    /// JetStream 名称。
    pub stream: String,
    /// NATS 事件主题。
    pub subject: String,
    /// JetStream Durable Consumer 名称。
    pub consumer: String,
    /// SIP 热路径到发布器的有界队列容量。
    pub queue_capacity: usize,
    /// 单次 HTTP 请求超时毫秒数。
    pub request_timeout_ms: u64,
    /// 单事件最大 HTTP 投递次数。
    pub max_deliveries: u32,
    /// 首次重试等待毫秒数，后续按指数增长。
    pub retry_delay_ms: u64,
    /// Redis 投递记录保留秒数。
    pub delivery_record_ttl_secs: u64,
    /// 呼叫控制模式: disabled / nats
    pub control_mode: String,
    /// 呼入事件通知发送的 NATS 主题
    pub control_incoming_subject: String,
    /// 呼叫控制命令接收的 NATS 主题
    pub control_command_subject: String,
    /// DTMF按键事件通知发送的 NATS 主题
    pub control_dtmf_subject: String,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint_url: String::new(),
            signing_secret: String::new(),
            stream: "VOS_RS_WEBHOOKS".to_string(),
            subject: "vos_rs.webhooks.calls".to_string(),
            consumer: "vos_rs_webhook_delivery".to_string(),
            queue_capacity: 4096,
            request_timeout_ms: 3000,
            max_deliveries: 5,
            retry_delay_ms: 1000,
            delivery_record_ttl_secs: 604_800,
            control_mode: "disabled".to_string(),
            control_incoming_subject: "vos_rs.call.incoming".to_string(),
            control_command_subject: "vos_rs.call.commands".to_string(),
            control_dtmf_subject: "vos_rs.call.dtmf".to_string(),
        }
    }
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct WebhookSection {
    enabled: Option<bool>,
    endpoint_url: Option<String>,
    signing_secret: Option<String>,
    stream: Option<String>,
    subject: Option<String>,
    consumer: Option<String>,
    queue_capacity: Option<usize>,
    request_timeout_ms: Option<u64>,
    max_deliveries: Option<u32>,
    retry_delay_ms: Option<u64>,
    delivery_record_ttl_secs: Option<u64>,
    control_mode: Option<String>,
    control_incoming_subject: Option<String>,
    control_command_subject: Option<String>,
    control_dtmf_subject: Option<String>,
}

impl WebhookSection {
    pub(super) fn into_config(self) -> WebhookConfig {
        let defaults = WebhookConfig::default();
        WebhookConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            endpoint_url: self.endpoint_url.unwrap_or(defaults.endpoint_url),
            signing_secret: self.signing_secret.unwrap_or(defaults.signing_secret),
            stream: self.stream.unwrap_or(defaults.stream),
            subject: self.subject.unwrap_or(defaults.subject),
            consumer: self.consumer.unwrap_or(defaults.consumer),
            queue_capacity: self.queue_capacity.unwrap_or(defaults.queue_capacity),
            request_timeout_ms: self
                .request_timeout_ms
                .unwrap_or(defaults.request_timeout_ms),
            max_deliveries: self.max_deliveries.unwrap_or(defaults.max_deliveries),
            retry_delay_ms: self.retry_delay_ms.unwrap_or(defaults.retry_delay_ms),
            delivery_record_ttl_secs: self
                .delivery_record_ttl_secs
                .unwrap_or(defaults.delivery_record_ttl_secs),
            control_mode: self.control_mode.unwrap_or(defaults.control_mode),
            control_incoming_subject: self
                .control_incoming_subject
                .unwrap_or(defaults.control_incoming_subject),
            control_command_subject: self
                .control_command_subject
                .unwrap_or(defaults.control_command_subject),
            control_dtmf_subject: self
                .control_dtmf_subject
                .unwrap_or(defaults.control_dtmf_subject),
        }
    }
}
