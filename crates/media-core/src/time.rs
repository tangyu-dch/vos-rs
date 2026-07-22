//! Media timestamp and compact-NTP helpers.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

pub fn compact_ntp_middle_32_now() -> u32 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let ntp_seconds = duration.as_secs().wrapping_add(2_208_988_800);
    let ntp_fraction = (u64::from(duration.subsec_nanos()) << 32) / 1_000_000_000;
    let ntp_timestamp = (ntp_seconds << 32) | ntp_fraction;
    ((ntp_timestamp >> 16) & u64::from(u32::MAX)) as u32
}

pub fn rtt_millis_from_compact_ntp(
    arrival_ntp_middle_32: u32,
    last_sender_report: u32,
    delay_since_last_sender_report: u32,
) -> Option<u32> {
    if last_sender_report == 0 || delay_since_last_sender_report == 0 {
        return None;
    }

    let rtt_units = arrival_ntp_middle_32
        .wrapping_sub(last_sender_report)
        .wrapping_sub(delay_since_last_sender_report);
    let millis = ((u64::from(rtt_units) * 1_000) + 32_768) / 65_536;
    Some(u32::try_from(millis).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_compact_ntp_rtt_in_milliseconds() {
        assert_eq!(
            rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0x0001_0000),
            Some(1_000)
        );
        assert_eq!(rtt_millis_from_compact_ntp(1, 0, 1), None);
    }
}
