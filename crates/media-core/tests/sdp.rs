use media_core::sdp::{
    negotiated_audio_codec, parse_sdp_dtmf_payload_type, parse_sdp_rtp_endpoint,
    rewrite_sdp_and_extract_endpoint, rewrite_sdp_body, validate_media_negotiation,
    MediaNegotiationPolicy, MediaSdpError,
};
use rtp_core::AudioCodec;
use sdp_core::{RtpEndpoint, SdpError};

const AUDIO_SDP: &str = concat!(
    "v=0\r\n",
    "o=- 1 1 IN IP4 192.0.2.10\r\n",
    "s=-\r\n",
    "c=IN IP4 192.0.2.10\r\n",
    "t=0 0\r\n",
    "m=audio 49170 RTP/AVP 111 0 101\r\n",
    "a=rtpmap:111 opus/48000/2\r\n",
    "a=rtpmap:0 PCMU/8000\r\n",
    "a=rtpmap:101 telephone-event/8000\r\n",
);

#[test]
fn validation_policy_preserves_t38_service_difference() {
    let t38 = b"v=0\r\nm=image 4000 udptl t38\r\n";

    assert!(validate_media_negotiation(t38, MediaNegotiationPolicy::AUDIO_OR_T38).is_ok());
    assert_eq!(
        validate_media_negotiation(t38, MediaNegotiationPolicy::AUDIO_ONLY).unwrap_err(),
        MediaSdpError::Sdp(SdpError::MissingCompatibleAudioCodec)
    );
}

#[test]
fn invalid_utf8_is_reported_by_result_apis() {
    assert_eq!(
        validate_media_negotiation(&[0xff], MediaNegotiationPolicy::AUDIO_ONLY).unwrap_err(),
        MediaSdpError::InvalidUtf8
    );
    assert_eq!(
        parse_sdp_rtp_endpoint(&[0xff]).unwrap_err(),
        MediaSdpError::InvalidUtf8
    );
}

#[test]
fn parses_endpoint_dtmf_and_first_supported_codec() {
    assert_eq!(
        parse_sdp_rtp_endpoint(AUDIO_SDP.as_bytes()).unwrap(),
        RtpEndpoint::new("192.0.2.10", 49_170)
    );
    assert_eq!(parse_sdp_dtmf_payload_type(AUDIO_SDP.as_bytes()), Some(101));
    assert_eq!(
        negotiated_audio_codec(AUDIO_SDP.as_bytes()),
        Some(AudioCodec::Opus)
    );
}

#[test]
fn invalid_first_dtmf_payload_does_not_fall_through_to_a_later_match() {
    let duplicate_dtmf = concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP invalid 101\r\n",
        "a=rtpmap:invalid telephone-event/8000\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n",
    );

    assert_eq!(parse_sdp_dtmf_payload_type(duplicate_dtmf.as_bytes()), None);
}

#[test]
fn rewrites_and_returns_the_original_endpoint() {
    let relay = RtpEndpoint::new("203.0.113.10", 40_000);
    let (rewritten, original) =
        rewrite_sdp_and_extract_endpoint(AUDIO_SDP.as_bytes(), &relay).unwrap();
    let rewritten = String::from_utf8(rewritten).unwrap();

    assert_eq!(original, RtpEndpoint::new("192.0.2.10", 49_170));
    assert!(rewritten.contains("c=IN IP4 203.0.113.10\r\n"));
    assert!(rewritten.contains("m=audio 40000 RTP/AVP 111 0 101\r\n"));
}

#[test]
fn rewrite_rejects_an_offer_without_a_voice_codec() {
    let dtmf_only = concat!(
        "v=0\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "m=audio 49170 RTP/AVP 101\r\n",
        "a=rtpmap:101 telephone-event/8000\r\n",
    );

    assert_eq!(
        rewrite_sdp_body(
            dtmf_only.as_bytes(),
            RtpEndpoint::new("203.0.113.10", 40_000)
        )
        .unwrap_err(),
        MediaSdpError::Sdp(SdpError::MissingCompatibleAudioCodec)
    );
}

#[test]
fn rewrite_supports_an_ipv6_relay_address() {
    let rewritten = rewrite_sdp_body(
        AUDIO_SDP.as_bytes(),
        RtpEndpoint::new("2001:db8::10", 40_002),
    )
    .unwrap();
    let rewritten = String::from_utf8(rewritten).unwrap();

    assert!(rewritten.contains("c=IN IP6 2001:db8::10\r\n"));
    assert!(rewritten.contains("m=audio 40002 RTP/AVP 111 0 101\r\n"));
}

#[test]
fn rewrite_fallback_supports_static_payload_without_rtpmap() {
    let static_payload = concat!(
        "v=0\r\n",
        "o=- 1 1 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 0\r\n",
    );

    let rewritten = rewrite_sdp_body(
        static_payload.as_bytes(),
        RtpEndpoint::new("203.0.113.10", 40_004),
    )
    .unwrap();
    let rewritten = String::from_utf8(rewritten).unwrap();

    assert!(rewritten.contains("c=IN IP4 203.0.113.10\r\n"));
    assert!(rewritten.contains("m=audio 40004 RTP/AVP 0\r\n"));
}
