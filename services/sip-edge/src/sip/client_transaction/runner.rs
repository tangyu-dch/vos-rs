use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;
use crate::sip::transaction::ClientTransactionKey;

pub(crate) fn spawn_client_transaction_retransmission(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    target: String,
    bytes: Vec<u8>,
    key: ClientTransactionKey,
    edge_config: Arc<EdgeConfig>,
) -> bool {
    let Some(registration) = edge_state.client_transactions.register(key.clone()) else {
        debug!(?key, "suppressed duplicate client transaction request");
        return false;
    };

    // Registration replays a response that may already have reached transport
    // ingress. Do not send a stale request after that response was observed.
    let should_send_initial_request = registration.control.should_retransmit();
    tokio::spawn(run_client_transaction(
        edge_state,
        socket,
        target,
        bytes,
        key,
        edge_config,
        registration,
    ));
    should_send_initial_request
}

#[allow(clippy::too_many_arguments)]
async fn run_client_transaction(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    target: String,
    bytes: Vec<u8>,
    key: ClientTransactionKey,
    edge_config: Arc<EdgeConfig>,
    registration: super::manager::ClientTransactionRegistration,
) {
    let initial_t1 = if cfg!(test) {
        Duration::from_millis(5)
    } else {
        Duration::from_millis(edge_config.sip_t1_initial_ms.max(50))
    };
    let transaction_timeout = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(edge_config.sip_transaction_timeout_secs.max(1))
    };
    let mut retransmit_interval = initial_t1;
    let retransmit_timer = tokio::time::sleep(retransmit_interval);
    let timeout_timer = tokio::time::sleep(transaction_timeout);
    tokio::pin!(retransmit_timer);
    tokio::pin!(timeout_timer);
    let mut timed_out = false;

    loop {
        edge_state
            .client_transactions
            .apply_observed_branch_response(&key, &registration.control);
        if registration.control.is_terminal() {
            break;
        }
        tokio::select! {
            _ = registration.control.changed() => {
                debug!(?key, state = ?registration.control.state(), "client transaction state changed");
            }
            _ = &mut retransmit_timer, if registration.control.should_retransmit() => {
                // A response can race with Timer A becoming ready. The atomic state
                // and branch ledger are checked again immediately before I/O so a
                // received 1xx always wins even if the active index missed the response.
                edge_state
                    .client_transactions
                    .apply_observed_branch_response(&key, &registration.control);
                if !registration.control.should_retransmit() {
                    continue;
                }
                if let Err(error) = socket.send_to(&bytes, &target).await {
                    warn!(%error, ?key, "failed to retransmit client transaction request");
                } else {
                    debug!(?key, ?retransmit_interval, "retransmitted client transaction request");
                }
                retransmit_interval = next_retransmit_interval(&key.method, retransmit_interval);
                retransmit_timer.as_mut().reset(tokio::time::Instant::now() + retransmit_interval);
            }
            _ = &mut timeout_timer, if registration.control.should_timeout() => {
                timed_out = true;
                registration.control.cancel();
                break;
            }
        }
    }

    edge_state
        .client_transactions
        .finish(&key, &registration.control);

    if timed_out {
        handle_transaction_timeout(&edge_state, &edge_config, &target, &key).await;
    } else {
        info!(?key, state = ?registration.control.state(), "client transaction terminated");
    }
}

fn next_retransmit_interval(method: &str, current: Duration) -> Duration {
    if method.eq_ignore_ascii_case("INVITE") {
        current * 2
    } else {
        std::cmp::min(current * 2, Duration::from_secs(4))
    }
}

async fn handle_transaction_timeout(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    target: &str,
    key: &ClientTransactionKey,
) {
    warn!(?key, "client transaction timed out without final response");
    if !matches!(key.method.as_str(), "INVITE" | "BYE") {
        return;
    }

    let local_503 = format!(
        "SIP/2.0 503 Service Unavailable\r\n\
         Via: SIP/2.0/UDP {target};branch={branch}\r\n\
         From: local;tag=timeout\r\n\
         To: local;tag=timeout\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 {method}\r\n\
         Content-Length: 0\r\n\r\n",
        branch = key.branch,
        call_id = key.call_id,
        method = key.method,
    );
    let Ok(target_addr) = target.parse::<SocketAddr>() else {
        warn!(%target, ?key, "cannot dispatch local timeout response for invalid target");
        return;
    };
    let _ =
        crate::handle_datagram(local_503.as_bytes(), target_addr, edge_state, edge_config).await;
}
