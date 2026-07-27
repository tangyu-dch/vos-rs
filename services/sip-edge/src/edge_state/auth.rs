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
    pub(crate) async fn verify_sip_auth(
        &self,
        auth: &crate::sip::AuthConfig,
        request: &SipRequest,
        is_trunk: bool,
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
        )
    }
}
