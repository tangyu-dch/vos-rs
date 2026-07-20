use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{ApiError, AppState, PaginatedResponse};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct IvrMapping {
    #[serde(default)]
    pub dtmf_key: String,
    #[serde(default)]
    pub action_type: String,
    #[serde(default)]
    pub action_target: String,
    #[serde(default)]
    pub waiting_prompt: Option<String>,
    #[serde(default)]
    pub webhook_method: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct IvrMenu {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub welcome_prompt: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub mappings: Vec<IvrMapping>,
}

pub(crate) async fn list_menus(
    State(state): State<AppState>,
) -> Result<Json<PaginatedResponse<IvrMenu>>, ApiError> {
    let pool = state.store.pool();
    let rows = sqlx::query("SELECT id, name, welcome_prompt, timeout_secs FROM ivr_menus ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|e| ApiError::internal(format!("查询 IVR 菜单失败: {e}")))?;

    let mut items = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        let name: String = row.get("name");
        let welcome_prompt: String = row.get("welcome_prompt");
        let timeout_secs: i32 = row.get("timeout_secs");

        let action_rows = sqlx::query("SELECT dtmf_key, action_type, action_target, waiting_prompt, webhook_method FROM ivr_actions WHERE ivr_id = $1")
            .bind(&id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

        let mappings = action_rows
            .into_iter()
            .map(|r| IvrMapping {
                dtmf_key: r.get("dtmf_key"),
                action_type: r.get("action_type"),
                action_target: r.get("action_target"),
                waiting_prompt: r.get("waiting_prompt"),
                webhook_method: r.get("webhook_method"),
            })
            .collect();

        items.push(IvrMenu {
            id,
            name,
            welcome_prompt: Some(welcome_prompt),
            timeout_secs: Some(timeout_secs as u32),
            mappings,
        });
    }

    let total = items.len() as i64;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page: 1,
        page_size: 20,
    }))
}

pub(crate) async fn create_menu(
    State(state): State<AppState>,
    Json(payload): Json<IvrMenu>,
) -> Result<(StatusCode, Json<IvrMenu>), ApiError> {
    let pool = state.store.pool();
    let welcome_prompt = payload.welcome_prompt.clone().unwrap_or_else(|| "welcome.wav".to_string());
    let timeout_secs = payload.timeout_secs.unwrap_or(10) as i32;

    sqlx::query(
        "INSERT INTO ivr_menus (id, name, welcome_prompt, timeout_secs) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO UPDATE SET name = $2, welcome_prompt = $3, timeout_secs = $4"
    )
    .bind(&payload.id)
    .bind(&payload.name)
    .bind(&welcome_prompt)
    .bind(timeout_secs)
    .execute(pool)
    .await
    .map_err(|e| ApiError::internal(format!("保存 IVR 菜单失败: {e}")))?;

    sqlx::query("DELETE FROM ivr_actions WHERE ivr_id = $1")
        .bind(&payload.id)
        .execute(pool)
        .await
        .ok();

    for m in &payload.mappings {
        sqlx::query(
            "INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET action_type = $3, action_target = $4, waiting_prompt = $5, webhook_method = $6"
        )
        .bind(&payload.id)
        .bind(&m.dtmf_key)
        .bind(&m.action_type)
        .bind(&m.action_target)
        .bind(&m.waiting_prompt)
        .bind(&m.webhook_method)
        .execute(pool)
        .await
        .ok();
    }

    Ok((StatusCode::CREATED, Json(payload)))
}

pub(crate) async fn update_menu(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(payload): Json<IvrMenu>,
) -> Result<Json<IvrMenu>, ApiError> {
    let _ = create_menu(State(state), Json(payload.clone())).await?;
    Ok(Json(payload))
}

pub(crate) async fn delete_menu(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state.store.pool();
    sqlx::query("DELETE FROM ivr_menus WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| ApiError::internal(format!("删除 IVR 菜单失败: {e}")))?;
    Ok(Json(serde_json::json!({"success": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ivr_menu_serialization() {
        let menu = IvrMenu {
            id: "m1".to_string(),
            name: "Main Menu".to_string(),
            welcome_prompt: Some("welcome.wav".to_string()),
            timeout_secs: Some(10),
            mappings: vec![IvrMapping {
                dtmf_key: "1".to_string(),
                action_type: "queue".to_string(),
                action_target: "q1".to_string(),
                waiting_prompt: None,
                webhook_method: None,
            }],
        };
        let serialized = serde_json::to_value(&menu).unwrap();
        assert_eq!(serialized["id"], "m1");
        assert_eq!(serialized["mappings"][0]["dtmf_key"], "1");
        assert_eq!(serialized["mappings"][0]["action_target"], "q1");
    }
}
