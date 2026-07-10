use rtp_core::{RtpError, RtpHeaderExtension, RtpPacket, RtpPacketView};

#[test]
fn parses_minimal_rtp_packet() {
    let raw = [
        0x80, 0x00, 0x12, 0x34, 0x00, 0x00, 0x10, 0x00, 0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03,
    ];

    let packet = RtpPacket::parse(&raw).expect("RTP should parse");

    assert!(!packet.marker);
    assert_eq!(packet.payload_type, 0);
    assert_eq!(packet.sequence_number, 0x1234);
    assert_eq!(packet.timestamp, 0x1000);
    assert_eq!(packet.ssrc, 0xdeadbeef);
    assert_eq!(packet.payload, vec![0x01, 0x02, 0x03]);
}

#[test]
fn encodes_and_parses_packet_with_extension_csrcs_and_padding() {
    let mut packet = RtpPacket::new(96, 7, 160, 0x01020304, vec![0xaa, 0xbb]).unwrap();
    packet.marker = true;
    packet.csrcs = vec![0x11111111, 0x22222222];
    packet.extension = Some(RtpHeaderExtension::new(0x1000, vec![1, 2, 3, 4]).unwrap());
    packet.padding_len = 4;

    let encoded = packet.encode().expect("RTP should encode");
    let parsed = RtpPacket::parse(&encoded).expect("encoded RTP should parse");

    assert_eq!(parsed, packet);
}

#[test]
fn parses_borrowed_packet_view_without_copying_payload() {
    let mut packet = RtpPacket::new(96, 7, 160, 0x01020304, vec![0xaa, 0xbb]).unwrap();
    packet.marker = true;
    packet.csrcs = vec![0x11111111];
    packet.extension = Some(RtpHeaderExtension::new(0x1000, vec![1, 2, 3, 4]).unwrap());
    let encoded = packet.encode().expect("RTP should encode");

    let view = RtpPacketView::parse(&encoded).expect("encoded RTP should parse as view");

    assert!(view.marker);
    assert_eq!(view.payload_type, 96);
    assert_eq!(view.sequence_number, 7);
    assert_eq!(view.timestamp, 160);
    assert_eq!(view.ssrc, 0x01020304);
    assert_eq!(view.csrcs, &[0x11, 0x11, 0x11, 0x11]);
    assert_eq!(view.extension.unwrap().data, &[1, 2, 3, 4]);
    assert_eq!(view.payload, &[0xaa, 0xbb]);
}

#[test]
fn rejects_wrong_version() {
    let raw = [0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1];

    let error = RtpPacket::parse(&raw).expect_err("packet should fail");

    assert_eq!(error, RtpError::UnsupportedVersion(1));
}

#[test]
fn rejects_short_packet() {
    let error = RtpPacket::parse(&[0x80, 0x00]).expect_err("packet should fail");

    assert_eq!(error, RtpError::PacketTooShort);
}

#[test]
fn rejects_invalid_extension_length() {
    let result = RtpHeaderExtension::new(0x1000, vec![1, 2, 3]);

    assert_eq!(result.unwrap_err(), RtpError::InvalidExtensionLength);
}

#[test]
fn rejects_invalid_padding() {
    let raw = [0xa0, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1, 0x11, 0x00];

    let error = RtpPacket::parse(&raw).expect_err("packet should fail");

    assert_eq!(error, RtpError::InvalidPadding);
}
