use sip_core::{parse_message, Method, SipMessage};

#[test]
fn parses_invite_request() {
    let raw = concat!(
        "INVITE sip:13800138000@gateway.example.com:5060;transport=udp SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
        "From: \"1001\" <sip:1001@example.com>;tag=abc\r\n",
        "To: <sip:13800138000@gateway.example.com>\r\n",
        "Call-ID: call-1@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Contact: <sip:1001@192.0.2.10:5060>\r\n",
        "Content-Type: application/sdp\r\n",
        "Content-Length: 10\r\n",
        "\r\n",
        "v=0\r\n",
        "s=-\r\n"
    );

    let message = parse_message(raw.as_bytes()).expect("request should parse");

    let SipMessage::Request(request) = message else {
        panic!("expected request");
    };

    assert_eq!(request.method, Method::Invite);
    assert_eq!(request.uri.user.as_deref(), Some("13800138000"));
    assert_eq!(request.uri.host, "gateway.example.com");
    assert_eq!(request.uri.port, Some(5060));
    assert_eq!(
        request.headers.get("call-id").map(|value| value.as_str()),
        Some("call-1@example.com")
    );
    assert_eq!(request.body, b"v=0\r\ns=-\r\n");
}

#[test]
fn parses_response() {
    let raw = concat!(
        "SIP/2.0 200 OK\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
        "Call-ID: call-1@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    );

    let message = parse_message(raw.as_bytes()).expect("response should parse");

    let SipMessage::Response(response) = message else {
        panic!("expected response");
    };

    assert_eq!(response.status_code, 200);
    assert_eq!(response.reason_phrase, "OK");
    assert_eq!(
        response.headers.get("CSeq").map(|value| value.as_str()),
        Some("1 INVITE")
    );
    assert!(response.body.is_empty());
}

#[test]
fn preserves_repeated_headers() {
    let raw = concat!(
        "OPTIONS sip:example.com SIP/2.0\r\n",
        "Route: <sip:a.example.com;lr>\r\n",
        "Route: <sip:b.example.com;lr>\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    );

    let message = parse_message(raw.as_bytes()).expect("message should parse");
    let values = message
        .headers()
        .get_all("route")
        .map(|value| value.as_str().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        values,
        vec![
            "<sip:a.example.com;lr>".to_string(),
            "<sip:b.example.com;lr>".to_string()
        ]
    );
}

#[test]
fn unfolds_header_continuations() {
    let raw = concat!(
        "MESSAGE sip:1002@example.com SIP/2.0\r\n",
        "Subject: first line\r\n",
        " second line\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    );

    let message = parse_message(raw.as_bytes()).expect("message should parse");

    assert_eq!(
        message.headers().get("subject").map(|value| value.as_str()),
        Some("first line second line")
    );
}

#[test]
fn resolves_compact_header_names() {
    let raw = concat!(
        "INVITE sip:1002@example.com SIP/2.0\r\n",
        "v: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
        "f: <sip:1001@example.com>;tag=abc\r\n",
        "t: <sip:1002@example.com>\r\n",
        "i: compact-call@example.com\r\n",
        "c: application/sdp\r\n",
        "l: 5\r\n",
        "\r\n",
        "v=0\r\n"
    );

    let message = parse_message(raw.as_bytes()).expect("message should parse");

    assert_eq!(
        message.headers().get("via").map(|value| value.as_str()),
        Some("SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1")
    );
    assert_eq!(
        message.headers().get("call-id").map(|value| value.as_str()),
        Some("compact-call@example.com")
    );
    assert_eq!(
        message
            .headers()
            .get("content-type")
            .map(|value| value.as_str()),
        Some("application/sdp")
    );
    assert_eq!(message.body(), b"v=0\r\n");
}

#[test]
fn content_length_truncates_extra_datagram_bytes() {
    let raw = concat!(
        "MESSAGE sip:1002@example.com SIP/2.0\r\n",
        "Content-Length: 5\r\n",
        "\r\n",
        "helloignored"
    );

    let message = parse_message(raw.as_bytes()).expect("message should parse");

    assert_eq!(message.body(), b"hello");
}

#[test]
fn content_length_rejects_short_body() {
    let raw = concat!(
        "MESSAGE sip:1002@example.com SIP/2.0\r\n",
        "Content-Length: 6\r\n",
        "\r\n",
        "hello"
    );

    let error = parse_message(raw.as_bytes()).expect_err("message should fail");

    assert!(error.to_string().contains("invalid SIP Content-Length"));
}
