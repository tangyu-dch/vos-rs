#!/usr/bin/env python3
"""
WAV file RTP sender for VOS-RS audio quality testing.
Reads a WAV file (8kHz, 16-bit PCM), encodes to PCMU, and sends via RTP.
Usage: python3 wav_rtp_sender.py <wav_file> <target_ip> <target_port> [pps] [loop]
"""
import math
import socket
import struct
import sys
import time
import random
import wave

SAMPLES_PER_FRAME = 160  # 20ms at 8kHz

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

def make_rtp_packet(seq, ts, ssrc, payload_type=0, payload=None):
    first_byte = (2 << 6) | (payload_type & 0x7f)
    header = struct.pack('!BBHII',
        first_byte, 0, seq & 0xffff, ts & 0xffffffff, ssrc & 0xffffffff)
    if payload is None:
        payload = bytes([0x7f] * 160)
    return header + payload

def main():
    if len(sys.argv) < 4:
        print(f"Usage: {sys.argv[0]} <wav_file> <target_ip> <target_port> [pps] [loop]")
        print(f"  Reads WAV file, encodes to PCMU, sends via RTP")
        sys.exit(1)

    wav_path = sys.argv[1]
    target_ip = sys.argv[2]
    target_port = int(sys.argv[3])
    pps = int(sys.argv[4]) if len(sys.argv) > 4 else 50
    loop = int(sys.argv[5]) if len(sys.argv) > 5 else 1

    with wave.open(wav_path, 'rb') as w:
        channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        framerate = w.getframerate()
        nframes = w.getnframes()
        raw = w.readframes(nframes)

    if framerate != 8000 or sampwidth != 2:
        print(f"ERROR: WAV must be 8kHz 16-bit PCM (got {framerate}Hz {sampwidth*8}bit)")
        sys.exit(1)

    # Convert raw bytes to samples
    num_samples = nframes * channels
    all_samples = struct.unpack(f'<{num_samples}h', raw)

    # If stereo, mix to mono
    if channels == 2:
        mono = []
        for i in range(0, num_samples, 2):
            mono.append((all_samples[i] + all_samples[i+1]) // 2)
        all_samples = mono
        num_samples = len(mono)

    # Pre-encode all frames to PCMU
    frames = []
    for i in range(0, num_samples, SAMPLES_PER_FRAME):
        chunk = all_samples[i:i+SAMPLES_PER_FRAME]
        if len(chunk) < SAMPLES_PER_FRAME:
            chunk = list(chunk) + [0] * (SAMPLES_PER_FRAME - len(chunk))
        pcmu_frame = bytes(encode_pcmu(s) for s in chunk)
        frames.append(pcmu_frame)

    total_frames = len(frames)
    duration_ms = total_frames * 20
    print(f"WAV: {wav_path} ({channels}ch, {framerate}Hz, {nframes} frames, {duration_ms}ms audio)")
    print(f"Sending to {target_ip}:{target_port} at {pps} pps, looping {loop}x")

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    ssrc = random.randint(1000, 99999)
    interval = 1.0 / pps

    seq = 0
    ts = 0
    total_sent = 0
    start = time.time()

    for iteration in range(loop):
        for frame in frames:
            pkt = make_rtp_packet(seq, ts, ssrc, payload=frame)
            sock.sendto(pkt, (target_ip, target_port))
            seq = (seq + 1) & 0xffff
            ts = (ts + SAMPLES_PER_FRAME) & 0xffffffff
            total_sent += 1
            time.sleep(interval)

    sock.close()
    print(f"Done. Sent {total_sent} frames ({total_sent * 20}ms of audio)")
    elapsed = time.time() - start
    print(f"Elapsed: {elapsed:.1f}s")

if __name__ == "__main__":
    main()
