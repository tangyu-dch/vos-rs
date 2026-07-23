use axum::{
    extract::{Multipart, State},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use rust_decimal::prelude::FromStr;
use crate::{ApiError, AppState};

async fn get_csv_content(mut multipart: Multipart) -> Result<String, ApiError> {
    while let Some(field) = multipart.next_field().await.map_err(|e| ApiError::internal(e.to_string()))? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let bytes = field.bytes().await.map_err(|e| ApiError::internal(e.to_string()))?;
            let content = String::from_utf8(bytes.to_vec())
                .map_err(|e| ApiError::internal(format!("CSV 不是有效的 UTF-8 编码: {e}")))?;
            return Ok(content);
        }
    }
    Err(ApiError::internal("未找到名为 file 的上传字段"))
}

async fn get_digest_realm(state: &AppState) -> Result<String, ApiError> {
    let realm = sqlx::query_scalar::<_, String>(
        "SELECT config_value FROM system_configs WHERE config_key = 'realm'",
    )
    .fetch_optional(state.store.pool())
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .unwrap_or_else(|| "vos-rs".to_string());
    Ok(realm)
}

// === SIP Users Import ===

pub async fn import_users_template() -> impl IntoResponse {
    crate::utils::to_csv_response(
        "users_import_template.csv",
        &["分机号", "注册密码"],
        &vec![vec!["8001".to_string(), "123456".to_string()]],
    )
}

pub async fn import_users(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let content = get_csv_content(multipart).await?;
    let parsed = crate::utils::parse_csv(&content);
    if parsed.len() < 2 {
        return Err(ApiError::internal("CSV 模板为空或缺少数据行"));
    }

    let realm = get_digest_realm(&state).await?;
    let pool = state.store.pool();
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let mut imported = 0;
    let mut hot_cache_updates = Vec::new();

    for (idx, row) in parsed.iter().skip(1).enumerate() {
        if row.len() < 2 {
            return Err(ApiError::internal(format!("第 {} 行格式错误：缺少密码列", idx + 2)));
        }
        let username = row[0].trim();
        let password = row[1].trim();
        if username.is_empty() || password.is_empty() {
            return Err(ApiError::internal(format!("第 {} 行包含空分机号或密码", idx + 2)));
        }

        let ha1 = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", username, realm, password).as_bytes())
        );

        sqlx::query("INSERT INTO sip_users (username, password) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET password = EXCLUDED.password")
            .bind(username)
            .bind(&ha1)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(format!("导入分机 {} 失败: {e}", username)))?;

        hot_cache_updates.push((username.to_string(), ha1));
        imported += 1;
    }

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    // 同步写入 Redis 缓存
    for (username, ha1) in hot_cache_updates {
        let _ = crate::hot_cache::set_auth_user(&state, &username, &ha1).await;
    }

    Ok(Json(json!({ "success": true, "imported_count": imported })))
}

// === Numbers Import ===

pub async fn import_numbers_template() -> impl IntoResponse {
    crate::utils::to_csv_response(
        "numbers_import_template.csv",
        &["号码", "关联分机", "落地中继", "呼叫方向", "最大并发", "状态"],
        &vec![vec![
            "13800138000".to_string(),
            "8001".to_string(),
            "carrier-a".to_string(),
            "both".to_string(),
            "10".to_string(),
            "active".to_string(),
        ]],
    )
}

