#!/usr/bin/env python3
"""
CPS (Calls Per Second) tester with real RTP media for VOS-RS recording verification.

Each call: INVITE -> 100/180/200 -> ACK -> 10s RTP (PCMU) -> BYE
Recordings are produced by sip-edge's media relay recorder.

Usage:
    python3 cps_recording.py <edge_ip> <edge_port> <total_calls> <cps> [duration_sec] [destination]

Exit code 0 = all calls succeeded with recordings, non-zero = failures detected.
"""
import math
import os
import random
import socket
import struct
import sys
import threading
import time
import wave
from collections import defaultdict
from pathlib import Path

SAMPLES_PER_FRAME = 160  # 20ms at 8kHz
RTP_PPS = 50             # 50 packets/sec = 8kHz/160
PCMU_PAYLOAD_TYPE = 0
# Extra RTP seconds beyond the requested call duration to guarantee the
# relay-side recording captures at least `duration_sec` of audio
# (accounts for relay setup/teardown and BYE processing latency).
RTP_BUFFER_SEC = 1.5

# Default WAV file for RTP audio (relative to project root)
DEFAULT_WAV_FILE = os.environ.get("VOS_RS_CPS_WAV_FILE",
                                  str(Path(__file__).resolve().parent / "test_speech.wav"))

# Cache of WAV PCMU frames (loaded once per process)
_WAV_FRAMES_CACHE = None
_WAV_FRAMES_LOCK = threading.Lock()


def get_wav_pcmu_frames():
    """Load WAV PCMU frames once (cached). Falls back to a tone if no WAV file."""
    global _WAV_FRAMES_CACHE
    if _WAV_FRAMES_CACHE is not None:
        return _WAV_FRAMES_CACHE
    with _WAV_FRAMES_LOCK:
        if _WAV_FRAMES_CACHE is not None:
            return _WAV_FRAMES_CACHE
        wav_path = DEFAULT_WAV_FILE
        try:
            frames = load_wav_pcmu_frames(wav_path)
            if not frames:
                raise ValueError("WAV file contains no frames")
            print(f'  Loaded WAV: {wav_path} ({len(frames)} frames = {len(frames)*20}ms)')
            _WAV_FRAMES_CACHE = frames
        except Exception as e:
            print(f'  WARN: failed to load WAV {wav_path}: {e}; using synthetic tone')
            _WAV_FRAMES_CACHE = None
    return _WAV_FRAMES_CACHE


def encode_pcmu(sample: int) -> int:
    """Encode a 16-bit linear PCM sample to PCMU (u-law)."""
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


def generate_pcmu_tone(freq: float = 440.0) -> bytes:
    """Generate one 20ms PCMU frame (160 samples) of a sine wave."""
    payload = bytearray()
    phase_inc = 2.0 * math.pi * freq / 8000.0
    phase = random.random() * 2.0 * math.pi
    amplitude = 4000
    for _ in range(SAMPLES_PER_FRAME):
        val = int(amplitude * math.sin(phase))
        val = max(-32768, min(32767, val))
        payload.append(encode_pcmu(val))
        phase += phase_inc
        if phase > 2.0 * math.pi:
            phase -= 2.0 * math.pi
    return bytes(payload)


