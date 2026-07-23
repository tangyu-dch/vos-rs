//! # LLM 配置管理 API
//!
//! 提供多厂商 LLM 配置的 CRUD 与启用切换能力。
//! Copilot 运行时从 `get_active_llm_config` 动态读取当前启用的配置，
//! 切换模型无需重启服务。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use cdr_core::{LlmConfigRecord, UpsertLlmConfigInput};

use crate::{ApiError, AppState};

/// 列出所有 LLM 配置
pub async fn list_llm_configs(
    State(state): State<AppState>,
) -> Result<Json<Vec<LlmConfigRecord>>, ApiError> {
    state
        .store
        .list_llm_configs()
        .await
        .map(Json)
        .map_err(|e| ApiError::internal(format!("查询 LLM 配置列表失败: {e}")))
}

/// 获取当前启用的 LLM 配置
pub async fn get_active_llm_config(
    State(state): State<AppState>,
) -> Result<Json<Option<LlmConfigRecord>>, ApiError> {
    state
        .store
        .get_active_llm_config()
        .await
        .map(Json)
        .map_err(|e| ApiError::internal(format!("查询当前 LLM 配置失败: {e}")))
}

/// 新建 LLM 配置
pub async fn create_llm_config(
    State(state): State<AppState>,
    Json(input): Json<UpsertLlmConfigInput>,
) -> Result<(StatusCode, Json<LlmConfigRecord>), ApiError> {
    validate_input(&input)?;
    let record = state
        .store
        .create_llm_config(&input)
        .await
        .map_err(|e| ApiError::internal(format!("创建 LLM 配置失败: {e}")))?;
    
    // 同步更新 Redis 缓存
    rebuild_llm_configs_in_redis(&state).await?;
    
    Ok((StatusCode::CREATED, Json(record)))
}

/// 更新 LLM 配置
pub async fn update_llm_config(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpsertLlmConfigInput>,
) -> Result<Json<LlmConfigRecord>, ApiError> {
    validate_input(&input)?;
    let record = state
        .store
        .update_llm_config(id, &input)
        .await
        .map_err(|e| ApiError::internal(format!("更新 LLM 配置失败: {e}")))?
        .ok_or_else(|| ApiError::not_found("LLM 配置不存在".to_string()))?;
    
    // 同步更新 Redis 缓存
    rebuild_llm_configs_in_redis(&state).await?;
    
    Ok(Json(record))
}

/// 删除 LLM 配置
pub async fn delete_llm_config(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_llm_config(id)
        .await
        .map_err(|e| ApiError::internal(format!("删除 LLM 配置失败: {e}")))?;
    if deleted {
        // 同步更新 Redis 缓存
        rebuild_llm_configs_in_redis(&state).await?;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("LLM 配置不存在".to_string()))
    }
}

