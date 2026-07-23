use dashmap::DashMap;
use media_core::conference::{
    encode_conference_frame, mix_conference_frames, take_conference_frame, ConferenceCodec,
    CONFERENCE_FRAME_SAMPLES,
};
use rtp_core::AudioCodec;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// 会议参会成员状态
pub struct ConferenceParticipant {
    pub port: u16,
    pub codec: AudioCodec,
    pub target_addr: SocketAddr,
    pub socket: Arc<UdpSocket>,
    pub pcm_buffer: Vec<i16>,
    pub ssrc: u32,
    pub sequence: u16,
    pub timestamp: u32,
    pub muted: bool,
}

/// 会议室结构，执行 mix-minus 混音算法
pub struct Conference {
    pub id: String,
    pub participants: HashMap<u16, ConferenceParticipant>,
}

impl Conference {
    pub fn new(id: String) -> Self {
        Self {
            id,
            participants: HashMap::new(),
        }
    }

    /// 执行混音并向所有参会成员发送混合后的音频
    pub fn mix_and_prepare(&mut self) -> Vec<(Arc<UdpSocket>, SocketAddr, Vec<u8>)> {
        if self.participants.len() < 2 {
            // 参会人数少于 2 人，清空现有缓存并返回
            for p in self.participants.values_mut() {
                p.pcm_buffer.clear();
            }
            return Vec::new();
        }

        let mut participant_frames = HashMap::with_capacity(self.participants.len());
        for (&port, p) in &mut self.participants {
            participant_frames.insert(port, take_conference_frame(&mut p.pcm_buffer, p.muted));
        }

        let mut packets_to_send = Vec::with_capacity(self.participants.len());

        for (&port, p) in &mut self.participants {
            let output_frame = mix_conference_frames(
                participant_frames
                    .iter()
                    .filter_map(|(other_port, frame)| (*other_port != port).then_some(frame)),
            );
            let codec = match p.codec {
                AudioCodec::Pcmu => ConferenceCodec::Pcmu,
                _ => ConferenceCodec::Pcma,
            };
            let payload = encode_conference_frame(&output_frame, codec);

            let pt = p.codec.static_payload_type().unwrap_or(8);
            let rtp_packet = build_rtp_packet(pt, p.sequence, p.timestamp, p.ssrc, &payload);

            packets_to_send.push((Arc::clone(&p.socket), p.target_addr, rtp_packet));

            // 更新 RTP 序号与时间戳
            p.sequence = p.sequence.wrapping_add(1);
            p.timestamp = p.timestamp.wrapping_add(CONFERENCE_FRAME_SAMPLES as u32);
        }

        packets_to_send
    }
}

/// 会议管理器
pub struct ConferenceManager {
    pub conferences: DashMap<String, Arc<Mutex<Conference>>>,
    pub port_to_conference: DashMap<u16, String>,
}

impl ConferenceManager {
    pub fn new() -> Self {
        Self {
            conferences: DashMap::new(),
            port_to_conference: DashMap::new(),
        }
    }

