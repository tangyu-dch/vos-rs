//! 控制台用户、动态角色权限和菜单资源存储。

use crate::PostgresCdrStore;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
/// 控制台用户在授权快照中的身份信息。
pub struct AccessIdentity {
    pub username: String,
    pub role_key: String,
    pub enabled: bool,
    pub auth_version: i64,
}

#[derive(Debug, Clone, Default)]
/// 用户身份与角色权限的内存快照。
pub struct AccessSnapshot {
    pub users: HashMap<String, AccessIdentity>,
    pub role_permissions: HashMap<String, HashSet<String>>,
}

impl AccessSnapshot {
    fn role_has_permission(items: &HashSet<String>, permission: &str) -> bool {
        items.contains("*")
            || items.contains(permission)
            || (permission.starts_with("access.accounts.") && items.contains("access.users"))
            || (permission.starts_with("access.roles.") && items.contains("access.roles"))
            || (permission.starts_with("llm.") && items.contains("llm.manage"))
    }

    /// 检查令牌中的用户、角色和权限版本是否仍与当前数据库快照一致。
    pub fn is_current_identity(&self, username: &str, role_key: &str, auth_version: i64) -> bool {
        self.users.get(username).is_some_and(|user| {
            user.enabled && user.role_key == role_key && user.auth_version == auth_version
        })
    }

    /// 验证用户、角色、权限版本和权限点是否同时匹配。
    pub fn allows(
        &self,
        username: &str,
        role_key: &str,
        auth_version: i64,
        permission: &str,
    ) -> bool {
        if !self.is_current_identity(username, role_key, auth_version) {
            return false;
        }
        self.role_permissions
            .get(role_key)
            .is_some_and(|items| Self::role_has_permission(items, permission))
    }
}

#[derive(Debug, Clone)]
/// 登录校验所需的数据库凭据记录。
pub struct ConsoleCredential {
    pub username: String,
    pub password_hash: String,
    pub display_name: String,
    pub role_key: String,
    pub role_name: String,
    pub enabled: bool,
    pub auth_version: i64,
}

#[derive(Debug, Clone, Serialize)]
/// 可展示的控制台用户信息，不包含密码摘要。
pub struct ConsoleUser {
    pub username: String,
    pub display_name: String,
    pub role_key: String,
    pub role_name: String,
    pub enabled: bool,
    pub is_builtin: bool,
    pub auth_version: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
/// 动态角色及其权限点集合。
pub struct AccessRole {
    pub role_key: String,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub enabled: bool,
    pub permission_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
/// 可分配给角色的最小权限点。
pub struct AccessPermission {
    pub permission_key: String,
    pub name: String,
    pub group_name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 控制台菜单项配置。
pub struct AccessMenuItem {
    pub item_key: String,
    pub label: String,
    pub path: String,
    pub icon_key: String,
    pub permission_key: String,
    pub sort_order: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 控制台菜单分组及其菜单项。
pub struct AccessMenuGroup {
    pub group_key: String,
    pub label: String,
    pub icon_key: String,
    pub sort_order: i32,
    pub enabled: bool,
    pub items: Vec<AccessMenuItem>,
}

impl PostgresCdrStore {
    /// 加载当前全部有效用户和角色权限快照。
    pub async fn load_access_snapshot(&self) -> Result<AccessSnapshot, sqlx::Error> {
        let user_rows =
            sqlx::query("SELECT username, role_key, enabled, auth_version FROM console_users")
                .fetch_all(&self.pool)
                .await?;
        let permission_rows = sqlx::query(
            "SELECT rp.role_key, rp.permission_key FROM access_role_permissions rp \
             JOIN access_roles r ON r.role_key = rp.role_key WHERE r.enabled = TRUE",
        )
        .fetch_all(&self.pool)
        .await?;
        let users = user_rows
            .into_iter()
            .map(|row| {
                let username: String = row.get("username");
                let identity = AccessIdentity {
                    username: username.clone(),
                    role_key: row.get("role_key"),
                    enabled: row.get("enabled"),
                    auth_version: row.get("auth_version"),
                };
                (username, identity)
            })
            .collect();
        let mut role_permissions: HashMap<String, HashSet<String>> = HashMap::new();
        for row in permission_rows {
            role_permissions
                .entry(row.get("role_key"))
                .or_default()
                .insert(row.get("permission_key"));
        }
        Ok(AccessSnapshot {
            users,
            role_permissions,
        })
    }

    /// 返回控制台用户数量，用于判断是否需要首次引导。
    pub async fn count_console_users(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM console_users")
            .fetch_one(&self.pool)
            .await
    }

