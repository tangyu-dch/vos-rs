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
    Ok(Json(record))
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