pub async fn import_numbers(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let content = get_csv_content(multipart).await?;
    let parsed = crate::utils::parse_csv(&content);
    if parsed.len() < 2 {
        return Err(ApiError::internal("CSV 模板为空或缺少数据行"));
    }

    let pool = state.store.pool();
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let mut imported = 0;

    for (idx, row) in parsed.iter().skip(1).enumerate() {
        if row.len() < 6 {
            return Err(ApiError::internal(format!("第 {} 行格式错误：需要包含 6 个字段", idx + 2)));
        }
        let number = row[0].trim();
        let username = row[1].trim();
        let gateway_id = row[2].trim();
        let direction = row[3].trim();
        let max_concurrent_str = row[4].trim();
        let status = row[5].trim();

        if number.is_empty() || status.is_empty() {
            return Err(ApiError::internal(format!("第 {} 行包含空号码或空状态", idx + 2)));
        }

        let username_opt = if username.is_empty() { None } else { Some(username) };
        let gw_opt = if gateway_id.is_empty() { None } else { Some(gateway_id) };
        let dir_opt = if direction.is_empty() { None } else { Some(direction) };
        let max_concurrent = max_concurrent_str.parse::<i32>().ok();

        sqlx::query(
            "INSERT INTO number_inventory (number, username, gateway_id, owner_egress_trunk_id, direction, max_concurrent, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, now()) \
             ON CONFLICT (number) DO UPDATE SET username=EXCLUDED.username, gateway_id=EXCLUDED.gateway_id, \
             owner_egress_trunk_id=EXCLUDED.owner_egress_trunk_id, direction=EXCLUDED.direction, \
             max_concurrent=EXCLUDED.max_concurrent, status=EXCLUDED.status, updated_at=now()"
        )
        .bind(number)
        .bind(username_opt)
        .bind(gw_opt)
        .bind(gw_opt) // owner_egress_trunk_id 同 gateway_id
        .bind(dir_opt)
        .bind(max_concurrent)
        .bind(status)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(format!("导入号码 {} 失败: {e}", number)))?;

        imported += 1;
    }

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    // 通知 sip-edge 重新加载路由
    crate::routes::publish_route_reload(&state.nats_client).await;

    Ok(Json(json!({ "success": true, "imported_count": imported })))
}

// === Rates Import ===

pub async fn import_rates_template() -> impl IntoResponse {
    crate::utils::to_csv_response(
        "rates_import_template.csv",
        &["费率标识", "前缀号码", "每分钟费率", "计费周期(秒)", "单周期价格"],
        &vec![vec![
            "cn-rate".to_string(),
            "86".to_string(),
            "0.1".to_string(),
            "60".to_string(),
            "0.1".to_string(),
        ]],
    )
}

pub async fn import_rates(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let content = get_csv_content(multipart).await?;
    let parsed = crate::utils::parse_csv(&content);
    if parsed.len() < 2 {
        return Err(ApiError::internal("CSV 模板为空或缺少数据行"));
    }

    let pool = state.store.pool();
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let mut imported = 0;

    for (idx, row) in parsed.iter().skip(1).enumerate() {
        if row.len() < 5 {
            return Err(ApiError::internal(format!("第 {} 行格式错误：需要包含 5 个字段", idx + 2)));
        }
        let id = row[0].trim();
        let prefix = row[1].trim();
        let rate_str = row[2].trim();
        let interval_str = row[3].trim();
        let price_str = row[4].trim();

        if id.is_empty() || prefix.is_empty() {
            return Err(ApiError::internal(format!("第 {} 行包含空标识或空前缀", idx + 2)));
        }

        let rate = rust_decimal::Decimal::from_str(rate_str)
            .map_err(|_| ApiError::internal(format!("第 {} 行费率数值无效: {}", idx + 2, rate_str)))?;
        let interval = interval_str.parse::<i32>()
            .map_err(|_| ApiError::internal(format!("第 {} 行计费周期数值无效: {}", idx + 2, interval_str)))?;
        let price = rust_decimal::Decimal::from_str(price_str)
            .map_err(|_| ApiError::internal(format!("第 {} 行单周期价格数值无效: {}", idx + 2, price_str)))?;

        sqlx::query(
            "INSERT INTO billing_rates (id, prefix, rate_per_minute, billing_interval_secs, price_per_interval, description) VALUES ($1,$2,$3,$4,$5,$6) \
             ON CONFLICT (id) DO UPDATE SET prefix=EXCLUDED.prefix, rate_per_minute=EXCLUDED.rate_per_minute, billing_interval_secs=EXCLUDED.billing_interval_secs, price_per_interval=EXCLUDED.price_per_interval, description=EXCLUDED.description"
        )
        .bind(id)
        .bind(prefix)
        .bind(rate)
        .bind(interval)
        .bind(price)
        .bind(None::<&str>)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(format!("导入费率 {} 失败: {e}", id)))?;

        imported += 1;
    }

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    // 预热/刷新内存中的费率配置
    let _ = crate::hot_cache::rebuild_billing_rates(&state).await;

    Ok(Json(json!({ "success": true, "imported_count": imported })))
}

