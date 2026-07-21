use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::Row;

use crate::{ApiError, AppState, PaginatedResponse};

// IVR 节点 (与前端 IvrNode 对齐)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub(crate) struct IvrNodeDto {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub position: IvrPositionDto,
    #[serde(default)]
    pub config: JsonValue,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub(crate) struct IvrPositionDto {
    pub x: f64,
    pub y: f64,
}

// IVR 连线 (与前端 IvrEdge 对齐)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub(crate) struct IvrEdgeDto {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub source_port: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

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

/// IVR 菜单完整结构 (含可视化拓扑 nodes/edges 与按键映射 mappings)
/// 与前端 IvrFlow 字段保持一致, 画布编辑后可直接持久化
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct IvrMenu {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub did: Option<String>,
    #[serde(default)]
    pub welcome_prompt: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 可视化拓扑: 节点列表 (画布编辑保存)
    #[serde(default)]
    pub nodes: Vec<IvrNodeDto>,
    /// 可视化拓扑: 连线列表 (画布编辑保存)
    #[serde(default)]
    pub edges: Vec<IvrEdgeDto>,
    /// 按键映射 (传统 DTMF 模式, 与 nodes/edges 二选一)
    #[serde(default)]
    pub mappings: Vec<IvrMapping>,
}

fn default_true() -> bool {
    true
}

pub(crate) async fn list_menus(
    State(state): State<AppState>,
) -> Result<Json<PaginatedResponse<IvrMenu>>, ApiError> {
    let pool = state.store.pool();
    let rows = sqlx::query(
        "SELECT id FROM ivr_menus ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal(format!("查询 IVR 菜单失败: {e}")))?;

    let mut items = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        items.push(load_menu_by_id(pool, &id).await?);
    }

    let total = items.len() as i64;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page: 1,
        page_size: 20,
    }))
}

/// 获取单个 IVR 菜单 (含 mappings + nodes + edges)
pub(crate) async fn get_menu(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<IvrMenu>, ApiError> {
    let pool = state.store.pool();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ivr_menus WHERE id = $1)")
        .bind(&id)
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::internal(format!("查询 IVR 菜单失败: {e}")))?;
    if !exists {
        return Err(ApiError::internal(format!("IVR 菜单 {id} 不存在")));
    }
    Ok(Json(load_menu_by_id(pool, &id).await?))
}

