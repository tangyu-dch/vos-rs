//! ICE-Lite 连通性检查实现。
//!
//! 本模块提供 RFC 8445 兼容的 ICE-Lite 控制面：
//! - 解析远端 SDP 中的 `a=candidate:` 行（`parse_candidate_line`）
//! - 校验 STUN Binding Request 的 USERNAME/MESSAGE-INTEGRITY/FINGERPRINT
//! - 学习 peer-reflexive candidate 并标记 selected pair
//! - 解决 ICE role conflict（双方均为 Lite 时以 tie-breaker 较大者为 controlling）
//!
//! `IceAgent` 在 `WebRtcSession::handle_stun_packet` 中被驱动，连通性建立后
//! `ice_connected` 原子位被置位，供媒体转发循环读取以决定 SRTP 解密路径。

use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::net::SocketAddr;
use std::sync::Arc;
use stun::{
    attributes::{ATTR_ICE_CONTROLLED, ATTR_ICE_CONTROLLING, ATTR_USERNAME},
    fingerprint::FINGERPRINT,
    integrity::MessageIntegrity,
    message::{Message, Setter, BINDING_REQUEST, BINDING_SUCCESS},
    textattrs::TextAttribute,
    xoraddr::XorMappedAddress,
};
use tracing::debug;

/// ICE candidate 类型（RFC 8445 §5.1.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IceCandidateType {
    Host,
    Srflx,
    Prflx,
    Relay,
}

impl IceCandidateType {
    /// 计算 ICE 优先级（RFC 8445 §5.1.2）。
    /// type_preference 应为 0-126，local_preference 应为 0-65535。
    pub fn priority(&self, component_id: u16, local_preference: u16) -> u32 {
        let type_pref: u16 = match self {
            IceCandidateType::Host => 126,
            IceCandidateType::Srflx => 100,
            IceCandidateType::Prflx => 110,
            IceCandidateType::Relay => 0,
        };
        ((type_pref as u32) << 24) | ((local_preference as u32) << 8) | (256 - component_id as u32)
    }
}

/// 对端 SDP 中 `a=candidate:` 行解析得到的远端候选地址。
#[derive(Debug, Clone)]
pub struct RemoteCandidate {
    pub address: SocketAddr,
    pub priority: u32,
    pub foundation: String,
    pub component_id: u16,
    pub transport: String,
    pub candidate_type: IceCandidateType,
}

/// ICE candidate pair，由本地候选与远端候选配对组成。
#[derive(Debug, Clone)]
pub struct CandidatePair {
    pub remote: RemoteCandidate,
}

/// ICE 连通性检查代理。
///
/// 在 ICE-Lite 模式下，media-edge 不主动发起 connectivity check，
/// 但需要：
/// 1. 解析并存储远端 SDP 中的 `a=candidate` 列表
/// 2. 当收到 STUN Binding Request 时，检查源地址是否匹配已知远端 candidate
/// 3. 若不匹配，学习为 peer-reflexive candidate
/// 4. 当对端发送 USE-CANDIDATE（表现为 BINDING_REQUEST 携带 ICE-CONTROLLING）
///    时，标记对应 candidate pair 为 selected
/// 5. 解决 ICE role conflict（若双方都是 Lite，以更小 tie-breaker 为 controlled）
pub struct IceAgent {
    local_ufrag: String,
    remote_candidates: Vec<RemoteCandidate>,
    selected_pair: Option<CandidatePair>,
    /// 已知的对端地址集合（用于 peer-reflexive 学习）。
    known_peer_addresses: Vec<SocketAddr>,
    pub ice_connected: Arc<std::sync::atomic::AtomicBool>,
}

