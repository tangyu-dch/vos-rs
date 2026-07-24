//! Copilot 工具实现：分机账号与 IVR 菜单管理

use serde_json::{json, Value};

use crate::copilot::TelecomCopilotEngine;

impl<'a> TelecomCopilotEngine<'a> {
    pub(crate) async fn tool_list_extensions(&self, args: &Value) -> Value {
        let username = args.get("username").and_then(|v| v.as_str());
        match self.state.store.list_users().await {
            Ok(users) => {
                let filtered: Vec<_> = if let Some(u) = username {
                    users
                        .into_iter()
                        .filter(|user| user.username.contains(u))
                        .collect()
                } else {
                    users
                };
                json!({ "total": filtered.len(), "extensions": filtered })
            }
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_create_extension(&self, args: &Value) -> Value {
        let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
        let password = args.get("password").and_then(|v| v.as_str()).unwrap_or("");
        if username.is_empty() || password.is_empty() {
            return json!({ "success": false, "error": "分机账号 username 和密码 password 不能为空" });
        }
        let ext_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_users WHERE username = $1)")
                .bind(username)
                .fetch_one(self.state.store.pool())
                .await
                .unwrap_or(false);
        if ext_exists {
            return json!({
                "success": false,
                "conflict": true,
                "error": format!("分机账号 `{}` 已存在，不能重复创建。", username)
            });
        }
        let realm = self
            .state
            .store
            .get_system_config("auth_realm")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "vos-rs".into());
        let ha1 = format!(
            "{:x}",
            md5::compute(format!("{username}:{realm}:{password}").as_bytes())
        );
        match self.state.store.insert_user(username, &ha1).await {
            Ok(_) => {
                let _ = crate::system::hot_cache::set_auth_user(self.state, username, &ha1).await;
                json!({ "success": true, "message": format!("分机账号 `{}` 创建成功", username) })
            }
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_extension(&self, args: &Value) -> Value {
        let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
        match self.state.store.delete_user(username).await {
            Ok(_) => json!({ "success": true, "message": format!("分机账号 {} 已删除", username) }),
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_list_ivr_menus(&self) -> Value {
        match self.state.store.list_ivr_menus().await {
            Ok(menus) => json!({ "ivr_menus": menus }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_create_ivr_menu(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let did = args.get("did").and_then(|v| v.as_str()).unwrap_or("");
        let welcome_prompt = args
            .get("welcome_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if id.is_empty() || name.is_empty() {
            return json!({ "success": false, "error": "IVR 菜单 ID 和名称不能为空" });
        }
        if !did.is_empty() {
            let dup_did: Option<(String, String)> =
                sqlx::query_as("SELECT id, name FROM ivr_menus WHERE did = $1 AND id != $2")
                    .bind(did)
                    .bind(id)
                    .fetch_optional(self.state.store.pool())
                    .await
                    .unwrap_or(None);

            if let Some((dup_id, dup_name)) = dup_did {
                return json!({
                    "success": false,
                    "conflict": true,
                    "warning": format!("DID 号码 `{}` 已被另一 IVR 菜单 `{}` ({}) 绑定。请换用其他 DID 号码。", did, dup_name, dup_id)
                });
            }
        }
        let res = sqlx::query(
            "INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) \
             VALUES ($1, $2, $3, $4, $5, 10, true, '[]'::jsonb, '[]'::jsonb) \
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, did = EXCLUDED.did, welcome_prompt = EXCLUDED.welcome_prompt"
        )
        .bind(id)
        .bind(name)
        .bind("由 Copilot 自动创建")
        .bind(did)
        .bind(welcome_prompt)
        .execute(self.state.store.pool())
        .await;
        match res {
            Ok(_) => {
                json!({ "success": true, "message": format!("IVR 菜单 `{}` ({}) 创建成功", name, id) })
            }
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_add_ivr_node(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let key = args.get("dtmf_key").and_then(|v| v.as_str()).unwrap_or("");
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || key.is_empty() || action.is_empty() {
            return json!({ "success": false, "error": "IVR ID、按键 key 与动作 action 不能为空" });
        }
        let node_id = format!("node_{key}");
        let new_node = json!({ "id": node_id, "dtmf": key, "action": action });
        let res = sqlx::query("UPDATE ivr_menus SET nodes = nodes || $1::jsonb WHERE id = $2")
            .bind(json!([new_node]))
            .bind(id)
            .execute(self.state.store.pool())
            .await;
        match res {
            Ok(r) if r.rows_affected() > 0 => {
                json!({ "success": true, "message": format!("IVR {} 成功新增按键 [{}] -> 动作 [{}]", id, key, action) })
            }
            Ok(_) => json!({ "success": false, "error": format!("IVR 菜单 {} 不存在", id) }),
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_ivr_menu(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let res = sqlx::query("DELETE FROM ivr_menus WHERE id = $1")
            .bind(id)
            .execute(self.state.store.pool())
            .await;
        match res {
            Ok(r) if r.rows_affected() > 0 => {
                json!({ "success": true, "message": format!("IVR 菜单 {} 已成功删除", id) })
            }
            Ok(_) => json!({ "success": false, "error": format!("IVR 菜单 {} 不存在", id) }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}