    /// 在用户表为空时创建首个数据库管理员。
    pub async fn bootstrap_console_admin(&self, password_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO console_users \
             (username, password_hash, display_name, role_key, enabled, is_builtin) \
             VALUES ('admin', $1, '系统管理员', 'admin', TRUE, TRUE) \
             ON CONFLICT (username) DO NOTHING",
        )
        .bind(password_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 按用户名读取登录凭据和角色状态。
    pub async fn get_console_credential(
        &self,
        username: &str,
    ) -> Result<Option<ConsoleCredential>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT u.username, u.password_hash, u.display_name, u.role_key, r.name AS role_name, \
                    (u.enabled AND r.enabled) AS enabled, u.auth_version \
             FROM console_users u JOIN access_roles r ON r.role_key = u.role_key \
             WHERE u.username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ConsoleCredential {
            username: row.get("username"),
            password_hash: row.get("password_hash"),
            display_name: row.get("display_name"),
            role_key: row.get("role_key"),
            role_name: row.get("role_name"),
            enabled: row.get("enabled"),
            auth_version: row.get("auth_version"),
        }))
    }

    /// 列出不含密码摘要的控制台用户。
    pub async fn list_console_users(&self) -> Result<Vec<ConsoleUser>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT u.username, u.display_name, u.role_key, r.name AS role_name, u.enabled, \
                    u.is_builtin, u.auth_version, u.created_at, u.updated_at \
             FROM console_users u JOIN access_roles r ON r.role_key = u.role_key \
             ORDER BY u.is_builtin DESC, u.created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ConsoleUser {
                username: row.get("username"),
                display_name: row.get("display_name"),
                role_key: row.get("role_key"),
                role_name: row.get("role_name"),
                enabled: row.get("enabled"),
                is_builtin: row.get("is_builtin"),
                auth_version: row.get("auth_version"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    /// 删除非内置控制台账户。
    pub async fn delete_console_user(&self, username: &str) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM console_users WHERE username = $1 AND NOT is_builtin")
                .bind(username)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 列出按钮级权限目录。
    pub async fn list_access_permissions(&self) -> Result<Vec<AccessPermission>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT permission_key, name, group_name, description FROM access_permissions \
             ORDER BY group_name, permission_key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AccessPermission {
                permission_key: row.get("permission_key"),
                name: row.get("name"),
                group_name: row.get("group_name"),
                description: row.get("description"),
            })
            .collect())
    }

