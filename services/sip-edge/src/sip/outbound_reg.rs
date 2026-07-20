use std::sync::Arc;
use std::time::Duration;
use crate::{EdgeConfig, EdgeState};
use crate::edge_state::OutboundRegState;
use tracing::{info, warn, error};
use std::net::SocketAddr;

pub(crate) fn spawn_outbound_registration_loop(
    edge_state: Arc<EdgeState>,
    config: Arc<EdgeConfig>,
) {
    tokio::spawn(async move {
        // 等待几秒以完成系统初始化
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            
            let Some(ref db) = edge_state.db_store else {
                continue;
            };
            
            let gateways = match db.load_outbound_registrations().await {
                Ok(gw) => gw,
                Err(e) => {
                    warn!("无法从数据库加载外呼注册配置: {}", e);
                    continue;
                }
            };
            
            for (id, host, port, transport, _reg_auth_type, username, password) in gateways {
                let port_val = port.unwrap_or(5060);
                
                // 检查是否已有注册状态，没有则新建
                let state_exists = edge_state.outbound_registrations.contains_key(&id);
                if !state_exists {
                    let call_id = format!("reg-{}", uuid::Uuid::new_v4());
                    let from_tag = format!("tag-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                    let reg_state = OutboundRegState {
                        gateway_id: id.clone(),
                        host: host.clone(),
                        port: Some(port_val),
                        transport: transport.clone(),
                        username: username.clone(),
                        password: password.clone(),
                        call_id,
                        cseq: 1,
                        from_tag,
                        expires: 120, // 默认 120 秒
                        last_reg_sent: None,
                        last_reg_success: None,
                        challenge: None,
                    };
                    edge_state.outbound_registrations.insert(id.clone(), reg_state);
                }
                
                // 检查是否需要触发注册请求
                if let Some(mut reg_state) = edge_state.outbound_registrations.get_mut(&id) {
                    let now = std::time::Instant::now();
                    let should_register = match (reg_state.last_reg_sent, reg_state.last_reg_success) {
                        (None, _) => true,
                        (Some(sent), None) => now.duration_since(sent).as_secs() > 15, // 首次发送未响应，15秒后重试
                        (Some(_), Some(success)) => {
                            // 在过期时间过半时触发续期注册
                            now.duration_since(success).as_secs() >= (reg_state.expires / 2) as u64
                        }
                    };
                    
                    if should_register {
                        reg_state.last_reg_sent = Some(now);
                        reg_state.cseq += 1;
                        
                        let request_bytes = build_register_request(&reg_state, &config, None);
                        let target = format!("{}:{}", reg_state.host, port_val);
                        
                        info!(gateway_id = %id, target = %target, cseq = reg_state.cseq, "主动向运营商中继发送 REGISTER 注册包");
                        
                        let edge_state_clone = Arc::clone(&edge_state);
                        tokio::spawn(async move {
                            send_datagram(&edge_state_clone, &target, &request_bytes).await;
                        });
                    }
                }
            }
        }
    });
}

pub(crate) fn build_register_request(
    reg_state: &OutboundRegState,
    config: &EdgeConfig,
    auth_header: Option<&str>,
) -> Vec<u8> {
    let local_contact = if config.advertised_addr.contains(':') {
        config.advertised_addr.clone()
    } else {
        format!("{}:5060", config.advertised_addr)
    };
    
    let mut request = format!(
        "REGISTER sip:{} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {};branch=z9hG4bK-{}\r\n\
         From: <sip:{}@{}>;tag={}\r\n\
         To: <sip:{}@{}>\r\n\
         Call-ID: {}\r\n\
         CSeq: {} REGISTER\r\n\
         Max-Forwards: 70\r\n\
         Contact: <sip:{}@{};transport=udp>\r\n\
         Expires: {}\r\n",
        reg_state.host,
        local_contact,
        &uuid::Uuid::new_v4().to_string()[..8],
        reg_state.username,
        reg_state.host,
        reg_state.from_tag,
        reg_state.username,
        reg_state.host,
        reg_state.call_id,
        reg_state.cseq,
        reg_state.username,
        local_contact,
        reg_state.expires
    );
    
    if let Some(auth) = auth_header {
        request.push_str(auth);
        request.push_str("\r\n");
    }
    
    request.push_str("Content-Length: 0\r\n\r\n");
    request.into_bytes()
}

pub(crate) async fn send_datagram(edge_state: &EdgeState, target: &str, bytes: &[u8]) {
    let target_addr: SocketAddr = if let Ok(addr) = target.parse::<SocketAddr>() {
        addr
    } else {
        match tokio::net::lookup_host(target).await {
            Ok(mut addrs) => {
                if let Some(addr) = addrs.next() {
                    addr
                } else {
                    error!("无法解析目标主机: {}", target);
                    return;
                }
            }
            Err(e) => {
                error!("解析目标主机失败 {}: {}", target, e);
                return;
            }
        }
    };

    if let Some(socket) = edge_state.socket.get() {
        if let Err(e) = socket.send_to(bytes, target_addr).await {
            error!("发送 REGISTER 数据包到 {} 失败: {}", target_addr, e);
        }
    } else {
        error!("EdgeState 未设置 socket，无法发送出站 REGISTER");
    }
}

