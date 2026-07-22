use crate::time::{rtt_millis_from_compact_ntp, unix_timestamp_millis};
use rtp_core::RtcpReportBlock;
use serde::{Deserialize, Serialize};

/// Duration of one rolling RTCP quality window in milliseconds.
pub const RTCP_QUALITY_WINDOW_MS: u128 = 60_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcpQualityWindow {
    pub started_at_unix_ms: u128,
    pub reports: u64,
    pub samples: u64,
    pub average_fraction_lost: Option<u8>,
    pub average_jitter: Option<u32>,
    pub average_rtt_ms: Option<u32>,
    pub r_factor_x100: Option<u16>,
    pub mos_x100: Option<u16>,
    total_fraction_lost: u64,
    total_jitter: u64,
    total_rtt_ms: u64,
    rtt_samples: u64,
}

impl RtcpQualityWindow {
    pub fn is_degraded(&self) -> bool {
        self.average_fraction_lost
            .map(|value| u32::from(value) * 10_000 / 255 > 1_000)
            .unwrap_or(false)
            || self
                .average_jitter
                .map(|value| value > 100 * 8)
                .unwrap_or(false)
            || self
                .average_rtt_ms
                .map(|value| value > 300)
                .unwrap_or(false)
            || self.mos_x100.map(|value| value < 350).unwrap_or(false)
    }

    pub fn observe(&mut self, snapshot: RtcpQualitySnapshot) {
        let now = unix_timestamp_millis();
        if self.started_at_unix_ms == 0
            || now.saturating_sub(self.started_at_unix_ms) >= RTCP_QUALITY_WINDOW_MS
        {
            *self = Self {
                started_at_unix_ms: now,
                ..Self::default()
            };
        }

        self.reports += snapshot.reports;
        let samples = snapshot.report_blocks;
        self.samples += samples;
        if let Some(value) = snapshot.last_fraction_lost {
            self.total_fraction_lost += u64::from(value) * samples.max(1);
        }
        if let Some(value) = snapshot.last_jitter {
            self.total_jitter += u64::from(value) * samples.max(1);
        }
        if let Some(value) = snapshot.last_rtt_ms {
            self.total_rtt_ms += u64::from(value);
            self.rtt_samples += 1;
        }
        self.recalculate();
    }

    pub fn merge(&mut self, other: Self) {
        if other.started_at_unix_ms == 0 {
            return;
        }
        if self.started_at_unix_ms == 0 {
            *self = other;
            return;
        }
        self.started_at_unix_ms = self.started_at_unix_ms.min(other.started_at_unix_ms);
        self.reports += other.reports;
        self.samples += other.samples;
        self.total_fraction_lost += other.total_fraction_lost;
        self.total_jitter += other.total_jitter;
        self.total_rtt_ms += other.total_rtt_ms;
        self.rtt_samples += other.rtt_samples;
        self.recalculate();
    }

    pub fn recalculate(&mut self) {
        if self.samples == 0 {
            return;
        }
        self.average_fraction_lost = Some((self.total_fraction_lost / self.samples) as u8);
        self.average_jitter = Some((self.total_jitter / self.samples) as u32);
        self.average_rtt_ms =
            (self.rtt_samples > 0).then_some((self.total_rtt_ms / self.rtt_samples) as u32);

        let loss_percent =
            f64::from(self.average_fraction_lost.unwrap_or_default()) * 100.0 / 255.0;
        let jitter_ms = f64::from(self.average_jitter.unwrap_or_default()) / 8.0;
        let rtt_ms = f64::from(self.average_rtt_ms.unwrap_or_default());
        let r_factor =
            (93.2 - 0.024 * rtt_ms - 0.11 * loss_percent - 0.01 * jitter_ms).clamp(0.0, 100.0);
        let mos =
            (1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor))
                .clamp(1.0, 4.5);
        self.r_factor_x100 = Some((r_factor * 100.0).round() as u16);
        self.mos_x100 = Some((mos * 100.0).round() as u16);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcpQualitySnapshot {
    pub reports: u64,
    pub sender_reports: u64,
    pub receiver_reports: u64,
    pub report_blocks: u64,
    pub last_fraction_lost: Option<u8>,
    pub max_fraction_lost: Option<u8>,
    pub last_cumulative_lost: Option<i32>,
    pub max_cumulative_lost: Option<i32>,
    pub last_jitter: Option<u32>,
    pub max_jitter: Option<u32>,
    pub last_sender_report: Option<u32>,
    pub delay_since_last_sender_report: Option<u32>,
    pub last_rtt_ms: Option<u32>,
    pub max_rtt_ms: Option<u32>,
}

impl RtcpQualitySnapshot {
    pub fn merge(&mut self, other: Self) {
        self.reports += other.reports;
        self.sender_reports += other.sender_reports;
        self.receiver_reports += other.receiver_reports;
        self.report_blocks += other.report_blocks;
        merge_last_max(
            &mut self.last_fraction_lost,
            &mut self.max_fraction_lost,
            other.last_fraction_lost,
            other.max_fraction_lost,
        );
        merge_last_max(
            &mut self.last_cumulative_lost,
            &mut self.max_cumulative_lost,
            other.last_cumulative_lost,
            other.max_cumulative_lost,
        );
        merge_last_max(
            &mut self.last_jitter,
            &mut self.max_jitter,
            other.last_jitter,
            other.max_jitter,
        );
        if let Some(value) = other.last_sender_report {
            self.last_sender_report = Some(value);
        }
        if let Some(value) = other.delay_since_last_sender_report {
            self.delay_since_last_sender_report = Some(value);
        }
        merge_last_max(
            &mut self.last_rtt_ms,
            &mut self.max_rtt_ms,
            other.last_rtt_ms,
            other.max_rtt_ms,
        );
    }

    pub fn record_report_block(&mut self, block: &RtcpReportBlock, arrival_ntp_middle_32: u32) {
        self.report_blocks += 1;
        self.last_fraction_lost = Some(block.fraction_lost);
        self.max_fraction_lost = max_option(self.max_fraction_lost, block.fraction_lost);
        self.last_cumulative_lost = Some(block.cumulative_lost);
        self.max_cumulative_lost = max_option(self.max_cumulative_lost, block.cumulative_lost);
        self.last_jitter = Some(block.interarrival_jitter);
        self.max_jitter = max_option(self.max_jitter, block.interarrival_jitter);
        self.last_sender_report = Some(block.last_sender_report);
        self.delay_since_last_sender_report = Some(block.delay_since_last_sender_report);
        if let Some(rtt_ms) = rtt_millis_from_compact_ntp(
            arrival_ntp_middle_32,
            block.last_sender_report,
            block.delay_since_last_sender_report,
        ) {
            self.last_rtt_ms = Some(rtt_ms);
            self.max_rtt_ms = max_option(self.max_rtt_ms, rtt_ms);
        }
    }
}

fn merge_last_max<T: Ord + Copy>(
    last: &mut Option<T>,
    maximum: &mut Option<T>,
    other_last: Option<T>,
    other_max: Option<T>,
) {
    if let Some(value) = other_last {
        *last = Some(value);
        *maximum = max_option(*maximum, value);
    }
    if let Some(value) = other_max {
        *maximum = max_option(*maximum, value);
    }
}

fn max_option<T: Ord + Copy>(current: Option<T>, candidate: T) -> Option<T> {
    Some(current.map_or(candidate, |value| value.max(candidate)))
}
