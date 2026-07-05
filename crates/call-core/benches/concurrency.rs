use call_core::{CallManager, Route, RouteTable, RouteTarget};
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use sip_core::{parse_message, SipMessage, SipUri};
use std::sync::Arc;
use std::str::FromStr;

fn test_routes() -> RouteTable {
    RouteTable::new(vec![Route::new(
        "default",
        "",
        100,
        RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
    )])
}

fn invite_request(call_id: &str, destination: &str) -> sip_core::SipRequest {
    let raw = format!(
        concat!(
            "INVITE sip:{destination}@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:{destination}@example.com>\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        call_id = call_id,
        destination = destination
    );

    let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
        panic!("expected request");
    };
    request
}

fn outbound_response(status_code: u16, reason_phrase: &str, call_id: &str) -> sip_core::SipResponse {
    let raw = format!(
        concat!(
            "SIP/2.0 {status_code} {reason_phrase}\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        status_code = status_code,
        reason_phrase = reason_phrase,
        call_id = call_id
    );

    let SipMessage::Response(response) = parse_message(raw.as_bytes()).unwrap() else {
        panic!("expected response");
    };
    response
}

fn bench_handle_inbound_invite(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle_inbound_invite");
    for num_workers in [1, 2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("concurrent", num_workers),
            &num_workers,
            |b, &num_workers| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(num_workers)
                    .enable_all()
                    .build()
                    .unwrap();
                let manager = Arc::new(CallManager::new(test_routes()));
                b.iter_custom(|iters| {
                    let manager = Arc::clone(&manager);
                    let start = std::time::Instant::now();
                    rt.block_on(async {
                        let mut handles = Vec::with_capacity(iters as usize);
                        for i in 0..iters {
                            let m = Arc::clone(&manager);
                            handles.push(tokio::spawn(async move {
                                let req = invite_request(&format!("call-{i}@bench"), "13800138000");
                                m.handle_inbound_invite(&req).unwrap();
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}

fn bench_handle_outbound_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle_outbound_response");
    for num_workers in [1, 2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("concurrent", num_workers),
            &num_workers,
            |b, &num_workers| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(num_workers)
                    .enable_all()
                    .build()
                    .unwrap();
                let manager = Arc::new(CallManager::new(test_routes()));
                b.iter_custom(|iters| {
                    let manager = Arc::clone(&manager);
                    let start = std::time::Instant::now();
                    rt.block_on(async {
                        // Pre-populate calls
                        for i in 0..iters {
                            let req = invite_request(&format!("call-resp-{i}@bench"), "13800138000");
                            manager.handle_inbound_invite(&req).unwrap();
                        }
                        // Now process responses concurrently
                        let mut handles = Vec::with_capacity(iters as usize);
                        for i in 0..iters {
                            let m = Arc::clone(&manager);
                            handles.push(tokio::spawn(async move {
                                let resp = outbound_response(200, "OK", &format!("call-resp-{i}@bench"));
                                m.handle_outbound_response(&resp).unwrap();
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}

fn bench_full_call_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_call_lifecycle");
    for num_workers in [1, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("concurrent", num_workers),
            &num_workers,
            |b, &num_workers| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(num_workers)
                    .enable_all()
                    .build()
                    .unwrap();
                let manager = Arc::new(CallManager::new(test_routes()));
                b.iter_custom(|iters| {
                    let manager = Arc::clone(&manager);
                    let start = std::time::Instant::now();
                    rt.block_on(async {
                        let mut handles = Vec::with_capacity(iters as usize);
                        for i in 0..iters {
                            let m = Arc::clone(&manager);
                            handles.push(tokio::spawn(async move {
                                let req = invite_request(&format!("lifecycle-{i}@bench"), "13800138000");
                                let out = m.handle_inbound_invite(&req).unwrap();
                                // Mark ringing
                                let resp180 = outbound_response(180, "Ringing", &format!("lifecycle-{i}@bench"));
                                m.handle_outbound_response(&resp180).unwrap();
                                // Mark answered
                                let resp200 = outbound_response(200, "OK", &format!("lifecycle-{i}@bench"));
                                m.handle_outbound_response(&resp200).unwrap();
                                // BYE
                                let bye_raw = format!(
                                    concat!(
                                        "BYE sip:13800138000@example.com SIP/2.0\r\n",
                                        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
                                        "From: <sip:1001@example.com>;tag=from-tag\r\n",
                                        "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                                        "Call-ID: lifecycle-{}@bench\r\n",
                                        "CSeq: 2 BYE\r\n",
                                        "Content-Length: 0\r\n",
                                        "\r\n"
                                    ),
                                    i
                                );
                                let SipMessage::Request(bye) = parse_message(bye_raw.as_bytes()).unwrap() else {
                                    panic!()
                                };
                                m.handle_inbound_termination(&bye, None, None).unwrap();
                            }));
                        }
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                    start.elapsed()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_handle_inbound_invite,
    bench_handle_outbound_response,
    bench_full_call_lifecycle,
);
criterion_main!(benches);