/// 启用指定 LLM 配置（其余自动设为 inactive）
pub async fn activate_llm_config(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<LlmConfigRecord>, ApiError> {
    let record = state
        .store
        .activate_llm_config(id)
        .await
        .map_err(|e| ApiError::internal(format!("启用 LLM 配置失败: {e}")))?
        .ok_or_else(|| ApiError::not_found("LLM 配置不存在".to_string()))?;
    
    // 同步更新 Redis 缓存
    rebuild_llm_configs_in_redis(&state).await?;
    
    Ok(Json(record))
}

/// 从数据库重建 Redis 缓存中的所有 LLM 配置
pub async fn rebuild_llm_configs_in_redis(state: &AppState) -> Result<(), ApiError> {
    let mut conn = state.redis_client.clone();
    
    // 1. 从 DB 中查询所有配置
    let list = state
        .store
        .list_llm_configs()
        .await
        .map_err(|e| ApiError::internal(format!("查询 LLM 配置列表失败: {e}")))?;
    
    // 2. 清空 Redis 中旧的 key
    let _: () = redis::cmd("DEL")
        .arg("vos_rs:llm_configs")
        .query_async(&mut conn)
        .await
        .unwrap_or(());
        
    let _: () = redis::cmd("DEL")
        .arg("vos_rs:active_llm_config")
        .query_async(&mut conn)
        .await
        .unwrap_or(());

    // 3. 逐个写入 Redis
    for rec in list {
        let json_str = serde_json::to_string(&rec)
            .map_err(|e| ApiError::internal(format!("序列化 LLM 配置失败: {e}")))?;
        
        let _: () = redis::cmd("HSET")
            .arg("vos_rs:llm_configs")
            .arg(rec.id.to_string())
            .arg(&json_str)
            .query_async(&mut conn)
            .await
            .map_err(|e| ApiError::internal(format!("写入 Redis hash 失败: {e}")))?;

        if rec.is_active {
            let _: () = redis::cmd("SET")
                .arg("vos_rs:active_llm_config")
                .arg(&json_str)
                .query_async(&mut conn)
                .await
                .map_err(|e| ApiError::internal(format!("写入 active Redis 失败: {e}")))?;
        }
    }
    
    Ok(())
}

/// 从 Redis 中获取 LLM 配置信息（支持特定 ID，或回退至当前启用配置）
pub async fn get_llm_config_from_redis(
    state: &AppState,
    model_id: Option<i64>,
) -> Option<LlmConfigRecord> {
    let mut conn = state.redis_client.clone();
    
    if let Some(id) = model_id {
        // 尝试从 Redis Hash 获取特定 ID
        let json_opt: Option<String> = redis::cmd("HGET")
            .arg("vos_rs:llm_configs")
            .arg(id.to_string())
            .query_async(&mut conn)
            .await
            .unwrap_or(None);
            
        if let Some(json_str) = json_opt {
            if let Ok(rec) = serde_json::from_str::<LlmConfigRecord>(&json_str) {
                return Some(rec);
            }
        }
        
        // 缓存未命中时回退到 DB 查询
        if let Ok(Some(rec)) = state.store.get_llm_config(id).await {
            // 顺便回填 Redis 缓存
            if let Ok(json_str) = serde_json::to_string(&rec) {
                let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                    .arg("vos_rs:llm_configs")
                    .arg(id.to_string())
                    .arg(&json_str)
                    .query_async(&mut conn)
                    .await;
            }
            return Some(rec);
        }
    } else {
        // 获取当前启用配置
        let json_opt: Option<String> = redis::cmd("GET")
            .arg("vos_rs:active_llm_config")
            .query_async(&mut conn)
            .await
            .unwrap_or(None);
            
        if let Some(json_str) = json_opt {
            if let Ok(rec) = serde_json::from_str::<LlmConfigRecord>(&json_str) {
                return Some(rec);
            }
        }
        
        // 缓存未命中时回退到 DB 查询
        if let Ok(Some(rec)) = state.store.get_active_llm_config().await {
            // 顺便回填 Redis 缓存
            if let Ok(json_str) = serde_json::to_string(&rec) {
                let _: Result<(), redis::RedisError> = redis::cmd("SET")
                    .arg("vos_rs:active_llm_config")
                    .arg(&json_str)
                    .query_async(&mut conn)
                    .await;
            }
            return Some(rec);
        }
    }
    None
}

/// 校验输入：name/provider/api_key/base_url/model 非空
fn validate_input(input: &UpsertLlmConfigInput) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request("配置名称不能为空".to_string()));
    }
    if input.provider.trim().is_empty() {
        return Err(ApiError::bad_request("厂商(provider)不能为空".to_string()));
    }
    if input.api_key.trim().is_empty() {
        return Err(ApiError::bad_request("API Key 不能为空".to_string()));
    }
    if input.base_url.trim().is_empty() {
        return Err(ApiError::bad_request("Base URL 不能为空".to_string()));
    }
    if input.model.trim().is_empty() {
        return Err(ApiError::bad_request("模型名称不能为空".to_string()));
    }
    if !(0.0..=2.0).contains(&input.temperature) {
        return Err(ApiError::bad_request("temperature 必须在 0.0 ~ 2.0 之间".to_string()));
    }
    Ok(())
}
