use sdp_core::{AudioFormat, RtpEndpoint, SdpError, SessionDescription};

#[test]
fn parses_first_audio_rtp_endpoint_from_session_connection() {
    let sdp = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0 8 101\r\n",
        "a=rtpmap:0 PCMU/8000\r\n"
    );

    let session = SessionDescription::parse(sdp).unwrap();

    assert_eq!(
        session.first_audio_rtp_endpoint().unwrap(),
        RtpEndpoint::new("192.0.2.10", 49170)
    );
    assert_eq!(session.media().len(), 1);
}

#[test]
fn rewrites_session_connection_and_audio_port() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0 8 101\r\n",
        "a=rtpmap:0 PCMU/8000\r\n"
    ))
    .unwrap();

    session
        .rewrite_first_audio_rtp_endpoint(RtpEndpoint::new("203.0.113.50", 40000))
        .unwrap();

    let output = session.to_string();
    assert!(output.contains("c=IN IP4 203.0.113.50\r\n"));
    assert!(output.contains("m=audio 40000 RTP/AVP 0 8 101\r\n"));
    assert!(output.ends_with("\r\n"));
}

#[test]
fn media_connection_overrides_session_connection() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=video 49172 RTP/AVP 99\r\n",
        "m=audio 49170 RTP/AVP 0\r\n",
        "c=IN IP4 198.51.100.10\r\n"
    ))
    .unwrap();

    assert_eq!(
        session.first_audio_rtp_endpoint().unwrap(),
        RtpEndpoint::new("198.51.100.10", 49170)
    );

    session
        .rewrite_first_audio_rtp_endpoint(RtpEndpoint::new("203.0.113.50", 40002))
        .unwrap();

    let output = session.to_string();
    assert!(output.contains("c=IN IP4 192.0.2.10\r\n"));
    assert!(output.contains("m=audio 40002 RTP/AVP 0\r\n"));
    assert!(output.contains("c=IN IP4 203.0.113.50\r\n"));
}

#[test]
fn inserts_connection_line_when_missing() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0\r\n"
    ))
    .unwrap();

    session
        .rewrite_first_audio_rtp_endpoint(RtpEndpoint::new("203.0.113.50", 40004))
        .unwrap();

    assert!(session
        .to_string()
        .contains("m=audio 40004 RTP/AVP 0\r\nc=IN IP4 203.0.113.50\r\n"));
}

#[test]
fn rejects_invalid_media_port() {
    let error = SessionDescription::parse("m=audio no RTP/AVP 0\r\n").unwrap_err();

    assert_eq!(error, SdpError::InvalidPort("no".to_string()));
}

#[test]
fn reports_missing_audio_rtp_media() {
    let session = SessionDescription::parse("v=0\r\nm=video 49172 RTP/AVP 99\r\n").unwrap();

    assert_eq!(
        session.first_audio_rtp_endpoint().unwrap_err(),
        SdpError::MissingAudioRtpMedia
    );
}

#[test]
fn lists_audio_formats_from_static_payloads_and_rtpmap() {
    let session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP 0 8 101\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n"
    ))
    .unwrap();

    assert_eq!(
        session.first_audio_rtp_formats().unwrap(),
        vec![
            AudioFormat {
                payload_type: "0".to_string(),
                encoding_name: Some("PCMU".to_string()),
                clock_rate: Some(8_000),
            },
            AudioFormat {
                payload_type: "8".to_string(),
                encoding_name: Some("PCMA".to_string()),
                clock_rate: Some(8_000),
            },
            AudioFormat {
                payload_type: "101".to_string(),
                encoding_name: Some("telephone-event".to_string()),
                clock_rate: Some(8_000),
            },
        ]
    );
}

#[test]
fn retains_selected_audio_payloads_and_removes_payload_attributes() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP 0 8 101\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n",
        "a=fmtp:101 0-16\r\n"
    ))
    .unwrap();

    session
        .retain_first_audio_rtp_payloads(&["0".to_string(), "8".to_string()])
        .unwrap();

    let output = session.to_string();
    assert!(output.contains("m=audio 49170 RTP/AVP 0 8\r\n"));
    assert!(output.contains("a=rtpmap:0 PCMU/8000\r\n"));
    assert!(output.contains("a=rtpmap:8 PCMA/8000\r\n"));
    assert!(!output.contains("telephone-event"));
    assert!(!output.contains("a=fmtp:101"));
}

#[test]
fn retaining_unknown_payloads_reports_missing_compatible_codec() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP 101\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n"
    ))
    .unwrap();

    assert_eq!(
        session
            .retain_first_audio_rtp_payloads(&["0".to_string(), "8".to_string()])
            .unwrap_err(),
        SdpError::MissingCompatibleAudioCodec
    );
}

#[test]
fn preserves_custom_attributes_like_direction_and_hold() {
    let mut session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0 8\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=sendonly\r\n"
    ))
    .unwrap();

    session
        .rewrite_first_audio_rtp_endpoint(RtpEndpoint::new("203.0.113.50", 40000))
        .unwrap();

    let output = session.to_string();
    assert!(output.contains("c=IN IP4 203.0.113.50\r\n"));
    assert!(output.contains("m=audio 40000 RTP/AVP 0 8\r\n"));
    assert!(output.contains("a=sendonly\r\n"));
}
