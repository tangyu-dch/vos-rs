use rtp_core::{AudioCodec, StaticAudioPayload};

#[test]
fn maps_static_payload_types_to_g711_codecs() {
    assert_eq!(
        StaticAudioPayload::from_payload_type(0).unwrap().codec,
        AudioCodec::Pcmu
    );
    assert_eq!(
        StaticAudioPayload::from_payload_type(8).unwrap().codec,
        AudioCodec::Pcma
    );
    assert!(StaticAudioPayload::from_payload_type(101).is_none());
}

#[test]
fn maps_rtpmap_names_to_g711_codecs() {
    assert_eq!(
        AudioCodec::from_rtpmap("PCMU", 8_000),
        Some(AudioCodec::Pcmu)
    );
    assert_eq!(
        AudioCodec::from_rtpmap("pcma", 8_000),
        Some(AudioCodec::Pcma)
    );
    assert_eq!(AudioCodec::from_rtpmap("PCMU", 16_000), None);
}
