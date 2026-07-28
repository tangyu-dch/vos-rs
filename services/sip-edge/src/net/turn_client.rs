//! TURN 客户端（RFC 5766）：ALLOCATION/REFRESH/CREATE-PERMISSION/SEND。
//!
//! 用于在 Symmetric NAT 环境下通过 TURN 中继转发 RTP/RTCP 媒体流。
//! 实现客户端侧的 ALLOCATE → REFRESH 续约 → CREATE-PERMISSION → SEND 路径。
//!
//! 启动阶段由 `net::run_turn_bootstrap` 完成分配并注入 `EdgeState`，
//! 媒体中继路径在 SDP 协商完成后通过 `TurnClient::create_permission` 注册对端，
//! 并在 RTP 转发循环中按需调用 `TurnClient::send_data` 走中继发送。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn};

use stun::{
    attributes::{
        ATTR_LIFETIME, ATTR_REQUESTED_TRANSPORT, ATTR_XOR_PEER_ADDRESS, ATTR_XOR_RELAYED_ADDRESS,
    },
    fingerprint::FINGERPRINT,
    integrity::MessageIntegrity,
    message::{
        Message, MessageType, Setter, CLASS_REQUEST, METHOD_ALLOCATE, METHOD_CREATE_PERMISSION,
        METHOD_REFRESH,
    },
    xoraddr::XorMappedAddress,
};

/// TURN 分配结果。
#[derive(Debug, Clone)]
pub struct TurnAllocation {
    /// TURN 服务器分配的 relayed 地址（对端可通过此地址发送数据到本端）。
    pub relayed_address: SocketAddr,
    /// TURN 服务器观察到的本端映射地址。
    pub mapped_address: SocketAddr,
    /// 分配的生命周期（秒），到期前需发送 REFRESH 续约。
    pub lifetime_secs: u32,
}

/// TURN 客户端配置。
#[derive(Debug, Clone)]
pub struct TurnClientConfig {
    pub server_addr: SocketAddr,
    pub username: String,
    pub password: String,
    pub realm: String,
}

/// TURN 客户端，管理单条 allocation 的生命周期。
pub struct TurnClient {
    config: TurnClientConfig,
    socket: Arc<UdpSocket>,
    allocation: Mutex<Option<TurnAllocation>>,
    refresh_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl std::fmt::Debug for TurnClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnClient")
            .field("server_addr", &self.config.server_addr)
            .field("username", &self.config.username)
            .field("realm", &self.config.realm)
            .finish_non_exhaustive()
    }
}

