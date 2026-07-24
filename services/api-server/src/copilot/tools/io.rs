//! Copilot 工具实现：CSV 导入导出

use rust_decimal::Decimal;
use serde_json::{json, Value};

use super::urlencoding_str;
use crate::copilot::TelecomCopilotEngine;

impl<'a> TelecomCopilotEngine<'a> {
    pub(crate) async fn tool_export_cdrs(&self, args: &Value) -> Value {
        let caller = args.get("caller").and_then(|v| v.as_str()).unwrap_or("");
        let callee = args.get("callee").and_then(|v| v.as_str()).unwrap_or("");
        let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let start_time = args
            .get("start_time")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let end_time = args.get("end_time").and_then(|v| v.as_str()).unwrap_or("");

        let (cdrs, total) = self
            .state
            .store
            .list_cdrs(
                1,
                5,
                if status.is_empty() {
                    None
                } else {
                    Some(status)
                },
                None,
                if caller.is_empty() {
                    None
                } else {
                    Some(caller)
                },
                if callee.is_empty() {
                    None
                } else {
                    Some(callee)
                },
                None,
                None,
                None,
            )
            .await
            .unwrap_or((vec![], 0));

        let mut download_url = "/api/v1/reports/export?limit=5000".to_string();
        if !caller.is_empty() {
            download_url.push_str(&format!("&caller={}", urlencoding_str(caller)));
        }
        if !callee.is_empty() {
            download_url.push_str(&format!("&callee={}", urlencoding_str(callee)));
        }
        if !status.is_empty() {
            download_url.push_str(&format!("&status={}", urlencoding_str(status)));
        }
        if !start_time.is_empty() {
            download_url.push_str(&format!("&start_time={}", urlencoding_str(start_time)));
        }
        if !end_time.is_empty() {
            download_url.push_str(&format!("&end_time={}", urlencoding_str(end_time)));
        }

        json!({
            "success": true,
            "total_matched": total,
            "download_endpoint": download_url,
            "preview_sample": cdrs,
            "message": format!("已根据筛选条件成功匹配出 {} 条 CDR 呼叫记录。", total),
            "download_markdown": format!("[📥 点击这里下载全量 CDR 呼叫详单数据报表 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_export_extensions(&self) -> Value {
        let download_url = "/api/v1/users?export=true";
        json!({
            "success": true,
            "download_endpoint": download_url,
            "download_markdown": format!("[📥 点击导出下载全量 SIP 分机账号数据报表 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_export_gateways(&self) -> Value {
        let download_url = "/api/v1/gateways?export=true";
        json!({
            "success": true,
            "download_endpoint": download_url,
            "download_markdown": format!("[📥 点击导出下载全量中继网关节点数据报表 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_export_routes(&self) -> Value {
        let download_url = "/api/v1/routes?export=true";
        json!({
            "success": true,
            "download_endpoint": download_url,
            "download_markdown": format!("[📥 点击导出下载全量前缀选路路由规则 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_export_rates(&self) -> Value {
        let download_url = "/api/v1/rates?export=true";
        json!({
            "success": true,
            "download_endpoint": download_url,
            "download_markdown": format!("[📥 点击导出下载全量呼叫资费表 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_export_billing_accounts(&self) -> Value {
        let download_url = "/api/v1/billing/accounts?export=true";
        json!({
            "success": true,
            "download_endpoint": download_url,
            "download_markdown": format!("[📥 点击导出下载全量计费账户数据报表 (CSV)]({})", download_url)
        })
    }

    pub(crate) async fn tool_import_extensions(&self, args: &Value) -> Value {
        let content = args
            .get("content")
            .or_else(|| args.get("csv_content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if content.is_empty() {
            return json!({ "success": false, "error": "导入内容不能为空" });
        }
        let rows = crate::system::utils::parse_csv(content);
        let realm = self
            .state
            .store
            .get_system_config("auth_realm")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "vos-rs".into());
        let mut imported = 0;
        let mut skipped = 0;
        let mut errors = vec![];

        for row in rows {
            if row.len() < 2 || row[0].eq_ignore_ascii_case("username") || row[0].contains("分机")
            {
                continue;
            }
            let username = row[0].trim();
            let password = row[1].trim();
            if username.is_empty() || password.is_empty() {
                continue;
            }

            let ext_exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_users WHERE username = $1)")
                    .bind(username)
                    .fetch_one(self.state.store.pool())
                    .await
                    .unwrap_or(false);

            if ext_exists {
                skipped += 1;
                errors.push(format!("分机 `{}` 已存在 (已跳过)", username));
                continue;
            }

            let ha1 = format!(
                "{:x}",
                md5::compute(format!("{username}:{realm}:{password}").as_bytes())
            );
            match self.state.store.insert_user(username, &ha1).await {
                Ok(_) => {
                    let _ =
                        crate::system::hot_cache::set_auth_user(self.state, username, &ha1).await;
                    imported += 1;
                }
                Err(e) => {
                    skipped += 1;
                    errors.push(format!("分机 `{}` 失败: {}", username, e));
                }
            }
        }
        json!({
            "success": true,
            "imported_count": imported,
            "skipped_count": skipped,
            "errors": errors,
            "message": format!("批量分机开户完成：成功导入 {} 条，跳过/失败 {} 条。", imported, skipped)
        })
    }

    pub(crate) async fn tool_import_gateways(&self, args: &Value) -> Value {
        let content = args
            .get("content")
            .or_else(|| args.get("csv_content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if content.is_empty() {
            return json!({ "success": false, "error": "导入内容不能为空" });
        }
        let rows = crate::system::utils::parse_csv(content);
        let mut imported = 0;
        let mut skipped = 0;
        let mut errors = vec![];

        for row in rows {
            if row.len() < 3 || row[0].eq_ignore_ascii_case("id") || row[0].contains("网关") {
                continue;
            }
            let id = row[0].trim();
            let name = row[1].trim();
            let ip = row[2].trim();
            let port: i32 = row
                .get(3)
                .and_then(|p| p.trim().parse().ok())
                .unwrap_or(5060);

            if id.is_empty() || name.is_empty() || ip.is_empty() {
                continue;
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
                Ok(_) => imported += 1,
                Err(e) => {
                    skipped += 1;
                    errors.push(format!("网关 `{}` 错误: {}", id, e));
                }
            }
        }
        json!({
            "success": true,
            "imported_count": imported,
            "skipped_count": skipped,
            "errors": errors,
            "message": format!("批量网关导入完成：成功导入/更新 {} 条，失败 {} 条。", imported, skipped)
        })
    }

    pub(crate) async fn tool_import_routes(&self, args: &Value) -> Value {
        let content = args
            .get("content")
            .or_else(|| args.get("csv_content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if content.is_empty() {
            return json!({ "success": false, "error": "导入内容不能为空" });
        }
        let rows = crate::system::utils::parse_csv(content);
        let mut imported = 0;
        let mut skipped = 0;
        let mut errors = vec![];

        for row in rows {
            if row.len() < 3 || row[0].eq_ignore_ascii_case("id") || row[0].contains("路由") {
                continue;
            }
            let id = row[0].trim();
            let prefix = row[1].trim();
            let gw_id = row[2].trim();
            let priority: i32 = row.get(3).and_then(|p| p.trim().parse().ok()).unwrap_or(1);

            if id.is_empty() || prefix.is_empty() || gw_id.is_empty() {
                continue;
            }

            let gw_exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id = $1)")
                    .bind(gw_id)
                    .fetch_one(self.state.store.pool())
                    .await
                    .unwrap_or(false);

            if !gw_exists {
                skipped += 1;
                errors.push(format!("路由 `{}` 跳过：关联网关 `{}` 不存在", id, gw_id));
                continue;
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
                Ok(_) => imported += 1,
                Err(e) => {
                    skipped += 1;
                    errors.push(format!("路由 `{}` 错误: {}", id, e));
                }
            }
        }
        let _ = crate::resources::routes::publish_route_reload(&self.state.nats_client).await;
        json!({
            "success": true,
            "imported_count": imported,
            "skipped_count": skipped,
            "errors": errors,
            "message": format!("批量路由导入完成并已重载选路引擎：成功 {} 条，失败/跳过 {} 条。", imported, skipped)
        })
    }

    pub(crate) async fn tool_import_rates(&self, args: &Value) -> Value {
        let content = args
            .get("content")
            .or_else(|| args.get("csv_content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if content.is_empty() {
            return json!({ "success": false, "error": "导入内容不能为空" });
        }
        let rows = crate::system::utils::parse_csv(content);
        let mut imported = 0;
        let mut skipped = 0;

        for row in rows {
            if row.len() < 2 || row[0].eq_ignore_ascii_case("prefix") || row[0].contains("前缀") {
                continue;
            }
            let prefix = row[0].trim();
            let rate_val: f64 = row[1].trim().parse().unwrap_or(0.0);
            let id = format!("rate_{}", prefix);

            let dec = Decimal::from_f64_retain(rate_val).unwrap_or_default();
            match self
                .state
                .store
                .upsert_rate(&id, prefix, dec, 60, dec, Some("由 Copilot 批量导入"))
                .await
            {
                Ok(_) => imported += 1,
                Err(_) => skipped += 1,
            }
        }
        json!({
            "success": true,
            "imported_count": imported,
            "skipped_count": skipped,
            "message": format!("资费表批量导入完成：成功 {} 条，失败 {} 条。", imported, skipped)
        })
    }
}
