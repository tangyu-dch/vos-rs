//! Versioned management API routes and response contract.

use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Router,
};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    anti_fraud, audit, auth, billing, call_center, calls, cdr, dashboard, details, gateways, ivr_menus, media_cluster,
    numbers, recording, registrations, report, routes, sip_cluster, system, termination, users,
    AppState,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Builds public v1 endpoints. These routes do not require a bearer token.
pub(crate) fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/sessions", post(auth::login))
        .route_layer(axum::middleware::from_fn(response_contract))
}

/// Builds authenticated v1 endpoints grouped by business domain.
pub(crate) fn protected_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/me", get(auth::current_session))
        .merge(overview_routes())
        .merge(subscriber_routes())
        .merge(interconnect_routes())
        .merge(termination_routes())
        .merge(routing_routes())
        .merge(call_routes())
        .merge(billing_routes())
        .merge(security_routes())
        .merge(infrastructure_routes())
        .merge(call_center_routes())
        .merge(ivr_routes())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::audit_log,
        ))
        .route_layer(axum::middleware::from_fn_with_state(state, crate::jwt_auth))
        .route_layer(axum::middleware::from_fn(response_contract))
}

fn overview_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/overview/summary",
            get(dashboard::get_dashboard_stats),
        )
        .route(
            "/api/v1/overview/trends",
            get(dashboard::get_dashboard_trend),
        )
        .route("/api/v1/overview/events", get(dashboard::dashboard_events))
}

fn subscriber_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/extensions",
            get(users::list_users).post(users::create_user),
        )
        .route(
            "/api/v1/extensions/:username",
            get(details::extension)
                .put(users::update_user)
                .delete(users::delete_user),
        )
        .route(
            "/api/v1/registrations",
            get(registrations::list_registrations),
        )
        .route(
            "/api/v1/numbers",
            get(numbers::list_numbers).post(numbers::create_number),
        )
        .route(
            "/api/v1/numbers/:number",
            put(numbers::update_number).delete(numbers::delete_number),
        )
}

fn interconnect_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/trunks",
            get(gateways::list_gateways).post(gateways::create_gateway),
        )
        .route(
            "/api/v1/trunks/:id",
            get(details::trunk)
                .put(gateways::update_gateway)
                .delete(gateways::delete_gateway),
        )
}

fn termination_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/trunks/:id/ip-rules",
            get(termination::list_ip_rules).put(termination::replace_ip_rules),
        )
        .route(
            "/api/v1/trunks/:id/egress-endpoints",
            get(termination::list_endpoints).put(termination::replace_endpoints),
        )
        .route(
            "/api/v1/trunks/:id/outbound-policy",
            get(termination::get_trunk_policy).put(termination::put_trunk_policy),
        )
        .route(
            "/api/v1/extensions/:username/outbound-policy",
            get(termination::get_extension_policy).put(termination::put_extension_policy),
        )
        .route(
            "/api/v1/numbers/:number/owner",
            put(termination::set_number_owner),
        )
        .route(
            "/api/v1/numbers/:number/allocations",
            get(termination::list_allocations).put(termination::replace_allocations),
        )
        .route(
            "/api/v1/numbers/:number/did-destination",
            get(termination::get_number_did).put(termination::put_number_did),
        )
        .route(
            "/api/v1/caller-pools",
            get(termination::list_caller_pools).post(termination::create_caller_pool),
        )
        .route(
            "/api/v1/caller-pools/:id",
            put(termination::update_caller_pool).delete(termination::delete_caller_pool),
        )
        .route(
            "/api/v1/caller-pools/:id/members",
            get(termination::list_caller_pool_members)
                .put(termination::replace_caller_pool_members),
        )
        .route(
            "/api/v1/egress-groups",
            get(termination::list_egress_groups).post(termination::create_egress_group),
        )
        .route(
            "/api/v1/egress-groups/:id",
            put(termination::update_egress_group).delete(termination::delete_egress_group),
        )
        .route(
            "/api/v1/egress-groups/:id/members",
            get(termination::list_egress_group_members)
                .put(termination::replace_egress_group_members),
        )
        .route("/api/v1/outbound-policies", get(termination::list_policies))
        .route(
            "/api/v1/outbound-policies/:source_type/:source_id",
            get(termination::get_policy)
                .put(termination::put_policy)
                .delete(termination::delete_policy),
        )
        .route(
            "/api/v1/did-destinations",
            get(termination::list_dids).post(termination::create_did),
        )
        .route(
            "/api/v1/did-destinations/:number",
            put(termination::update_did).delete(termination::delete_did),
        )
}

fn routing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/routing/rules",
            get(routes::list_routes).post(routes::create_route),
        )
        .route(
            "/api/v1/routing/rules/:id",
            put(routes::update_route).delete(routes::delete_route),
        )
        .route("/api/v1/routing/simulations", get(calls::route_preview))
}

