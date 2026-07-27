use sip_core::{SipResponse, SipUri};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::outbound;

/// 构建 fork CANCEL 数据报：当收到某 fork 的 2xx 响应时，向其他 fork 发送 CANCEL；
/// 当收到 3xx+ 响应时，仅执行 decrement_active 副作用。
///
/// 返回 `cancel_datagrams`（2xx 时非空，3xx+ 时为空但已副作用 decrement_active）。
pub(crate) fn build_fork_cancel_datagrams(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    session_key: Option<&str>,
    call_id: Option<&str>,
    gateway_call_id: &str,
) -> Vec<PendingDatagram> {
    let mut cancel_datagrams = Vec::new();

    let is_invite = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str().contains("INVITE"))
        .unwrap_or(false);

    if is_invite {
        if let Some(cid) = call_id {
            if (200..300).contains(&sip_response.status_code) {
                let mut forks_to_cancel = Vec::new();
                let mut request_user = None;
                let mut from_header = String::new();
                let mut to_header = String::new();
                let mut invite_cseq = 1;

                if let Some(mut t_mut) = edge_state
                    .inbound_transactions
                    .get_mut(session_key.unwrap_or(cid))
                {
                    if !t_mut.fork_dialogs.is_empty() {
                        for (fork_cid, fork) in t_mut.fork_dialogs.iter() {
                            if fork_cid != gateway_call_id {
                                forks_to_cancel.push((fork_cid.clone(), fork.gateway_id.clone()));
                            }
                        }
                        t_mut.fork_dialogs.clear();
                    }
                    if let Some(ref orig_req) = t_mut.original_request {
                        from_header = orig_req
                            .headers
                            .get("from")
                            .map(|v| v.as_str().to_string())
                            .unwrap_or_default();
                        to_header = orig_req
                            .headers
                            .get("to")
                            .map(|v| v.as_str().to_string())
                            .unwrap_or_default();
                        invite_cseq = orig_req
                            .headers
                            .get("cseq")
                            .and_then(|v| crate::sip::dialog::cseq_number(v.as_str()))
                            .unwrap_or(1);
                        request_user = orig_req.uri.user.clone();
                    }
                }

                for (fork_cid, fork_gw) in forks_to_cancel {
                    if !fork_gw.is_empty() {
                        edge_state.gateway_health.decrement_active(&fork_gw);
                        let status = edge_state.gateway_health.get_gateway_status(&fork_gw);
                        crate::timers::persist_gateway_health(edge_state, fork_gw.clone(), status);
                    }

                    if let Some(ref user) = request_user {
                        let routes = edge_state.call_manager.routes();
                        let gateway_target = routes
                            .routes()
                            .iter()
                            .find(|r| r.target.gateway_id.as_str() == fork_gw)
                            .map(|r| r.target.clone());
                        if let Some(target) = gateway_target {
                            let outbound_uri = SipUri {
                                secure: false,
                                user: Some(user.clone()),
                                host: target.host.clone().into(),
                                port: target.port,
                                params: Vec::new(),
                            };
                            let target_addr = outbound::target_addr_for(&outbound_uri);
                            let branch = format!("z9hG4bK-cancel-{}", fork_cid);
                            let cancel_bytes = format!(
                                "CANCEL {uri} SIP/2.0\r\n\
                                 Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
                                 Max-Forwards: 70\r\n\
                                 From: {from}\r\n\
                                 To: {to}\r\n\
                                 Call-ID: {fork_cid}\r\n\
                                 CSeq: {cseq} CANCEL\r\n\
                                 Content-Length: 0\r\n\r\n",
                                uri = outbound_uri,
                                addr = edge_config.advertised_addr,
                                branch = branch,
                                from = from_header,
                                to = to_header,
                                fork_cid = fork_cid,
                                cseq = invite_cseq
                            )
                            .into_bytes();
                            cancel_datagrams.push(PendingDatagram::new(target_addr, cancel_bytes));
                        }
                    }
                }
            } else if sip_response.status_code >= 300 {
                let mut fork_gw_to_decrement = None;
                if let Some(mut t_mut) = edge_state
                    .inbound_transactions
                    .get_mut(session_key.unwrap_or(cid))
                {
                    if let Some(fork) = t_mut.fork_dialogs.remove(gateway_call_id) {
                        fork_gw_to_decrement = Some(fork.gateway_id);
                    }
                }
                if let Some(gw_id) = fork_gw_to_decrement {
                    if !gw_id.is_empty() {
                        edge_state.gateway_health.decrement_active(&gw_id);
                        let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                        crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                    }
                }
            }
        }
    }

    cancel_datagrams
}
