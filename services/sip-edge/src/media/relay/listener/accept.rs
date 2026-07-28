use super::*;

pub(super) fn accept_media_source(
    relay: &MediaRelayState,
    local_port: u16,
    source: SocketAddr,
    anti_spoofing: bool,
    symmetric_rtp_learning: bool,
    relearn_after_secs: u64,
    binding: &mut Option<CachedSourceBinding>,
) -> bool {
    if !anti_spoofing {
        return true;
    }

    let now = std::time::Instant::now();
    match binding {
        Some(current) if current.address == source => {
            current.last_seen = now;
            true
        }
        Some(current)
            if !symmetric_rtp_learning
                && now.duration_since(current.last_seen)
                    < std::time::Duration::from_secs(relearn_after_secs) =>
        {
            relay.record_metric(local_port, |metrics| metrics.dropped_spoofed_packets += 1);
            false
        }
        _ => {
            *binding = Some(CachedSourceBinding {
                address: source,
                last_seen: now,
            });
            relay
                .source_bindings
                .insert(local_port, SourceBinding { address: source });
            true
        }
    }
}