    /// 列出角色及其权限点。
    pub async fn list_access_roles(&self) -> Result<Vec<AccessRole>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT r.role_key, r.name, r.description, r.is_system, r.enabled, \
                    COALESCE(array_agg(rp.permission_key ORDER BY rp.permission_key) \
                    FILTER (WHERE rp.permission_key IS NOT NULL), ARRAY[]::TEXT[]) AS permissions \
             FROM access_roles r LEFT JOIN access_role_permissions rp ON rp.role_key = r.role_key \
             GROUP BY r.role_key ORDER BY r.is_system DESC, r.created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AccessRole {
                role_key: row.get("role_key"),
                name: row.get("name"),
                description: row.get("description"),
                is_system: row.get("is_system"),
                enabled: row.get("enabled"),
                permission_keys: row.get("permissions"),
            })
            .collect())
    }

    /// 创建一个空权限的动态角色。
    pub async fn create_access_role(
        &self,
        role_key: &str,
        name: &str,
        description: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO access_roles (role_key, name, description) VALUES ($1, $2, $3)")
            .bind(role_key)
            .bind(name)
            .bind(description)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 更新动态角色的显示信息和启用状态。
    pub async fn update_access_role(
        &self,
        role_key: &str,
        name: &str,
        description: &str,
        enabled: bool,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE access_roles SET name = $2, description = $3, enabled = $4, updated_at = now() \
             WHERE role_key = $1",
        )
        .bind(role_key)
        .bind(name)
        .bind(description)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 删除未分配账户且不是系统管理员的角色。
    pub async fn delete_access_role(&self, role_key: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM access_roles r WHERE r.role_key = $1 AND r.role_key <> 'admin' \
             AND NOT EXISTS (SELECT 1 FROM console_users u WHERE u.role_key = r.role_key)",
        )
        .bind(role_key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 批量调整控制台账户的所属角色，并使旧会话失效。
    pub async fn assign_console_user_roles(
        &self,
        assignments: &[(String, String)],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        for (username, role_key) in assignments {
            sqlx::query(
                "UPDATE console_users SET role_key = $2, auth_version = auth_version + 1, \
                 updated_at = now() WHERE username = $1 AND role_key <> $2",
            )
            .bind(username)
            .bind(role_key)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// 原子替换角色权限，并使该角色已有会话失效。
    pub async fn replace_role_permissions(
        &self,
        role_key: &str,
        permission_keys: &[String],
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM access_role_permissions WHERE role_key = $1")
            .bind(role_key)
            .execute(&mut *transaction)
            .await?;
        for permission_key in permission_keys {
            sqlx::query(
                "INSERT INTO access_role_permissions (role_key, permission_key) VALUES ($1, $2)",
            )
            .bind(role_key)
            .bind(permission_key)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE console_users SET auth_version = auth_version + 1, updated_at = now() \
             WHERE role_key = $1",
        )
        .bind(role_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await
    }

    /// 创建数据库控制台用户。
    pub async fn create_console_user(
        &self,
        username: &str,
        password_hash: &str,
        display_name: &str,
        role_key: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO console_users (username, password_hash, display_name, role_key) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(username)
        .bind(password_hash)
        .bind(display_name)
        .bind(role_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 更新控制台用户，并使其已有会话失效。
    pub async fn update_console_user(
        &self,
        username: &str,
        display_name: &str,
        role_key: &str,
        enabled: bool,
        password_hash: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE console_users SET display_name = $2, role_key = $3, enabled = $4, \
                    password_hash = COALESCE($5, password_hash), auth_version = auth_version + 1, \
                    updated_at = now() WHERE username = $1",
        )
        .bind(username)
        .bind(display_name)
        .bind(role_key)
        .bind(enabled)
        .bind(password_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 按配置顺序列出菜单树。
    pub async fn list_access_menus(&self) -> Result<Vec<AccessMenuGroup>, sqlx::Error> {
        let groups = sqlx::query(
            "SELECT group_key, label, icon_key, sort_order, enabled FROM access_menu_groups \
             ORDER BY sort_order, group_key",
        )
        .fetch_all(&self.pool)
        .await?;
        let items = sqlx::query(
            "SELECT item_key, group_key, label, path, icon_key, permission_key, sort_order, enabled \
             FROM access_menu_items ORDER BY sort_order, item_key",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut items_by_group: HashMap<String, Vec<AccessMenuItem>> = HashMap::new();
        for row in items {
            items_by_group
                .entry(row.get("group_key"))
                .or_default()
                .push(AccessMenuItem {
                    item_key: row.get("item_key"),
                    label: row.get("label"),
                    path: row.get("path"),
                    icon_key: row.get("icon_key"),
                    permission_key: row.get("permission_key"),
                    sort_order: row.get("sort_order"),
                    enabled: row.get("enabled"),
                });
        }
        Ok(groups
            .into_iter()
            .map(|row| {
                let group_key: String = row.get("group_key");
                AccessMenuGroup {
                    items: items_by_group.remove(&group_key).unwrap_or_default(),
                    group_key,
                    label: row.get("label"),
                    icon_key: row.get("icon_key"),
                    sort_order: row.get("sort_order"),
                    enabled: row.get("enabled"),
                }
            })
            .collect())
    }

    /// 更新菜单项名称、顺序和启用状态。
    pub async fn update_access_menu_item(
        &self,
        item_key: &str,
        label: &str,
        sort_order: i32,
        enabled: bool,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE access_menu_items SET label = $2, sort_order = $3, enabled = $4, \
             updated_at = now() WHERE item_key = $1",
        )
        .bind(item_key)
        .bind(label)
        .bind(sort_order)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_requires_matching_user_version_and_permission() {
        let mut snapshot = AccessSnapshot::default();
        snapshot.users.insert(
            "alice".to_string(),
            AccessIdentity {
                username: "alice".to_string(),
                role_key: "ops".to_string(),
                enabled: true,
                auth_version: 2,
            },
        );
        snapshot
            .role_permissions
            .insert("ops".to_string(), HashSet::from(["calls.view".to_string()]));
        assert!(snapshot.is_current_identity("alice", "ops", 2));
        assert!(!snapshot.is_current_identity("alice", "admin", 2));
        assert!(!snapshot.is_current_identity("missing", "ops", 2));
        assert!(snapshot.allows("alice", "ops", 2, "calls.view"));
        assert!(!snapshot.allows("alice", "ops", 1, "calls.view"));
        assert!(!snapshot.allows("alice", "ops", 2, "billing.manage"));
    }

    #[test]
    fn snapshot_allows_wildcard_role_but_never_bypasses_user_state() {
        let mut snapshot = AccessSnapshot::default();
        snapshot.users.insert(
            "admin".to_string(),
            AccessIdentity {
                username: "admin".to_string(),
                role_key: "admin".to_string(),
                enabled: true,
                auth_version: 3,
            },
        );
        snapshot
            .role_permissions
            .insert("admin".to_string(), HashSet::from(["*".to_string()]));

        assert!(snapshot.allows("admin", "admin", 3, "unregistered.future.permission"));
        assert!(!snapshot.allows("admin", "admin", 2, "calls.view"));

        snapshot
            .users
            .get_mut("admin")
            .expect("测试用户存在")
            .enabled = false;
        assert!(!snapshot.allows("admin", "admin", 3, "calls.view"));
    }

    #[test]
    fn snapshot_accepts_legacy_access_permissions_for_new_action_keys() {
        let mut snapshot = AccessSnapshot::default();
        snapshot.users.insert(
            "operator".to_string(),
            AccessIdentity {
                username: "operator".to_string(),
                role_key: "legacy".to_string(),
                enabled: true,
                auth_version: 1,
            },
        );
        snapshot.role_permissions.insert(
            "legacy".to_string(),
            HashSet::from([
                "access.users".to_string(),
                "access.roles".to_string(),
                "llm.manage".to_string(),
            ]),
        );
        assert!(snapshot.allows("operator", "legacy", 1, "access.accounts.view"));
        assert!(snapshot.allows("operator", "legacy", 1, "access.accounts.delete"));
        assert!(snapshot.allows("operator", "legacy", 1, "access.roles.assign"));
        assert!(snapshot.allows("operator", "legacy", 1, "llm.activate"));
    }
}