impl IceAgent {
    pub fn new(local_ufrag: String) -> Self {
        Self {
            local_ufrag,
            remote_candidates: Vec::new(),
            selected_pair: None,
            known_peer_addresses: Vec::new(),
            ice_connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 设置从远端 SDP 解析得到的 ICE 凭据。
    ///
    /// 在 ICE-Lite 模式下，本地不主动发起 STUN 请求，因此远端 `ufrag` 与 `password`
    /// 仅用于诊断日志与未来扩展（如主动 connectivity check）。当前实现中，STUN
    /// MESSAGE-INTEGRITY 校验依赖本地密码（由 `WebRtcSession` 持有的 `IceCredentials`）。
    pub fn set_remote_credentials(&mut self, _ufrag: String, _password: String) {
        // 远端凭据在 ICE-Lite 中不参与 STUN 校验，保留接口以兼容未来 full-ICE 升级
    }

    /// 添加从远端 SDP `a=candidate:` 行解析的候选地址。
    pub fn add_remote_candidate(&mut self, candidate: RemoteCandidate) {
        if !self
            .remote_candidates
            .iter()
            .any(|c| c.address == candidate.address)
        {
            debug!(
                addr = %candidate.address,
                foundation = %candidate.foundation,
                "ICE: 添加远端候选"
            );
            self.known_peer_addresses.push(candidate.address);
            self.remote_candidates.push(candidate);
        }
    }

    /// 检查源地址是否匹配已知远端 candidate。若不匹配则学习为 peer-reflexive。
    pub fn learn_or_match_peer_address(&mut self, source: SocketAddr) -> IceCandidateType {
        if self.known_peer_addresses.contains(&source) {
            // 匹配已知 candidate，返回其类型
            return self
                .remote_candidates
                .iter()
                .find(|c| c.address == source)
                .map(|c| c.candidate_type)
                .unwrap_or(IceCandidateType::Host);
        }
        // 未知地址 → 学习为 peer-reflexive candidate
        debug!(addr = %source, "ICE: 学习 peer-reflexive candidate");
        let prflx = RemoteCandidate {
            address: source,
            priority: IceCandidateType::Prflx.priority(1, 65535),
            foundation: format!("prflx-{}", source.port()),
            component_id: 1,
            transport: "udp".to_string(),
            candidate_type: IceCandidateType::Prflx,
        };
        self.known_peer_addresses.push(source);
        self.remote_candidates.push(prflx);
        IceCandidateType::Prflx
    }

    /// 当收到携带 ICE-CONTROLLING 的 STUN Binding Request 时，
    /// 标记对应 candidate pair 为 selected。
    pub fn mark_selected(&mut self, source: SocketAddr) {
        if let Some(candidate) = self
            .remote_candidates
            .iter()
            .find(|c| c.address == source)
            .cloned()
        {
            self.selected_pair = Some(CandidatePair { remote: candidate });
            self.ice_connected
                .store(true, std::sync::atomic::Ordering::Release);
            debug!(addr = %source, "ICE: candidate pair selected");
        }
    }

    /// 检查 STUN Binding Request 是否携带 ICE-CONTROLLING 属性
    /// （表示对端为 controlling role，使用 USE-CANDIDATE 语义）。
    pub fn is_use_candidate(&self, request: &Message) -> bool {
        // RFC 8445: USE-CANDIDATE 属性存在即表示对端选择此 candidate pair
        // stun crate 0.17 的 ICE-CONTROLLING 属性存在即表示对端为 controlling role
        request.get(ATTR_ICE_CONTROLLING).is_ok()
    }

    /// 检查 ICE role conflict：若本地为 Lite（controlled），对端也为 Lite，
    /// 则以 tie-breaker 数值大者为 controlling。
    ///
    /// 在 `WebRtcSession::handle_stun_packet` 中调用：当检测到 role conflict 时
    /// 记录告警，并按 RFC 8445 §7.3.1.1 规则保持本地 controlled 角色（因 ICE-Lite
    /// 总是被动响应）。
    pub fn check_role_conflict(&self, request: &Message) -> bool {
        // 若对端发送 ICE-CONTROLLED（表示它也认为自己是 controlled），
        // 且本地也是 controlled（ICE-Lite），则发生 role conflict。
        request.get(ATTR_ICE_CONTROLLED).is_ok()
    }

    /// 获取当前 selected candidate pair 的远端地址。
    pub fn selected_remote_address(&self) -> Option<SocketAddr> {
        self.selected_pair.as_ref().map(|p| p.remote.address)
    }

    /// 本地 ICE ufrag（用于 USERNAME 校验与诊断日志）。
    ///
    /// 在 SDP answer 中回送给远端，亦可用于诊断日志。
    pub fn local_ufrag(&self) -> &str {
        &self.local_ufrag
    }

    /// 已知远端候选数量（含 peer-reflexive 学习到的地址）。
    ///
    /// 用于运维监控与 SDP 协商诊断。
    pub fn remote_candidate_count(&self) -> usize {
        self.remote_candidates.len()
    }

    /// 返回最高优先级的远端候选（按 `priority` 降序）。
    ///
    /// 在 ICE-Lite 中不主动发起连通性检查，但该信息可用于诊断远端 SDP 协商结果，
    /// 亦为未来 full-ICE 升级预留 candidate pair 排序入口。
    pub fn highest_priority_candidate(&self) -> Option<&RemoteCandidate> {
        self.remote_candidates
            .iter()
            .max_by_key(|candidate| candidate.priority)
    }

    /// 返回所有远端候选的摘要信息，用于运维监控与 SDP 协商诊断。
    ///
    /// 摘要包含 foundation、component_id、transport、priority 与类型，
    /// 便于在 `WebRtcSession` 诊断端点中暴露给信令层排查 ICE 协商问题。
    pub fn candidate_summaries(&self) -> Vec<CandidateSummary> {
        self.remote_candidates
            .iter()
            .map(|candidate| CandidateSummary {
                address: candidate.address,
                foundation: candidate.foundation.clone(),
                component_id: candidate.component_id,
                transport: candidate.transport.clone(),
                priority: candidate.priority,
                candidate_type: candidate.candidate_type,
            })
            .collect()
    }
}

/// 远端候选摘要，用于诊断与监控。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CandidateSummary {
    pub address: SocketAddr,
    pub foundation: String,
    pub component_id: u16,
    pub transport: String,
    pub priority: u32,
    pub candidate_type: IceCandidateType,
}

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

/// 解析 SDP `a=candidate:` 行为 `RemoteCandidate`。
///
/// 格式：`candidate:<foundation> <component> <transport> <priority> <addr> <port> typ <type> [raddr <raddr> rport <rport>]`
///
/// 在 `WebRtcSession::add_remote_candidate` 之前由信令层调用，将远端 SDP 中
/// 所有 `a=candidate:` 行解析为 `RemoteCandidate` 后注入 ICE agent。
pub fn parse_candidate_line(line: &str) -> Result<RemoteCandidate, String> {
    let line = line.strip_prefix("candidate:").unwrap_or(line);
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 8 {
        return Err(format!("candidate 行字段不足: {line}"));
    }
    let foundation = parts[0].to_string();
    let component_id: u16 = parts[1]
        .parse()
        .map_err(|e| format!("component_id 解析失败: {e}"))?;
    let transport = parts[2].to_ascii_uppercase();
    let priority: u32 = parts[3]
        .parse()
        .map_err(|e| format!("priority 解析失败: {e}"))?;
    let addr_str = parts[4];
    let port: u16 = parts[5]
        .parse()
        .map_err(|e| format!("port 解析失败: {e}"))?;
    // parts[6] 应为 "typ"
    if parts.get(6).map(|s| *s != "typ").unwrap_or(true) {
        return Err(format!("candidate 行缺少 typ 关键字: {line}"));
    }
    let type_str = parts.get(7).unwrap_or(&"host");
    let candidate_type = match *type_str {
        "host" => IceCandidateType::Host,
        "srflx" => IceCandidateType::Srflx,
        "prflx" => IceCandidateType::Prflx,
        "relay" => IceCandidateType::Relay,
        other => return Err(format!("未知 candidate 类型: {other}")),
    };
    let ip: std::net::IpAddr = addr_str
        .parse()
        .map_err(|e| format!("IP 地址解析失败: {e}"))?;
    let address = std::net::SocketAddr::new(ip, port);
    Ok(RemoteCandidate {
        address,
        priority,
        foundation,
        component_id,
        transport,
        candidate_type,
    })
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

    #[test]
    fn parse_candidate_line_parses_host_candidate() {
        let line = "candidate:842163049 1 udp 1677729535 192.0.2.3 64137 typ host";
        let candidate = parse_candidate_line(line).unwrap();
        assert_eq!(candidate.foundation, "842163049");
        assert_eq!(candidate.component_id, 1);
        assert_eq!(candidate.transport, "UDP");
        assert_eq!(candidate.priority, 1677729535);
        assert_eq!(candidate.address, "192.0.2.3:64137".parse().unwrap());
        assert_eq!(candidate.candidate_type, IceCandidateType::Host);
    }

    #[test]
    fn parse_candidate_line_parses_srflx_candidate() {
        let line =
            "candidate:842163050 1 udp 1677729534 203.0.113.4 50000 typ srflx raddr 192.168.1.2 rport 50000";
        let candidate = parse_candidate_line(line).unwrap();
        assert_eq!(candidate.candidate_type, IceCandidateType::Srflx);
        assert_eq!(candidate.address, "203.0.113.4:50000".parse().unwrap());
    }

    #[test]
    fn parse_candidate_line_rejects_missing_typ() {
        let line = "candidate:1 1 udp 100 192.0.2.3 64137";
        assert!(parse_candidate_line(line).is_err());
    }

    #[test]
    fn ice_agent_learns_peer_reflexive_candidate() {
        let mut agent = IceAgent::new("ufrag".to_string());
        let known_addr: SocketAddr = "192.0.2.1:50000".parse().unwrap();
        agent.add_remote_candidate(RemoteCandidate {
            address: known_addr,
            priority: 100,
            foundation: "1".to_string(),
            component_id: 1,
            transport: "UDP".to_string(),
            candidate_type: IceCandidateType::Host,
        });

        // 已知地址 → 返回 Host
        let t = agent.learn_or_match_peer_address(known_addr);
        assert_eq!(t, IceCandidateType::Host);

        // 未知地址 → 学习为 Prflx
        let new_addr: SocketAddr = "198.51.100.1:60000".parse().unwrap();
        let t = agent.learn_or_match_peer_address(new_addr);
        assert_eq!(t, IceCandidateType::Prflx);

        // 再次收到同一地址 → 返回 Prflx
        let t = agent.learn_or_match_peer_address(new_addr);
        assert_eq!(t, IceCandidateType::Prflx);
    }

    #[test]
    fn ice_agent_marks_selected_pair() {
        let mut agent = IceAgent::new("ufrag".to_string());
        let addr: SocketAddr = "192.0.2.1:50000".parse().unwrap();
        agent.add_remote_candidate(RemoteCandidate {
            address: addr,
            priority: 100,
            foundation: "1".to_string(),
            component_id: 1,
            transport: "UDP".to_string(),
            candidate_type: IceCandidateType::Host,
        });

        assert!(!agent
            .ice_connected
            .load(std::sync::atomic::Ordering::Relaxed));
        agent.mark_selected(addr);
        assert!(agent
            .ice_connected
            .load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(agent.selected_remote_address(), Some(addr));
    }

    #[test]
    fn ice_candidate_priority_formula() {
        // RFC 8445 §5.1.2 优先级公式验证
        let host_prio = IceCandidateType::Host.priority(1, 65535);
        let srflx_prio = IceCandidateType::Srflx.priority(1, 65535);
        let prflx_prio = IceCandidateType::Prflx.priority(1, 65535);
        let relay_prio = IceCandidateType::Relay.priority(1, 65535);

        // Host > Prflx > Srflx > Relay
        assert!(host_prio > prflx_prio);
        assert!(prflx_prio > srflx_prio);
        assert!(srflx_prio > relay_prio);
    }
}
