    #[tokio::test]
    async fn test_nat_traversal_registered_contact_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Receive an inbound INVITE to 1001
        let call_id = "invite-nat-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-invite-nat\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to the public NAT address of client 1001, NOT the private Contact IP!
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "192.0.2.10:5070");
        let forwarded_msg = datagram_text(&datagrams[1]);
        assert!(forwarded_msg
            .starts_with("INVITE sip:1001@192.168.1.100:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_traversal_in_dialog_callee_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private Contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 198.51.100.20:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.100.200:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Establish initial call: inbound INVITE from caller 1002 to registered contact 1001
        let call_id = "nat-indialog-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to callee 1001 at public NAT address "198.51.100.20:5070"
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "198.51.100.20:5070");

        // 3. Callee 1001 responds 200 OK from public NAT address 198.51.100.20:5070
        let ok_body = sdp_body();
        let ok_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>;tag=callee-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:1001@192.168.100.200:5060>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            ok_body.len(),
            ok_body,
            call_id = call_id
        );

        let _ = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify outbound_peer NAT target and callee_behind_nat flag are registered
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            assert_eq!(tx.outbound_peer.as_deref(), Some("198.51.100.20:5070"));
            assert!(tx.callee_behind_nat);
        }

        // 4. Caller sends BYE to callee
        let bye = format!(
            "BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-2\r\n\
             From: <sip:1002@example.com>;tag=caller-tag\r\n\
             To: <sip:1001@example.com>;tag=callee-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let bye_datagrams = handle_datagram(
            bye.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify BYE is routed to the public source socket address of the callee (198.51.100.20:5070), NOT the private Contact IP!
        assert_eq!(bye_datagrams.len(), 2);
        assert_eq!(bye_datagrams[1].target, "198.51.100.20:5070");
        let forwarded_bye = datagram_text(&bye_datagrams[1]);
        assert!(forwarded_bye.starts_with("BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_keepalive_background_loop() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        edge_state.set_socket(Arc::clone(&socket));

        let local_addr = socket.local_addr().unwrap();

        // 1. Register a contact pointing to local receiver port so we can capture the keepalive datagram
        let register = format!(
            "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP {addr};branch=z9hG4bK-regka\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-ka-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n",
            addr = local_addr
        );
        let _ = handle_datagram(register.as_bytes(), local_addr, &edge_state, &edge_config()).await;

        // Discard the 200 OK registration response from the socket buffer
        let mut resp_buf = [0u8; 1000];
        let (resp_size, _) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut resp_buf))
                .await
                .expect("timeout waiting for 200 OK registration response")
                .unwrap();
        assert!(std::str::from_utf8(&resp_buf[..resp_size])
            .unwrap()
            .starts_with("SIP/2.0 200 OK\r\n"));

        // 2. Start the NAT keepalive loop
        spawn_nat_keepalive_loop(Arc::clone(&edge_state), Arc::clone(&socket));

        // 3. Receive the NAT keepalive packet
        let mut buffer = [0u8; 100];
        let (size, src) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buffer))
                .await
                .expect("timeout waiting for keepalive probe")
                .unwrap();

        // Verify the keepalive probe matches single CRLF "\r\n"
        assert_eq!(&buffer[..size], b"\r\n");
        assert_eq!(src, local_addr);
    }

    #[tokio::test]
    async fn test_websocket_transport() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let edge_state = Arc::new(state_with_default_route());

        // Start WS listener on random port
        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let edge_state_clone = Arc::clone(&edge_state);
        tokio::spawn(async move {
            let (stream, peer) = ws_listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(100);
            edge_state_clone.register_tcp_connection(peer, tx.clone());

            let on_msg_state = Arc::clone(&edge_state_clone);
            handle_ws_connection(
                ws_stream,
                peer,
                tx,
                rx,
                move |msg_bytes: Vec<u8>,
                      peer_addr: SocketAddr,
                      connection_tx: tokio::sync::mpsc::Sender<Vec<u8>>| {
                    let state = Arc::clone(&on_msg_state);
                    async move {
                        let datagrams =
                            handle_datagram(&msg_bytes, peer_addr, &state, &edge_config()).await;
                        for d in datagrams {
                            let _ = connection_tx.send(d.bytes).await;
                        }
                    }
                },
            )
            .await;
        });

        // Connect client
        let (mut client_ws, _) = connect_async(format!("ws://{}", ws_addr)).await.unwrap();

        // Send REGISTER request over WS
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/WS 127.0.0.1:5062;branch=z9hG4bK-ws-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: ws-reg-001\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@127.0.0.1:5062;transport=ws>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        client_ws
            .send(WsMessage::Text(register.to_string()))
            .await
            .unwrap();

        // Receive response over WS
        let msg = tokio::time::timeout(Duration::from_millis(1000), client_ws.next())
            .await
            .expect("timeout waiting for WS response")
            .unwrap()
            .unwrap();

        let resp_text = match msg {
            WsMessage::Text(t) => t,
            _ => panic!("expected text frame"),
        };

        assert!(resp_text.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(resp_text.contains("Call-ID: ws-reg-001\r\n"));
    }

    #[tokio::test]
    async fn test_tcp_stream_framing() {
        use crate::transport::read_frame;

        // Case 1: Complete single message
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
        );
        assert!(buf.is_empty());

        // Case 2: Compact length header l
        let mut buf = b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(frame, b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec());
        assert!(buf.is_empty());

        // Case 3: Partial message (header not complete)
        let mut buf = b"SIP/2.0 200 OK\r\nContent-L".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 25);

        // Case 4: Header complete but body incomplete
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 42);

        // Feed rest of body
        buf.extend_from_slice(b" rest1");
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody rest1".to_vec()
        );
        assert!(buf.is_empty());

        // Case 5: Multiple concatenated messages
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec();
        let frame1 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame1,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123".to_vec()
        );
        let frame2 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame2,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec()
        );
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn test_tcp_tls_transport_dispatch_and_reuse() {
        use tokio::io::AsyncReadExt;

        let edge_state = Arc::new(state_with_default_route());
        let edge_config = edge_config();

        // 1. Setup local TCP listener
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();

        // Spawn local test TCP server task
        let (server_tx, mut server_rx) = tokio::sync::mpsc::channel(10);
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = server_tx.send(buf[..n].to_vec()).await;
                }
            }
        });

        // Send a mock SIP request targeting this server using TCP transport (Via has SIP/2.0/TCP)
        let request_bytes = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-001\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram = PendingDatagram::new(listen_addr.to_string(), request_bytes.clone());
        let dummy_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let res = edge_state
            .send_sip_datagram(datagram, &dummy_udp, &edge_config)
            .await;
        assert!(res.is_ok());

        // Verify message received by the server
        let received = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, request_bytes);

        // Check that the connection was registered in the connection pool
        {
            assert!(edge_state.tcp_connections.contains_key(&listen_addr));
        }

        // Send a second message to check connection reuse
        let request_bytes2 = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-002\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram2 = PendingDatagram::new(listen_addr.to_string(), request_bytes2.clone());
        let res2 = edge_state
            .send_sip_datagram(datagram2, &dummy_udp, &edge_config)
            .await;
        assert!(res2.is_ok());

        let received2 = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received2, request_bytes2);
    }
