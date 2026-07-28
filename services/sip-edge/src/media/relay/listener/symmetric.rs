use super::*;

pub(super) fn track_symmetric_source(
    relay: &MediaRelayState,
    local_port: u16,
    source: SocketAddr,
    packet_kind: MediaPacketKind,
    learned_source: &mut Option<SocketAddr>,
    binding: &mut Option<CachedSourceBinding>,
) {
    if *learned_source == Some(source) {
        return;
    }
    *learned_source = Some(source);
    *binding = Some(CachedSourceBinding {
        address: source,
        last_seen: std::time::Instant::now(),
    });
    if let Some(update) = relay.learn_symmetric_source(local_port, source) {
        debug!(
            source_port = update.source_port,
            target_port = update.target_port,
            learned_target = %update.learned_target,
            previous_target = ?update.previous_target,
            packet_kind = packet_kind.label(),
            "learned symmetric media source"
        );
    }
}
