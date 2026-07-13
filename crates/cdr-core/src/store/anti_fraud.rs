use crate::models::{AntiFraudConfigItem, AntiFraudRule};
use crate::PostgresCdrStore;

impl PostgresCdrStore {
    pub async fn insert_anti_fraud_rule(&self, rule: &AntiFraudRule) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO anti_fraud_rules (id, rule_type, target_value, limit_number, enabled) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (id) DO UPDATE \
             SET rule_type = EXCLUDED.rule_type, \
                 target_value = EXCLUDED.target_value, \
                 limit_number = EXCLUDED.limit_number, \
                 enabled = EXCLUDED.enabled",
        )
        .bind(&rule.id)
        .bind(&rule.rule_type)
        .bind(&rule.target_value)
        .bind(rule.limit_number)
        .bind(rule.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_anti_fraud_rules(&self) -> Result<Vec<AntiFraudRule>, sqlx::Error> {
        sqlx::query_as::<_, AntiFraudRule>(
            "SELECT id, rule_type, target_value, limit_number, enabled FROM anti_fraud_rules ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn delete_anti_fraud_rule(&self, id: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM anti_fraud_rules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn list_anti_fraud_configs(&self) -> Result<Vec<AntiFraudConfigItem>, sqlx::Error> {
        sqlx::query_as::<_, AntiFraudConfigItem>(
            "SELECT config_key, config_value, description, updated_at FROM anti_fraud_config ORDER BY config_key"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_anti_fraud_config(
        &self,
        key: &str,
        value: &str,
    ) -> Result<bool, sqlx::Error> {
        let r = sqlx::query(
            "UPDATE anti_fraud_config SET config_value = $1, updated_at = NOW() WHERE config_key = $2"
        )
        .bind(value)
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }
}
