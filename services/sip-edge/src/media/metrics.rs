//! # 媒体质量指标
//!
//! 本模块实现了 RTP/RTCP 质量监控和指标收集，包括：
//!
//! - **RTP 统计**：接收/转发/丢弃包计数
//! - **RTCP 质量窗口**：60 秒滚动窗口的丢包率、抖动、RTT
//! - **MOS 计算**：基于 E-model 的 MOS 分数计算
//! - **录音统计**：录音队列深度、错误计数
//! - **DTMF 统计**：DTMF 事件计数
//!
//! ## MOS 计算公式
//!
//! ```text
//! R = 93.2 - 0.024 * d - 0.11 * e - 0.01 * j
//! MOS = 1 + 0.035 * R + 0.000007 * R * (R - 60) * (100 - R)
//! ```
//!
//! 其中：
//! - `d`：单向延迟（ms）
//! - `e`：丢包率（%）
//! - `j`：抖动（ms）

use crate::media::utils::{rtt_millis_from_compact_ntp, unix_timestamp_millis};
use rtp_core::{RtcpReportBlock, RtpPacketView};
use serde::Serialize;

pub(crate) const RTCP_QUALITY_WINDOW_MS: u128 = 60_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct MediaRelayMetrics {
    pub received_packets: u64,
    pub forwarded_packets: u64,
    pub dropped_invalid_packets: u64,
    pub dropped_no_target_packets: u64,
    pub send_errors: u64,
    pub learned_source_updates: u64,
    pub dropped_spoofed_packets: u64,
    pub rtcp_quality: RtcpQualitySnapshot,
    pub rtcp_window: RtcpQualityWindow,
    pub rtcp_quality_alerts: u64,
    pub rtcp_quality_degraded: bool,
    pub recorded_packets: u64,
    pub recording_dropped_packets: u64,
    pub recording_errors: u64,
    pub recording_queue_depth: u64,
    pub recording_queue_capacity: u64,
    pub recording_workers: u64,
    pub dtmf_events: u64,
    /// 通过轻量直转路径处理 of RTP 包数量。
    pub fast_path_packets: u64,
    pub webrtc_ice_connected: bool,
    pub webrtc_dtls_connected: bool,
    pub webrtc_dtls_failed: bool,
}

impl MediaRelayMetrics {
    pub(crate) fn merge(&mut self, other: Self) {
        self.received_packets += other.received_packets;
        self.forwarded_packets += other.forwarded_packets;
        self.dropped_invalid_packets += other.dropped_invalid_packets;
        self.dropped_no_target_packets += other.dropped_no_target_packets;
        self.send_errors += other.send_errors;
        self.learned_source_updates += other.learned_source_updates;
        self.dropped_spoofed_packets += other.dropped_spoofed_packets;
        self.rtcp_quality.merge(other.rtcp_quality);
        self.rtcp_window.merge(other.rtcp_window);
        self.rtcp_quality_alerts += other.rtcp_quality_alerts;
        self.rtcp_quality_degraded |= other.rtcp_quality_degraded;
        self.recorded_packets += other.recorded_packets;
        self.recording_dropped_packets += other.recording_dropped_packets;
        self.recording_errors += other.recording_errors;
        self.recording_queue_depth += other.recording_queue_depth;
        self.recording_queue_capacity += other.recording_queue_capacity;
        self.recording_workers += other.recording_workers;
        self.dtmf_events += other.dtmf_events;
        self.fast_path_packets += other.fast_path_packets;
        self.webrtc_ice_connected |= other.webrtc_ice_connected;
        self.webrtc_dtls_connected |= other.webrtc_dtls_connected;
        self.webrtc_dtls_failed |= other.webrtc_dtls_failed;
    }
}

