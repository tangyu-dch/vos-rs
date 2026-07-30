use crate::edge_state::EdgeState;
use sip_core::SipRequest;

impl EdgeState {
    /// 从 Redis 读取 SIP 鉴权凭据，不回退查询 PostgreSQL。
    pub(crate) async fn redis_auth_password(
        &self,
        username: &str,
        is_trunk: bool,
    ) -> Option<String> {
        let mut connection = self.redis_connection()?;
        let hash_key = if is_trunk {
            "vos_rs:auth:trunks"
        } else {
            "vos_rs:auth:extensions"
        };
        redis::cmd("HGET")
            .arg(hash_key)
            .arg(username)
            .query_async(&mut connection)
            .await
            .ok()
            .flatten()
    }

    /// 使用 Redis 凭据执行 SIP Digest 鉴权，不访问 PostgreSQL。
    ///
    /// `realm_override` 为 Some 时使用传入的 realm（如关联租户的域名），
    /// 为 None 时回退到 auth 配置的默认 realm。
    pub(crate) async fn verify_sip_auth(
        &self,
        auth: &crate::sip::AuthConfig,
        request: &SipRequest,
        is_trunk: bool,
        realm_override: Option<&str>,
    ) -> crate::sip::AuthDecision {
        let username = auth.authorization_username(request);
        let password = if let Some(ref username) = username {
            let pwd = self.redis_auth_password(username, is_trunk).await;
            if pwd.is_some() {
                pwd
            } else if let Some(ref db) = self.db_store {
                match db.get_user_password(username).await {
                    Ok(Some(db_pwd)) => Some(db_pwd),
                    _ => auth.configured_password(username),
                }
            } else {
                auth.configured_password(username)
            }
        } else {
            None
        };
        let is_bypass = std::env::var("VOS_RS_AUTH_BYPASS").ok().as_deref() == Some("true");
        let auth_required = if is_bypass {
            false
        } else {
            auth.is_enabled() || self.redis_connection().is_some() || self.db_store.is_some()
        };
        auth.verify_request_with_password(
            request,
            password,
            auth_required,
            Some(&self.nonce_replay_cache),
            realm_override,
        )
    }

    /// 根据 SIP 请求的 From 头与用户绑定的租户解析鉴权 realm。
    ///
    /// - 若分机关联了租户，From 头或者 Digest 请求的域名必须匹配租户域名；若不匹配将无法通过鉴权挑战。
    /// - 若分机未关联租户，优先使用 From 头中命中的租户域名；无命中时返回 None（使用系统默认 realm）。
    pub(crate) async fn resolve_auth_realm(&self, request: &SipRequest) -> Option<String> {
        let username_opt = request
            .headers
            .get("authorization")
            .and_then(|auth_hdr| {
                crate::sip::auth::parse_digest_authorization(auth_hdr.as_str())
                    .and_then(|params| params.get("username").cloned())
            })
            .or_else(|| EdgeState::username_from_request(request));

        if let Some(ref u) = username_opt {
            // 优先查 Redis 快速定位分机对应的租户 ID
            let tenant_id_opt = if let Some(mut conn) = self.redis_connection() {
                redis::cmd("HGET")
                    .arg("vos_rs:auth:extension_tenants")
                    .arg(u)
                    .query_async::<Option<String>>(&mut conn)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };

            let tenant_id = match tenant_id_opt {
                Some(tid) => Some(tid),
                None => {
                    if let Some(ref db) = self.db_store {
                        if let Ok(users) = db.list_users_page(1, 0, Some(u), None).await {
                            users
                                .into_iter()
                                .find(|user| &user.username == u)
                                .and_then(|user| user.tenant_id)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            };

            if let Some(ref tid) = tenant_id {
                if let Some(registry) = self.tenant_registry.get() {
                    let map = registry.read_map().await;
                    if let Some(tenant) = map.values().find(|t| &t.id == tid) {
                        return Some(tenant.domain.clone());
                    }
                }
                if let Some(ref db) = self.db_store {
                    if let Ok(Some(tenant)) = db.get_tenant(tid).await {
                        if !tenant.domain.trim().is_empty() {
                            return Some(tenant.domain);
                        }
                    }
                }
            }
        }

        if let Some(registry) = self.tenant_registry.get() {
            if let Some(from_header) = request.headers.get("from") {
                let ctx = registry.context_for_from_header(from_header.as_str()).await;
                return ctx.domain;
            }
        }
        None
    }
}
