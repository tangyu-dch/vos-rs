use super::*;
use crate::media::metrics::RtcpQualityWindow;
use crate::media::recording::{decode_pcma, decode_pcmu, RecordingPool};
use crate::media::sdp::{is_sdp_body, parse_sdp_rtp_endpoint, rewrite_sdp_body};
use crate::media::utils::rtt_millis_from_compact_ntp;
use rtp_core::{SrtpConfig, SrtpContext};
use sdp_core::RtpEndpoint;
use sip_core::{HeaderMap, HeaderName, HeaderValue};
use std::{fs, net::SocketAddr, path::PathBuf};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout, Duration};

#[test]
fn recording_pool_reports_capacity_and_queue_depth() {
    let pool = RecordingPool::new(2, 3);

    assert_eq!(pool.worker_count(), 2);
    assert_eq!(pool.total_capacity(), 6);
    assert_eq!(pool.queued_commands(), 0);
}

#[test]
fn media_crypto_session_round_trips_rtp_payload() {
    let config = SrtpConfig {
        master_key: [7u8; 16],
        master_salt: [9u8; 14],
        profile: rtp_core::SrtpProfile::Aes128CmHmacSha1_80,
    };
    let mut sender = MediaCryptoSession {
        context: SrtpContext::new(config.clone(), 0x0102_0304),
    };
    let mut receiver = MediaCryptoSession {
        context: SrtpContext::new(config, 0x0102_0304),
    };
    let mut packet = vec![
        0x80, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 1, 2, 3, 4,
    ];

    let original = packet.clone();
    sender.encrypt(&mut packet).unwrap();
    assert_ne!(packet, original);
    let decrypted_len = receiver.decrypt(&mut packet).unwrap();
    packet.truncate(decrypted_len);
    assert_eq!(packet, original);
}

#[test]
fn rtcp_quality_window_calculates_averages_and_mos() {
    let mut window = RtcpQualityWindow::default();
    window.observe(RtcpQualitySnapshot {
        reports: 2,
        report_blocks: 2,
        last_fraction_lost: Some(13),
        last_jitter: Some(80),
        last_rtt_ms: Some(40),
        ..RtcpQualitySnapshot::default()
    });

    assert_eq!(window.samples, 2);
    assert_eq!(window.average_fraction_lost, Some(13));
    assert_eq!(window.average_jitter, Some(80));
    assert_eq!(window.average_rtt_ms, Some(40));
    assert!(window.r_factor_x100.is_some());
    assert!(window.mos_x100.is_some());
}

#[test]
fn detects_application_sdp_with_parameters() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::new("Content-Type").unwrap(),
        HeaderValue::new("application/sdp; charset=utf-8"),
    );

    assert!(is_sdp_body(&headers, b"v=0\r\n"));
}

#[tokio::test]
async fn allocates_even_ports_without_reusing_active_leases() {
    let config = MediaConfig::new("203.0.113.10", 40_001, 40_004);
    let relay = MediaRelayState::new();

    assert_eq!(
        relay.allocate_endpoint(&config).unwrap(),
        RtpEndpoint::new("203.0.113.10", 40_002)
    );
    assert_eq!(
        relay.allocate_endpoint(&config).unwrap(),
        RtpEndpoint::new("203.0.113.10", 40_004)
    );
    assert_eq!(
        relay.allocate_endpoint(&config).unwrap_err(),
        MediaError::PortRangeExhausted {
            port_min: 40_002,
            port_max: 40_004
        }
    );

    relay.clear_target(40_002);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        relay.allocate_endpoint(&config).unwrap(),
        RtpEndpoint::new("203.0.113.10", 40_002)
    );
}

#[test]
fn estimates_rtcp_rtt_from_compact_ntp_values() {
    assert_eq!(
        rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0x0001_0000),
        Some(1_000)
    );
    assert_eq!(
        rtt_millis_from_compact_ntp(0x0003_0000, 0, 0x0001_0000),
        None
    );
    assert_eq!(
        rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0),
        None
    );
}

#[test]
fn decodes_g711_static_payloads_to_pcm() {
    assert_eq!(decode_pcmu(0xff), 0);
    assert_eq!(decode_pcmu(0x7f), 0);
    assert_eq!(decode_pcma(0xd5), 8);
    assert_eq!(decode_pcma(0x55), -8);
}