/// Rolling RTCP quality aggregates for the current 60-second window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
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

        if let Some(value) = other.last_fraction_lost {
            self.last_fraction_lost = Some(value);
            self.max_fraction_lost = max_option(self.max_fraction_lost, value);
        }
        if let Some(value) = other.max_fraction_lost {
            self.max_fraction_lost = max_option(self.max_fraction_lost, value);
        }
        if let Some(value) = other.last_cumulative_lost {
            self.last_cumulative_lost = Some(value);
            self.max_cumulative_lost = max_option(self.max_cumulative_lost, value);
        }
        if let Some(value) = other.max_cumulative_lost {
            self.max_cumulative_lost = max_option(self.max_cumulative_lost, value);
        }
        if let Some(value) = other.last_jitter {
            self.last_jitter = Some(value);
            self.max_jitter = max_option(self.max_jitter, value);
        }
        if let Some(value) = other.max_jitter {
            self.max_jitter = max_option(self.max_jitter, value);
        }
        if let Some(value) = other.last_sender_report {
            self.last_sender_report = Some(value);
        }
        if let Some(value) = other.delay_since_last_sender_report {
            self.delay_since_last_sender_report = Some(value);
        }
        if let Some(value) = other.last_rtt_ms {
            self.last_rtt_ms = Some(value);
            self.max_rtt_ms = max_option(self.max_rtt_ms, value);
        }
        if let Some(value) = other.max_rtt_ms {
            self.max_rtt_ms = max_option(self.max_rtt_ms, value);
        }
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

fn max_option<T: Ord + Copy>(current: Option<T>, candidate: T) -> Option<T> {
    Some(current.map_or(candidate, |value| value.max(candidate)))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RtpReceiveStats {
    pub ssrc: u32,
    pub base_sequence: u16,
    pub highest_sequence: u16,
    pub received: u32,
    pub jitter: u32,
    pub last_transit: Option<i64>,
    pub last_report_unix_ms: u128,
}

impl RtpReceiveStats {
    pub fn observe(&mut self, packet: RtpPacketView<'_>) {
        if self.received == 0 {
            self.ssrc = packet.ssrc;
            self.base_sequence = packet.sequence_number;
            self.highest_sequence = packet.sequence_number;
        } else if packet.sequence_number.wrapping_sub(self.highest_sequence) < 0x8000 {
            self.highest_sequence = packet.sequence_number;
        }
        self.received = self.received.saturating_add(1);

        let arrival_units = (unix_timestamp_millis() as i64).saturating_mul(8);
        let transit = arrival_units.saturating_sub(i64::from(packet.timestamp));
        if let Some(previous) = self.last_transit {
            let delta = (transit - previous).unsigned_abs();
            let jitter = u64::from(self.jitter);
            self.jitter =
                ((jitter.saturating_mul(15) + delta) / 16).min(u64::from(u32::MAX)) as u32;
        }
        self.last_transit = Some(transit);
    }

    pub fn receiver_report(&mut self) -> Option<Vec<u8>> {
        let now = unix_timestamp_millis();
        if self.received == 0 {
            return None;
        }
        if self.last_report_unix_ms == 0 {
            self.last_report_unix_ms = now;
            return None;
        }
        if now.saturating_sub(self.last_report_unix_ms) < 5_000 {
            return None;
        }
        self.last_report_unix_ms = now;
        let expected = u32::from(self.highest_sequence.wrapping_sub(self.base_sequence)) + 1;
        let lost = expected.saturating_sub(self.received);
        let fraction_lost = if expected == 0 {
            0
        } else {
            ((u64::from(lost) * 256) / u64::from(expected)).min(255) as u8
        };
        let mut payload = Vec::with_capacity(28);
        payload.extend_from_slice(&self.ssrc.wrapping_add(1).to_be_bytes());
        payload.extend_from_slice(&self.ssrc.to_be_bytes());
        payload.push(fraction_lost);
        let cumulative_lost = i32::try_from(lost).unwrap_or(i32::MAX).clamp(0, 0x7f_ffff);
        let lost_bytes = cumulative_lost.to_be_bytes();
        payload.extend_from_slice(&lost_bytes[1..]);
        payload.extend_from_slice(&u32::from(self.highest_sequence).to_be_bytes());
        payload.extend_from_slice(&self.jitter.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        rtp_core::RtcpPacket::new(1, rtp_core::RtcpPacketType::ReceiverReport, payload)
            .ok()?
            .encode()
            .ok()
    }
}
