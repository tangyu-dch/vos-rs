use sdp_core::{
    AudioFormat, DtlsFingerprint, DtlsParameters, IceCandidate, IceParameters, RtpEndpoint,
    SdpError, SessionDescription, SrtpCryptoAttribute,
};

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
fn parses_audio_sdes_srtp_crypto_attributes() {
    let session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/SAVP 0\r\n",
        "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:dGVzdA==|2^31|1:32\r\n",
        "m=video 49172 RTP/AVP 99\r\n",
        "a=crypto:2 AES_CM_128_HMAC_SHA1_80 inline:dmlkZW8=\r\n"
    ))
    .unwrap();

    assert_eq!(
        session.first_audio_srtp_crypto().unwrap(),
        vec![SrtpCryptoAttribute {
            tag: 1,
            suite: "AES_CM_128_HMAC_SHA1_80".to_string(),
            key_params: "inline:dGVzdA==|2^31|1:32".to_string(),
            session_params: None,
        }]
    );
}

#[test]
fn parses_audio_ice_and_dtls_parameters() {
    let session = SessionDescription::parse(concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP 0\r\n",
        "a=ice-ufrag:offer-ufrag\r\n",
        "a=ice-pwd:offer-password\r\n",
        "a=ice-options:trickle renomination\r\n",
        "a=candidate:1 1 UDP 2130706431 192.0.2.10 49170 typ host\r\n",
        "a=candidate:2 1 UDP 1694498815 198.51.100.10 62000 typ srflx raddr 192.0.2.10 rport 49170\r\n",
        "a=end-of-candidates\r\n",
        "a=fingerprint:sha-256 aa:bb:cc\r\n",
        "a=setup:actpass\r\n"
    ))
    .unwrap();

    assert_eq!(
        session.first_audio_ice_parameters().unwrap(),
        IceParameters {
            username_fragment: Some("offer-ufrag".to_string()),
            password: Some("offer-password".to_string()),
            options: vec!["trickle".to_string(), "renomination".to_string()],
            candidates: vec![
                IceCandidate {
                    foundation: "1".to_string(),
                    component: 1,
                    transport: "udp".to_string(),
                    priority: 2_130_706_431,
                    address: "192.0.2.10".to_string(),
                    port: 49170,
                    candidate_type: "host".to_string(),
                    related_address: None,
                    related_port: None,
                    tcp_type: None,
                },
                IceCandidate {
                    foundation: "2".to_string(),
                    component: 1,
                    transport: "udp".to_string(),
                    priority: 1_694_498_815,
                    address: "198.51.100.10".to_string(),
                    port: 62000,
                    candidate_type: "srflx".to_string(),
                    related_address: Some("192.0.2.10".to_string()),
                    related_port: Some(49170),
                    tcp_type: None,
                },
            ],
            end_of_candidates: true,
        }
    );
    assert_eq!(
        session.first_audio_dtls_parameters().unwrap(),
        DtlsParameters {
            fingerprint: Some(DtlsFingerprint {
                algorithm: "sha-256".to_string(),
                value: "AA:BB:CC".to_string(),
            }),
            setup: Some("actpass".to_string()),
        }
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
