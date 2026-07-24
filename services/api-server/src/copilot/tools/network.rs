//! Copilot 工具实现：中继网关与前缀路由管理

use serde_json::{json, Value};

use super::urlencoding_str;
use crate::copilot::TelecomCopilotEngine;

impl<'a> TelecomCopilotEngine<'a> {
    pub(crate) async fn tool_list_gateways(&self) -> Value {
        let gws = self
            .state
            .store
            .list_gateways_full()
            .await
            .unwrap_or_default();
        json!({ "gateways": gws })
    }

    pub(crate) async fn tool_preview_route(&self, args: &Value) -> Value {
        let destination = args
            .get("callee")
            .or_else(|| args.get("destination"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let url = format!(
            "{}/manage/route-preview?destination={}",
            self.state.sip_manage_base,
            urlencoding_str(destination)
        );
        let req = self.state.internal_client.get(&url);
        let req = if !self.state.internal_secret.is_empty() {
            req.header("X-VOS-Token", &self.state.internal_secret)
        } else {
            req
        };
        match req.send().await {
            Ok(r) => r.json::<Value>().await.unwrap_or_else(|_| json!({})),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_create_gateway(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let ip = args
            .get("ip_address")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let port = args.get("port").and_then(|v| v.as_i64()).unwrap_or(5060) as i32;
        if id.is_empty() || name.is_empty() || ip.is_empty() {
            return json!({ "success": false, "error": "网关 ID、名称与 IP 地址不能为空" });
        }
        let existing_gw: Option<(String, String)> =
            sqlx::query_as("SELECT id, name FROM sip_gateways WHERE ip_address = $1 AND id != $2")
                .bind(ip)
                .bind(id)
                .fetch_optional(self.state.store.pool())
                .await
                .unwrap_or(None);

        if let Some((dup_id, dup_name)) = existing_gw {
            return json!({
                "success": false,
                "conflict": true,
                "warning": format!("目标 IP 地址 `{}` 已被已有网关 `{}` ({}) 使用。请确认 IP 是否重复。", ip, dup_name, dup_id)
            });
        }

        let res = sqlx::query(
            "INSERT INTO sip_gateways (id, name, ip_address, port, enabled) VALUES ($1, $2, $3, $4, true) \
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, ip_address = EXCLUDED.ip_address, port = EXCLUDED.port"
        )
        .bind(id)
        .bind(name)
        .bind(ip)
        .bind(port)
        .execute(self.state.store.pool())
        .await;
        match res {
            Ok(_) => {
                json!({ "success": true, "message": format!("中继网关 `{}` ({}:{}) 创建成功", name, ip, port) })
            }
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_gateway(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        match self.state.store.delete_gateway(id).await {
            Ok(true) => json!({ "success": true, "message": format!("网关 {} 已成功删除", id) }),
            Ok(false) => json!({ "success": false, "error": format!("网关 {} 不存在", id) }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_create_route(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
        let gw_id = args
            .get("gateway_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let priority = args.get("priority").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        if id.is_empty() || prefix.is_empty() || gw_id.is_empty() {
            return json!({ "success": false, "error": "路由 ID、号码前缀与网关 ID 不能为空" });
        }
        // 1. 校验目标中继网关是否存在
        let gw_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id = $1)")
                .bind(gw_id)
                .fetch_one(self.state.store.pool())
                .await
                .unwrap_or(false);
        if !gw_exists {
            return json!({
                "success": false,
                "conflict": true,
                "error": format!("目标中继网关 `{}` 不存在，无法创建路由。请先创建该网关或指定已有的网关 ID。", gw_id)
            });
        }
        // 2. 校验前缀重复或覆盖冲突
        let existing_route: Option<(String, String)> =
            sqlx::query_as("SELECT id, gateway_id FROM sip_routes WHERE prefix = $1 AND id != $2")
                .bind(prefix)
                .bind(id)
                .fetch_optional(self.state.store.pool())
                .await
                .unwrap_or(None);

        if let Some((ext_id, ext_gw)) = existing_route {
            return json!({
                "success": false,
                "conflict": true,
                "warning": format!("前缀号码 `{}` 已被路由 `{}` 使用（指向网关 `{}`）。如需覆盖请明确指定路由 ID。", prefix, ext_id, ext_gw),
                "existing_route_id": ext_id
            });
        }

        let res = sqlx::query(
            "INSERT INTO sip_routes (id, prefix, priority, gateway_id) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (id) DO UPDATE SET prefix = EXCLUDED.prefix, priority = EXCLUDED.priority, gateway_id = EXCLUDED.gateway_id"
        )
        .bind(id)
        .bind(prefix)
        .bind(priority)
        .bind(gw_id)
        .execute(self.state.store.pool())
        .await;
        match res {
            Ok(_) => {
                let _ =
                    crate::resources::routes::publish_route_reload(&self.state.nats_client).await;
                json!({ "success": true, "message": format!("前缀路由 `{}` (前缀: {}, 网关: {}) 创建成功并实时重载选路引擎", id, prefix, gw_id) })
            }
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_list_routes(&self) -> Value {
        match self.state.store.list_routes_full().await {
            Ok(routes) => json!({ "routes": routes }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_route(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        match self.state.store.delete_route(id).await {
            Ok(true) => {
                let _ =
                    crate::resources::routes::publish_route_reload(&self.state.nats_client).await;
                json!({ "success": true, "message": format!("路由 {} 已成功删除", id) })
            }
            Ok(false) => json!({ "success": false, "error": format!("路由 {} 不存在", id) }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}