pub(crate) fn handle_outbound_register_response(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    response: &sip_core::SipResponse,
    call_id: &str,
) -> Vec<crate::edge_state::PendingDatagram> {
    let mut target_gw_id = None;
    for entry in edge_state.outbound_registrations.iter() {
        if entry.value().call_id == call_id {
            target_gw_id = Some(entry.key().clone());
            break;
        }
    }
    
    let Some(gw_id) = target_gw_id else {
        return vec![];
    };
    
    let mut reg_state = match edge_state.outbound_registrations.get_mut(&gw_id) {
        Some(s) => s,
        None => return vec![],
    };
    
    let status = response.status_code;
    if status == 200 {
        info!(gateway_id = %gw_id, "向运营商注册成功 (200 OK)");
        reg_state.last_reg_success = Some(std::time::Instant::now());
        if let Some(exp_hdr) = response.headers.get("expires") {
            if let Ok(exp) = exp_hdr.as_str().parse::<u32>() {
                reg_state.expires = exp;
            }
        }
        return vec![];
    }
    
    if status == 401 || status == 407 {
        let auth_hdr_name = if status == 401 { "www-authenticate" } else { "proxy-authenticate" };
        let auth_hdr_val = match response.headers.get(auth_hdr_name) {
            Some(val) => val.as_str(),
            None => {
                warn!(gateway_id = %gw_id, status = status, "运营商返回挑战但缺少认证标头");
                return vec![];
            }
        };
        
        let auth_params = match crate::sip::auth::parse_digest_authorization(auth_hdr_val) {
            Some(p) => p,
            None => {
                warn!(gateway_id = %gw_id, header = %auth_hdr_val, "无法解析挑战认证参数");
                return vec![];
            }
        };
        
        let realm = auth_params.get("realm").cloned().unwrap_or_default();
        let nonce = auth_params.get("nonce").cloned().unwrap_or_default();
        let opaque = auth_params.get("opaque").cloned();
        let algorithm = auth_params.get("algorithm").cloned().unwrap_or_else(|| "MD5".to_string());
        
        let method = "REGISTER";
        let req_uri = format!("sip:{}", reg_state.host);
        
        let qop_val = auth_params.get("qop").cloned();
        let auth_header = if let Some(qop) = qop_val {
            let cnonce = &uuid::Uuid::new_v4().to_string()[..8].to_string();
            let nc = "00000001";
            let resp = crate::sip::auth::digest_response(
                &reg_state.username,
                &reg_state.password,
                &realm,
                &nonce,
                method,
                &req_uri,
                Some((&qop, nc, cnonce)),
            );
            
            let opaque_str = opaque.map(|o| format!(", opaque=\"{}\"", o)).unwrap_or_default();
            format!(
                "Authorization: Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\", algorithm=\"{}\", qop=\"{}\", nc={}, cnonce=\"{}\"{}",
                reg_state.username, realm, nonce, req_uri, resp, algorithm, qop, nc, cnonce, opaque_str
            )
        } else {
            let resp = crate::sip::auth::digest_response(
                &reg_state.username,
                &reg_state.password,
                &realm,
                &nonce,
                method,
                &req_uri,
                None,
            );
            let opaque_str = opaque.map(|o| format!(", opaque=\"{}\"", o)).unwrap_or_default();
            format!(
                "Authorization: Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\", algorithm=\"{}\"{}",
                reg_state.username, realm, nonce, req_uri, resp, algorithm, opaque_str
            )
        };
        
        reg_state.cseq += 1;
        let request_bytes = build_register_request(&reg_state, edge_config, Some(&auth_header));
        let target = format!("{}:{}", reg_state.host, reg_state.port.unwrap_or(5060));
        
        info!(gateway_id = %gw_id, cseq = reg_state.cseq, "重新发送带有摘要认证凭据的 REGISTER 请求");
        return vec![crate::edge_state::PendingDatagram::new(target, request_bytes)];
    }
    
    warn!(gateway_id = %gw_id, status = status, "运营商拒绝了我们的 REGISTER 注册");
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sip_core::{SipResponse, HeaderMap, HeaderName, HeaderValue};
    use crate::edge_state::OutboundRegState;
    use crate::EdgeConfig;
    use call_core::{CallManager, RouteTable};
    
    #[test]
    fn test_build_register_request() {
        let reg_state = OutboundRegState {
            gateway_id: "test_gw".to_string(),
            host: "sip.operator.com".to_string(),
            port: Some(5060),
            transport: "udp".to_string(),
            username: "user123".to_string(),
            password: "password123".to_string(),
            call_id: "test-call-id-12345".to_string(),
            cseq: 101,
            from_tag: "abcde".to_string(),
            expires: 3600,
            last_reg_sent: None,
            last_reg_success: None,
            challenge: None,
        };
        
        let config = EdgeConfig {
            advertised_addr: "192.168.1.100:5060".to_string(),
            ..EdgeConfig::default()
        };
        
        let bytes = build_register_request(&reg_state, &config, None);
        let request_str = String::from_utf8(bytes).unwrap();
        
        assert!(request_str.contains("REGISTER sip:sip.operator.com SIP/2.0"));
        assert!(request_str.contains("From: <sip:user123@sip.operator.com>;tag=abcde"));
        assert!(request_str.contains("To: <sip:user123@sip.operator.com>"));
        assert!(request_str.contains("Call-ID: test-call-id-12345"));
        assert!(request_str.contains("CSeq: 101 REGISTER"));
        assert!(request_str.contains("Expires: 3600"));
        assert!(request_str.contains("Contact: <sip:user123@192.168.1.100:5060;transport=udp>"));
    }
    
    #[tokio::test]
    async fn test_handle_outbound_register_response_200_ok() {
        let (tx, _rx) = tokio::sync::mpsc::channel(100);
        let call_manager = CallManager::new(RouteTable::default(), tx);
        let edge_state = EdgeState::new(call_manager);
        
        let reg_state = OutboundRegState {
            gateway_id: "test_gw".to_string(),
            host: "sip.operator.com".to_string(),
            port: Some(5060),
            transport: "udp".to_string(),
            username: "user123".to_string(),
            password: "password123".to_string(),
            call_id: "test-call-id-12345".to_string(),
            cseq: 101,
            from_tag: "abcde".to_string(),
            expires: 3600,
            last_reg_sent: None,
            last_reg_success: None,
            challenge: None,
        };
        
        edge_state.outbound_registrations.insert("test_gw".to_string(), reg_state);
        
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::new("expires").unwrap(),
            HeaderValue::new("1800")
        );
        
        let response = SipResponse {
            version: std::borrow::Cow::Borrowed("SIP/2.0"),
            status_code: 200,
            reason_phrase: std::borrow::Cow::Borrowed("OK"),
            headers,
            body: std::borrow::Cow::Borrowed(&[]),
        };
        
        let pending = handle_outbound_register_response(&edge_state, &EdgeConfig::default(), &response, "test-call-id-12345");
        
        assert!(pending.is_empty());
        let updated_state = edge_state.outbound_registrations.get("test_gw").unwrap();
        assert_eq!(updated_state.expires, 1800);
        assert!(updated_state.last_reg_success.is_some());
    }

    #[tokio::test]
    async fn test_handle_outbound_register_response_401_challenge() {
        let (tx, _rx) = tokio::sync::mpsc::channel(100);
        let call_manager = CallManager::new(RouteTable::default(), tx);
        let edge_state = EdgeState::new(call_manager);
        
        let reg_state = OutboundRegState {
            gateway_id: "test_gw".to_string(),
            host: "sip.operator.com".to_string(),
            port: Some(5060),
            transport: "udp".to_string(),
            username: "user123".to_string(),
            password: "password123".to_string(),
            call_id: "test-call-id-12345".to_string(),
            cseq: 101,
            from_tag: "abcde".to_string(),
            expires: 3600,
            last_reg_sent: None,
            last_reg_success: None,
            challenge: None,
        };
        
        edge_state.outbound_registrations.insert("test_gw".to_string(), reg_state);
        
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::new("www-authenticate").unwrap(),
            HeaderValue::new("Digest realm=\"sip.operator.com\", nonce=\"1a2b3c4d\", algorithm=MD5")
        );
        
        let response = SipResponse {
            version: std::borrow::Cow::Borrowed("SIP/2.0"),
            status_code: 401,
            reason_phrase: std::borrow::Cow::Borrowed("Unauthorized"),
            headers,
            body: std::borrow::Cow::Borrowed(&[]),
        };
        
        let pending = handle_outbound_register_response(&edge_state, &EdgeConfig::default(), &response, "test-call-id-12345");
        
        assert_eq!(pending.len(), 1);
        let pending_dg = &pending[0];
        assert_eq!(pending_dg.target, "sip.operator.com:5060");
        
        let request_str = String::from_utf8(pending_dg.bytes.clone()).unwrap();
        assert!(request_str.contains("REGISTER sip:sip.operator.com SIP/2.0"));
        assert!(request_str.contains("Authorization: Digest"));
        assert!(request_str.contains("username=\"user123\""));
        assert!(request_str.contains("realm=\"sip.operator.com\""));
        assert!(request_str.contains("nonce=\"1a2b3c4d\""));
        assert!(request_str.contains("response=\""));
    }
}
