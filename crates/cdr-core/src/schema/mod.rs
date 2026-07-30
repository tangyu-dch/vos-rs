//! # 数据库表结构定义

pub mod access_control_tables;
pub mod accounting_tables;
pub mod announcement_tables;
pub mod billing_tables;
pub mod cdr_tables;
pub mod copilot_tables;
pub mod notification_tables;
pub mod sip_tables;

pub(crate) use access_control_tables::*;
pub(crate) use accounting_tables::*;
pub(crate) use announcement_tables::*;
pub(crate) use billing_tables::*;
pub(crate) use cdr_tables::*;
pub(crate) use copilot_tables::*;
pub(crate) use notification_tables::*;
pub(crate) use sip_tables::*;

#[cfg(test)]
mod tests {
    use super::SEED_SYSTEM_CONFIGS_SQL;

    #[test]
    fn system_config_seed_covers_high_frequency_domains() {
        for key in [
            "session_expires_gateway",
            "database_routes_enabled",
            "gateway_health_checks_enabled",
            "rtp_symmetric_learning",
            "recording_enabled",
            "balance_enforcement_enabled",
            "billing_settlement_enabled",
            "sbc_rate_limit_enabled",
            "cluster_heartbeat_interval_secs",
            "cluster_node_timeout_secs",
            "cdr_persistence_enabled",
        ] {
            assert!(
                SEED_SYSTEM_CONFIGS_SQL.contains(&format!("('{key}',")),
                "missing default for {key}"
            );
        }
        assert!(SEED_SYSTEM_CONFIGS_SQL.contains("ON CONFLICT (config_key) DO NOTHING"));
    }
}