    /// 加入会议
    pub async fn join_conference(
        &self,
        conference_id: &str,
        port: u16,
        codec: AudioCodec,
        target_addr: SocketAddr,
        socket: Arc<UdpSocket>,
    ) {
        info!(conference_id, port, %target_addr, "participant joining conference");

        let conf_arc = self
            .conferences
            .entry(conference_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Conference::new(conference_id.to_string()))))
            .clone();

        let mut conf = conf_arc.lock().await;
        let participant = ConferenceParticipant {
            port,
            codec,
            target_addr,
            socket,
            pcm_buffer: Vec::new(),
            ssrc: 12345 + port as u32,
            sequence: 1,
            timestamp: CONFERENCE_FRAME_SAMPLES as u32,
            muted: false,
        };
        conf.participants.insert(port, participant);
        self.port_to_conference
            .insert(port, conference_id.to_string());
    }

    /// 离开会议
    /// 离开会议
    pub async fn leave_conference(&self, port: u16) {
        if let Some((_, conference_id)) = self.port_to_conference.remove(&port) {
            info!(conference_id, port, "participant leaving conference");
            if let Some(conf_ref) = self.conferences.get(&conference_id) {
                let conf_arc = conf_ref.value().clone();
                drop(conf_ref); // 显式释放 DashMap 的读锁以避免持锁跨 await 发生死锁

                let mut conf = conf_arc.lock().await;
                conf.participants.remove(&port);
                if conf.participants.is_empty() {
                    drop(conf);
                    self.conferences.remove(&conference_id);
                }
            }
        }
    }

    /// 设置参会成员静音状态
    pub async fn set_participant_mute(&self, conference_id: &str, port: u16, mute: bool) -> bool {
        if let Some(conf_ref) = self.conferences.get(conference_id) {
            let conf_arc = conf_ref.value().clone();
            drop(conf_ref); // 显式释放以避免 await 锁竞争

            let mut conf = conf_arc.lock().await;
            if let Some(p) = conf.participants.get_mut(&port) {
                p.muted = mute;
                info!(conference_id, port, mute, "participant mute state changed");
                return true;
            }
        }
        false
    }

    /// 接收外部输入的 RTP 报文并解码加入成员缓冲
    pub fn handle_rtp_packet(&self, port: u16, payload: &[u8], codec: AudioCodec) {
        if let Some(conf_id) = self.port_to_conference.get(&port) {
            if let Some(conf_arc) = self.conferences.get(conf_id.value()) {
                // 为了极高性能且避免异步 lock 锁竞争，这里使用 try_lock。
                // 混音循环每 20ms 持有锁一次，这里极少会发生死锁。
                if let Ok(mut conf) = conf_arc.try_lock() {
                    if let Some(p) = conf.participants.get_mut(&port) {
                        let pcm: Vec<i16> = match codec {
                            AudioCodec::Pcmu => payload
                                .iter()
                                .map(|&byte| crate::media::recording::decode_pcmu(byte))
                                .collect(),
                            _ => payload
                                .iter()
                                .map(|&byte| crate::media::recording::decode_pcma(byte))
                                .collect(),
                        };
                        p.pcm_buffer.extend_from_slice(&pcm);
                    }
                }
            }
        }
    }
}

/// 快速构建原始 RTP 报文
fn build_rtp_packet(pt: u8, seq: u16, ts: u32, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(12 + payload.len());
    pkt.push(0x80);
    pkt.push(pt);
    pkt.extend_from_slice(&seq.to_be_bytes());
    pkt.extend_from_slice(&ts.to_be_bytes());
    pkt.extend_from_slice(&ssrc.to_be_bytes());
    pkt.extend_from_slice(payload);
    pkt
}

/// 启动会议混音后台任务，每 20ms 混音并发送一次
pub fn start_mixer_loop(manager: Arc<ConferenceManager>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20));
            loop {
                interval.tick().await;

                // 1. 先把所有会议的 Arc 收集起来，这样就能在 mix_and_send() 的 await 之前释放 DashMap 的迭代锁
                let active_conferences: Vec<Arc<Mutex<Conference>>> = manager
                    .conferences
                    .iter()
                    .map(|entry| entry.value().clone())
                    .collect();

                // 2. 在没有持有 DashMap 锁的情况下，安全地逐个混音与发送
                for conf_arc in active_conferences {
                    let packets = {
                        let mut conf = conf_arc.lock().await;
                        conf.mix_and_prepare()
                    };
                    for (socket, target_addr, rtp_packet) in packets {
                        if let Err(e) = socket.send_to(&rtp_packet, target_addr).await {
                            debug!(
                                error = %e,
                                "failed to send mixed RTP packet to conference participant"
                            );
                        }
                    }
                }
            }
        });
    }
}
