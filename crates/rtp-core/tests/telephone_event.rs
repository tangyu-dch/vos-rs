use rtp_core::{RtpError, TelephoneEvent};

#[test]
fn parses_rfc4733_telephone_event_payload() {
    let event = TelephoneEvent::parse(&[5, 0x8a, 0x01, 0x40]).unwrap();

    assert_eq!(event.event, 5);
    assert_eq!(event.digit(), Some('5'));
    assert!(event.end);
    assert!(!event.reserved);
    assert_eq!(event.volume, 10);
    assert_eq!(event.duration, 320);
}

#[test]
fn maps_named_dtmf_events_to_digits() {
    assert_eq!(
        TelephoneEvent::parse(&[10, 0, 0, 80]).unwrap().digit(),
        Some('*')
    );
    assert_eq!(
        TelephoneEvent::parse(&[11, 0, 0, 80]).unwrap().digit(),
        Some('#')
    );
    assert_eq!(
        TelephoneEvent::parse(&[12, 0, 0, 80]).unwrap().digit(),
        Some('A')
    );
    assert_eq!(
        TelephoneEvent::parse(&[15, 0, 0, 80]).unwrap().digit(),
        Some('D')
    );
    assert_eq!(
        TelephoneEvent::parse(&[16, 0, 0, 80]).unwrap().digit(),
        None
    );
}

#[test]
fn rejects_short_telephone_event_payload() {
    let error = TelephoneEvent::parse(&[1, 0, 0]).unwrap_err();

    assert_eq!(error, RtpError::TelephoneEventPayloadTooShort);
}
