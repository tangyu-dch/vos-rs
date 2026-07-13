use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use crate::media::relay::{MediaRelayState, PlaybackMode};

/// 生成一个标准的 8000Hz 16-bit Mono PCM 1秒钟的测试 WAV 文件
fn create_test_wav(path: &std::path::Path) {
    let mut file = File::create(path).unwrap();
    // 44字节标准的 WAV 文件头
    file.write_all(b"RIFF").unwrap();
    let file_size = 44_u32 + 16000_u32 - 8_u32;
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap(); // Chunk size
    file.write_all(&1_u16.to_le_bytes()).unwrap();  // PCM format
    file.write_all(&1_u16.to_le_bytes()).unwrap();  // Channels: 1 (Mono)
    file.write_all(&8000_u32.to_le_bytes()).unwrap(); // Sample rate: 8000
    let byte_rate = 8000_u32 * 2_u32;
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap(); // Block align
    file.write_all(&16_u16.to_le_bytes()).unwrap(); // Bits per sample
    file.write_all(b"data").unwrap();
    file.write_all(&16000_u32.to_le_bytes()).unwrap(); // Data segment size (8000 samples * 2 bytes)
    
    // 写入 8000 个静音采样点 (0)
    let pcm_data = vec![0_u8; 16000];
    file.write_all(&pcm_data).unwrap();
}

#[tokio::test]
async fn test_playback_and_mute_control_flow() {
    let wav_path = PathBuf::from("test_playback.wav");
    create_test_wav(&wav_path);

    let relay = MediaRelayState::new();
    let port = 45000;

    // 绑定本地 Socket
    let socket = UdpSocket::bind("127.0.0.1:45000").await.unwrap();
    let socket = Arc::new(socket);
    relay.active_sockets.insert(port, Arc::clone(&socket));

    // 设置对端路由接收地址（模拟主叫）
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver_addr = receiver.local_addr().unwrap();
    relay.targets.insert(port, receiver_addr);

    // 1. 验证静音插入与移出
    assert!(!relay.muted_ports.contains(&port));
    relay.muted_ports.insert(port);
    assert!(relay.muted_ports.contains(&port));
    relay.muted_ports.remove(&port);
    assert!(!relay.muted_ports.contains(&port));

    // 2. 启动音频播放
    relay.start_playback(port, wav_path.clone(), PlaybackMode::Exclusive, true).unwrap();
    assert!(relay.playbacks.contains_key(&port));

    // 3. 验证接收播放的 RTP 数据包，并且验证首包 Marker Bit 为 true
    let mut buffer = [0_u8; 1500];
    let (len, from) = tokio::time::timeout(Duration::from_millis(500), receiver.recv_from(&mut buffer))
        .await
        .expect("接收 RTP 播放包超时")
        .expect("读取数据失败");

    assert!(len > 12); // RTP 头至少有 12 字节
    assert_eq!(from, socket.local_addr().unwrap());
    
    // 解析 RTP 包验证 Marker Bit
    let rtp_view = rtp_core::RtpPacketView::parse(&buffer[..len]).unwrap();
    assert!(rtp_view.marker); // 首包 Marker Bit 应为 true

    // 4. 停止播放
    relay.stop_playback(port);
    assert!(!relay.playbacks.contains_key(&port));

    // 清理测试文件
    let _ = std::fs::remove_file(wav_path);
}

