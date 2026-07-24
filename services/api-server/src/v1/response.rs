use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub(super) async fn response_contract(mut request: Request, next: Next) -> Response {
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
    if (!is_json_response(&response) && status.is_success() && is_raw_response(path))
        || is_csv_response(&response)
        || is_sse_response(&response)
    {
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

fn is_csv_response(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/csv"))
}

/// SSE 流式响应（`text/event-stream`）不做 envelope 包装，直接透传
fn is_sse_response(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
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
