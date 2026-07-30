use sqlx::PgPool;

use crate::schema::*;
use crate::termination_schema;

pub(crate) async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(75812903)")
        .execute(&mut *tx)
        .await?;

    sqlx::query(CREATE_CDR_TABLE_SQL).execute(&mut *tx).await?;
    sqlx::query(MIGRATE_CDR_AUDIT_SQL).execute(&mut *tx).await?;
    // MIGRATE_CDR_TIMING_BILLING_SQL 包含多条 ALTER TABLE 语句，必须用 raw_sql 执行
    sqlx::raw_sql(MIGRATE_CDR_TIMING_BILLING_SQL)
        .execute(&mut *tx)
        .await?;
    // 回填历史 CDR 的振铃时长和通话时长（仅更新 NULL 行，幂等）
    sqlx::raw_sql(BACKFILL_CDR_TIMING_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CALL_ID_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_CDR_IDEMPOTENCY_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_STARTED_AT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_STATUS_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CDR_CALLER_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CDR_CALLEE_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    for sql in MIGRATE_CDR_TENANT_REALM_SQL {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    sqlx::query(CREATE_CDR_DIRECTION_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CDR_STATUS_STARTED_AT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CDR_CALLER_STARTED_AT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CDR_CALLEE_STARTED_AT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_USERS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_GATEWAYS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;

    for migration_sql in MIGRATE_SIP_GATEWAYS_SQL {
        sqlx::query(migration_sql).execute(&mut *tx).await?;
    }
    for index_sql in [
        CREATE_GATEWAYS_TYPE_INDEX_SQL,
        CREATE_GATEWAYS_PARENT_INDEX_SQL,
        CREATE_GATEWAYS_ACCOUNT_INDEX_SQL,
        CREATE_GATEWAYS_TENANT_INDEX_SQL,
        CREATE_GATEWAYS_ENABLED_INDEX_SQL,
    ] {
        sqlx::query(index_sql).execute(&mut *tx).await?;
    }

    sqlx::query(CREATE_SIP_ROUTES_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ROUTES_PRIORITY_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ROUTES_PREFIX_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ROUTES_GATEWAY_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_TENANTS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_TENANTS_DOMAIN_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_TENANTS_ENABLED_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(ADD_GATEWAY_TENANT_FOREIGN_KEY_SQL)
        .execute(&mut *tx)
        .await?;
    // tenants 表创建后再为 sip_users 添加 tenant_id 列与索引（引用 tenants(id)）
    sqlx::query(MIGRATE_SIP_USERS_TENANT_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_USERS_TENANT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_USERS_UNIQUE_TENANT_USERNAME_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATION_ADD_ROUTE_WEIGHT)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATION_ADD_ROUTE_TOPOLOGY)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_REGISTRATIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE sip_registrations ADD COLUMN IF NOT EXISTS path TEXT")
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_REGISTRATIONS_USER_AGENT_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_REGISTRATIONS_EXPIRES_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_capacity INTEGER")
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS cost DOUBLE PRECISION NOT NULL DEFAULT 0.0")
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_start TEXT")
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_end TEXT")
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_DTMF_EVENTS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_DTMF_CALL_ID_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_ACCOUNTS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;

    for migration_sql in MIGRATE_BILLING_ACCOUNTS_SQL {
        sqlx::query(migration_sql).execute(&mut *tx).await?;
    }
    sqlx::query(MIGRATE_TENANT_ACCOUNT_LINK_SQL)
        .execute(&mut *tx)
        .await?;

    sqlx::query(ADD_GATEWAY_ACCOUNT_FOREIGN_KEY_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_LEDGER_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_CREDITS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(MIGRATE_BILLING_CREDITS_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_CREDITS_USERNAME_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_CREDITS_ACCOUNT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_BILLING_JOURNAL_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(MIGRATE_BILLING_JOURNAL_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(MIGRATE_BILLING_INTERVALS_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(MIGRATE_DUAL_CALL_ACCOUNTING_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_LEDGER_USERNAME_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_LEDGER_CREATED_AT_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NUMBER_INVENTORY_TABLE_SQL)
        .execute(&mut *tx)
        .await?;

    for migration_sql in MIGRATE_NUMBER_INVENTORY_SQL {
        sqlx::query(migration_sql).execute(&mut *tx).await?;
    }
    for index_sql in [
        CREATE_NUMBERS_GATEWAY_INDEX_SQL,
        CREATE_NUMBERS_STATUS_INDEX_SQL,
        CREATE_NUMBERS_USERNAME_INDEX_SQL,
    ] {
        sqlx::query(index_sql).execute(&mut *tx).await?;
    }

    sqlx::query(CREATE_GATEWAY_NUMBER_ASSIGNMENTS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_GATEWAY_PEER_LINKS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;

    for index_sql in CREATE_GATEWAY_ASSIGNMENT_INDEXES_SQL {
        sqlx::query(index_sql).execute(&mut *tx).await?;
    }

    sqlx::query(CREATE_GATEWAY_HEALTH_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_GATEWAY_HEALTH_STATE_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'closed'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS last_failure_at TIMESTAMPTZ",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS half_open_successes INTEGER NOT NULL DEFAULT 0")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS last_probe_at TIMESTAMPTZ",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS active_calls INTEGER NOT NULL DEFAULT 0")
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANTI_FRAUD_RULES_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP2_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP3_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP4_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANTI_FRAUD_CONFIG_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_ANTI_FRAUD_CONFIG_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_AUDIT_LOGS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_AUDIT_LOGS_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE api_audit_logs ADD COLUMN IF NOT EXISTS query_params TEXT")
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE api_audit_logs ADD COLUMN IF NOT EXISTS request_body TEXT")
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SYSTEM_CONFIGS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_SYSTEM_CONFIGS_SQL)
        .execute(&mut *tx)
        .await?;

    if let Ok(Some(relkind)) = sqlx::query_scalar::<_, String>(
        "SELECT relkind::text FROM pg_class WHERE relname = 'sip_flows'",
    )
    .fetch_optional(&mut *tx)
    .await
    {
        if relkind == "r" {
            let _ = sqlx::query("DROP TABLE IF EXISTS sip_flows_old CASCADE")
                .execute(&mut *tx)
                .await;
            let _ = sqlx::query("ALTER TABLE sip_flows RENAME TO sip_flows_old")
                .execute(&mut *tx)
                .await;
        }
    }

    sqlx::query(CREATE_SIP_FLOWS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_FLOWS_CALL_ID_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_SIP_FLOWS_TIMESTAMP_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_SESSIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_SESSIONS_OPERATOR_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_MESSAGES_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_MESSAGES_SESSION_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_ACTIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_COPILOT_ACTIONS_SESSION_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(MIGRATE_COPILOT_MESSAGES_IMAGES_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_LLM_CONFIGS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_LLM_CONFIGS_ACTIVE_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_DEFAULT_LLM_CONFIG_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NOTIFICATIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NOTIFICATIONS_ACTIVE_DEDUP_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NOTIFICATIONS_CREATED_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NOTIFICATION_READS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_NOTIFICATION_READS_OPERATOR_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANNOUNCEMENTS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANNOUNCEMENTS_STATUS_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANNOUNCEMENTS_CATEGORY_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANNOUNCEMENT_READS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ANNOUNCEMENT_READS_OPERATOR_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ACCESS_ROLES_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ACCESS_PERMISSIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_ACCESS_ROLE_PERMISSIONS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CONSOLE_USERS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_CONSOLE_USERS_ROLE_INDEX_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_MENU_GROUPS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(CREATE_MENU_ITEMS_TABLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_ACCESS_ROLES_SQL).execute(&mut *tx).await?;
    sqlx::query(NORMALIZE_SYSTEM_ROLE_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_ACCESS_PERMISSIONS_SQL)
        .execute(&mut *tx)
        .await?;
    sqlx::query(SEED_ACCESS_ROLE_PERMISSIONS_SQL)
        .execute(&mut *tx)
        .await?;
    for migration_sql in MIGRATE_LEGACY_ACCESS_ROLE_PERMISSIONS_SQL {
        sqlx::query(migration_sql).execute(&mut *tx).await?;
    }
    sqlx::query(SEED_MENU_GROUPS_SQL).execute(&mut *tx).await?;
    sqlx::query(SEED_MENU_ITEMS_SQL).execute(&mut *tx).await?;
    sqlx::query(REMOVE_LEGACY_BILLING_ACCOUNT_MENU_SQL)
        .execute(&mut *tx)
        .await?;

    for migration_sql in termination_schema::MIGRATE_TERMINATION_DOMAIN_SQL {
        sqlx::query(migration_sql).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}