#[test]
fn records_pcmu_and_pcma_rtp_to_stereo_wav() {
    let dir = test_recording_dir("records_pcmu_and_pcma_rtp_to_stereo_wav");
    let config = MediaConfig::new("127.0.0.1", 40_000, 40_002).with_recording(true, &dir);
    let relay = MediaRelayState::new();
    let wav_path = relay
        .start_call_recording("call/with:unsafe@example.com", 40_000, 40_002, &config)
        .unwrap()
        .expect("recording should be enabled");

    let caller_packet = rtp_core::RtpPacket::new(0, 1, 0, 42, vec![0xff, 0xff])
        .unwrap()
        .encode()
        .unwrap();
    let caller_packet = RtpPacketView::parse(&caller_packet).unwrap();
    assert!(relay.record_rtp_packet(40_000, caller_packet).unwrap());

    let gateway_packet = rtp_core::RtpPacket::new(8, 1, 0, 24, vec![0xd5, 0xd5])
        .unwrap()
        .encode()
        .unwrap();
    let gateway_packet = RtpPacketView::parse(&gateway_packet).unwrap();
    assert!(relay.record_rtp_packet(40_002, gateway_packet).unwrap());
    relay.flush_recording_for_test(40_000).unwrap();

    let bytes = fs::read(&wav_path).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        8_000
    );
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
        4096
    );
    assert_eq!(bytes.len(), 8192);
    assert_eq!(i16::from_le_bytes([bytes[4096], bytes[4097]]), 0);
    assert_eq!(i16::from_le_bytes([bytes[4098], bytes[4099]]), 8);
    assert_eq!(i16::from_le_bytes([bytes[4100], bytes[4101]]), 0);
    assert_eq!(i16::from_le_bytes([bytes[4102], bytes[4103]]), 8);

    assert!(!wav_path.with_extension("json").exists());
}

#[test]
fn rotates_recording_when_segment_size_is_reached() {
    let dir = test_recording_dir("rotates_recording_when_segment_size_is_reached");
    let mut config = MediaConfig::new("127.0.0.1", 40_000, 40_002).with_recording(true, &dir);
    config.recording_max_file_bytes = 4100;
    config.recording_max_duration_secs = 0;
    let relay = MediaRelayState::new();
    let first_path = relay
        .start_call_recording("rotating-call", 40_000, 40_002, &config)
        .unwrap()
        .expect("recording should be enabled");

    for (sequence, timestamp) in [(1, 0), (2, 2)] {
        let packet = rtp_core::RtpPacket::new(0, sequence, timestamp, 42, vec![0xff, 0xff])
            .unwrap()
            .encode()
            .unwrap();
        let packet = RtpPacketView::parse(&packet).unwrap();
        assert!(relay.record_rtp_packet(40_000, packet).unwrap());
    }
    relay.flush_recording_for_test(40_000).unwrap();

    let second_path = first_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(
            "{}-part-0001.wav",
            first_path.file_stem().unwrap().to_string_lossy()
        ));
    assert!(first_path.is_file());
    assert!(second_path.is_file());
    assert!(!second_path.with_extension("json").exists());
}

#[test]
fn rewrites_sdp_body_for_relay_endpoint() {
    let body = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0 8 101\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n"
    );

    let rewritten =
        rewrite_sdp_body(body.as_bytes(), RtpEndpoint::new("203.0.113.10", 40_000)).unwrap();
    let rewritten = String::from_utf8(rewritten).unwrap();

    assert!(rewritten.contains("c=IN IP4 203.0.113.10\r\n"));
    assert!(rewritten.contains("m=audio 40000 RTP/AVP 0 8 101\r\n"));
    assert!(rewritten.contains("a=rtpmap:0 PCMU/8000\r\n"));
    assert!(rewritten.contains("a=rtpmap:8 PCMA/8000\r\n"));
    assert!(rewritten.contains("a=rtpmap:101 telephone-event/8000\r\n"));
}

#[test]
fn rejects_sdp_without_pcmu_or_pcma() {
    let body = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 101\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n"
    );

    let error =
        rewrite_sdp_body(body.as_bytes(), RtpEndpoint::new("203.0.113.10", 40_000)).unwrap_err();

    assert!(error.to_string().contains("missing compatible audio codec"));
}

