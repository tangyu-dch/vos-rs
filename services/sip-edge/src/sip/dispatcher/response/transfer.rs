use std::net::SocketAddr;

use sdp_core::RtpEndpoint;
use sip_core::SipResponse;

use crate::config::EdgeConfig;
use crate::edge_state::{DialogLeg, EdgeState, PendingDatagram};
use crate::media;
use crate::sip::outbound;

use super::{build_dialog_bye, build_gateway_non_2xx_ack, build_gateway_success_ack, gateway_peer};

/// 处理 REFER 转移场景下的 INVITE 响应。
///
/// 该函数执行 remove → 处理 → insert 的原子性语义：
/// 1. 从 `inbound_transactions` 中移除 transaction
/// 2. 处理转移响应（更新 dialog 状态、发送 ACK、NOTIFY、BYE 等）
/// 3. 将 transaction 重新插入 `inbound_transactions`
///
/// 任何提前返回路径都会确保 transaction 被重新插入（除非 session 不存在）。
pub(super) async fn handle_transfer_response(
    sip_response: &SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    session_key: Option<&str>,
    is_invite: bool,
    response_cseq: Option<u32>,
) -> Vec<PendingDatagram> {
    let Some(session_id) = session_key else {
        return Vec::new();
    };
    let Some((_, mut transaction)) = edge_state.inbound_transactions.remove(session_id) else {
        return Vec::new();
    };
    let Some(mut subscription) = transaction.refer_subscription.take() else {
        edge_state.inbound_transactions.insert(transaction);
        return Vec::new();
    };

    let status = sip_response.status_code;
    let Some(transfer) = transaction.transfer_dialog.as_mut() else {
        edge_state.inbound_transactions.insert(transaction);
        return Vec::new();
    };
    if status >= 180 {
        transfer.dialog.remote_tag = sip_response
            .headers
            .get("to")
            .and_then(|value| crate::sip::dialog::tag_param(value.as_str()));
        if let Some(uri) = sip_response
            .headers
            .get("from")
            .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
        {
            transfer.dialog.local_uri = uri;
        }
        if let Some(uri) = sip_response
            .headers
            .get("to")
            .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
        {
            transfer.dialog.remote_uri = uri;
        }
        if let Some(uri) = sip_response
            .headers
            .get("contact")
            .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
        {
            transfer.dialog.remote_target = uri;
        }
        let mut route_set = sip_response
            .headers
            .get_all("record-route")
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>();
        route_set.reverse();
        transfer.dialog.route_set = route_set;
        transfer.dialog.peer = Some(peer.to_string());
        if let Some(cseq) = response_cseq {
            transfer.dialog.local_cseq = cseq;
        }
    }
    let transfer_snapshot = transfer.clone();
    let transferee_leg = transfer.transferee_leg;

    let mut datagrams = Vec::new();
    if is_invite && (200..300).contains(&status) {
        let ack = build_gateway_success_ack(
            sip_response,
            &transfer_snapshot.dialog,
            &edge_config.advertised_addr,
        );
        datagrams.push(PendingDatagram::new(
            gateway_peer(&transfer_snapshot.dialog, peer),
            ack,
        ));
    } else if is_invite && status >= 300 {
        let ack = build_gateway_non_2xx_ack(sip_response, &transfer_snapshot.dialog);
        datagrams.push(PendingDatagram::new(
            gateway_peer(&transfer_snapshot.dialog, peer),
            ack,
        ));
    }

    subscription.notify_cseq = subscription.notify_cseq.saturating_add(1);
    let referrer_dialog = match transferee_leg {
        DialogLeg::Caller => &transaction.dialogs.gateway,
        DialogLeg::Gateway => &transaction.dialogs.caller,
        DialogLeg::Transfer => &transaction.dialogs.caller,
    };
    let notify = outbound::build_notify_sipfrag_with_state(
        &referrer_dialog.call_id,
        &subscription.from_header,
        &subscription.to_header,
        subscription.notify_cseq,
        &edge_config.advertised_addr,
        &format!("SIP/2.0 {} {}\r\n", status, sip_response.reason_phrase),
        if status >= 200 {
            "terminated;reason=noresource"
        } else {
            "active;expires=60"
        },
    );
    datagrams.push(PendingDatagram::new(
        subscription.referrer_peer.clone(),
        notify,
    ));

    if (200..300).contains(&status) {
        if let (Some(target_port), Ok(target_endpoint)) = (
            subscription.target_relay_port,
            media::parse_sdp_rtp_endpoint(&sip_response.body),
        ) {
            let transferee_relay = match transferee_leg {
                DialogLeg::Caller => transaction.caller_relay_rtp.clone(),
                DialogLeg::Gateway => transaction.gateway_relay_rtp.clone(),
                DialogLeg::Transfer => None,
            };
            let transferee_destination = match transferee_leg {
                DialogLeg::Caller => transaction.caller_rtp.clone(),
                DialogLeg::Gateway => transaction.gateway_rtp.clone(),
                DialogLeg::Transfer => None,
            };
            if let Some(transferee_relay) = transferee_relay {
                let _ = edge_state
                    .media_relay
                    .set_target(&transferee_relay, &target_endpoint);
            }
            if let Some(destination) = transferee_destination {
                let target_relay = RtpEndpoint {
                    address: edge_config
                        .advertised_addr
                        .split(':')
                        .next()
                        .unwrap_or("127.0.0.1")
                        .to_string(),
                    port: target_port,
                };
                let _ = edge_state
                    .media_relay
                    .set_target(&target_relay, &destination);
            }
        }
        let referrer_dialog = match transferee_leg {
            DialogLeg::Caller => &mut transaction.dialogs.gateway,
            DialogLeg::Gateway => &mut transaction.dialogs.caller,
            DialogLeg::Transfer => &mut transaction.dialogs.caller,
        };
        let (target, bye) = build_dialog_bye(referrer_dialog, &edge_config.advertised_addr);
        datagrams.push(PendingDatagram::new(target, bye));
    } else if status >= 300 {
        if let Some(target_port) = subscription.target_relay_port {
            edge_state.media_relay.clear_target(target_port);
        }
        if let (Some(caller_relay), Some(gateway_relay)) = (
            transaction.caller_relay_rtp.clone(),
            transaction.gateway_relay_rtp.clone(),
        ) {
            edge_state
                .media_relay
                .pair_ports(caller_relay.port, gateway_relay.port);
            if let Some(gateway) = transaction.gateway_rtp.clone() {
                let _ = edge_state.media_relay.set_target(&caller_relay, &gateway);
            }
            if let Some(caller) = transaction.caller_rtp.clone() {
                let _ = edge_state.media_relay.set_target(&gateway_relay, &caller);
            }
        }
        transaction.transfer_dialog = None;
    }

    if status < 200 {
        transaction.refer_subscription = Some(subscription);
    }
    edge_state.inbound_transactions.insert(transaction);
    datagrams
}
