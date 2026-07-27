use call_core::VciInstruction;

#[test]
fn test_vci_instruction_parsing() {
    let say_json = r#"{
        "action": "say",
        "text": "Hello world",
        "voice": "zh-CN",
        "speed": 1.0,
        "pitch": 0
    }"#;
    let inst: VciInstruction = serde_json::from_str(say_json).unwrap();
    assert!(matches!(
        inst,
        VciInstruction::Say { text, voice, speed: _, pitch: _ } if text == "Hello world" && voice == "zh-CN"
    ));

    let hangup_json = r#"{
        "action": "hangup",
        "reason_code": 16,
        "sip_cause": 200
    }"#;
    let inst: VciInstruction = serde_json::from_str(hangup_json).unwrap();
    assert!(matches!(
        inst,
        VciInstruction::Hangup {
            reason_code: 16,
            sip_cause: Some(200)
        }
    ));

    let redirect_json = r#"{
        "action": "redirect",
        "url": "http://new-url/webhook"
    }"#;
    let inst: VciInstruction = serde_json::from_str(redirect_json).unwrap();
    assert!(matches!(
        inst,
        VciInstruction::Redirect { url } if url == "http://new-url/webhook"
    ));

    let stream_json = r#"{
        "action": "stream",
        "websocket_url": "wss://audio-server/stream",
        "format": "pcm",
        "barge_in": false
    }"#;
    let inst: VciInstruction = serde_json::from_str(stream_json).unwrap();
    assert!(matches!(
        inst,
        VciInstruction::Stream { websocket_url, format, barge_in: false } if websocket_url == "wss://audio-server/stream" && format == "pcm"
    ));
}