#[test]
fn parses_original_sdp_rtp_endpoint() {
    let body = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0\r\n"
    );

    assert_eq!(
        parse_sdp_rtp_endpoint(body.as_bytes()).unwrap(),
        RtpEndpoint::new("192.0.2.10", 49170)
    );
}

#[test]
fn stores_and_clears_relay_targets() {
    let relay = MediaRelayState::new();
    let relay_endpoint = RtpEndpoint::new("203.0.113.10", 40_000);
    let target_endpoint = RtpEndpoint::new("127.0.0.1", 49_170);
    let target: SocketAddr = "127.0.0.1:49170".parse().unwrap();
    let rtcp_target: SocketAddr = "127.0.0.1:49171".parse().unwrap();

    relay.set_target(&relay_endpoint, &target_endpoint).unwrap();
    assert_eq!(relay.target_for_port(40_000), Some(target));
    assert_eq!(relay.target_for_port(40_001), Some(rtcp_target));

    relay.clear_target(40_000);
    assert_eq!(relay.target_for_port(40_000), None);
    assert_eq!(relay.target_for_port(40_001), None);
}

#[test]
fn pairs_ports_and_learns_symmetric_source() {
    let relay = MediaRelayState::new();
    let original_target: SocketAddr = "127.0.0.1:49170".parse().unwrap();
    let learned_source: SocketAddr = "127.0.0.1:53000".parse().unwrap();

    relay.pair_ports(40_000, 40_002);
    relay.set_target_addr(40_002, original_target);

    assert_eq!(relay.peer_port_for(40_000), Some(40_002));
    assert_eq!(relay.peer_port_for(40_002), Some(40_000));
    assert_eq!(relay.peer_port_for(40_001), Some(40_003));
    assert_eq!(relay.peer_port_for(40_003), Some(40_001));

    let update = relay
        .learn_symmetric_source(40_000, learned_source)
        .expect("symmetric source should be learned");
    assert_eq!(update.source_port, 40_000);
    assert_eq!(update.target_port, 40_002);
    assert_eq!(update.previous_target, Some(original_target));
    assert_eq!(update.learned_target, learned_source);
    assert_eq!(relay.target_for_port(40_002), Some(learned_source));
    assert_eq!(relay.metrics_for_port(40_000).learned_source_updates, 1);

    assert_eq!(relay.learn_symmetric_source(40_000, learned_source), None);
    assert_eq!(relay.metrics_for_port(40_000).learned_source_updates, 1);

    relay.clear_target(40_000);
    assert_eq!(relay.peer_port_for(40_000), None);
    assert_eq!(relay.peer_port_for(40_002), None);
    assert_eq!(relay.peer_port_for(40_001), None);
    assert_eq!(relay.peer_port_for(40_003), None);
}

#[tokio::test]
async fn rtp_relay_listener_forwards_valid_rtp_packets() {
    let relay_port = unused_even_udp_port();
    let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();

    let target_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target_port = target_socket.local_addr().unwrap().port();
    relay
        .set_target(
            &RtpEndpoint::new("127.0.0.1", config.port_min),
            &RtpEndpoint::new("127.0.0.1", target_port),
        )
        .unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
        .unwrap()
        .encode()
        .unwrap();
    sender
        .send_to(&packet, ("127.0.0.1", config.port_min))
        .await
        .unwrap();

    let mut buffer = [0_u8; 1500];
    let (size, _) = timeout(Duration::from_secs(1), target_socket.recv_from(&mut buffer))
        .await
        .expect("RTP packet should be relayed")
        .unwrap();
    assert_eq!(&buffer[..size], packet.as_slice());
    let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
        metrics.received_packets == 1 && metrics.forwarded_packets == 1
    })
    .await;
    assert_eq!(metrics.dropped_invalid_packets, 0);
    assert_eq!(metrics.dropped_no_target_packets, 0);
    assert_eq!(metrics.send_errors, 0);
    let totals = relay.metrics_totals();
    assert_eq!(totals.received_packets, metrics.received_packets);
    assert_eq!(totals.forwarded_packets, metrics.forwarded_packets);
    assert!(totals.recording_workers > 0);
    assert!(totals.recording_queue_capacity >= totals.recording_workers);

    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
