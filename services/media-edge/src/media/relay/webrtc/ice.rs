use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::net::SocketAddr;
use stun::{
    attributes::ATTR_USERNAME,
    fingerprint::FINGERPRINT,
    integrity::MessageIntegrity,
    message::{Message, Setter, BINDING_REQUEST, BINDING_SUCCESS},
    textattrs::TextAttribute,
    xoraddr::XorMappedAddress,
};

/// ICE-Lite 会话凭据。
#[derive(Debug, Clone, serde::Serialize)]
pub struct IceCredentials {
    pub username_fragment: String,
    pub password: String,
}

impl IceCredentials {
    /// 生成符合浏览器 WebRTC 要求的随机 ICE 凭据。
    pub fn generate() -> Self {
        Self {
            username_fragment: random_alphanumeric(16),
            password: random_alphanumeric(32),
        }
    }
}

fn random_alphanumeric(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub(super) fn binding_success_response(
    packet: &[u8],
    source: SocketAddr,
    credentials: &IceCredentials,
) -> Result<Vec<u8>, String> {
    let mut request = Message::new();
    request.raw.clear();
    request.raw.extend_from_slice(packet);
    request.decode().map_err(|error| error.to_string())?;
    if request.typ != BINDING_REQUEST {
        return Err("仅接受 STUN Binding Request".to_string());
    }

    FINGERPRINT
        .check(&request)
        .map_err(|error| format!("STUN FINGERPRINT 校验失败: {error}"))?;
    let username = TextAttribute::get_from_as(&request, ATTR_USERNAME)
        .map_err(|error| format!("STUN USERNAME 缺失: {error}"))?;
    if username.text.split(':').next() != Some(&credentials.username_fragment) {
        return Err("STUN USERNAME 与本地 ICE ufrag 不匹配".to_string());
    }

    let integrity = MessageIntegrity::new_short_term_integrity(credentials.password.clone());
    integrity
        .check(&mut request)
        .map_err(|error| format!("STUN MESSAGE-INTEGRITY 校验失败: {error}"))?;

    let mut response = Message::new();
    response.typ = BINDING_SUCCESS;
    response.transaction_id = request.transaction_id;
    response.write_header();
    XorMappedAddress {
        ip: source.ip(),
        port: source.port(),
    }
    .add_to(&mut response)
    .map_err(|error| error.to_string())?;
    integrity
        .add_to(&mut response)
        .map_err(|error| error.to_string())?;
    FINGERPRINT
        .add_to(&mut response)
        .map_err(|error| error.to_string())?;
    Ok(response.raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stun::{agent::TransactionId, message::Getter, xoraddr::XorMappedAddress};

    #[test]
    fn binding_response_is_authenticated_and_contains_mapped_address() {
        let credentials = IceCredentials {
            username_fragment: "server".to_string(),
            password: "server-password".to_string(),
        };
        let integrity = MessageIntegrity::new_short_term_integrity(credentials.password.clone());
        let mut request = Message::new();
        request.typ = BINDING_REQUEST;
        request.transaction_id = TransactionId::new();
        request.write_header();
        TextAttribute::new(
            ATTR_USERNAME,
            format!("{}:browser", credentials.username_fragment),
        )
        .add_to(&mut request)
        .unwrap();
        integrity.add_to(&mut request).unwrap();
        FINGERPRINT.add_to(&mut request).unwrap();

        let source: SocketAddr = "192.0.2.8:49152".parse().unwrap();
        let raw = binding_success_response(&request.raw, source, &credentials).unwrap();
        let mut response = Message::new();
        response.raw = raw;
        response.decode().unwrap();
        integrity.check(&mut response).unwrap();
        FINGERPRINT.check(&response).unwrap();

        let mut mapped = XorMappedAddress::default();
        mapped.get_from(&response).unwrap();
        assert_eq!(mapped.ip, source.ip());
        assert_eq!(mapped.port, source.port());
    }
}