fn call_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/calls", get(cdr::list_cdrs))
        .route("/api/v1/calls/active", get(calls::list_active))
        .route("/api/v1/calls/:call_id", get(calls::call_detail))
        .route("/api/v1/calls/:call_id/media", get(calls::call_media))
        .route("/api/v1/calls/:call_id/dtmf", get(cdr::get_dtmf_events))
        .route("/api/v1/calls/:call_id/sipflow", get(calls::call_sipflow))
        .route(
            "/api/v1/calls/:call_id/recording",
            get(recording::get_recording_audio),
        )
        .route(
            "/api/v1/calls/:call_id/actions/terminate",
            post(calls::terminate_call),
        )
        .route("/api/v1/calls/:call_id/actions/play", post(calls::play))
        .route(
            "/api/v1/calls/:call_id/actions/stop-play",
            post(calls::stop_play),
        )
        .route("/api/v1/calls/:call_id/actions/mute", post(calls::mute))
        .route("/api/v1/calls/:call_id/actions/unmute", post(calls::unmute))
        .route(
            "/api/v1/calls/:call_id/actions/monitor",
            post(calls::monitor),
        )
        .route(
            "/api/v1/calls/:call_id/actions/stop-monitor",
            post(calls::stop_monitor),
        )
        .route("/api/v1/reports/summary", get(report::get_report_summary))
        .route("/api/v1/reports/export", get(report::export_cdrs_csv))
}

fn billing_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/billing/rates",
            get(billing::list_rates).post(billing::create_rate),
        )
        .route(
            "/api/v1/billing/rates/:id",
            put(billing::update_rate).delete(billing::delete_rate),
        )
        .route("/api/v1/billing/accounts", get(billing::list_accounts))
        .route(
            "/api/v1/billing/accounts/:username/credit",
            post(billing::credit_account),
        )
        .route("/api/v1/billing/transactions", get(billing::list_ledger))
        .route("/api/v1/billing/reconciliations", post(billing::reconcile))
}

fn security_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/security/anti-fraud/policies",
            get(anti_fraud::list_anti_fraud_rules).post(anti_fraud::create_anti_fraud_rule),
        )
        .route(
            "/api/v1/security/anti-fraud/policies/:id",
            put(anti_fraud::update_anti_fraud_rule).delete(anti_fraud::delete_anti_fraud_rule),
        )
        .route(
            "/api/v1/security/anti-fraud/settings",
            get(anti_fraud::list_anti_fraud_config),
        )
        .route(
            "/api/v1/security/anti-fraud/settings/:key",
            put(anti_fraud::update_anti_fraud_config),
        )
        .route("/api/v1/security/audit-logs", get(audit::list_audit_logs))
}

fn infrastructure_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/infrastructure/settings",
            get(system::get_system_configs).post(system::update_system_configs),
        )
        .route(
            "/api/v1/infrastructure/media-cluster",
            get(media_cluster::get_media_cluster).put(media_cluster::update_media_cluster),
        )
        .route(
            "/api/v1/infrastructure/sip-cluster",
            get(sip_cluster::get_sip_cluster_status),
        )
        .route(
            "/api/v1/infrastructure/sip-cluster/nodes/:node_id/:action",
            post(sip_cluster::control_sip_cluster_node),
        )
        .route(
            "/api/v1/infrastructure/media/metrics",
            get(calls::media_metrics),
        )
}

fn call_center_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/call-center/queues",
            get(call_center::list_queues).post(call_center::create_queue),
        )
        .route(
            "/api/v1/call-center/queues/:id",
            put(call_center::update_queue).delete(call_center::delete_queue),
        )
        .route(
            "/api/v1/call-center/agents",
            get(call_center::list_agents).post(call_center::create_agent),
        )
        .route(
            "/api/v1/call-center/agents/:id",
            put(call_center::update_agent).delete(call_center::delete_agent),
        )
}

fn ivr_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/ivr/menus",
            get(ivr_menus::list_menus).post(ivr_menus::create_menu),
        )
        .route(
            "/api/v1/ivr/menus/:id",
            get(ivr_menus::get_menu)
                .put(ivr_menus::update_menu)
                .delete(ivr_menus::delete_menu),
        )
}

async fn response_contract(mut request: Request, next: Next) -> Response {
    let request_id = request_id(&request);
    let path = request.uri().path().to_string();
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        request.headers_mut().insert("x-request-id", value);
    }
    let response = next.run(request).await;
    envelope_response(response, &request_id, &path).await
}

