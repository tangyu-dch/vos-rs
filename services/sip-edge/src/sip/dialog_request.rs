use sip_core::{Method, SipRequest};

use crate::edge_state::{DialogLegState, InboundTransaction, PendingDatagram};

use super::outbound;

pub(crate) fn target_for_dialog(dialog: &DialogLegState) -> String {
    dialog
        .route_set
        .first()
        .map(|route| outbound::target_addr_for_str(route))
        .or_else(|| dialog.peer.clone())
        .unwrap_or_else(|| outbound::target_addr_for(&dialog.remote_target))
}

pub(crate) fn build_dialog_request(
    template: &SipRequest,
    dialog: &mut DialogLegState,
    method: Method,
    advertised_addr: &str,
    body: &[u8],
) -> Option<PendingDatagram> {
    if dialog.call_id.is_empty() || dialog.remote_tag.is_none() {
        return None;
    }

    dialog.local_cseq = dialog.local_cseq.saturating_add(1);
    let mut request = template.clone();
    request.method = method;
    let target = target_for_dialog(dialog);
    let bytes = outbound::build_b2bua_in_dialog_request(
        &request,
        &dialog.remote_target,
        advertised_addr,
        &dialog.route_set,
        &dialog.call_id,
        &dialog.local_uri,
        &dialog.local_tag,
        &dialog.remote_uri,
        dialog.remote_tag.as_deref(),
        dialog.local_cseq,
        body,
    );
    Some(PendingDatagram::new(target, bytes))
}

pub(crate) fn build_session_byes(
    transaction: &mut InboundTransaction,
    advertised_addr: &str,
) -> Vec<PendingDatagram> {
    let Some(template) = transaction.original_request.clone() else {
        return Vec::new();
    };
    let mut datagrams = Vec::new();
    if let Some(datagram) = build_dialog_request(
        &template,
        &mut transaction.dialogs.caller,
        Method::Bye,
        advertised_addr,
        &[],
    ) {
        datagrams.push(datagram);
    }
    if let Some(datagram) = build_dialog_request(
        &template,
        &mut transaction.dialogs.gateway,
        Method::Bye,
        advertised_addr,
        &[],
    ) {
        datagrams.push(datagram);
    }
    if let Some(transfer) = &mut transaction.transfer_dialog {
        if let Some(datagram) = build_dialog_request(
            &template,
            &mut transfer.dialog,
            Method::Bye,
            advertised_addr,
            &[],
        ) {
            datagrams.push(datagram);
        }
    }
    datagrams
}