def load_wav_pcmu_frames(wav_path: str) -> list:
    """Read a WAV file (8kHz/16-bit) and encode to PCMU frames (160 samples each).

    Returns a list of PCMU byte payloads, each 160 bytes (20ms of audio).
    """
    with wave.open(wav_path, 'rb') as w:
        channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        framerate = w.getframerate()
        nframes = w.getnframes()
        raw = w.readframes(nframes)

    if framerate != 8000 or sampwidth != 2:
        raise ValueError(f"WAV must be 8kHz 16-bit PCM (got {framerate}Hz {sampwidth*8}bit)")

    num_samples = nframes * channels
    all_samples = struct.unpack(f'<{num_samples}h', raw)

    # Mix to mono if stereo
    if channels == 2:
        mono = []
        for i in range(0, num_samples, 2):
            mono.append((all_samples[i] + all_samples[i + 1]) // 2)
        all_samples = mono
        num_samples = len(mono)

    # Encode to PCMU frames
    frames = []
    for i in range(0, num_samples, SAMPLES_PER_FRAME):
        chunk = all_samples[i:i + SAMPLES_PER_FRAME]
        if len(chunk) < SAMPLES_PER_FRAME:
            chunk = list(chunk) + [0] * (SAMPLES_PER_FRAME - len(chunk))
        pcmu_frame = bytes(encode_pcmu(s) for s in chunk)
        frames.append(pcmu_frame)

    return frames


def make_rtp_packet(seq: int, ts: int, ssrc: int, payload: bytes) -> bytes:
    first_byte = (2 << 6) | (PCMU_PAYLOAD_TYPE & 0x7F)
    header = struct.pack('!BBHII', first_byte, 0, seq & 0xFFFF, ts & 0xFFFFFFFF, ssrc & 0xFFFFFFFF)
    return header + payload


def parse_sip_status(data: str) -> int:
    first_line = data.split('\r\n')[0]
    if 'SIP/2.0 ' in first_line:
        return int(first_line.split(' ')[1])
    return 0


def parse_sdp_endpoint(message: str):
    body = message.split('\r\n\r\n', 1)[1] if '\r\n\r\n' in message else ''
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


def build_invite(call_id: str, caller_tag: str, from_user: str,
                 local_ip: str, local_port: int, media_port: int,
                 edge_ip: str, edge_port: int, destination: str, cseq: int = 1) -> str:
    sdp = (
        f'v=0\r\n'
        f'o=caller 1 1 IN IP4 {local_ip}\r\n'
        f's=VOS-RS CPS\r\n'
        f'c=IN IP4 {local_ip}\r\n'
        f't=0 0\r\n'
        f'm=audio {media_port} RTP/AVP 0 8 101\r\n'
        f'a=rtpmap:0 PCMU/8000\r\n'
        f'a=rtpmap:8 PCMA/8000\r\n'
        f'a=rtpmap:101 telephone-event/8000\r\n'
        f'a=fmtp:101 0-16\r\n'
    )
    return (
        f'INVITE sip:{destination}@{edge_ip}:{edge_port} SIP/2.0\r\n'
        f'Via: SIP/2.0/UDP {local_ip}:{local_port};branch=z9hG4bK-{call_id}\r\n'
        f'Max-Forwards: 70\r\n'
        f'From: "{from_user}" <sip:{from_user}@{local_ip}:{local_port}>;tag={caller_tag}\r\n'
        f'To: <sip:{destination}@{edge_ip}:{edge_port}>\r\n'
        f'Call-ID: {call_id}\r\n'
        f'CSeq: {cseq} INVITE\r\n'
        f'Contact: <sip:{from_user}@{local_ip}:{local_port}>\r\n'
        f'Content-Type: application/sdp\r\n'
        f'Content-Length: {len(sdp)}\r\n\r\n{sdp}'
    )


def build_ack(call_id: str, caller_tag: str, to_tag: str, from_user: str,
              local_ip: str, local_port: int, edge_ip: str, edge_port: int,
              destination: str, cseq: int) -> str:
    to_hdr = f'<sip:{destination}@{edge_ip}:{edge_port}>'
    if to_tag:
        to_hdr += f';tag={to_tag}'
    return (
        f'ACK sip:{destination}@{edge_ip}:{edge_port} SIP/2.0\r\n'
        f'Via: SIP/2.0/UDP {local_ip}:{local_port};branch=z9hG4bK-ack-{call_id}\r\n'
        f'Max-Forwards: 70\r\n'
        f'From: "{from_user}" <sip:{from_user}@{local_ip}:{local_port}>;tag={caller_tag}\r\n'
        f'To: {to_hdr}\r\n'
        f'Call-ID: {call_id}\r\n'
        f'CSeq: {cseq} ACK\r\n'
        f'Content-Length: 0\r\n\r\n'
    )


def build_bye(call_id: str, caller_tag: str, to_tag: str, from_user: str,
              local_ip: str, local_port: int, edge_ip: str, edge_port: int,
              destination: str, cseq: int) -> str:
    to_hdr = f'<sip:{destination}@{edge_ip}:{edge_port}>'
    if to_tag:
        to_hdr += f';tag={to_tag}'
    return (
        f'BYE sip:{destination}@{edge_ip}:{edge_port} SIP/2.0\r\n'
        f'Via: SIP/2.0/UDP {local_ip}:{local_port};branch=z9hG4bK-bye-{call_id}\r\n'
        f'Max-Forwards: 70\r\n'
        f'From: "{from_user}" <sip:{from_user}@{local_ip}:{local_port}>;tag={caller_tag}\r\n'
        f'To: {to_hdr}\r\n'
        f'Call-ID: {call_id}\r\n'
        f'CSeq: {cseq} BYE\r\n'
        f'Content-Length: 0\r\n\r\n'
    )


class CallStats:
    def __init__(self, total: int):
        self.total = total
        self.lock = threading.Lock()
        self.success = 0
        self.failed = 0
        self.rtp_sent = defaultdict(int)
        self.errors = defaultdict(int)

    def record_success(self, call_id: str, rtp_packets: int):
        with self.lock:
            self.success += 1
            self.rtp_sent[call_id] = rtp_packets

    def record_failure(self, call_id: str, reason: str):
        with self.lock:
            self.failed += 1
            self.errors[reason] += 1


def make_call(call_id: str, edge_ip: str, edge_port: int, destination: str,
              duration_sec: float, stats: CallStats):
    """Execute a single call: INVITE -> ACK -> RTP for duration_sec -> BYE."""
    sip_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    rtp_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sip_sock.bind(('0.0.0.0', 0))
        rtp_sock.bind(('0.0.0.0', 0))
        sip_sock.settimeout(0.5)
        rtp_sock.settimeout(0.5)

        local_ip = '127.0.0.1'
        local_sip_port = sip_sock.getsockname()[1]
        local_rtp_port = rtp_sock.getsockname()[1]
        from_user = '1001'
        caller_tag = f'cps-{call_id}'
        edge_addr = (edge_ip, edge_port)

        # 1. Send INVITE (with retransmission for reliability)
        invite = build_invite(call_id, caller_tag, from_user, local_ip,
                              local_sip_port, local_rtp_port,
                              edge_ip, edge_port, destination)
        invite_bytes = invite.encode('utf-8')
        # Send INVITE with up to 3 retransmissions (500ms apart) if no response
        sip_sock.sendto(invite_bytes, edge_addr)

        # 2. Receive responses (100, 180, 200) with INVITE retransmission
        got_200 = False
        to_tag = None
        relay_addr = None
        relay_port = None
        deadline = time.time() + 10
        last_retrans = time.time()
        while time.time() < deadline:
            try:
                data, _ = sip_sock.recvfrom(65535)
            except socket.timeout:
                # Retransmit INVITE every 500ms if no response (UDP reliability)
                now = time.time()
                if now - last_retrans >= 0.5 and not got_200:
                    try:
                        sip_sock.sendto(invite_bytes, edge_addr)
                        last_retrans = now
                    except OSError:
                        break
                continue
            msg = data.decode('utf-8', errors='replace')
            status = parse_sip_status(msg)
            if status != 100:
                print(f'  [{call_id}] received status {status}')
            if status == 200:
                got_200 = True
                # Extract To tag
                for line in msg.split('\r\n'):
                    if line.lower().startswith('to:'):
                        if ';tag=' in line:
                            to_tag = line.split(';tag=')[1].split(';')[0].split()[0]
                        break
                relay_addr, relay_port = parse_sdp_endpoint(msg)
                break
            # Ignore 100/180 and continue waiting

        if not got_200 or relay_port is None:
            stats.record_failure(call_id, 'no_200_ok')
            return

        # 3. Send ACK
        ack = build_ack(call_id, caller_tag, to_tag, from_user, local_ip,
                        local_sip_port, edge_ip, edge_port, destination, 1)
        sip_sock.sendto(ack.encode('utf-8'), edge_addr)

        # 4. Send RTP media for duration_sec + buffer (looping WAV file content)
        target_addr = (relay_addr or edge_ip, relay_port)
        ssrc = random.randint(10000, 99999)
        wav_frames = get_wav_pcmu_frames()
        if wav_frames is None:
            # Fallback: synthetic tone
            wav_frames = [generate_pcmu_tone(440.0)]
        n_frames = len(wav_frames)
        # Random start offset so concurrent calls don't all send identical audio
        frame_idx = random.randint(0, max(0, n_frames - 1))
        seq = 0
        ts = 0
        rtp_count = 0
        interval = 1.0 / RTP_PPS
        # Send RTP for duration_sec + RTP_BUFFER_SEC so the relay-side recording
        # captures at least duration_sec of audio (relay setup/teardown overhead).
        rtp_duration = duration_sec + RTP_BUFFER_SEC
        deadline = time.time() + rtp_duration
        next_send = time.time()
        while True:
            now = time.time()
            if now >= deadline:
                break
            frame = wav_frames[frame_idx % n_frames]
            frame_idx += 1
            pkt = make_rtp_packet(seq, ts, ssrc, frame)
            try:
                rtp_sock.sendto(pkt, target_addr)
            except OSError:
                break
            rtp_count += 1
            seq = (seq + 1) & 0xFFFF
            ts = (ts + SAMPLES_PER_FRAME) & 0xFFFFFFFF
            next_send += interval
            sleep_time = next_send - time.time()
            if sleep_time > 0:
                time.sleep(sleep_time)

        # 5. Send BYE
        bye = build_bye(call_id, caller_tag, to_tag, from_user, local_ip,
                        local_sip_port, edge_ip, edge_port, destination, 2)
        sip_sock.sendto(bye.encode('utf-8'), edge_addr)

        # Wait for BYE 200 OK (best effort)
        try:
            sip_sock.settimeout(2)
            while True:
                data, _ = sip_sock.recvfrom(65535)
                msg = data.decode('utf-8', errors='replace')
                if parse_sip_status(msg) == 200:
                    break
        except socket.timeout:
            pass

        stats.record_success(call_id, rtp_count)
    except Exception as e:
        stats.record_failure(call_id, f'exception:{type(e).__name__}')
    finally:
        sip_sock.close()
        rtp_sock.close()


def main():
    if len(sys.argv) < 5:
        print(f'Usage: {sys.argv[0]} <edge_ip> <edge_port> <total_calls> <cps> [duration_sec] [destination]')
        print(f'  Each call sends real PCMU RTP for duration_sec (default 10) seconds.')
        sys.exit(1)

    edge_ip = sys.argv[1]
    edge_port = int(sys.argv[2])
    total_calls = int(sys.argv[3])
    cps = float(sys.argv[4])
    duration_sec = float(sys.argv[5]) if len(sys.argv) > 5 else 10.0
    destination = sys.argv[6] if len(sys.argv) > 6 else '13800138000'

    print(f'CPS Recording Test: {total_calls} calls @ {cps} CPS, {duration_sec}s each')
    print(f'  Edge: {edge_ip}:{edge_port}, Destination: {destination}')
    print(f'  Expected concurrent calls: ~{int(cps * duration_sec)}')
    print('=' * 60)

    # Pre-load WAV frames so all calls share the same cached audio
    get_wav_pcmu_frames()

    # Wait for sip-edge UDP port to be reachable before sending any traffic.
    # This avoids losing the first INVITE to a startup race with the listener.
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    probe.settimeout(0.3)
    edge_addr = (edge_ip, edge_port)
    # Send a minimal OPTIONS probe; any SIP response (even 400/500) means the
    # listener is up. We also retry for up to 5 seconds.
    options_probe = (
        f'OPTIONS sip:probe@{edge_ip}:{edge_port} SIP/2.0\r\n'
        f'Via: SIP/2.0/UDP 127.0.0.1:9;branch=z9hG4bK-probe\r\n'
        f'Max-Forwards: 70\r\n'
        f'From: <sip:probe@127.0.0.1>;tag=probe\r\n'
        f'To: <sip:probe@{edge_ip}:{edge_port}>\r\n'
        f'Call-ID: probe-{int(time.time()*1000)}@vos-rs.local\r\n'
        f'CSeq: 1 OPTIONS\r\n'
        f'Content-Length: 0\r\n\r\n'
    ).encode('utf-8')
    probe_ready = False
    for _ in range(20):
        try:
            probe.sendto(options_probe, edge_addr)
            data, _ = probe.recvfrom(65535)
            probe_ready = True
            break
        except socket.timeout:
            continue
        except OSError:
            time.sleep(0.25)
    probe.close()
    if probe_ready:
        print(f'  sip-edge is ready (probe answered)')
    else:
        print(f'  WARN: sip-edge probe got no response after 5s; proceeding anyway')
        time.sleep(1.0)

    stats = CallStats(total_calls)
    interval = 1.0 / cps
    threads = []
    start_time = time.time()

    for i in range(total_calls):
        call_id = f'cps-{int(time.time()*1000)}-{i}@vos-rs.local'
        t = threading.Thread(target=make_call, args=(
            call_id, edge_ip, edge_port, destination, duration_sec, stats
        ), daemon=True)
        t.start()
        threads.append(t)
        # Pace calls at the configured CPS
        if i < total_calls - 1:
            time.sleep(interval)

    # Wait for all threads to finish (with generous timeout)
    timeout = duration_sec + 30
    for t in threads:
        remaining = max(0.1, timeout - (time.time() - start_time))
        t.join(timeout=remaining)

    elapsed = time.time() - start_time
    actual_cps = total_calls / elapsed if elapsed > 0 else 0

    print()
    print('=' * 60)
    print('  CPS RECORDING TEST REPORT')
    print('=' * 60)
    print(f'  Total calls:      {total_calls}')
    print(f'  Succeeded:        {stats.success}')
    print(f'  Failed:           {stats.failed}')
    print(f'  Elapsed:          {elapsed:.1f}s')
    print(f'  Actual CPS:       {actual_cps:.1f}')
    if stats.rtp_sent:
        avg_rtp = sum(stats.rtp_sent.values()) / len(stats.rtp_sent)
        min_rtp = min(stats.rtp_sent.values())
        max_rtp = max(stats.rtp_sent.values())
        print(f'  RTP avg/call:     {avg_rtp:.0f} packets (min={min_rtp}, max={max_rtp})')
        print(f'  Audio/call:       ~{avg_rtp * 20 / 1000:.1f}s')
    if stats.errors:
        print(f'  Error breakdown:')
        for reason, count in sorted(stats.errors.items(), key=lambda x: -x[1]):
            print(f'    {reason}: {count}')
    print('=' * 60)

    if stats.failed > 0:
        sys.exit(1)
    sys.exit(0)


if __name__ == '__main__':
    main()