/// 从数据库加载单个 IVR 菜单 (含 mappings + nodes + edges)
async fn load_menu_by_id(pool: &sqlx::PgPool, id: &str) -> Result<IvrMenu, ApiError> {
    let row = sqlx::query(
        "SELECT id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges \
         FROM ivr_menus WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::internal(format!("查询 IVR 菜单 {id} 失败: {e}")))?;

    let menu_id: String = row.get("id");
    let name: String = row.get("name");
    let description: Option<String> = row.get("description");
    let did: Option<String> = row.get("did");
    let welcome_prompt: Option<String> = row.get("welcome_prompt");
    let timeout_secs: i32 = row.get("timeout_secs");
    let enabled: bool = row.get("enabled");

    // 解析 nodes/edges JSONB
    let nodes_json: JsonValue = row.get("nodes");
    let edges_json: JsonValue = row.get("edges");
    let nodes: Vec<IvrNodeDto> = serde_json::from_value(nodes_json).unwrap_or_default();
    let edges: Vec<IvrEdgeDto> = serde_json::from_value(edges_json).unwrap_or_default();

    // 加载按键映射 (兼容旧 DTMF 模式)
    let action_rows = sqlx::query(
        "SELECT dtmf_key, action_type, action_target, waiting_prompt, webhook_method \
         FROM ivr_actions WHERE ivr_id = $1",
    )
    .bind(&menu_id)
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

    Ok(IvrMenu {
        id: menu_id,
        name,
        description,
        did,
        welcome_prompt,
        timeout_secs: Some(timeout_secs as u32),
        enabled,
        nodes,
        edges,
        mappings,
    })
}

pub(crate) async fn create_menu(
    State(state): State<AppState>,
    Json(payload): Json<IvrMenu>,
) -> Result<(StatusCode, Json<IvrMenu>), ApiError> {
    save_menu(state.store.pool(), &payload).await?;
    Ok((StatusCode::CREATED, Json(payload)))
}

pub(crate) async fn update_menu(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(payload): Json<IvrMenu>,
) -> Result<Json<IvrMenu>, ApiError> {
    save_menu(state.store.pool(), &payload).await?;
    Ok(Json(payload))
}

/// 保存 IVR 菜单 (upsert ivr_menus + 替换 ivr_actions)
async fn save_menu(pool: &sqlx::PgPool, payload: &IvrMenu) -> Result<(), ApiError> {
    let welcome_prompt = payload
        .welcome_prompt
        .clone()
        .unwrap_or_else(|| "welcome.wav".to_string());
    let timeout_secs = payload.timeout_secs.unwrap_or(10) as i32;
    let description = payload.description.clone().unwrap_or_default();
    let did = payload.did.clone().unwrap_or_default();

    let nodes_json = serde_json::to_value(&payload.nodes).unwrap_or(JsonValue::Array(vec![]));
    let edges_json = serde_json::to_value(&payload.edges).unwrap_or(JsonValue::Array(vec![]));

    sqlx::query(
        "INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (id) DO UPDATE SET \
            name = EXCLUDED.name, \
            description = EXCLUDED.description, \
            did = EXCLUDED.did, \
            welcome_prompt = EXCLUDED.welcome_prompt, \
            timeout_secs = EXCLUDED.timeout_secs, \
            enabled = EXCLUDED.enabled, \
            nodes = EXCLUDED.nodes, \
            edges = EXCLUDED.edges",
    )
    .bind(&payload.id)
    .bind(&payload.name)
    .bind(&description)
    .bind(&did)
    .bind(&welcome_prompt)
    .bind(timeout_secs)
    .bind(payload.enabled)
    .bind(&nodes_json)
    .bind(&edges_json)
    .execute(pool)
    .await
    .map_err(|e| ApiError::internal(format!("保存 IVR 菜单失败: {e}")))?;

    // 按键映射 (兼容旧模式) - 全量替换
    sqlx::query("DELETE FROM ivr_actions WHERE ivr_id = $1")
        .bind(&payload.id)
        .execute(pool)
        .await
        .ok();

    for m in &payload.mappings {
        sqlx::query(
            "INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET \
                action_type = EXCLUDED.action_type, \
                action_target = EXCLUDED.action_target, \
                waiting_prompt = EXCLUDED.waiting_prompt, \
                webhook_method = EXCLUDED.webhook_method",
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

    Ok(())
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
            description: Some("主菜单".to_string()),
            did: Some("4001".to_string()),
            welcome_prompt: Some("welcome.wav".to_string()),
            timeout_secs: Some(10),
            enabled: true,
            nodes: vec![IvrNodeDto {
                id: "n1".to_string(),
                node_type: "start".to_string(),
                title: "入口".to_string(),
                description: "DID 入口".to_string(),
                position: IvrPositionDto { x: 80.0, y: 200.0 },
                config: JsonValue::Null,
            }],
            edges: vec![IvrEdgeDto {
                id: "e1".to_string(),
                source: "n1".to_string(),
                target: "n2".to_string(),
                source_port: Some("out".to_string()),
                label: Some("进入".to_string()),
            }],
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
        assert_eq!(serialized["nodes"][0]["type"], "start");
        assert_eq!(serialized["edges"][0]["source"], "n1");
        assert_eq!(serialized["mappings"][0]["dtmf_key"], "1");
        assert_eq!(serialized["mappings"][0]["action_target"], "q1");
    }
}
