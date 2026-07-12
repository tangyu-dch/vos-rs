#!/usr/bin/env python3
"""
VOS-RS Stress Test: 10 minutes of variable CPS (10-200), random call duration (10-120s),
random answer rate (50-90%), WAV audio playback, random caller/callee hangup.

Features:
  - CPS random walk between 10-200
  - Call duration uniformly random 10-120s
  - Answer rate random walk between 50%-90%
  - Caller-initiated or callee-initiated BYE (random)
  - WAV file resampled from 44100Hz to 8kHz PCMU and sent as RTP
  - Continuous operation for configurable duration (default 10 min)

Usage:
  python3 stress_test.py <edge_ip> <edge_port> [duration_sec] [wav_file]
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
RTP_PPS = 50
PCMU_PAYLOAD_TYPE = 0
RTP_BUFFER_SEC = 0.5

CPS_MIN = 10.0
CPS_MAX = 200.0
CPS_STEP = 5.0
CPS_CHANGE_INTERVAL = 3.0  # seconds between CPS changes

ANSWER_RATE_MIN = 0.5
ANSWER_RATE_MAX = 0.9
ANSWER_RATE_STEP = 0.05
ANSWER_RATE_CHANGE_INTERVAL = 10.0

MIN_DURATION = 10
MAX_DURATION = 120
MIN_RING_DURATION = 3
MAX_RING_DURATION = 10

# stress_test.py 位于 tools/sipp/，默认音频位于 tools/ 根目录。
# 旧路径误拼成 tools/tools，导致未显式传参时无法加载 WAV。
DEFAULT_WAV_FILE = str(Path(__file__).resolve().parent.parent / "sample-speech-1m.wav")

_WAV_FRAMES_CACHE = None
_WAV_FRAMES_LOCK = threading.Lock()


def resample_linear(samples, src_rate, dst_rate):
    """Simple linear interpolation resampler."""
    if src_rate == dst_rate:
        return samples
    ratio = src_rate / dst_rate
    num_out = int(len(samples) / ratio)
    out = []
    for i in range(num_out):
        pos = i * ratio
        idx = int(pos)
        frac = pos - idx
        if idx + 1 < len(samples):
            val = samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        else:
            val = samples[idx] if idx < len(samples) else 0
        out.append(int(max(-32768, min(32767, val))))
    return out


def get_wav_pcmu_frames():
    global _WAV_FRAMES_CACHE
    if _WAV_FRAMES_CACHE is not None:
        return _WAV_FRAMES_CACHE
    with _WAV_FRAMES_LOCK:
        if _WAV_FRAMES_CACHE is not None:
            return _WAV_FRAMES_CACHE
        wav_path = DEFAULT_WAV_FILE
        env_path = os.environ.get("VOS_RS_CPS_WAV_FILE")
        if env_path and os.path.exists(env_path):
            wav_path = env_path
        try:
            frames = load_wav_pcmu_frames(wav_path)
            if not frames:
                raise ValueError("WAV file contains no frames")
            print(f'  Loaded WAV: {wav_path} ({len(frames)} frames, {len(frames)*0.02:.1f}s at 8kHz)')
            _WAV_FRAMES_CACHE = frames
        except Exception as e:
            print(f'  WARN: failed to load WAV {wav_path}: {e}; using synthetic tone')
            _WAV_FRAMES_CACHE = None
    return _WAV_FRAMES_CACHE


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


def generate_pcmu_tone(freq=440.0):
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


def load_wav_pcmu_frames(wav_path):
    with wave.open(wav_path, 'rb') as w:
        channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        framerate = w.getframerate()
        nframes = w.getnframes()
        raw = w.readframes(nframes)

    num_samples = nframes * channels
    all_samples = list(struct.unpack(f'<{num_samples}h', raw))

    if channels == 2:
        mono = []
        for i in range(0, num_samples, 2):
            mono.append((all_samples[i] + all_samples[i + 1]) // 2)
        all_samples = mono
        num_samples = len(mono)

    if framerate != 8000:
        print(f'  Resampling WAV from {framerate}Hz to 8000Hz...')
        all_samples = resample_linear(all_samples, framerate, 8000)
        num_samples = len(all_samples)

    frames = []
    for i in range(0, num_samples, SAMPLES_PER_FRAME):
        chunk = all_samples[i:i + SAMPLES_PER_FRAME]
        if len(chunk) < SAMPLES_PER_FRAME:
            chunk = list(chunk) + [0] * (SAMPLES_PER_FRAME - len(chunk))
        pcmu_frame = bytes(encode_pcmu(s) for s in chunk)
        frames.append(pcmu_frame)

    return frames


def make_rtp_packet(seq, ts, ssrc, payload):
    first_byte = (2 << 6) | (PCMU_PAYLOAD_TYPE & 0x7F)
    header = struct.pack('!BBHII', first_byte, 0, seq & 0xFFFF, ts & 0xFFFFFFFF, ssrc & 0xFFFFFFFF)
    return header + payload


def parse_sip_status(data):
    first_line = data.split('\r\n')[0]
    if 'SIP/2.0 ' in first_line:
        return int(first_line.split(' ')[1])
    return 0


def parse_sdp_endpoint(message):
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


def build_in_dialog_ok(request):
    """Build a minimal 200 response for an in-dialog re-INVITE or UPDATE."""
    headers = {}
    for line in request.split('\r\n')[1:]:
        if not line:
            break
        name, separator, value = line.partition(':')
        if separator:
            headers.setdefault(name.strip().lower(), []).append(value.strip())
    required = ('via', 'from', 'to', 'call-id', 'cseq')
    if any(name not in headers for name in required):
        return None
    response_headers = []
    for via in headers['via']:
        response_headers.append(f'Via: {via}')
    response_headers.extend([
        f"From: {headers['from'][0]}",
        f"To: {headers['to'][0]}",
        f"Call-ID: {headers['call-id'][0]}",
        f"CSeq: {headers['cseq'][0]}",
        'Content-Length: 0',
    ])
    return 'SIP/2.0 200 OK\r\n' + '\r\n'.join(response_headers) + '\r\n\r\n'


def reply_to_in_dialog_refreshes(sip_sock):
    """Drain pending SIP requests and acknowledge session refreshes."""
    while True:
        try:
            data, peer = sip_sock.recvfrom(65535)
        except (BlockingIOError, socket.timeout):
            return
        message = data.decode('utf-8', errors='replace')
        method = message.split(' ', 1)[0]
        if method not in ('INVITE', 'UPDATE'):
            continue
        response = build_in_dialog_ok(message)
        if response is not None:
            sip_sock.sendto(response.encode('utf-8'), peer)


def build_invite(call_id, caller_tag, from_user, local_ip, local_port, media_port,
                 edge_ip, edge_port, destination, cseq=1, direction='outbound'):
    sdp = (
        f'v=0\r\n'
        f'o=caller 1 1 IN IP4 {local_ip}\r\n'
        f's=VOS-RS Stress Test\r\n'
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


class StressStats:
    def __init__(self):
        self.lock = threading.Lock()
        self.total_initiated = 0
        self.answered = 0
        self.no_answer = 0
        self.failed = 0
        self.caller_hangup = 0
        self.callee_hangup = 0
        self.durations = []
        self.errors = defaultdict(int)
        self.cps_samples = []
        self.answer_rate_samples = []
        self.start_time = time.time()

    def record_answered(self, call_id, duration, caller_hung_up):
        with self.lock:
            self.answered += 1
            self.durations.append(duration)
            if caller_hung_up:
                self.caller_hangup += 1
            else:
                self.callee_hangup += 1

    def record_no_answer(self, call_id, reason):
        with self.lock:
            self.no_answer += 1
            self.errors[reason] += 1

    def record_failure(self, call_id, reason):
        with self.lock:
            self.failed += 1
            self.errors[reason] += 1

    def record_initiated(self):
        with self.lock:
            self.total_initiated += 1

    def record_cps(self, cps):
        with self.lock:
            self.cps_samples.append((time.time() - self.start_time, cps))

    def record_answer_rate(self, rate):
        with self.lock:
            self.answer_rate_samples.append((time.time() - self.start_time, rate))

    def summary(self, elapsed):
        with self.lock:
            avg_dur = sum(self.durations) / len(self.durations) if self.durations else 0
            return {
                'total_initiated': self.total_initiated,
                'answered': self.answered,
                'no_answer': self.no_answer,
                'failed': self.failed,
                'caller_hangup': self.caller_hangup,
                'callee_hangup': self.callee_hangup,
                'avg_duration': avg_dur,
                'min_duration': min(self.durations) if self.durations else 0,
                'max_duration': max(self.durations) if self.durations else 0,
                'errors': dict(self.errors),
                'elapsed': elapsed,
                'effective_cps': self.total_initiated / max(1, elapsed),
                'answer_rate': self.answered / max(1, self.answered + self.no_answer),
            }


def make_call_no_answer(call_id, edge_ip, edge_port, destination, stats, direction='outbound'):
    print(f'  [NO_ANSWER] {call_id}')
    sip_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sip_sock.bind(('0.0.0.0', 0))
        sip_sock.settimeout(0.5)

        local_ip = '127.0.0.1'
        local_sip_port = sip_sock.getsockname()[1]
        from_user = '1001'
        caller_tag = f'str-{call_id}'
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
    print(f'  [ANSWERED] {call_id} dur={duration_sec:.0f}s')
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
        caller_tag = f'str-{call_id}'
        edge_addr = (edge_ip, edge_port)

        invite = build_invite(call_id, caller_tag, from_user, local_ip,
                              local_sip_port, local_rtp_port,
                              edge_ip, edge_port, destination, direction=direction)
        invite_bytes = invite.encode('utf-8')
        sip_sock.sendto(invite_bytes, edge_addr)

        got_200 = False
        got_180 = False
        got_183 = False
        to_tag = None
        relay_addr = None
        relay_port = None

        deadline = time.time() + 60
        last_retrans = time.time()
        while time.time() < deadline:
            try:
                data, _ = sip_sock.recvfrom(65535)
            except socket.timeout:
                now = time.time()
                if now - last_retrans >= 0.5 and not got_200 and not got_180:
                    try:
                        sip_sock.sendto(invite_bytes, edge_addr)
                        last_retrans = now
                    except OSError:
                        break
                continue
            msg = data.decode('utf-8', errors='replace')
            status = parse_sip_status(msg)
            if status == 180 and not got_180:
                got_180 = True
                for line in msg.split('\r\n'):
                    if line.lower().startswith('to:'):
                        if ';tag=' in line:
                            to_tag = line.split(';tag=')[1].split(';')[0].split()[0]
                        break
                ringing = True
            elif status == 183 and not got_183:
                got_183 = True
                for line in msg.split('\r\n'):
                    if line.lower().startswith('to:'):
                        if ';tag=' in line:
                            to_tag = line.split(';tag=')[1].split(';')[0].split()[0]
                        break
                addr, port = parse_sdp_endpoint(msg)
                if port:
                    relay_addr, relay_port = addr, port
                    early_rtp_started = True
                # Still ringing — keep waiting until ring_wait expires
            elif status == 200:
                got_200 = True
                for line in msg.split('\r\n'):
                    if line.lower().startswith('to:'):
                        if ';tag=' in line:
                            to_tag = line.split(';tag=')[1].split(';')[0].split()[0]
                        break
                addr, port = parse_sdp_endpoint(msg)
                if port:
                    relay_addr, relay_port = addr, port
                break

        if not got_200 or relay_port is None:
            stats.record_failure(call_id, 'no_200_ok')
            return

        # Random delay after early media: simulates user listening to ringback before answering
        early_delay = random.uniform(MIN_RING_DURATION, MAX_RING_DURATION)
        time.sleep(early_delay)

        ack = build_ack(call_id, caller_tag, to_tag, from_user, local_ip,
                        local_sip_port, edge_ip, edge_port, destination, 1)
        sip_sock.sendto(ack.encode('utf-8'), edge_addr)
        sip_sock.setblocking(False)

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

        # Send RTP for duration_sec, then send BYE while RTP is still running,
        # then keep RTP going for 2s after BYE to ensure recording captures full call.
        rtp_phase_deadline = time.time() + duration_sec
        bye_sent = False
        bye_deadline = time.time() + duration_sec + 3.0  # stop RTP 3s after BYE
        next_send = time.time()
        while True:
            now = time.time()
            if now >= bye_deadline:
                break

            reply_to_in_dialog_refreshes(sip_sock)

            # Send BYE after duration_sec but while RTP is still flowing
            if not bye_sent and now >= rtp_phase_deadline:
                bye_sent = True
                caller_hung_up = random.random() < 0.5
                bye = build_bye(call_id, caller_tag, to_tag, from_user, local_ip,
                                local_sip_port, edge_ip, edge_port, destination, 2)
                try:
                    sip_sock.sendto(bye.encode('utf-8'), edge_addr)
                except OSError:
                    pass

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

        # Wait for 200 OK to BYE (if we sent one)
        if bye_sent:
            try:
                sip_sock.settimeout(2)
                while True:
                    data, _ = sip_sock.recvfrom(65535)
                    msg = data.decode('utf-8', errors='replace')
                    if parse_sip_status(msg) == 200:
                        break
            except socket.timeout:
                pass

        stats.record_answered(call_id, duration_sec, caller_hung_up)
    except Exception as e:
        stats.record_failure(call_id, f'exception:{type(e).__name__}')
    finally:
        sip_sock.close()
        rtp_sock.close()


def cps_walker(initial_cps):
    """Random walk CPS between CPS_MIN and CPS_MAX."""
    cps = initial_cps
    while True:
        time.sleep(CPS_CHANGE_INTERVAL)
        delta = random.uniform(-CPS_STEP, CPS_STEP)
        cps = max(CPS_MIN, min(CPS_MAX, cps + delta))
        yield cps


def answer_rate_walker(initial_rate):
    """Random walk answer rate between ANSWER_RATE_MIN and ANSWER_RATE_MAX."""
    rate = initial_rate
    while True:
        time.sleep(ANSWER_RATE_CHANGE_INTERVAL)
        delta = random.uniform(-ANSWER_RATE_STEP, ANSWER_RATE_STEP)
        rate = max(ANSWER_RATE_MIN, min(ANSWER_RATE_MAX, rate + delta))
        yield rate


def main():
    if len(sys.argv) < 3:
        print(f'Usage: {sys.argv[0]} <edge_ip> <edge_port> [duration_sec] [wav_file]')
        print(f'  CPS: {CPS_MIN}-{CPS_MAX} random walk, Duration: {MIN_DURATION}-{MAX_DURATION}s')
        print(f'  Answer rate: {ANSWER_RATE_MIN*100:.0f}%-{ANSWER_RATE_MAX*100:.0f}% random walk')
        sys.exit(1)

    edge_ip = sys.argv[1]
    edge_port = int(sys.argv[2])
    total_duration = int(sys.argv[3]) if len(sys.argv) > 3 else 600  # 10 min default
    wav_path = sys.argv[4] if len(sys.argv) > 4 else None

    if wav_path:
        global DEFAULT_WAV_FILE
        DEFAULT_WAV_FILE = wav_path

    print('=' * 70)
    print('  VOS-RS STRESS TEST')
    print('=' * 70)
    print(f'  Duration:        {total_duration}s ({total_duration/60:.1f} min)')
    print(f'  CPS:             {CPS_MIN:.0f}-{CPS_MAX:.0f} (initial ~5, random walk ±{CPS_STEP:.0f} every {CPS_CHANGE_INTERVAL:.0f}s)')
    print(f'  Call duration:   {MIN_DURATION}-{MAX_DURATION}s random')
    print(f'  Answer rate:     {ANSWER_RATE_MIN*100:.0f}%-{ANSWER_RATE_MAX*100:.0f}% (random walk)')
    print(f'  RTP audio:       WAV -> 8kHz PCMU @ {RTP_PPS} pps')
    print(f'  Edge:            {edge_ip}:{edge_port}')
    print(f'  Destination:     13800138000')
    print('=' * 70)
    print()

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
        print('  sip-edge is ready')
    else:
        print('  WARN: sip-edge probe got no response; proceeding anyway')
        time.sleep(1.0)
    print()

    stats = StressStats()
    stop_event = threading.Event()
    threads = []
    max_concurrent = 200
    semaphore = threading.Semaphore(max_concurrent)

    def run_with_limit(func, *args):
        semaphore.acquire()
        try:
            func(*args)
        finally:
            semaphore.release()

    shared_state = {'cps': 5.0, 'answer_rate': 0.7}

    def call_generator():
        call_counter = [0]
        current_cps = [shared_state['cps']]
        current_answer_rate = [shared_state['answer_rate']]

        def cps_updater():
            for new_cps in cps_walker(current_cps[0]):
                if stop_event.is_set():
                    break
                current_cps[0] = new_cps
                shared_state['cps'] = new_cps
                stats.record_cps(new_cps)

        def answer_rate_updater():
            for new_rate in answer_rate_walker(current_answer_rate[0]):
                if stop_event.is_set():
                    break
                current_answer_rate[0] = new_rate
                shared_state['answer_rate'] = new_rate
                stats.record_answer_rate(new_rate)

        threading.Thread(target=cps_updater, daemon=True).start()
        threading.Thread(target=answer_rate_updater, daemon=True).start()

        while not stop_event.is_set():
            cps = current_cps[0]
            answer_rate = current_answer_rate[0]
            interval = 1.0 / max(0.1, cps)

            call_counter[0] += 1
            cid = call_counter[0]
            stats.record_initiated()

            call_id = f'str-{int(time.time()*1000)}-{cid}@vos-rs.local'
            will_answer = random.random() < answer_rate
            duration = random.uniform(MIN_DURATION, MAX_DURATION)
            direction = 'outbound'

            if will_answer:
                t = threading.Thread(
                    target=run_with_limit,
                    args=(make_call_answered, call_id, edge_ip, edge_port, '13800138000', duration, stats, direction),
                    daemon=True
                )
            else:
                t = threading.Thread(
                    target=run_with_limit,
                    args=(make_call_no_answer, call_id, edge_ip, edge_port, '13800138000', stats, direction),
                    daemon=True
                )
            t.start()
            threads.append(t)

            time.sleep(interval)

    generator_thread = threading.Thread(target=call_generator, daemon=True)
    generator_thread.start()

    start_time = time.time()
    last_report = start_time
    report_interval = 10.0

    try:
        while time.time() - start_time < total_duration:
            time.sleep(1)
            now = time.time()
            if now - last_report >= report_interval:
                elapsed = now - start_time
                s = stats.summary(elapsed)
                pct = elapsed / total_duration * 100
                print(f'  [{pct:5.1f}%] {elapsed:6.0f}s | initiated={s["total_initiated"]:5d} ans={s["answered"]:4d} no_ans={s["no_answer"]:4d} fail={s["failed"]:3d} | CPS~{shared_state["cps"]:.0f} ans_rate~{shared_state["answer_rate"]:.0%}')
                last_report = now
    except KeyboardInterrupt:
        print('\n  Interrupted by user')

    stop_event.set()
    generator_thread.join(timeout=5)

    for t in threads:
        remaining = max(0.1, 30 - (time.time() - start_time))
        t.join(timeout=remaining)

    elapsed = time.time() - start_time
    s = stats.summary(elapsed)

    print()
    print('=' * 70)
    print('  STRESS TEST REPORT')
    print('=' * 70)
    print(f'  Duration:          {s["elapsed"]:.1f}s')
    print(f'  Total initiated:   {s["total_initiated"]}')
    print(f'  Answered:          {s["answered"]}')
    print(f'  No answer:         {s["no_answer"]}')
    print(f'  Failed:            {s["failed"]}')
    print(f'  Answer rate:       {s["answer_rate"]:.1%}')
    print(f'  Effective CPS:     {s["effective_cps"]:.1f}')
    if s['answered'] > 0:
        print(f'  Avg duration:      {s["avg_duration"]:.1f}s')
        print(f'  Duration range:    {s["min_duration"]:.1f}s - {s["max_duration"]:.1f}s')
    print(f'  Caller hangup:     {s["caller_hangup"]}')
    print(f'  Callee hangup:     {s["callee_hangup"]}')
    if s['errors']:
        print(f'  Error breakdown:')
        for reason, count in sorted(s['errors'].items(), key=lambda x: -x[1]):
            print(f'    {reason}: {count}')
    print('=' * 70)

    sys.exit(1 if s['failed'] > 0 else 0)


if __name__ == '__main__':
    main()