async fn rtp_relay_transcodes_pcma_to_pcmu() {
    let (caller_port, gateway_port) = unused_even_udp_port_pair();
    let config = MediaConfig::new("127.0.0.1", caller_port, gateway_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();

    relay.pair_ports(caller_port, gateway_port);

    // Register codecs
    relay.register_port_codec(caller_port, rtp_core::AudioCodec::Pcma);
    relay.register_port_codec(gateway_port, rtp_core::AudioCodec::Pcmu);

    let target_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target_port = target_socket.local_addr().unwrap().port();

    relay
        .set_target(
            &RtpEndpoint::new("127.0.0.1", caller_port),
            &RtpEndpoint::new("127.0.0.1", target_port),
        )
        .unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let packet = rtp_core::RtpPacket::new(8, 1, 160, 42, vec![0xd5, 0xd5])
        .unwrap()
        .encode()
        .unwrap();
    sender
        .send_to(&packet, ("127.0.0.1", caller_port))
        .await
        .unwrap();

    let mut buffer = [0_u8; 1500];
    let (size, _) = timeout(Duration::from_secs(2), target_socket.recv_from(&mut buffer))
        .await
        .expect("RTP packet should be relayed")
        .unwrap();

    let relayed_rtp = rtp_core::RtpPacket::parse(&buffer[..size]).unwrap();
    assert_eq!(relayed_rtp.payload_type, 0); // PCMU static payload type is 0
    assert_eq!(relayed_rtp.payload.len(), 2);
    let decoded_sample0 = crate::media::recording::decode_pcmu(relayed_rtp.payload[0]);
    assert!((decoded_sample0 - 8).abs() < 50);

    let metrics = wait_for_metrics(&relay, caller_port, |metrics| {
        metrics.received_packets == 1 && metrics.forwarded_packets == 1
    })
    .await;
    assert_eq!(metrics.dropped_invalid_packets, 0);

    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
async fn rtp_relay_listener_learns_symmetric_source_for_paired_port() {
    let (caller_bound_port, gateway_bound_port) = unused_even_udp_port_pair();
    let config = MediaConfig::new("127.0.0.1", caller_bound_port, gateway_bound_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();
    relay.pair_ports(caller_bound_port, gateway_bound_port);

    let original_caller_target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    relay.set_target_addr(
        caller_bound_port,
        original_caller_target.local_addr().unwrap(),
    );

    let gateway_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    relay.set_target_addr(gateway_bound_port, gateway_socket.local_addr().unwrap());

    let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let learned_caller_addr = caller_socket.local_addr().unwrap();
    let caller_packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
        .unwrap()
        .encode()
        .unwrap();
    caller_socket
        .send_to(&caller_packet, ("127.0.0.1", gateway_bound_port))
        .await
        .unwrap();

    let mut gateway_buffer = [0_u8; 1500];
    let (gateway_size, _) = timeout(
        Duration::from_secs(1),
        gateway_socket.recv_from(&mut gateway_buffer),
    )
    .await
    .expect("caller RTP should be relayed to gateway target")
    .unwrap();
    assert_eq!(&gateway_buffer[..gateway_size], caller_packet.as_slice());

    wait_for_target(&relay, caller_bound_port, learned_caller_addr).await;
    assert_eq!(
        relay
            .metrics_for_port(gateway_bound_port)
            .learned_source_updates,
        1
    );

    let gateway_packet = rtp_core::RtpPacket::new(8, 2, 320, 24, vec![0x11, 0x12])
        .unwrap()
        .encode()
        .unwrap();
    gateway_socket
        .send_to(&gateway_packet, ("127.0.0.1", caller_bound_port))
        .await
        .unwrap();

    let mut caller_buffer = [0_u8; 1500];
    let (caller_size, _) = timeout(
        Duration::from_secs(1),
        caller_socket.recv_from(&mut caller_buffer),
    )
    .await
    .expect("gateway RTP should use learned caller source")
    .unwrap();
    assert_eq!(&caller_buffer[..caller_size], gateway_packet.as_slice());
    let fast_metrics = wait_for_metrics(&relay, caller_bound_port, |metrics| {
        metrics.fast_path_packets >= 1
    })
    .await;
    assert_eq!(fast_metrics.dropped_invalid_packets, 0);

    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
async fn rtcp_relay_listener_forwards_compound_packets() {
    let relay_port = unused_even_udp_port();
    let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();

    let target_rtp_port = unused_even_udp_port();
    let target_rtcp_socket = UdpSocket::bind(("127.0.0.1", target_rtp_port + 1))
        .await
        .unwrap();
    relay
        .set_target(
            &RtpEndpoint::new("127.0.0.1", relay_port),
            &RtpEndpoint::new("127.0.0.1", target_rtp_port),
        )
        .unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver_report = rtp_core::RtcpPacket::new(
        1,
        rtp_core::RtcpPacketType::ReceiverReport,
        vec![
            0xde, 0xad, 0xbe, 0xef, // reporter SSRC
            0x01, 0x02, 0x03, 0x04, // source SSRC
            0x20, 0x00, 0x00, 0x03, // fraction lost, cumulative lost
            0x00, 0x00, 0x10, 0x00, // extended highest sequence
            0x00, 0x00, 0x00, 0x2a, // jitter
            0x12, 0x34, 0x56, 0x78, // LSR
            0x00, 0x00, 0x00, 0x09, // DLSR
        ],
    )
    .unwrap()
    .encode()
    .unwrap();
    let source_description = rtp_core::RtcpPacket::new(
        1,
        rtp_core::RtcpPacketType::SourceDescription,
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap()
    .encode()
    .unwrap();
    let mut compound = receiver_report;
    compound.extend_from_slice(&source_description);

    sender
        .send_to(&compound, ("127.0.0.1", relay_port + 1))
        .await
        .unwrap();

    let mut buffer = [0_u8; 1500];
    let (size, _) = timeout(
        Duration::from_secs(1),
        target_rtcp_socket.recv_from(&mut buffer),
    )
    .await
    .expect("RTCP packet should be relayed")
    .unwrap();
    assert_eq!(&buffer[..size], compound.as_slice());

    let metrics = wait_for_metrics(&relay, relay_port + 1, |metrics| {
        metrics.received_packets == 1
            && metrics.forwarded_packets == 1
            && metrics.rtcp_quality.reports == 1
            && metrics.rtcp_quality.receiver_reports == 1
            && metrics.rtcp_quality.report_blocks == 1
    })
    .await;
    assert_eq!(metrics.dropped_invalid_packets, 0);
    assert_eq!(metrics.dropped_no_target_packets, 0);
    assert_eq!(metrics.send_errors, 0);
    assert_eq!(metrics.rtcp_quality.sender_reports, 0);
    assert_eq!(metrics.rtcp_quality.last_fraction_lost, Some(32));
    assert_eq!(metrics.rtcp_quality.max_fraction_lost, Some(32));
    assert_eq!(metrics.rtcp_quality.last_cumulative_lost, Some(3));
    assert_eq!(metrics.rtcp_quality.max_cumulative_lost, Some(3));
    assert_eq!(metrics.rtcp_quality.last_jitter, Some(42));
    assert_eq!(metrics.rtcp_quality.max_jitter, Some(42));
    assert_eq!(metrics.rtcp_quality.last_sender_report, Some(0x12345678));
    assert_eq!(metrics.rtcp_quality.delay_since_last_sender_report, Some(9));
    assert!(metrics.rtcp_quality.last_rtt_ms.is_some());
    assert_eq!(
        metrics.rtcp_quality.max_rtt_ms,
        metrics.rtcp_quality.last_rtt_ms
    );

    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
async fn rtp_relay_listener_tracks_invalid_rtp_packets() {
    let relay_port = unused_even_udp_port();
    let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender
        .send_to(&[0x80, 0x00], ("127.0.0.1", config.port_min))
        .await
        .unwrap();

    let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
        metrics.received_packets == 1 && metrics.dropped_invalid_packets == 1
    })
    .await;
    assert_eq!(metrics.forwarded_packets, 0);
    assert_eq!(metrics.dropped_no_target_packets, 0);
    assert_eq!(metrics.send_errors, 0);

    for handle in handles {
        handle.abort();
    }
}

#[tokio::test]
async fn rtp_relay_listener_tracks_packets_without_target() {
    let relay_port = unused_even_udp_port();
    let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
    let relay = MediaRelayState::new();
    let handles = spawn_rtp_relay_listeners(&config, relay.clone())
        .await
        .unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
        .unwrap()
        .encode()
        .unwrap();
    sender
        .send_to(&packet, ("127.0.0.1", config.port_min))
        .await
        .unwrap();

    let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
        metrics.received_packets == 1 && metrics.dropped_no_target_packets == 1
    })
    .await;
    assert_eq!(metrics.forwarded_packets, 0);
    assert_eq!(metrics.dropped_invalid_packets, 0);
    assert_eq!(metrics.send_errors, 0);

    for handle in handles {
        handle.abort();
    }
}

async fn wait_for_metrics(
    relay: &MediaRelayState,
    port: u16,
    predicate: impl Fn(MediaRelayMetrics) -> bool,
) -> MediaRelayMetrics {
    timeout(Duration::from_secs(1), async {
        loop {
            let metrics = relay.metrics_for_port(port);
            if predicate(metrics) {
                return metrics;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RTP relay metrics should be updated")
}

async fn wait_for_target(relay: &MediaRelayState, port: u16, target: SocketAddr) {
    timeout(Duration::from_secs(1), async {
        loop {
            if relay.target_for_port(port) == Some(target) {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RTP relay target should be updated")
}

fn unused_even_udp_port() -> u16 {
    loop {
        let rtp_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = rtp_socket.local_addr().unwrap().port();
        let Some(rtcp_port) = port.checked_add(1) else {
            continue;
        };
        if port % 2 == 0 && std::net::UdpSocket::bind(("127.0.0.1", rtcp_port)).is_ok() {
            return port;
        }
    }
}

fn unused_even_udp_port_pair() -> (u16, u16) {
    loop {
        let first_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let first_port = first_socket.local_addr().unwrap().port();
        let Some(second_port) = first_port.checked_add(2) else {
            continue;
        };
        if first_port % 2 != 0 {
            continue;
        }

        let first_rtcp = std::net::UdpSocket::bind(("127.0.0.1", first_port + 1));
        let second_rtp = std::net::UdpSocket::bind(("127.0.0.1", second_port));
        let second_rtcp = std::net::UdpSocket::bind(("127.0.0.1", second_port + 1));
        if first_rtcp.is_ok() && second_rtp.is_ok() && second_rtcp.is_ok() {
            return (first_port, second_port);
        }
    }
}

fn test_recording_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from("target")
        .join("test-recordings")
        .join(format!("{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn relay_plan_uses_fast_path_for_same_codec_without_media_features() {
    let relay = MediaRelayState::new();
    relay.pair_ports(40_000, 40_002);
    relay.register_port_codec(40_000, rtp_core::AudioCodec::Pcma);
    relay.register_port_codec(40_002, rtp_core::AudioCodec::Pcma);
    relay.set_target_addr(40_000, "127.0.0.1:9000".parse().unwrap());

    assert_eq!(relay.relay_plan(40_000).path, RelayPath::Fast);
}

#[test]
fn relay_plan_uses_processed_path_when_transcoding_is_required() {
    let relay = MediaRelayState::new();
    relay.pair_ports(40_000, 40_002);
    relay.register_port_codec(40_000, rtp_core::AudioCodec::Pcma);
    relay.register_port_codec(40_002, rtp_core::AudioCodec::Pcmu);
    relay.set_target_addr(40_000, "127.0.0.1:9000".parse().unwrap());

    assert_eq!(relay.relay_plan(40_000).path, RelayPath::Processed);
}

#[test]
fn relay_plan_downgrades_and_restores_when_monitoring_changes() {
    let relay = MediaRelayState::new();
    let monitor: SocketAddr = "127.0.0.1:9100".parse().unwrap();
    relay.pair_ports(40_000, 40_002);
    relay.register_port_codec(40_000, rtp_core::AudioCodec::Pcma);
    relay.register_port_codec(40_002, rtp_core::AudioCodec::Pcma);
    relay.set_target_addr(40_000, "127.0.0.1:9000".parse().unwrap());

    relay.start_monitoring(40_000, monitor);
    assert_eq!(relay.relay_plan(40_000).path, RelayPath::Processed);

    relay.stop_monitoring(40_000, monitor);
    assert_eq!(relay.relay_plan(40_000).path, RelayPath::Fast);
}
