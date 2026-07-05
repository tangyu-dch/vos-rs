use rtp_core::{RtcpPacket, RtcpPacketType, RtcpReportBlock, RtpError};

#[test]
fn parses_receiver_report_packet() {
    let raw = [
        0x81, 201, 0x00, 0x07, 0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04, 0, 0, 0, 1, 0, 0, 0,
        2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5,
    ];

    let packet = RtcpPacket::parse(&raw).expect("RTCP RR should parse");

    assert_eq!(packet.count, 1);
    assert_eq!(packet.packet_type, RtcpPacketType::ReceiverReport);
    assert_eq!(packet.payload, raw[4..].to_vec());
    assert_eq!(packet.padding_len, 0);

    let report = packet
        .receiver_report()
        .expect("RR should parse")
        .expect("packet should be RR");
    assert_eq!(report.reporter_ssrc, 0xdeadbeef);
    assert_eq!(
        report.report_blocks,
        vec![RtcpReportBlock {
            ssrc: 0x01020304,
            fraction_lost: 0,
            cumulative_lost: 1,
            extended_highest_sequence_number: 2,
            interarrival_jitter: 3,
            last_sender_report: 4,
            delay_since_last_sender_report: 5,
        }]
    );
}

#[test]
fn encodes_and_parses_sender_report_packet() {
    let mut packet = RtcpPacket::new(
        0,
        RtcpPacketType::SenderReport,
        vec![
            0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5,
        ],
    )
    .unwrap();
    packet.padding_len = 4;

    let encoded = packet.encode().expect("RTCP should encode");
    let parsed = RtcpPacket::parse(&encoded).expect("encoded RTCP should parse");

    assert_eq!(parsed, packet);
    assert_eq!(encoded.len() % 4, 0);

    let report = parsed
        .sender_report()
        .expect("SR should parse")
        .expect("packet should be SR");
    assert_eq!(report.sender_ssrc, 0xdeadbeef);
    assert_eq!(report.ntp_timestamp_msw, 1);
    assert_eq!(report.ntp_timestamp_lsw, 2);
    assert_eq!(report.rtp_timestamp, 3);
    assert_eq!(report.sender_packet_count, 4);
    assert_eq!(report.sender_octet_count, 5);
    assert!(report.report_blocks.is_empty());
}

#[test]
fn parses_compound_rtcp_packets() {
    let rr = RtcpPacket::new(
        0,
        RtcpPacketType::ReceiverReport,
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap()
    .encode()
    .unwrap();
    let sdes = RtcpPacket::new(
        1,
        RtcpPacketType::SourceDescription,
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap()
    .encode()
    .unwrap();
    let mut compound = rr.clone();
    compound.extend_from_slice(&sdes);

    let packets = RtcpPacket::parse_compound(&compound).expect("compound RTCP should parse");

    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].packet_type, RtcpPacketType::ReceiverReport);
    assert_eq!(packets[1].packet_type, RtcpPacketType::SourceDescription);
}

#[test]
fn rejects_short_rtcp_packet() {
    let error = RtcpPacket::parse(&[0x80, 201]).expect_err("packet should fail");

    assert_eq!(error, RtpError::RtcpPacketTooShort);
}

#[test]
fn rejects_invalid_rtcp_length() {
    let raw = [0x80, 201, 0x00, 0x02, 0xde, 0xad, 0xbe, 0xef];

    let error = RtcpPacket::parse(&raw).expect_err("packet should fail");

    assert_eq!(error, RtpError::RtcpInvalidLength);
}

#[test]
fn rejects_invalid_rtcp_padding() {
    let raw = [0xa0, 201, 0x00, 0x01, 0xde, 0xad, 0xbe, 0xef];

    let error = RtcpPacket::parse(&raw).expect_err("packet should fail");

    assert_eq!(error, RtpError::RtcpInvalidPadding);
}

#[test]
fn rejects_wrong_rtcp_version() {
    let raw = [0x40, 201, 0x00, 0x01, 0xde, 0xad, 0xbe, 0xef];

    let error = RtcpPacket::parse(&raw).expect_err("packet should fail");

    assert_eq!(error, RtpError::UnsupportedVersion(1));
}

#[test]
fn rejects_out_of_range_rtcp_count() {
    let error = RtcpPacket::new(32, RtcpPacketType::ReceiverReport, Vec::new())
        .expect_err("packet should fail");

    assert_eq!(error, RtpError::RtcpCountOutOfRange(32));
}

#[test]
fn parses_receiver_report_with_negative_cumulative_loss() {
    let packet = RtcpPacket::new(
        1,
        RtcpPacketType::ReceiverReport,
        vec![
            0xde, 0xad, 0xbe, 0xef, // reporter SSRC
            0x01, 0x02, 0x03, 0x04, // source SSRC
            0x80, 0xff, 0xff, 0xfe, // fraction lost, cumulative lost -2
            0x00, 0x00, 0x10, 0x00, // extended highest sequence
            0x00, 0x00, 0x00, 0x40, // jitter
            0x12, 0x34, 0x56, 0x78, // LSR
            0x00, 0x00, 0x00, 0x20, // DLSR
        ],
    )
    .unwrap();

    let report = packet
        .receiver_report()
        .expect("RR should parse")
        .expect("packet should be RR");

    assert_eq!(report.report_blocks[0].fraction_lost, 128);
    assert_eq!(report.report_blocks[0].cumulative_lost, -2);
    assert_eq!(report.report_blocks[0].interarrival_jitter, 64);
    assert_eq!(report.report_blocks[0].last_sender_report, 0x12345678);
    assert_eq!(report.report_blocks[0].delay_since_last_sender_report, 32);
}

#[test]
fn rejects_malformed_receiver_report_payload() {
    let packet = RtcpPacket::new(
        1,
        RtcpPacketType::ReceiverReport,
        vec![0xde, 0xad, 0xbe, 0xef],
    )
    .unwrap();

    let error = packet
        .receiver_report()
        .expect_err("RR with missing report block should fail");

    assert_eq!(error, RtpError::RtcpInvalidReportLength);
}
