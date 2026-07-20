//! # 跨语言标准 AI 语音插件协议 (AI Voice Plugin Protocol)
//!
//! 本模块实现将 VOS-rs 媒体数据面解密后的 PCM 音频流，封装为 16 字节头 + 320 字节 PCM 载荷的标准二进制包，
//! 通过语言无关的 UDP Socket/WebSocket 实时与外部大语言模型 (OpenAI Realtime/Gemini Live 等) 插件双向同步。

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub const AI_FRAME_HEADER_SIZE: usize = 16;
pub const AI_FRAME_PAYLOAD_SIZE: usize = 320; // 20ms, 8000Hz, 16bit mono PCM = 160 samples * 2 bytes = 320 bytes
pub const AI_FRAME_TOTAL_SIZE: usize = AI_FRAME_HEADER_SIZE + AI_FRAME_PAYLOAD_SIZE;

/// VOS-rs 与外部 AI 语音插件交互的标准二进制帧
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiVoiceFrame {
    pub call_id: u32,
    pub seq: u32,
    pub timestamp: u64,
    pub pcm_data: Vec<u8>, // 长度必须为 320 字节
}

impl AiVoiceFrame {
    /// 序列化为 336 字节的标准二进制帧
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = vec![0u8; AI_FRAME_TOTAL_SIZE];
        buf[0..4].copy_from_slice(&self.call_id.to_be_bytes());
        buf[4..8].copy_from_slice(&self.seq.to_be_bytes());
        buf[8..16].copy_from_slice(&self.timestamp.to_be_bytes());
        let copy_len = std::cmp::min(self.pcm_data.len(), AI_FRAME_PAYLOAD_SIZE);
        buf[16..16 + copy_len].copy_from_slice(&self.pcm_data[..copy_len]);
        buf
    }

    /// 反序列化 336 字节的标准二进制帧
    pub fn deserialize(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < AI_FRAME_TOTAL_SIZE {
            return Err(format!(
                "Packet too short: expected {}, got {}",
                AI_FRAME_TOTAL_SIZE,
                buf.len()
            ));
        }
        let call_id = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let seq = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let timestamp = u64::from_be_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        let pcm_data = buf[16..AI_FRAME_TOTAL_SIZE].to_vec();
        Ok(Self {
            call_id,
            seq,
            timestamp,
            pcm_data,
        })
    }
}

/// AI 语音插件双向通信代理上下文
pub struct AiVoicePluginProxy {
    /// 用于向上行（AI 插件）推送音频数据包的通道
    pub upstream_tx: mpsc::Sender<AiVoiceFrame>,
    /// 用于向媒体通道接收大语言模型 TTS 产生的下行音频的通道
    pub downstream_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<AiVoiceFrame>>>,
}

impl AiVoicePluginProxy {
    /// 建立一个 AI 流式交互代理会话。
    /// bind_addr 为媒体面绑定的本地 UDP 地址，plugin_addr 为外部 AI 插件的 UDP 监听端口。
    pub async fn start(bind_addr: SocketAddr, plugin_addr: SocketAddr) -> Result<Self, String> {
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| format!("Failed to bind AI proxy socket: {:?}", e))?;
        let socket = Arc::new(socket);

        let (up_tx, mut up_rx) = mpsc::channel::<AiVoiceFrame>(1000);
        let (down_tx, down_rx) = mpsc::channel::<AiVoiceFrame>(1000);

        // 启动上行发送 Loop (VOS-rs -> AI Plugin)
        let socket_up = Arc::clone(&socket);
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                let bytes = frame.serialize();
                if let Err(e) = socket_up.send_to(&bytes, plugin_addr).await {
                    warn!(?e, "Failed to send audio frame to AI voice plugin");
                }
            }
        });

        // 启动下行接收 Loop (AI Plugin -> VOS-rs)
        let socket_down = Arc::clone(&socket);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            loop {
                match socket_down.recv_from(&mut buf).await {
                    Ok((size, from)) => {
                        if from == plugin_addr {
                            match AiVoiceFrame::deserialize(&buf[..size]) {
                                Ok(frame) => {
                                    let _ = down_tx.send(frame).await;
                                }
                                Err(e) => {
                                    debug!(?e, "Failed to deserialize incoming AI voice frame");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(?e, "AI proxy socket recv error");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            upstream_tx: up_tx,
            downstream_rx: Arc::new(tokio::sync::Mutex::new(down_rx)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_frame_serialization_roundtrip() {
        let original = AiVoiceFrame {
            call_id: 12345,
            seq: 678,
            timestamp: 999999,
            pcm_data: vec![0xaa; AI_FRAME_PAYLOAD_SIZE],
        };
        let bytes = original.serialize();
        assert_eq!(bytes.len(), AI_FRAME_TOTAL_SIZE);

        let decoded = AiVoiceFrame::deserialize(&bytes).unwrap();
        assert_eq!(decoded.call_id, 12345);
        assert_eq!(decoded.seq, 678);
        assert_eq!(decoded.timestamp, 999999);
        assert_eq!(decoded.pcm_data, vec![0xaa; AI_FRAME_PAYLOAD_SIZE]);
    }

    #[tokio::test]
    async fn test_ai_plugin_voice_protocol_exchange() {
        // 模拟外部 AI 插件端的 UDP 服务监听 (动态绑定空闲端口)
        let plugin_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let plugin_addr = plugin_socket.local_addr().unwrap();
        let plugin_socket = Arc::new(plugin_socket);

        let proxy_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_socket.local_addr().unwrap();
        drop(proxy_socket);

        // 启动 AI 代理
        let proxy = AiVoicePluginProxy::start(proxy_addr, plugin_addr)
            .await
            .unwrap();

        // 上行发送数据包
        let test_frame = AiVoiceFrame {
            call_id: 99,
            seq: 1,
            timestamp: 1000,
            pcm_data: vec![0x55; AI_FRAME_PAYLOAD_SIZE],
        };
        proxy.upstream_tx.send(test_frame.clone()).await.unwrap();

        // AI 插件端接收并断言
        let mut buf = vec![0u8; 1024];
        let (size, from) = plugin_socket.recv_from(&mut buf).await.unwrap();
        assert_eq!(from, proxy_addr);
        let received_frame = AiVoiceFrame::deserialize(&buf[..size]).unwrap();
        assert_eq!(received_frame.call_id, 99);
        assert_eq!(received_frame.pcm_data, vec![0x55; AI_FRAME_PAYLOAD_SIZE]);

        // AI 插件回送下行音频数据
        let response_frame = AiVoiceFrame {
            call_id: 99,
            seq: 2,
            timestamp: 1020,
            pcm_data: vec![0x77; AI_FRAME_PAYLOAD_SIZE],
        };
        plugin_socket
            .send_to(&response_frame.serialize(), proxy_addr)
            .await
            .unwrap();

        // 代理端接收下行数据并断言
        let mut rx = proxy.downstream_rx.lock().await;
        let received_down = rx.recv().await.unwrap();
        assert_eq!(received_down.call_id, 99);
        assert_eq!(received_down.seq, 2);
        assert_eq!(received_down.pcm_data, vec![0x77; AI_FRAME_PAYLOAD_SIZE]);
    }
}
