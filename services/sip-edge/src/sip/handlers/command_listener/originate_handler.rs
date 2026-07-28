//! VCI Originate 命令处理。
//!
//! 主动发起一通出局呼叫（无对应的 parked 呼叫），分配本地媒体端点、
//! 构造 InboundTransaction 占位并直接发送 INVITE 给目标。

use std::str::FromStr;
use std::sync::Arc;

use sip_core::SipUri;
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::outbound;

/// 处理 Originate 命令：主动发起出局呼叫。
pub(super) async fn handle_originate(
    call_id: &str,
    target_uri: String,
    caller_id: String,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    info!(call_id, "Executing Originate command");
    let session_id = uuid::Uuid::new_v4().to_string();
    let gateway_call_id = uuid::Uuid::new_v4().to_string();

    let caller_relay_rtp = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, &session_id)
    {
        Ok(ep) => ep,
        Err(e) => {
            warn!(call_id, error = %e, "Failed to allocate media relay endpoint for originate");
            return;
        }
    };

    let sdp_offer = format!(
        "v=0\r\n\
         o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
         s=vos-rs-originate\r\n\
         c=IN IP4 {addr}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP 8\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=sendrecv\r\n",
        addr = edge_config.media.advertised_addr,
        port = caller_relay_rtp.port,
    );

    let outbound_uri = match SipUri::from_str(&target_uri) {
        Ok(uri) => uri,
        Err(_) => {
            warn!(call_id, "Invalid target URI for originate");
            return;
        }
    };

    let mut dialogs = crate::edge_state::B2buaDialogPair::placeholder(
        call_id.to_string(),
        outbound_uri.clone(),
        "local-originate",
    );
    dialogs.gateway.call_id = gateway_call_id.clone();
    dialogs.gateway.local_uri = SipUri::from_str(&format!(
        "sip:{}@{}",
        caller_id, edge_config.advertised_addr
    ))
    .unwrap_or_else(|_| outbound_uri.clone());
    dialogs.gateway.remote_uri = outbound_uri.clone();
    dialogs.gateway.remote_target = outbound_uri.clone();
    dialogs.gateway.peer = Some(outbound::target_addr_for(&outbound_uri));
    let gateway_local_tag = dialogs.gateway.local_tag.clone();

    let tx = crate::edge_state::InboundTransaction {
        session_id,
        dialogs,
        original_request: None,
        caller_rtp: None,
        gateway_relay_rtp: None,
        gateway_rtp: None,
        caller_relay_rtp: Some(caller_relay_rtp),
        session_expires: None,
        session_refresher: None,
        last_session_refresh: None,
        prack_rseq: 1,
        gateway_100rel: false,
        refer_subscription: None,
        transfer_dialog: None,
        fork_dialogs: Default::default(),
        max_duration_secs: None,
        established_at: Some(std::time::Instant::now()),
        invite_response_order: Arc::new(tokio::sync::Mutex::new(
            crate::edge_state::InviteResponseOrder::default(),
        )),
        tenant: None,
    };

    edge_state.inbound_transactions.insert(tx);

    let branch = format!("z9hG4bK-originate-{}", call_id);
    let sdp_len = sdp_offer.len();
    let invite_str = format!(
        "INVITE {target_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {adv};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:{caller_id}@{adv}>;tag={gateway_local_tag}\r\n\
         To: <{target_uri}>\r\n\
         Call-ID: {gateway_call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:vosrs-originate@{adv}>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {sdp_len}\r\n\r\n\
         {sdp_offer}",
        adv = edge_config.advertised_addr,
        target_uri = target_uri,
        branch = branch,
        caller_id = caller_id,
        gateway_call_id = gateway_call_id,
        gateway_local_tag = gateway_local_tag,
        sdp_len = sdp_len,
        sdp_offer = sdp_offer,
    );

    if let Some(socket) = edge_state.get_socket() {
        let target_peer = outbound::target_addr_for_str(&target_uri);
        let dg = PendingDatagram::new(target_peer, invite_str.into_bytes());
        let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;
    }
}
