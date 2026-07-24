//! # LLM 配置存储
//!
//! 管理多个大模型厂商配置（zhipu/openai/deepseek/gemini/local_vllm/ollama），
//! 通过 `is_active` 字段标识当前启用的配置。Copilot 运行时从该表读取
//! `is_active=true` 的记录动态调用 LLM，无需重启即可切换厂商/模型。

use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

/// LLM 配置记录（对齐 `llm_configs` 表）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct LlmConfigRecord {
    pub id: i64,
    pub name: String,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: f32,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// 新建/更新 LLM 配置时的输入参数
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpsertLlmConfigInput {
    pub name: String,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_temperature() -> f32 {
    0.3
}

impl PostgresCdrStore {
    /// 列出所有 LLM 配置，按 `is_active DESC, updated_at DESC` 排序
    pub async fn list_llm_configs(&self) -> Result<Vec<LlmConfigRecord>, sqlx::Error> {
        sqlx::query_as::<_, LlmConfigRecord>(
            "SELECT id, name, provider, api_key, base_url, model, temperature, is_active, \
             created_at, updated_at FROM llm_configs \
             ORDER BY is_active DESC, updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await
    }

    /// 获取指定 ID 的 LLM 配置
    pub async fn get_llm_config(&self, id: i64) -> Result<Option<LlmConfigRecord>, sqlx::Error> {
        sqlx::query_as::<_, LlmConfigRecord>(
            "SELECT id, name, provider, api_key, base_url, model, temperature, is_active, \
             created_at, updated_at FROM llm_configs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    /// 获取当前启用的 LLM 配置（`is_active=true`，全局唯一）
    pub async fn get_active_llm_config(&self) -> Result<Option<LlmConfigRecord>, sqlx::Error> {
        sqlx::query_as::<_, LlmConfigRecord>(
            "SELECT id, name, provider, api_key, base_url, model, temperature, is_active, \
             created_at, updated_at FROM llm_configs WHERE is_active = true LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// 新建 LLM 配置。若当前无启用配置，自动将新记录设为启用。
    pub async fn create_llm_config(
        &self,
        input: &UpsertLlmConfigInput,
    ) -> Result<LlmConfigRecord, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // 若当前无 active 配置，新建的自动启用
        let has_active: bool =
            sqlx::query("SELECT EXISTS(SELECT 1 FROM llm_configs WHERE is_active = true)")
                .fetch_one(&mut *tx)
                .await?
                .get(0);
        let record = sqlx::query_as::<_, LlmConfigRecord>(
            "INSERT INTO llm_configs (name, provider, api_key, base_url, model, temperature, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, name, provider, api_key, base_url, model, temperature, is_active, created_at, updated_at",
        )
        .bind(&input.name)
        .bind(&input.provider)
        .bind(&input.api_key)
        .bind(&input.base_url)
        .bind(&input.model)
        .bind(input.temperature)
        .bind(!has_active)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    /// 更新指定 LLM 配置。`is_active` 不在此处修改，需通过 `activate_llm_config` 切换。
    pub async fn update_llm_config(
        &self,
        id: i64,
        input: &UpsertLlmConfigInput,
    ) -> Result<Option<LlmConfigRecord>, sqlx::Error> {
        sqlx::query_as::<_, LlmConfigRecord>(
            "UPDATE llm_configs SET name=$1, provider=$2, api_key=$3, base_url=$4, model=$5, \
             temperature=$6, updated_at=now() WHERE id=$7 \
             RETURNING id, name, provider, api_key, base_url, model, temperature, is_active, created_at, updated_at",
        )
        .bind(&input.name)
        .bind(&input.provider)
        .bind(&input.api_key)
        .bind(&input.base_url)
        .bind(&input.model)
        .bind(input.temperature)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    /// 删除指定 LLM 配置。若删除的是当前启用配置，自动将最近更新的另一条设为启用。
    pub async fn delete_llm_config(&self, id: i64) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let was_active: Option<bool> = sqlx::query("SELECT is_active FROM llm_configs WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| r.get(0));
        let deleted = sqlx::query("DELETE FROM llm_configs WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        // 若删除的是 active 配置，自动激活最近更新的一条
        if deleted && was_active == Some(true) {
            sqlx::query(
                "UPDATE llm_configs SET is_active=true, updated_at=now() \
                 WHERE id = (SELECT id FROM llm_configs ORDER BY updated_at DESC LIMIT 1)",
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    /// 启用指定 LLM 配置（事务内先将所有配置设为 inactive，再将目标设为 active）。
    pub async fn activate_llm_config(
        &self,
        id: i64,
    ) -> Result<Option<LlmConfigRecord>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let affected = sqlx::query("UPDATE llm_configs SET is_active=false WHERE is_active=true")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let _ = affected;
        let record = sqlx::query_as::<_, LlmConfigRecord>(
            "UPDATE llm_configs SET is_active=true, updated_at=now() WHERE id=$1 \
             RETURNING id, name, provider, api_key, base_url, model, temperature, is_active, created_at, updated_at",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(record)
    }
}