impl TurnClient {
    /// 创建 TURN 客户端并绑定本地 UDP socket。
    pub async fn new(config: TurnClientConfig) -> Result<Self, String> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| format!("TURN 客户端 socket 绑定失败: {e}"))?;
        socket
            .connect(config.server_addr)
            .await
            .map_err(|e| format!("TURN 服务器连接失败: {e}"))?;
        Ok(Self {
            config,
            socket: Arc::new(socket),
            allocation: Mutex::new(None),
            refresh_handle: Mutex::new(None),
        })
    }

    /// 向 TURN 服务器发送 ALLOCATE 请求并获取 relayed 地址。
    pub async fn allocate(&self) -> Result<TurnAllocation, String> {
        let mut request = build_request(MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST));
        // 请求 UDP 传输 (protocol number 17)
        request.add(ATTR_REQUESTED_TRANSPORT, &17u32.to_be_bytes());

        // 长期认证
        let integrity = MessageIntegrity::new_long_term_integrity(
            self.config.username.clone(),
            self.config.realm.clone(),
            self.config.password.clone(),
        );
        integrity
            .add_to(&mut request)
            .map_err(|e| format!("添加 MESSAGE-INTEGRITY 失败: {e}"))?;
        FINGERPRINT
            .add_to(&mut request)
            .map_err(|e| format!("添加 FINGERPRINT 失败: {e}"))?;

        let response = self.send_request(&request.raw).await?;

        // 解析 relayed address
        let mut relayed_addr = XorMappedAddress::default();
        relayed_addr
            .get_from_as(&response, ATTR_XOR_RELAYED_ADDRESS)
            .map_err(|e| format!("解析 XOR-RELAYED-ADDRESS 失败: {e}"))?;

        // 解析 mapped address（XOR-MAPPED-ADDRESS，标准属性）
        let mut mapped_addr = XorMappedAddress::default();
        let _ = mapped_addr.get_from_as(&response, stun::attributes::ATTR_XORMAPPED_ADDRESS);

        // 解析 lifetime
        let lifetime = response
            .get(ATTR_LIFETIME)
            .ok()
            .and_then(|v| {
                if v.len() >= 4 {
                    Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
                } else {
                    None
                }
            })
            .unwrap_or(600);

        let allocation = TurnAllocation {
            relayed_address: SocketAddr::new(relayed_addr.ip, relayed_addr.port),
            mapped_address: SocketAddr::new(mapped_addr.ip, mapped_addr.port),
            lifetime_secs: lifetime,
        };

        info!(
            relayed = %allocation.relayed_address,
            mapped = %allocation.mapped_address,
            lifetime = lifetime,
            "TURN allocation 成功"
        );

        *self.allocation.lock().await = Some(allocation.clone());

        // 启动自动刷新
        self.start_refresh_loop(lifetime).await;

        Ok(allocation)
    }

    /// 启动后台 REFRESH 循环，在 lifetime 的 2/3 处自动续约。
    async fn start_refresh_loop(&self, lifetime_secs: u32) {
        let refresh_interval = if lifetime_secs > 30 {
            Duration::from_secs((lifetime_secs as u64 * 2) / 3)
        } else {
            Duration::from_secs(lifetime_secs as u64 / 2)
        };

        let socket = Arc::clone(&self.socket);
        let username = self.config.username.clone();
        let password = self.config.password.clone();
        let realm = self.config.realm.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = interval(refresh_interval);
            ticker.tick().await; // 跳过首次立即触发
            loop {
                ticker.tick().await;
                if let Err(e) = send_refresh(&socket, &username, &realm, &password).await {
                    warn!(error = %e, "TURN REFRESH 失败，将在下次重试");
                } else {
                    debug!("TURN REFRESH 成功");
                }
            }
        });

        *self.refresh_handle.lock().await = Some(handle);
    }

    /// 创建到对端的 PERMISSION（CREATE-PERMISSION）。
    pub async fn create_permission(&self, peer: SocketAddr) -> Result<(), String> {
        let mut request = build_request(MessageType::new(METHOD_CREATE_PERMISSION, CLASS_REQUEST));

        XorMappedAddress {
            ip: peer.ip(),
            port: peer.port(),
        }
        .add_to_as(&mut request, ATTR_XOR_PEER_ADDRESS)
        .map_err(|e| format!("添加 XOR-PEER-ADDRESS 失败: {e}"))?;

        let integrity = MessageIntegrity::new_long_term_integrity(
            self.config.username.clone(),
            self.config.realm.clone(),
            self.config.password.clone(),
        );
        integrity
            .add_to(&mut request)
            .map_err(|e| format!("添加 MESSAGE-INTEGRITY 失败: {e}"))?;
        FINGERPRINT
            .add_to(&mut request)
            .map_err(|e| format!("添加 FINGERPRINT 失败: {e}"))?;

        let _ = self.send_request(&request.raw).await?;
        debug!(peer = %peer, "TURN CREATE-PERMISSION 成功");
        Ok(())
    }

    /// 通过 TURN 中继发送数据（Send Indication）。
    pub async fn send_data(&self, data: &[u8], peer: SocketAddr) -> Result<(), String> {
        let mut request =
            build_request(MessageType::new(stun::message::METHOD_SEND, CLASS_REQUEST));

        XorMappedAddress {
            ip: peer.ip(),
            port: peer.port(),
        }
        .add_to_as(&mut request, ATTR_XOR_PEER_ADDRESS)
        .map_err(|e| format!("添加 XOR-PEER-ADDRESS 失败: {e}"))?;

        request.add(stun::attributes::ATTR_DATA, data);

        self.socket
            .send(&request.raw)
            .await
            .map_err(|e| format!("TURN SEND 发送失败: {e}"))?;
        Ok(())
    }

    /// 获取当前 allocation 信息。
    pub async fn allocation(&self) -> Option<TurnAllocation> {
        self.allocation.lock().await.clone()
    }

    /// 发送 STUN 请求并等待响应（5 秒超时）。
    async fn send_request(&self, raw: &[u8]) -> Result<Message, String> {
        self.socket
            .send(raw)
            .await
            .map_err(|e| format!("TURN 请求发送失败: {e}"))?;

        let mut buf = vec![0u8; 4096];
        let n = timeout(Duration::from_secs(5), self.socket.recv(&mut buf))
            .await
            .map_err(|_| "TURN 响应超时".to_string())?
            .map_err(|e| format!("TURN 响应接收失败: {e}"))?;

        let mut response = Message::new();
        response.raw = buf[..n].to_vec();
        response
            .decode()
            .map_err(|e| format!("TURN 响应解码失败: {e}"))?;
        Ok(response)
    }

    /// 停止 REFRESH 循环并释放 allocation（lifetime=0）。
    pub async fn destroy(&self) {
        if let Some(handle) = self.refresh_handle.lock().await.take() {
            handle.abort();
        }
        let _ = send_refresh_with_lifetime(
            &self.socket,
            &self.config.username,
            &self.config.realm,
            &self.config.password,
            0,
        )
        .await;
        *self.allocation.lock().await = None;
        info!("TURN allocation 已释放");
    }
}

