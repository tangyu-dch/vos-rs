//! Aggregated resource details used by the management console.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::AppState;

type DetailError = (StatusCode, String);

/// Returns an extension together with its active contacts and assigned numbers.
pub(crate) async fn extension(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<Value>, DetailError> {
    let (users, registrations, numbers, allocations) = tokio::try_join!(
        state.store.list_users(),
        state.store.list_registrations(),
        state.store.list_numbers(),
        state.store.list_number_allocations(None),
    )
    .map_err(database_error)?;
    let user = users
        .into_iter()
        .find(|user| user.username == username)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "extension not found".to_string()))?;
    let registrations = registrations
        .into_iter()
        .filter(|registration| aor_username(&registration.aor) == username)
        .collect::<Vec<_>>();
    let allocated_numbers = allocations
        .into_iter()
        .filter(|allocation| {
            allocation.enabled
                && allocation.source_type == "extension"
                && allocation.source_id == username
        })
        .map(|allocation| allocation.number)
        .collect::<std::collections::HashSet<_>>();
    let numbers = numbers
        .into_iter()
        .filter(|number| {
            allocated_numbers.contains(&number.number)
                || number.username.as_deref() == Some(username.as_str())
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "extension": user,
        "credential": {"configured": true, "storage": "digest_ha1"},
        "registrations": registrations,
        "numbers": numbers,
    })))
}

/// Returns a trunk with health, number inventory and routing dependencies.
pub(crate) async fn trunk(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, DetailError> {
    let (gateways, numbers, allocations, routes, registrations) = tokio::try_join!(
        state.store.list_gateways_full(),
        state.store.list_numbers(),
        state.store.list_number_allocations(None),
        state.store.list_routes_full(),
        state.store.list_registrations(),
    )
    .map_err(database_error)?;
    let trunk = gateways
        .into_iter()
        .find(|gateway| gateway.id == id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "trunk not found".to_string()))?;
    let allocated_numbers = allocations
        .into_iter()
        .filter(|allocation| {
            allocation.enabled && allocation.source_type == "trunk" && allocation.source_id == id
        })
        .map(|allocation| allocation.number)
        .collect::<std::collections::HashSet<_>>();
    let numbers = numbers
        .into_iter()
        .filter(|number| {
            if trunk.role.as_deref() == Some("egress") {
                number.owner_egress_trunk_id.as_deref() == Some(id.as_str())
                    || number.gateway_id.as_deref() == Some(id.as_str())
            } else {
                allocated_numbers.contains(&number.number)
            }
        })
        .collect::<Vec<_>>();
    let routes = routes
        .into_iter()
        .filter(|route| route.gateway_id == id)
        .collect::<Vec<_>>();
    let registrations = registrations
        .into_iter()
        .filter(|r| {
            if let Some(ref reg_user) = trunk.access_username {
                if aor_username(&r.aor) == reg_user {
                    return true;
                }
            }
            false
        })
        .collect::<Vec<_>>();
    let health = json!({
        "state": trunk.circuit_state,
        "active_calls": trunk.current_concurrent,
        "capacity": trunk.max_capacity,
        "enabled": trunk.enabled,
    });
    Ok(Json(json!({
        "trunk": trunk,
        "health": health,
        "numbers": numbers,
        "routes": routes,
        "registrations": registrations,
    })))
}

fn aor_username(aor: &str) -> &str {
    aor.strip_prefix("sip:")
        .unwrap_or(aor)
        .split('@')
        .next()
        .unwrap_or(aor)
}

fn database_error(error: sqlx::Error) -> DetailError {
    tracing::error!(%error, "读取资源详情失败");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "database read failed".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::aor_username;
    use cdr_core::SipRegistration;
    use serde_json::json;
    use time::OffsetDateTime;

    #[test]
    fn extracts_username_from_common_aor_forms() {
        assert_eq!(aor_username("alice"), "alice");
        assert_eq!(aor_username("sip:alice@example.com"), "alice");
    }

    #[test]
    fn test_registration_details_serialization_compatible_with_web_frontend() {
        let registration = SipRegistration {
            aor: "sip:1001@127.0.0.1".to_string(),
            contact_uri: "sip:1001@192.168.1.100:5060".to_string(),
            received_from: "192.168.1.100:5060".to_string(),
            expires_at: OffsetDateTime::now_utc(),
            path: vec![],
            updated_at: None,
        };
        let val = json!(registration);
        assert_eq!(val["contact_uri"], "sip:1001@192.168.1.100:5060");
        assert_eq!(val["received_from"], "192.168.1.100:5060");
    }
}
