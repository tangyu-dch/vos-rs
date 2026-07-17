//! Preloaded DID-to-extension routing for the INVITE hot path.

use crate::EdgeState;
use cdr_core::PostgresCdrStore;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const NUMBER_RELOAD_SUBJECT: &str = "vos_rs.numbers.reload";
const NUMBER_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// Reloads active DID-to-extension mappings from PostgreSQL.
pub(crate) async fn reload_number_routes(
    edge_state: &EdgeState,
    database: &PostgresCdrStore,
) -> Result<(), AnyError> {
    let dids = database.list_did_destinations().await?;
    let count = dids.len();
    edge_state.replace_did_destinations(dids.into_iter().map(|d| (d.number.clone(), d)).collect());
    info!(count, "号码路由缓存已刷新");
    Ok(())
}

/// Starts NATS-triggered reloads with a periodic database refresh as fallback.
pub(crate) fn spawn_number_route_refresh(
    edge_state: Arc<EdgeState>,
    database: PostgresCdrStore,
    nats_url: Option<String>,
) {
    spawn_periodic_refresh(Arc::clone(&edge_state), database.clone());
    let Some(nats_url) = nats_url.filter(|url| !url.trim().is_empty()) else {
        return;
    };
    tokio::spawn(async move {
        let Ok(client) = async_nats::connect(&nats_url).await else {
            warn!(%nats_url, "号码路由刷新器无法连接 NATS，将依赖周期刷新");
            return;
        };
        let Ok(mut subscriber) = client.subscribe(NUMBER_RELOAD_SUBJECT).await else {
            warn!("号码路由刷新器无法订阅 NATS，将依赖周期刷新");
            return;
        };
        while subscriber.next().await.is_some() {
            if let Err(error) = reload_number_routes(&edge_state, &database).await {
                warn!(%error, "NATS 触发的号码路由刷新失败");
            }
        }
    });
}

fn spawn_periodic_refresh(edge_state: Arc<EdgeState>, database: PostgresCdrStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(NUMBER_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = reload_number_routes(&edge_state, &database).await {
                warn!(%error, "周期号码路由刷新失败");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use call_core::{CallManager, RouteTable};
    use sip_core::{parse_message, SipMessageBorrow, SipRequest};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::time::SystemTime;

    fn state() -> EdgeState {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        EdgeState::new(CallManager::new(RouteTable::default(), sender))
    }

    fn register_request(username: &str) -> SipRequest {
        let raw = format!(
            "REGISTER sip:tenant.example SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5070;branch=z9hG4bK-number-route\r\n\
             From: <sip:{username}@tenant.example>;tag=reg\r\n\
             To: <sip:{username}@tenant.example>\r\n\
             Call-ID: number-route-register\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:{username}@192.0.2.10:5070>;expires=300\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let SipMessageBorrow::Request(request) =
            parse_message(raw.as_bytes()).expect("valid REGISTER")
        else {
            panic!("expected REGISTER request");
        };
        request.into_owned()
    }

    #[test]
    fn mapped_did_resolves_to_extension_and_preserves_domain() {
        let state = state();
        let did = cdr_core::DidDestination {
            number: "4008001".to_string(),
            tenant_id: None,
            target_type: "extension".to_string(),
            target_id: "1001".to_string(),
            enabled: true,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        state.replace_did_destinations(HashMap::from([("4008001".to_string(), did)]));
        let destination = "sip:4008001@tenant.example:5060;transport=udp"
            .parse()
            .expect("valid destination URI");

        let resolved = state.resolve_number_destination(&destination);

        assert_eq!(resolved.user.as_deref(), Some("1001"));
        assert_eq!(resolved.host, destination.host);
        assert_eq!(resolved.port, destination.port);
        assert_eq!(resolved.params, destination.params);
    }

    #[test]
    fn unknown_number_remains_unchanged() {
        let state = state();
        let destination = "sip:1002@tenant.example"
            .parse()
            .expect("valid destination URI");

        assert_eq!(state.resolve_number_destination(&destination), destination);
    }

    #[tokio::test]
    async fn mapped_did_finds_extension_registration_contact() {
        let state = state();
        let request = register_request("1001");
        state
            .registrar
            .write()
            .await
            .handle_register(
                &request,
                "192.0.2.10:5070".parse::<SocketAddr>().expect("peer"),
                SystemTime::now(),
                None,
            )
            .await
            .expect("registration succeeds");
        let did_dest = cdr_core::DidDestination {
            number: "4008001".to_string(),
            tenant_id: None,
            target_type: "extension".to_string(),
            target_id: "1001".to_string(),
            enabled: true,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        state.replace_did_destinations(HashMap::from([("4008001".to_string(), did_dest)]));
        let did = "sip:4008001@tenant.example".parse().expect("valid DID");

        let contact = state
            .lookup_destination_contact(&did)
            .await
            .expect("mapped extension registration");

        assert_eq!(contact.uri, "sip:1001@192.0.2.10:5070");
        assert_eq!(contact.received_from, "192.0.2.10:5070");
    }
}