/// 构造 STUN 请求消息（含事务 ID 与头部）。
fn build_request(message_type: MessageType) -> Message {
    let mut request = Message::new();
    request.typ = message_type;
    request.transaction_id = stun::agent::TransactionId::new();
    request.write_header();
    request
}

/// 发送 REFRESH 请求（使用默认 lifetime 续约）。
async fn send_refresh(
    socket: &UdpSocket,
    username: &str,
    realm: &str,
    password: &str,
) -> Result<(), String> {
    send_refresh_with_lifetime(socket, username, realm, password, 600).await
}

/// 发送指定 lifetime 的 REFRESH 请求。
async fn send_refresh_with_lifetime(
    socket: &UdpSocket,
    username: &str,
    realm: &str,
    password: &str,
    lifetime_secs: u32,
) -> Result<(), String> {
    let mut request = build_request(MessageType::new(METHOD_REFRESH, CLASS_REQUEST));
    request.add(ATTR_LIFETIME, &lifetime_secs.to_be_bytes());

    let integrity = MessageIntegrity::new_long_term_integrity(
        username.to_string(),
        realm.to_string(),
        password.to_string(),
    );
    integrity
        .add_to(&mut request)
        .map_err(|e| format!("添加 MESSAGE-INTEGRITY 失败: {e}"))?;
    FINGERPRINT
        .add_to(&mut request)
        .map_err(|e| format!("添加 FINGERPRINT 失败: {e}"))?;

    socket
        .send(&request.raw)
        .await
        .map_err(|e| format!("REFRESH 发送失败: {e}"))?;

    let mut buf = vec![0u8; 2048];
    let _ = timeout(Duration::from_secs(5), socket.recv(&mut buf))
        .await
        .map_err(|_| "REFRESH 响应超时".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_client_config_construction() {
        let config = TurnClientConfig {
            server_addr: "203.0.113.10:3478".parse().unwrap(),
            username: "user".to_string(),
            password: "pass".to_string(),
            realm: "example.com".to_string(),
        };
        assert_eq!(config.username, "user");
        assert_eq!(config.realm, "example.com");
    }

    #[test]
    fn turn_allocation_clone() {
        let alloc = TurnAllocation {
            relayed_address: "203.0.113.1:50000".parse().unwrap(),
            mapped_address: "198.51.100.1:60000".parse().unwrap(),
            lifetime_secs: 600,
        };
        let cloned = alloc.clone();
        assert_eq!(alloc.relayed_address, cloned.relayed_address);
        assert_eq!(alloc.lifetime_secs, cloned.lifetime_secs);
    }

    #[tokio::test]
    async fn turn_client_creation_binds_socket() {
        let config = TurnClientConfig {
            server_addr: "127.0.0.1:3478".parse().unwrap(),
            username: "test".to_string(),
            password: "test".to_string(),
            realm: "test".to_string(),
        };
        let client = TurnClient::new(config).await.unwrap();
        assert!(client.allocation().await.is_none());
    }

    #[test]
    fn build_request_generates_valid_stun_header() {
        let request = build_request(MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST));
        assert!(request.raw.len() >= 20, "STUN 消息头至少 20 字节");
        // 前 2 字节为消息类型
        assert_eq!(
            request.typ,
            MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST)
        );
    }
}
