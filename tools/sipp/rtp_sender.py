#!/usr/bin/env python3
"""
轻量级 RTP 发送器 — 配合 SIPp 使用
监听 SIPp 的 INVITE/200 OK 交换，自动向 relay 端口发送 RTP 音频。
"""
import socket
import struct
import wave
import threading
import time
import sys
import random

SAMPLES_PER_FRAME = 160
RTP_PPS = 50
PCMU_PAYLOAD_TYPE = 0


def encode_pcmu(sample):
    BIAS = 0x84
    CLIP = 32635
    sign = 0
    if sample < 0:
        sign = 0x80
        sample = -sample
    sample = min(sample + BIAS, CLIP)
    exponent = 7
    mask = 0x4000
    while exponent > 0 and not (sample & mask):
        exponent -= 1
        mask >>= 1
    mantissa = (sample >> (exponent + 3)) & 0x0F
    return ~(sign | (exponent << 4) | mantissa) & 0xFF


def load_pcmu_frames(wav_path):
    with wave.open(wav_path, 'rb') as w:
        channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        framerate = w.getframerate()
        nframes = w.getnframes()
        raw = w.readframes(nframes)

    if framerate != 8000 or sampwidth != 2:
        raise ValueError(f"WAV must be 8kHz 16-bit (got {framerate}Hz {sampwidth*8}bit)")

    num_samples = nframes * channels
    all_samples = struct.unpack(f'<{num_samples}h', raw)

    if channels == 2:
        mono = [(all_samples[i] + all_samples[i + 1]) // 2 for i in range(0, num_samples, 2)]
        all_samples = mono

    frames = []
    for i in range(0, len(all_samples), SAMPLES_PER_FRAME):
        chunk = list(all_samples[i:i + SAMPLES_PER_FRAME])
        if len(chunk) < SAMPLES_PER_FRAME:
            chunk += [0] * (SAMPLES_PER_FRAME - len(chunk))
        frames.append(bytes(encode_pcmu(s) for s in chunk))
    return frames


def make_rtp_packet(seq, ts, ssrc, payload):
    first_byte = (2 << 6) | (PCMU_PAYLOAD_TYPE & 0x7F)
    header = struct.pack('!BBHII', first_byte, 0, seq & 0xFFFF, ts & 0xFFFFFFFF, ssrc & 0xFFFFFFFF)
    return header + payload


def send_rtp_to_relay(relay_addr, wav_frames, duration_sec):
    """向指定 relay 地址发送 RTP 音频"""
    rtp_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    rtp_sock.settimeout(0.1)
    try:
        ssrc = random.randint(10000, 99999)
        n_frames = len(wav_frames)
        frame_idx = random.randint(0, max(0, n_frames - 1))
        seq = 0
        ts = 0
        interval = 1.0 / RTP_PPS
        deadline = time.time() + duration_sec + 0.5
        next_send = time.time()
        count = 0

        while time.time() < deadline:
            frame = wav_frames[frame_idx % n_frames]
            frame_idx += 1
            pkt = make_rtp_packet(seq, ts, ssrc, frame)
            try:
                rtp_sock.sendto(pkt, relay_addr)
            except OSError:
                break
            count += 1
            seq = (seq + 1) & 0xFFFF
            ts = (ts + SAMPLES_PER_FRAME) & 0xFFFFFFFF
            next_send += interval
            sleep_time = next_send - time.time()
            if sleep_time > 0:
                time.sleep(sleep_time)
        return count
    finally:
        rtp_sock.close()


def parse_sdp_endpoint(body):
    conn = None
    port = None
    for line in body.splitlines():
        line = line.strip()
        if line.startswith('c=IN IP'):
            parts = line.split()
            if len(parts) >= 3:
                conn = parts[2]
        elif line.startswith('m=audio'):
            parts = line.split()
            if len(parts) >= 2:
                port = int(parts[1])
    return conn, port


def rtp_sender_worker(wav_frames, duration_sec, relay_queue, stats):
    """工作线程：从队列中取出 relay 地址并发送 RTP"""
    while True:
        item = relay_queue.get()
        if item is None:
            break
        relay_addr, dur = item
        try:
            count = send_rtp_to_relay(relay_addr, wav_frames, dur)
            with stats['lock']:
                stats['rtp_packets'] += count
                stats['rtp_calls'] += 1
        except Exception as e:
            with stats['lock']:
                stats['errors'] += 1


LOCAL_IP = '127.0.0.1'

def main():
    if len(sys.argv) < 4:
        print(f'Usage: {sys.argv[0]} <sip_port> <wav_file> <max_workers>')
        sys.exit(1)

    sip_port = int(sys.argv[1])
    wav_file = sys.argv[2]
    max_workers = int(sys.argv[3])

    print(f'RTP Sender: port={sip_port}, wav={wav_file}, workers={max_workers}')
    wav_frames = load_pcmu_frames(wav_file)
    print(f'  Loaded {len(wav_frames)} frames ({len(wav_frames)*20}ms)')

    # 共享队列
    relay_queue = queue.Queue(maxsize=max_workers * 2)
    stats = {'lock': threading.Lock(), 'rtp_packets': 0, 'rtp_calls': 0, 'errors': 0}

    # 启动工作线程
    workers = []
    for i in range(max_workers):
        t = threading.Thread(target=rtp_sender_worker, args=(wav_frames, 10, relay_queue, stats), daemon=True)
        t.start()
        workers.append(t)

    # 监听所有接口的 SIP 消息
    sip_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sip_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sip_sock.bind(('0.0.0.0', 0))
    sip_sock.settimeout(0.5)

    my_port = sip_sock.getsockname()[1]
    print(f'RTP Sender listening on port {my_port}')
    print(f'  Sending RTP to sip-edge relay ports (30000-60000)')
    print(f'Press Ctrl+C to stop')

    # 同时向 sip-edge relay 端口范围发送 RTP（被动模式）
    # sip-edge relay 端口范围: 30000-60000, 偶数=RTP, 奇数=RTCP
    relay_base = 30000
    relay_max = 60000
    current_relay = relay_base

    try:
        while True:
            try:
                data, addr = sip_sock.recvfrom(65535)
                msg = data.decode('utf-8', errors='replace')

                if 'SIP/2.0 200' in msg and 'm=audio' in msg:
                    _, body = msg.split('\r\n\r\n', 1) if '\r\n\r\n' in msg else (msg, '')
                    conn, port = parse_sdp_endpoint(body)
                    if conn and port:
                        relay_queue.put(((conn, port), 8), block=False)
                        print(f'  RTP → {conn}:{port}')

            except socket.timeout:
                # 被动模式：向当前 relay 端口发送 RTP
                if relay_queue.empty():
                    relay_addr = (LOCAL_IP, current_relay)
                    relay_queue.put((relay_addr, 3), block=False)
                    current_relay += 2
                    if current_relay >= relay_max:
                        current_relay = relay_base
                continue
            except KeyboardInterrupt:
                break
    except KeyboardInterrupt:
        pass
    finally:
        for _ in workers:
            relay_queue.put(None)
        for t in workers:
            t.join(timeout=2)

        print(f'\nStats: {stats["rtp_calls"]} calls, {stats["rtp_packets"]} packets, {stats["errors"]} errors')


if __name__ == '__main__':
    import queue
    main()
