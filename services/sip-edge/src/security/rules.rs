use tracing::{info, warn};
use crate::edge_state::EdgeState;

pub(crate) async fn refresh_anti_fraud_rules(edge_state: &EdgeState) {
    if let Some(ref db) = edge_state.db_store {
        match db.list_anti_fraud_rules().await {
            Ok(rules) => {
                let enabled_rules: Vec<_> = rules.into_iter().filter(|r| r.enabled).collect();
                let count = enabled_rules.len();
                let mut guard = edge_state
                    .anti_fraud_rules
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                *guard = enabled_rules;
                info!(count, "已成功刷新防盗打控制规则缓存");
            }
            Err(e) => warn!("无法从数据库加载防盗打规则: {}", e),
        }
    }
}