// === Routes Import ===

pub async fn import_routes_template() -> impl IntoResponse {
    crate::utils::to_csv_response(
        "routes_import_template.csv",
        &["路由标识", "号码前缀", "优先级", "目标网关", "每呼叫成本", "权重", "生效时间(开始)", "生效时间(结束)"],
        &vec![vec![
            "to-carrier-a".to_string(),
            "86".to_string(),
            "100".to_string(),
            "carrier-a".to_string(),
            "0.01".to_string(),
            "10".to_string(),
            "00:00".to_string(),
            "23:59".to_string(),
        ]],
    )
}

pub async fn import_routes(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let content = get_csv_content(multipart).await?;
    let parsed = crate::utils::parse_csv(&content);
    if parsed.len() < 2 {
        return Err(ApiError::internal("CSV 模板为空或缺少数据行"));
    }

    let pool = state.store.pool();
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let mut imported = 0;

    for (idx, row) in parsed.iter().skip(1).enumerate() {
        if row.len() < 8 {
            return Err(ApiError::internal(format!("第 {} 行格式错误：需要包含 8 个字段", idx + 2)));
        }
        let id = row[0].trim();
        let prefix = row[1].trim();
        let priority_str = row[2].trim();
        let gateway_id = row[3].trim();
        let cost_str = row[4].trim();
        let weight_str = row[5].trim();
        let time_start = row[6].trim();
        let time_end = row[7].trim();

        if id.is_empty() || prefix.is_empty() || gateway_id.is_empty() {
            return Err(ApiError::internal(format!("第 {} 行包含空标识、空前缀或空网关", idx + 2)));
        }

        let priority = priority_str.parse::<i32>()
            .map_err(|_| ApiError::internal(format!("第 {} 行优先级数值无效: {}", idx + 2, priority_str)))?;
        let cost = f64::from_str(cost_str)
            .map_err(|_| ApiError::internal(format!("第 {} 行呼叫成本数值无效: {}", idx + 2, cost_str)))?;
        let weight = weight_str.parse::<i32>()
            .map_err(|_| ApiError::internal(format!("第 {} 行权重数值无效: {}", idx + 2, weight_str)))?;

        let start_opt = if time_start.is_empty() { None } else { Some(time_start) };
        let end_opt = if time_end.is_empty() { None } else { Some(time_end) };

        sqlx::query(
            "INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight, time_start, time_end, topology) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb) \
             ON CONFLICT (id) DO UPDATE \
             SET prefix = EXCLUDED.prefix, \
                 priority = EXCLUDED.priority, \
                 gateway_id = EXCLUDED.gateway_id, \
                 cost = EXCLUDED.cost, \
                 weight = EXCLUDED.weight, \
                 time_start = EXCLUDED.time_start, \
                 time_end = EXCLUDED.time_end, \
                 topology = EXCLUDED.topology"
        )
        .bind(id)
        .bind(prefix)
        .bind(priority)
        .bind(gateway_id)
        .bind(cost)
        .bind(weight)
        .bind(start_opt)
        .bind(end_opt)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(format!("导入路由 {} 失败: {e}", id)))?;

        imported += 1;
    }

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    // 通知 sip-edge 重新加载路由
    crate::routes::publish_route_reload(&state.nats_client).await;

    Ok(Json(json!({ "success": true, "imported_count": imported })))
}