fn request_id(request: &Request) -> String {
    request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

async fn envelope_response(response: Response, request_id: &str, path: &str) -> Response {
    let status = response.status();
    if !is_json_response(&response) && status.is_success() && is_raw_response(path) {
        return with_request_id(response, request_id);
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_RESPONSE_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, %request_id, "读取 API 响应失败");
            return contract_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                request_id,
                json!({
                    "code": 50001,
                    "message": "internal_error",
                    "details": "无法读取服务响应"
                }),
            );
        }
    };
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        let detail = String::from_utf8_lossy(&bytes).trim().to_string();
        if detail.is_empty() {
            Value::Null
        } else {
            Value::String(detail)
        }
    });
    let payload = contract_payload(status, value, request_id, path);
    let encoded = serde_json::to_vec(&payload).unwrap_or_default();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(request_id) {
        parts.headers.insert("x-request-id", value);
    }
    Response::from_parts(parts, Body::from(encoded))
}

fn is_raw_response(path: &str) -> bool {
    path.ends_with("/recording") || path == "/api/v1/reports/export"
}

fn is_json_response(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
}

fn contract_payload(status: StatusCode, value: Value, request_id: &str, path: &str) -> Value {
    let timestamp = OffsetDateTime::now_utc().unix_timestamp();
    if status.is_success() {
        let data = success_data(value, path);
        json!({"code": 0, "message": "success", "data": data, "timestamp": timestamp, "request_id": request_id})
    } else {
        let details = error_details(value, status);
        json!({"code": error_code(status), "message": error_message(status), "details": details, "timestamp": timestamp, "request_id": request_id})
    }
}

fn success_data(value: Value, path: &str) -> Value {
    if let Value::Object(mut object) = value {
        if let (Some(items), Some(total), Some(page), Some(page_size)) = (
            object.remove("items"),
            object.remove("total"),
            object.remove("page"),
            object.remove("page_size"),
        ) {
            let total_value = total.as_i64().unwrap_or_default();
            let size_value = page_size.as_i64().unwrap_or(1).max(1);
            return json!({"items": items, "pagination": {"page": page, "page_size": page_size, "total": total, "total_pages": (total_value + size_value - 1) / size_value}});
        }
        let value = Value::Object(object);
        if path == "/api/v1/infrastructure/settings" {
            return json!({"values": value, "apply_mode": "restart_required", "restart_required": true});
        }
        return value;
    }
    if path == "/api/v1/infrastructure/settings" {
        return json!({"values": value, "apply_mode": "restart_required", "restart_required": true});
    }
    value
}

fn error_details(value: Value, status: StatusCode) -> Value {
    match value {
        Value::Object(mut object) => object
            .remove("details")
            .or_else(|| object.remove("error"))
            .or_else(|| object.remove("message"))
            .unwrap_or_else(|| {
                Value::String(
                    status
                        .canonical_reason()
                        .unwrap_or("request failed")
                        .to_string(),
                )
            }),
        Value::Null => Value::String(
            status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_string(),
        ),
        other => other,
    }
}

fn error_code(status: StatusCode) -> u16 {
    status.as_u16().saturating_mul(100).saturating_add(1)
}

fn error_message(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "invalid_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::UNPROCESSABLE_ENTITY => "validation_failed",
        StatusCode::BAD_GATEWAY => "upstream_error",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        _ => "internal_error",
    }
}

fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

fn contract_response(status: StatusCode, request_id: &str, value: Value) -> Response {
    let mut response = (status, axum::Json(value)).into_response();
    response = with_request_id(response, request_id);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paginated_payload_uses_v1_pagination_shape() {
        let value = json!({"items": [1, 2], "total": 21, "page": 2, "page_size": 10});
        let payload = contract_payload(StatusCode::OK, value, "req-1", "/api/v1/calls");
        assert_eq!(payload["data"]["pagination"]["total_pages"], 3);
        assert_eq!(payload["data"]["items"], json!([1, 2]));
    }

    #[test]
    fn error_payload_does_not_leak_legacy_shape() {
        let payload = contract_payload(
            StatusCode::UNAUTHORIZED,
            json!({"error": "bad token"}),
            "req-2",
            "/api/v1/calls",
        );
        assert_eq!(payload["code"], 40101);
        assert_eq!(payload["message"], "unauthorized");
        assert_eq!(payload["details"], "bad token");
    }

    #[test]
    fn settings_explicitly_require_restart() {
        let data = success_data(
            json!({"recording_enabled": "true"}),
            "/api/v1/infrastructure/settings",
        );
        assert_eq!(data["apply_mode"], "restart_required");
        assert_eq!(data["restart_required"], true);
    }

    #[test]
    fn only_download_endpoints_bypass_json_contract() {
        assert!(is_raw_response("/api/v1/calls/call-1/recording"));
        assert!(is_raw_response("/api/v1/reports/export"));
        assert!(!is_raw_response("/api/v1/extensions/alice"));
    }
}
