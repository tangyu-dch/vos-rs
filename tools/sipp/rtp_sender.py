#!/usr/bin/env python3
"""
RTP media sender for VOS-RS performance testing.
Generates a sine wave tone encoded in PCMU and sends it via RTP.
Usage: python3 rtp_sender.py <target_ip> <target_port> <duration_sec> [pps] [freq]
"""
import math
import socket
import struct
import sys
import time
import random

SAMPLE_RATE = 8000
SAMPLES_PER_FRAME = 160  # 20ms at 8kHz
TONE_AMPLITUDE = 4000  # out of 32767

def encode_pcmu(sample):
    """Encode a 16-bit linear PCM sample to PCMU (μ-law)."""
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
    mulaw_byte = ~(sign | (exponent << 4) | mantissa) & 0xFF
    return mulaw_byte

def make_rtp_packet(seq, ts, ssrc, payload_type=0, payload=None):
    first_byte = (2 << 6) | (payload_type & 0x7f)
    header = struct.pack('!BBHII',
        first_byte, 0,
        seq & 0xffff, ts & 0xffffffff, ssrc & 0xffffffff,
    )
    if payload is None:
        payload = bytes([0x7f] * 160)
    return header + payload

def generate_pcmu_tone(num_samples, freq, sample_rate, amplitude):
    """Generate PCMU-encoded sine wave samples."""
    payload = bytearray()
    phase = 0.0
    phase_inc = 2.0 * math.pi * freq / sample_rate
    for _ in range(num_samples):
        sample_val = int(amplitude * math.sin(phase))
        sample_val = max(-32768, min(32767, sample_val))
        payload.append(encode_pcmu(sample_val))
        phase += phase_inc
        if phase > 2 * math.pi:
            phase -= 2 * math.pi
    return bytes(payload)

def main():
    if len(sys.argv) < 4:
        print(f"Usage: {sys.argv[0]} <target_ip> <target_port> <duration_sec> [pps] [freq]")
        print(f"  Generates a sine wave tone in PCMU and sends via RTP")
        sys.exit(1)

    target_ip = sys.argv[1]
    target_port = int(sys.argv[2])
    duration = int(sys.argv[3])
    pps = int(sys.argv[4]) if len(sys.argv) > 4 else 50  # 50 pps = 8kHz/160
    freq = float(sys.argv[5]) if len(sys.argv) > 5 else 440.0

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    ssrc = random.randint(1000, 99999)
    interval = 1.0 / pps

    # Pre-generate one frame of audio (160 samples = 20ms at 8kHz)
    frame = generate_pcmu_tone(SAMPLES_PER_FRAME, freq, SAMPLE_RATE, TONE_AMPLITUDE)

    print(f"Sending {freq}Hz PCMU tone to {target_ip}:{target_port} for {duration}s at {pps} pps")
    start = time.time()
    total_sent = 0

    while time.time() - start < duration:
        ts = (total_sent * SAMPLES_PER_FRAME) & 0xffffffff
        pkt = make_rtp_packet(total_sent & 0xffff, ts, ssrc, payload=frame)
        sock.sendto(pkt, (target_ip, target_port))
        total_sent += 1
        time.sleep(interval)

    sock.close()
    print(f"Done. Sent {total_sent} frames ({total_sent * 20}ms of audio)")

if __name__ == "__main__":
    main()
