use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{ApiError, AppState, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub(crate) struct ExportQuery {
    pub export: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct CallQueue {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub moh_file: Option<String>,
    #[serde(default)]
    pub max_wait_secs: Option<i32>,
    #[serde(default)]
    pub agents: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct AgentStatus {
    pub agent_id: String,
    pub name: String,
    pub extension: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_call: Option<String>,
    #[serde(default)]
    pub idle_duration_secs: Option<u64>,
    #[serde(default)]
    pub total_calls: Option<u64>,
}

pub(crate) async fn list_queues(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, ApiError> {
    let pool = state.store.pool();
    let rows = sqlx::query("SELECT id, name, strategy, moh_file, max_wait_secs FROM call_queues ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|e| ApiError::internal(format!("查询呼叫队列失败: {e}")))?;

    let mut items = Vec::new();
    for r in rows {
        let id: String = r.get("id");
        let name: String = r.get("name");
        let strategy: String = r.get("strategy");
        let moh_file: String = r.get("moh_file");
        let max_wait_secs: i32 = r.get("max_wait_secs");

        let agent_rows = sqlx::query("SELECT agent_id FROM queue_agents WHERE queue_id = $1")
            .bind(&id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

        let agents: Vec<String> = agent_rows
            .into_iter()
            .map(|ar| ar.get("agent_id"))
            .collect();

        items.push(CallQueue {
            id,
            name,
            strategy: Some(strategy),
            moh_file: Some(moh_file),
            max_wait_secs: Some(max_wait_secs),
            agents: Some(agents),
        });
    }

    if q.export.unwrap_or(false) {
        let headers = vec![
            "队列 ID",
            "队列名称",
            "分配策略",
            "MOH 背景音乐文件",
            "最大排队等待超时(秒)",
            "绑定座席数",
        ];
        let mut rows = Vec::new();
        for item in items {
            let strat = match item.strategy.as_deref().unwrap_or("longest_idle") {
                "round_robin" => "轮询分发 (Round Robin)",
                "ring_all" => "群响 (Ring All)",
                "random" => "随机 (Random)",
                _ => "最长空闲优先 (Longest Idle)",
            };
            let moh = item
                .moh_file
                .clone()
                .unwrap_or_else(|| "moh.wav".to_string());
            let max_wait = item
                .max_wait_secs
                .map(|s| format!("{s}s"))
                .unwrap_or_else(|| "300s".to_string());
            let agent_count = item
                .agents
                .as_ref()
                .map(|a| format!("{} 个座席", a.len()))
                .unwrap_or_else(|| "0 个座席".to_string());
            rows.push(vec![
                item.id.clone(),
                item.name.clone(),
                strat.to_string(),
                moh,
                max_wait,
                agent_count,
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "queues.csv",
            &headers,
            &rows,
        ));
    }

    use axum::response::IntoResponse;
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page: 1,
        page_size: 20,
    })
    .into_response())
}

pub(crate) async fn create_queue(
    State(state): State<AppState>,
    Json(payload): Json<CallQueue>,
) -> Result<(StatusCode, Json<CallQueue>), ApiError> {
    let pool = state.store.pool();
    let strategy = payload.strategy.as_deref().unwrap_or("longest_idle");
    let moh_file = payload.moh_file.as_deref().unwrap_or("moh.wav");
    let max_wait_secs = payload.max_wait_secs.unwrap_or(300);

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(format!("开启队列事务失败: {e}")))?;

    sqlx::query(
        "INSERT INTO call_queues (id, name, strategy, moh_file, max_wait_secs) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO UPDATE SET name = $2, strategy = $3, moh_file = $4, max_wait_secs = $5"
    )
    .bind(&payload.id)
    .bind(&payload.name)
    .bind(strategy)
    .bind(moh_file)
    .bind(max_wait_secs)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(format!("保存呼叫队列失败: {e}")))?;

    sqlx::query("DELETE FROM queue_agents WHERE queue_id = $1")
        .bind(&payload.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(format!("清理队列座席失败: {e}")))?;

    if let Some(agents) = &payload.agents {
        for agent_id in agents {
            sqlx::query("INSERT INTO queue_agents (queue_id, agent_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(&payload.id)
                .bind(agent_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| ApiError::internal(format!("关联队列座席失败: {e}")))?;
        }
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(format!("提交队列事务失败: {e}")))?;

    Ok((StatusCode::CREATED, Json(payload)))
}

pub(crate) async fn update_queue(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(payload): Json<CallQueue>,
) -> Result<Json<CallQueue>, ApiError> {
    let _ = create_queue(State(state), Json(payload.clone())).await?;
    Ok(Json(payload))
}

pub(crate) async fn delete_queue(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state.store.pool();
    sqlx::query("DELETE FROM call_queues WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| ApiError::internal(format!("删除呼叫队列失败: {e}")))?;
    Ok(Json(serde_json::json!({"success": true})))
}

pub(crate) async fn list_agents(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, ApiError> {
    let pool = state.store.pool();
    let rows = sqlx::query(
        "SELECT agent_id, name, extension, status FROM call_agents ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::internal(format!("查询座席列表失败: {e}")))?;

    let mut items = Vec::new();
    for r in rows {
        let agent_id: String = r.get("agent_id");
        let name: String = r.get("name");
        let extension: String = r.get("extension");
        let status: String = r.get("status");

        items.push(AgentStatus {
            agent_id,
            name,
            extension,
            status: Some(status),
            current_call: None,
            idle_duration_secs: Some(120),
            total_calls: Some(0),
        });
    }

    if q.export.unwrap_or(false) {
        let headers = vec!["座席姓名", "座席工号", "SIP分机号", "当前状态", "当前通话"];
        let mut rows = Vec::new();
        for item in items {
            let status_str = match item.status.as_deref().unwrap_or("offline") {
                "idle" => "空闲 (Ready)",
                "in_call" => "通话中 (In Call)",
                "busy" => "示忙 (Busy)",
                _ => "离线 (Offline)",
            };
            rows.push(vec![
                item.name.clone(),
                item.agent_id.clone(),
                format!("sip:{}", item.extension),
                status_str.to_string(),
                item.current_call
                    .clone()
                    .unwrap_or_else(|| "无通话".to_string()),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "agents.csv",
            &headers,
            &rows,
        ));
    }

    use axum::response::IntoResponse;
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page: 1,
        page_size: 20,
    })
    .into_response())
}

pub(crate) async fn create_agent(
    State(state): State<AppState>,
    Json(payload): Json<AgentStatus>,
) -> Result<(StatusCode, Json<AgentStatus>), ApiError> {
    let pool = state.store.pool();
    let status = payload.status.as_deref().unwrap_or("idle");

    sqlx::query(
        "INSERT INTO call_agents (agent_id, name, extension, status) VALUES ($1, $2, $3, $4) ON CONFLICT (agent_id) DO UPDATE SET name = $2, extension = $3, status = $4"
    )
    .bind(&payload.agent_id)
    .bind(&payload.name)
    .bind(&payload.extension)
    .bind(status)
    .execute(pool)
    .await
    .map_err(|e| ApiError::internal(format!("保存座席信息失败: {e}")))?;

    Ok((StatusCode::CREATED, Json(payload)))
}

pub(crate) async fn update_agent(
    State(state): State<AppState>,
    Path(_id): Path<String>,
    Json(payload): Json<AgentStatus>,
) -> Result<Json<AgentStatus>, ApiError> {
    let _ = create_agent(State(state), Json(payload.clone())).await?;
    Ok(Json(payload))
}

pub(crate) async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let pool = state.store.pool();
    sqlx::query("DELETE FROM call_agents WHERE agent_id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| ApiError::internal(format!("删除座席失败: {e}")))?;
    Ok(Json(serde_json::json!({"success": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_queue_serialization() {
        let queue = CallQueue {
            id: "q1".to_string(),
            name: "Support".to_string(),
            strategy: Some("ringall".to_string()),
            moh_file: Some("default".to_string()),
            max_wait_secs: Some(300),
            agents: Some(vec!["agent1".to_string(), "agent2".to_string()]),
        };
        let serialized = serde_json::to_value(&queue).unwrap();
        assert_eq!(serialized["id"], "q1");
    }

    #[test]
    fn test_agent_status_serialization() {
        let agent = AgentStatus {
            agent_id: "agent1".to_string(),
            name: "张三".to_string(),
            extension: "1001".to_string(),
            status: Some("idle".to_string()),
            current_call: None,
            idle_duration_secs: Some(120),
            total_calls: Some(10),
        };
        let serialized = serde_json::to_value(&agent).unwrap();
        assert_eq!(serialized["agent_id"], "agent1");
    }
}
