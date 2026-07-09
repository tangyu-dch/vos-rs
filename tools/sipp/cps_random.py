#!/usr/bin/env python3
"""
CPS tester with random duration and random answer/no-answer scenarios.

Each call randomly either:
  - Answers (INVITE → 200 → ACK → RTP → BYE) with random duration (3-15s)
  - No answer (INVITE → timeout/no 200 → CANCEL or just timeout)

Generates varied CDR records: answered calls with recordings, and failed/no-answer calls.
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

SAMPLES_PER_FRAME = 160
RTP_PPS = 50
PCMU_PAYLOAD_TYPE = 0
RTP_BUFFER_SEC = 0.5
MIN_DURATION = 3
MAX_DURATION = 15
ANSWER_RATE = 0.7  # 70% answered, 30% no-answer

DEFAULT_WAV_FILE = os.environ.get("VOS_RS_CPS_WAV_FILE",
                                  str(Path(__file__).resolve().parent / "test_speech.wav"))

_WAV_FRAMES_CACHE = None
_WAV_FRAMES_LOCK = threading.Lock()


def get_wav_pcmu_frames():
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
            print(f'  Loaded WAV: {wav_path} ({len(frames)} frames)')
            _WAV_FRAMES_CACHE = frames
        except Exception as e:
            print(f'  WARN: failed to load WAV {wav_path}: {e}; using synthetic tone')
            _WAV_FRAMES_CACHE = None
    return _WAV_FRAMES_CACHE


def encode_pcmu(sample: int) -> int:
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

    if channels == 2:
        mono = []
        for i in range(0, num_samples, 2):
            mono.append((all_samples[i] + all_samples[i + 1]) // 2)
        all_samples = mono
        num_samples = len(mono)

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


def build_invite(call_id, caller_tag, from_user, local_ip, local_port, media_port,
                 edge_ip, edge_port, destination, cseq=1, direction='outbound'):
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
        f'X-Call-Direction: {direction}\r\n'
        f'Contact: <sip:{from_user}@{local_ip}:{local_port}>\r\n'
        f'Content-Type: application/sdp\r\n'
        f'Content-Length: {len(sdp)}\r\n\r\n{sdp}'
    )


def build_ack(call_id, caller_tag, to_tag, from_user, local_ip, local_port,
              edge_ip, edge_port, destination, cseq):
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


def build_bye(call_id, caller_tag, to_tag, from_user, local_ip, local_port,
              edge_ip, edge_port, destination, cseq):
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


def build_cancel(call_id, caller_tag, from_user, local_ip, local_port,
                 edge_ip, edge_port, destination):
    return (
        f'CANCEL sip:{destination}@{edge_ip}:{edge_port} SIP/2.0\r\n'
        f'Via: SIP/2.0/UDP {local_ip}:{local_port};branch=z9hG4bK-{call_id}\r\n'
        f'Max-Forwards: 70\r\n'
        f'From: "{from_user}" <sip:{from_user}@{local_ip}:{local_port}>;tag={caller_tag}\r\n'
        f'To: <sip:{destination}@{edge_ip}:{edge_port}>\r\n'
        f'Call-ID: {call_id}\r\n'
        f'CSeq: 1 CANCEL\r\n'
        f'Content-Length: 0\r\n\r\n'
    )


class CallStats:
    def __init__(self, total):
        self.total = total
        self.lock = threading.Lock()
        self.answered = 0
        self.no_answer = 0
        self.failed = 0
        self.durations = []
        self.errors = defaultdict(int)

    def record_answered(self, call_id, duration, rtp_packets):
        with self.lock:
            self.answered += 1
            self.durations.append(duration)

    def record_no_answer(self, call_id, reason):
        with self.lock:
            self.no_answer += 1
            self.errors[reason] += 1

    def record_failure(self, call_id, reason):
        with self.lock:
            self.failed += 1
            self.errors[reason] += 1


def make_call_no_answer(call_id, edge_ip, edge_port, destination, stats, direction='outbound'):
    """发送 INVITE 后不接通（模拟未接听场景）：收到 180 后发 CANCEL"""
    sip_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sip_sock.bind(('0.0.0.0', 0))
        sip_sock.settimeout(0.5)

        local_ip = '127.0.0.1'
        local_sip_port = sip_sock.getsockname()[1]
        from_user = '1001'
        caller_tag = f'cps-{call_id}'
        edge_addr = (edge_ip, edge_port)

        invite = build_invite(call_id, caller_tag, from_user, local_ip,
                              local_sip_port, 0, edge_ip, edge_port, destination, direction=direction)
        invite_bytes = invite.encode('utf-8')
        sip_sock.sendto(invite_bytes, edge_addr)

        got_180 = False
        deadline = time.time() + 5
        last_retrans = time.time()
        while time.time() < deadline:
            try:
                data, _ = sip_sock.recvfrom(65535)
                msg = data.decode('utf-8', errors='replace')
                status = parse_sip_status(msg)
                if status == 100:
                    continue
                if status >= 180:
                    got_180 = True
                    # 收到 180 后发 CANCEL 模拟未接听
                    break
            except socket.timeout:
                now = time.time()
                if now - last_retrans >= 0.5:
                    try:
                        sip_sock.sendto(invite_bytes, edge_addr)
                        last_retrans = now
                    except OSError:
                        break
                continue

        # 发送 CANCEL
        try:
            cancel = build_cancel(call_id, caller_tag, from_user, local_ip,
                                  local_sip_port, edge_ip, edge_port, destination)
            sip_sock.sendto(cancel.encode('utf-8'), edge_addr)
        except OSError:
            pass

        reason = 'got_180_cancel' if got_180 else 'timeout_cancel'
        stats.record_no_answer(call_id, reason)
    except Exception as e:
        stats.record_failure(call_id, f'exception:{type(e).__name__}')
    finally:
        sip_sock.close()


def make_call_answered(call_id, edge_ip, edge_port, destination, duration_sec, stats, direction='outbound'):
    """接通呼叫，发送 RTP 媒体，持续 duration_sec 秒后挂断"""
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

        invite = build_invite(call_id, caller_tag, from_user, local_ip,
                              local_sip_port, local_rtp_port,
                              edge_ip, edge_port, destination, direction=direction)
        invite_bytes = invite.encode('utf-8')
        sip_sock.sendto(invite_bytes, edge_addr)

        got_200 = False
        to_tag = None
        relay_addr = None
        relay_port = None
        deadline = time.time() + 30
        last_retrans = time.time()
        while time.time() < deadline:
            try:
                data, _ = sip_sock.recvfrom(65535)
            except socket.timeout:
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
            if status == 200:
                got_200 = True
                for line in msg.split('\r\n'):
                    if line.lower().startswith('to:'):
                        if ';tag=' in line:
                            to_tag = line.split(';tag=')[1].split(';')[0].split()[0]
                        break
                relay_addr, relay_port = parse_sdp_endpoint(msg)
                break

        if not got_200 or relay_port is None:
            stats.record_failure(call_id, 'no_200_ok')
            return

        ack = build_ack(call_id, caller_tag, to_tag, from_user, local_ip,
                        local_sip_port, edge_ip, edge_port, destination, 1)
        sip_sock.sendto(ack.encode('utf-8'), edge_addr)

        target_addr = (relay_addr or edge_ip, relay_port)
        ssrc = random.randint(10000, 99999)
        wav_frames = get_wav_pcmu_frames()
        if wav_frames is None:
            wav_frames = [generate_pcmu_tone(440.0)]
        n_frames = len(wav_frames)
        frame_idx = random.randint(0, max(0, n_frames - 1))
        seq = 0
        ts = 0
        rtp_count = 0
        interval = 1.0 / RTP_PPS
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

        bye = build_bye(call_id, caller_tag, to_tag, from_user, local_ip,
                        local_sip_port, edge_ip, edge_port, destination, 2)
        sip_sock.sendto(bye.encode('utf-8'), edge_addr)

        try:
            sip_sock.settimeout(2)
            while True:
                data, _ = sip_sock.recvfrom(65535)
                msg = data.decode('utf-8', errors='replace')
                if parse_sip_status(msg) == 200:
                    break
        except socket.timeout:
            pass

        stats.record_answered(call_id, duration_sec, rtp_count)
    except Exception as e:
        stats.record_failure(call_id, f'exception:{type(e).__name__}')
    finally:
        sip_sock.close()
        rtp_sock.close()


def main():
    if len(sys.argv) < 5:
        print(f'Usage: {sys.argv[0]} <edge_ip> <edge_port> <total_calls> <cps> [answer_rate] [inbound_rate] [destination]')
        print(f'  Random duration ({MIN_DURATION}-{MAX_DURATION}s), random answer/no-answer')
        sys.exit(1)

    edge_ip = sys.argv[1]
    edge_port = int(sys.argv[2])
    total_calls = int(sys.argv[3])
    cps = float(sys.argv[4])
    answer_rate = float(sys.argv[5]) if len(sys.argv) > 5 else ANSWER_RATE
    inbound_rate = float(sys.argv[6]) if len(sys.argv) > 6 else 0.4
    destination = sys.argv[7] if len(sys.argv) > 7 else '13800138000'

    print(f'CPS Random Test: {total_calls} calls @ {cps} CPS')
    print(f'  Answer rate: {answer_rate*100:.0f}%, Duration: {MIN_DURATION}-{MAX_DURATION}s random')
    print(f'  Direction: {inbound_rate*100:.0f}% inbound, {(1-inbound_rate)*100:.0f}% outbound')
    print(f'  Edge: {edge_ip}:{edge_port}, Destination: {destination}')
    print(f'  Expected answered: ~{int(total_calls * answer_rate)}, no-answer: ~{int(total_calls * (1 - answer_rate))}')
    print('=' * 60)

    get_wav_pcmu_frames()

    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    probe.settimeout(0.3)
    edge_addr = (edge_ip, edge_port)
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
        print(f'  sip-edge is ready')
    else:
        print(f'  WARN: sip-edge probe got no response; proceeding anyway')
        time.sleep(1.0)

    stats = CallStats(total_calls)
    interval = 1.0 / cps
    threads = []
    start_time = time.time()
    max_concurrent = min(50, total_calls)
    semaphore = threading.Semaphore(max_concurrent)

    def run_with_limit(func, *args):
        semaphore.acquire()
        try:
            func(*args)
        finally:
            semaphore.release()

    for i in range(total_calls):
        call_id = f'cps-{int(time.time()*1000)}-{i}@vos-rs.local'
        will_answer = random.random() < answer_rate
        direction = 'inbound' if random.random() < inbound_rate else 'outbound'
        if will_answer:
            dur = random.uniform(MIN_DURATION, MAX_DURATION)
            t = threading.Thread(target=run_with_limit, args=(make_call_answered, call_id, edge_ip, edge_port, destination, dur, stats, direction), daemon=True)
        else:
            t = threading.Thread(target=run_with_limit, args=(make_call_no_answer, call_id, edge_ip, edge_port, destination, stats, direction), daemon=True)
        t.start()
        threads.append(t)
        if i < total_calls - 1:
            time.sleep(interval)

    timeout = max(MAX_DURATION + 30, total_calls / max(1, cps) + MAX_DURATION + 30)
    for t in threads:
        remaining = max(0.1, timeout - (time.time() - start_time))
        t.join(timeout=remaining)

    elapsed = time.time() - start_time

    print()
    print('=' * 60)
    print('  CPS RANDOM TEST REPORT')
    print('=' * 60)
    print(f'  Total calls:    {total_calls}')
    print(f'  Answered:       {stats.answered}')
    print(f'  No answer:      {stats.no_answer}')
    print(f'  Failed:         {stats.failed}')
    print(f'  Elapsed:        {elapsed:.1f}s')
    if stats.durations:
        print(f'  Duration range: {min(stats.durations):.1f}s - {max(stats.durations):.1f}s')
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
