use crate::cluster::MediaClusterConfig;

use super::EdgeConfig;

async fn get_config_val(
    redis_configs: &std::collections::HashMap<String, String>,
    db: &cdr_core::PostgresCdrStore,
    key: &str,
) -> Option<String> {
    if let Some(val) = redis_configs.get(key) {
        if !val.is_empty() {
            return Some(val.clone());
        }
    }
    if let Ok(Some(val)) = db.get_system_config(key).await {
        return Some(val);
    }
    None
}

impl EdgeConfig {
    pub async fn override_from_db(&mut self, db: &cdr_core::PostgresCdrStore) {
        // Try reading from Redis first
        let mut redis_configs = std::collections::HashMap::new();
        let redis_url = self
            .redis_url
            .clone()
            .unwrap_or_else(|| "redis://127.0.0.1:6379".to_string());

        if let Ok(client) = redis::Client::open(redis_url) {
            if let Ok(mut con) = client.get_multiplexed_tokio_connection().await {
                let res: Result<std::collections::HashMap<String, String>, redis::RedisError> =
                    redis::cmd("HGETALL")
                        .arg("vos_rs:system_configs")
                        .query_async(&mut con)
                        .await;
                if let Ok(hash) = res {
                    redis_configs = hash;
                    tracing::info!("Successfully loaded system configs from Redis");
                }
            }
        }

        // Helper macro to get config either from Redis or fallback to PostgreSQL
        macro_rules! get_val {
            ($key:expr) => {
                get_config_val(&redis_configs, db, $key)
            };
        }

        if let Some(val) = get_val!("session_expires_gateway").await {
            if let Ok(v) = val.parse() {
                self.session_expires_gateway = v;
            }
        }
        if let Some(val) = get_val!("session_expires_caller").await {
            if let Ok(v) = val.parse() {
                self.session_expires_caller = v;
            }
        }
        if let Some(val) = get_val!("database_routes_enabled").await {
            self.database_routes_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("default_gateway").await {
            self.default_gateway = val;
        }
        if let Some(val) = get_val!("sbc_rate_limit_enabled").await {
            self.sbc_rate_limit_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("cluster_enabled").await {
            self.cluster.enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("cluster_heartbeat_interval_secs").await {
            if let Ok(value) = val.parse::<u64>() {
                self.cluster.heartbeat_interval_secs = value.max(1);
            }
        }
        if let Some(val) = get_val!("cluster_node_timeout_secs").await {
            if let Ok(value) = val.parse::<u64>() {
                self.cluster.node_timeout_secs = value.max(1);
            }
        }
        if let Some(val) = get_val!("cluster_dialog_ttl_secs").await {
            if let Ok(value) = val.parse::<u64>() {
                self.cluster.dialog_ttl_secs = value.max(1);
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_enabled").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_enabled = v;
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_capacity").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_capacity = v;
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_fill_rate").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_fill_rate = v;
            }
        }
        if let Some(val) = get_val!("sbc_max_concurrency").await {
            if let Ok(v) = val.parse() {
                self.sbc_max_concurrency = v;
            }
        }
        if let Some(val) = get_val!("tls_cert_path").await {
            self.tls_cert_path = Some(val);
        }
        if let Some(val) = get_val!("tls_key_path").await {
            self.tls_key_path = Some(val);
        }
        if let Some(val) = get_val!("tls_bind_addr").await {
            self.tls_bind_addr = Some(val);
        }
        if let Some(val) = get_val!("tls_allow_test_certificate").await {
            self.tls_allow_test_certificate = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("tls_ca_path").await {
            self.tls_ca_path = Some(val);
        }
        if let Some(val) = get_val!("tls_insecure_skip_verify").await {
            self.tls_insecure_skip_verify = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("tls_server_name").await {
            self.tls_server_name = Some(val);
        }
        if let Some(val) = get_val!("udp_workers").await {
            if let Ok(v) = val.parse() {
                self.udp_workers = v;
            }
        }
        if let Some(val) = get_val!("udp_workers_auto").await {
            self.udp_workers_auto = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("udp_receive_buffer_bytes").await {
            if let Ok(v) = val.parse() {
                self.udp_receive_buffer_bytes = v;
            }
        }
        if let Some(val) = get_val!("udp_send_buffer_bytes").await {
            if let Ok(v) = val.parse() {
                self.udp_send_buffer_bytes = v;
            }
        }
        if let Some(val) = get_val!("sip_dscp").await {
            if let Ok(v) = val.parse() {
                self.sip_dscp = v;
            }
        }
        if let Some(val) = get_val!("rtp_dscp").await {
            if let Ok(v) = val.parse() {
                self.rtp_dscp = v;
            }
        }
        if let Some(val) = get_val!("cdr_queue_capacity").await {
            if let Ok(v) = val.parse::<usize>() {
                self.cdr_queue_capacity = v.max(1);
            }
        }
        if let Some(val) = get_val!("cdr_persistence_enabled").await {
            self.cdr_persistence_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("recording_workers").await {
            if let Ok(v) = val.parse::<usize>() {
                self.recording_workers = v.max(1);
            }
        }
        if let Some(val) = get_val!("recording_queue_capacity").await {
            if let Ok(v) = val.parse::<usize>() {
                self.recording_queue_capacity = v.max(1);
            }
        }
        if let Some(val) = get_val!("media_metrics_log").await {
            self.media_metrics_log = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("media_cluster_json").await {
            match serde_json::from_str::<MediaClusterConfig>(&val) {
                Ok(config)
                    if self
                        .cluster
                        .validate(self.redis_url.as_deref(), self.nats_url.as_deref(), &config)
                        .is_ok() =>
                {
                    self.media_cluster = config;
                }
                Ok(_) => tracing::warn!("忽略未通过校验的动态媒体集群配置"),
                Err(error) => tracing::warn!(%error, "动态媒体集群配置 JSON 无效"),
            }
        }
        if let Some(val) = get_val!("balance_enforcement_enabled").await {
            self.balance_enforcement_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("billing_settlement_enabled").await {
            self.billing_settlement_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("gateway_health_checks_enabled").await {
            self.gateway_health_checks_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("sipflow_enabled").await {
            self.sipflow_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("sipflow_whitelist").await {
            self.sipflow_whitelist = val;
        }
        if let Some(val) = get_val!("sipflow_retention_days").await {
            if let Ok(v) = val.parse() {
                self.sipflow_retention_days = v;
            }
        }

        // 地址和端口由 media_cluster_json 的 nodes[] 管理；这里只覆盖全局媒体行为。
        if let Some(val) = get_val!("rtp_symmetric_learning").await {
            self.media.symmetric_rtp_learning = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_anti_spoofing").await {
            self.media.anti_spoofing = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_source_relearn_secs").await {
            if let Ok(v) = val.parse() {
                self.media.source_relearn_after_secs = v;
            }
        }
        if let Some(val) = get_val!("recording_enabled").await {
            self.media.recording_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("recording_dir").await {
            self.media.recording_dir = std::path::PathBuf::from(val);
        }
        if let Some(val) = get_val!("recording_retention_secs").await {
            if let Ok(v) = val.parse() {
                self.media.recording_retention_secs = v;
            }
        }
        if let Some(val) = get_val!("recording_min_free_bytes").await {
            if let Ok(v) = val.parse() {
                self.media.recording_min_free_bytes = v;
            }
        }
        if let Some(val) = get_val!("recording_max_file_bytes").await {
            if let Ok(v) = val.parse() {
                self.media.recording_max_file_bytes = v;
            }
        }
        if let Some(val) = get_val!("recording_max_duration_secs").await {
            if let Ok(v) = val.parse() {
                self.media.recording_max_duration_secs = v;
            }
        }

        // 覆盖 Auth Config 中的相关属性
        if let Some(val) = get_val!("realm").await {
            self.auth.realm = val;
        }
        if let Some(val) = get_val!("nonce").await {
            self.auth.nonce = val;
        }
        if let Some(val) = get_val!("secret_key").await {
            self.auth.secret_key = val;
        }
    }
}