#[tokio::test]
async fn test_smooth_sequence_and_timestamp_transition() {
    let relay = MediaRelayState::new();
    let local_port = 45010;
    let peer_port = 45012;

    relay.peer_ports.insert(local_port, peer_port);
    relay.peer_ports.insert(peer_port, local_port);

    // 1. 记录初始流发送
    relay.last_sent_seq.insert(local_port, 100);
    relay.last_sent_ts.insert(local_port, 1000);

    // 2. 模拟 Exclusive 播放启动与停止（表示发生了 Exclusive 拦截）
    // 此时 was_in_exclusive 会被标记为 true
    relay.was_in_exclusive.insert(local_port, true);

    // 3. 模拟在停止 Exclusive 播放后，收到原音频发送端发来的下一个非连续包（如 seq=105, ts=2000）
    let incoming_rtp = rtp_core::RtpPacket {
        marker: false,
        payload_type: 8,
        sequence_number: 105,
        timestamp: 2000,
        ssrc: 12345,
        csrcs: Vec::new(),
        extension: None,
        payload: vec![0; 160],
        padding_len: 0,
    };
    let encoded = incoming_rtp.encode().unwrap();

    // 4. 在 relay_media_port 中的相同改写逻辑校验：
    let mut rewritten_packet = None;
    if let Ok(mut rtp) = rtp_core::RtpPacket::parse(&encoded) {
        let local_was_blocked = relay.was_in_exclusive.remove(&local_port).map(|(_, val)| val).unwrap_or(false);
        assert!(local_was_blocked); // 验证独占标记被正确读出

        if local_was_blocked {
            if let (Some(last_seq), Some(last_ts)) = (
                relay.last_sent_seq.get(&local_port).map(|entry| *entry),
                relay.last_sent_ts.get(&local_port).map(|entry| *entry),
            ) {
                let seq_offset = rtp.sequence_number.wrapping_sub(last_seq.wrapping_add(1));
                let ts_offset = rtp.timestamp.wrapping_sub(last_ts.wrapping_add(160));
                
                assert_eq!(seq_offset, 105 - 101); // 105 - 101 = 4
                assert_eq!(ts_offset, 2000 - 1160); // 2000 - 1160 = 840

                relay.seq_offsets.insert(local_port, seq_offset);
                relay.ts_offsets.insert(local_port, ts_offset);
            }
        }

        let seq_offset = relay.seq_offsets.get(&local_port).map(|entry| *entry).unwrap_or(0);
        let ts_offset = relay.ts_offsets.get(&local_port).map(|entry| *entry).unwrap_or(0);

        if seq_offset != 0 || ts_offset != 0 {
            rtp.sequence_number = rtp.sequence_number.wrapping_sub(seq_offset);
            rtp.timestamp = rtp.timestamp.wrapping_sub(ts_offset);
            if let Ok(encoded) = rtp.encode() {
                rewritten_packet = Some(encoded);
            }
        }
    }

    let parsed_rewritten = rtp_core::RtpPacket::parse(&rewritten_packet.unwrap()).unwrap();
    // 验证修改后的包序列号与时间戳是完全连续的（接上 100 和 1000 + 160）
    assert_eq!(parsed_rewritten.sequence_number, 101);
    assert_eq!(parsed_rewritten.timestamp, 1160);
}

#[tokio::test]
async fn test_audio_resampling_from_higher_rate() {
    let wav_path = PathBuf::from("test_resample_16k.wav");
    
    // 生成一个 16000Hz (16kHz) 16-bit Mono LPCM WAV 文件，包含 16000 个采样（即 1秒钟长度）
    let mut file = File::create(&wav_path).unwrap();
    file.write_all(b"RIFF").unwrap();
    let file_size = 44_u32 + 32000_u32 - 8_u32;
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap(); // LPCM
    file.write_all(&1_u16.to_le_bytes()).unwrap(); // Mono
    file.write_all(&16000_u32.to_le_bytes()).unwrap(); // Sample rate: 16000 Hz
    let byte_rate = 16000_u32 * 2_u32;
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&32000_u32.to_le_bytes()).unwrap(); // 16000 samples * 2 bytes
    
    let pcm_data = vec![0_u8; 32000];
    file.write_all(&pcm_data).unwrap();
    drop(file);

    // 载入 WAV 并自动进行重采样
    let samples = crate::media::wav::load_wav_pcm(&wav_path).unwrap();
    
    // 验证经过重采样（16000Hz -> 8000Hz）后，采样点个数正好减半为 8000 个左右
    assert_eq!(samples.len(), 8000);

    // 清理测试文件
    let _ = std::fs::remove_file(wav_path);
}
