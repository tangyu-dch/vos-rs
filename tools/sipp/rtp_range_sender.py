#!/usr/bin/env python3
"""
Send PCMU RTP packets across a relay port range.

This is intentionally simple for SIPp load tests: sip-edge allocates relay RTP
ports sequentially, so a bounded even-port range gives deterministic media load
without trying to sniff SIP messages.
"""

import argparse
import random
import signal
import socket
import struct
import sys
import time
import wave

SAMPLES_PER_FRAME = 160
PAYLOAD_TYPE_PCMU = 0
STOP_REQUESTED = False


def request_stop(_signum, _frame):
    global STOP_REQUESTED
    STOP_REQUESTED = True


def encode_pcmu(sample):
    bias = 0x84
    clip = 32635
    sign = 0
    if sample < 0:
        sign = 0x80
        sample = -sample
    sample = min(sample + bias, clip)
    exponent = 7
    mask = 0x4000
    while exponent > 0 and not (sample & mask):
        exponent -= 1
        mask >>= 1
    mantissa = (sample >> (exponent + 3)) & 0x0F
    return ~(sign | (exponent << 4) | mantissa) & 0xFF


def load_pcmu_frames(path):
    with wave.open(path, "rb") as wav:
        channels = wav.getnchannels()
        sample_width = wav.getsampwidth()
        frame_rate = wav.getframerate()
        raw = wav.readframes(wav.getnframes())

    if frame_rate != 8000 or sample_width != 2:
        raise ValueError(
            f"WAV must be 8kHz 16-bit PCM, got {frame_rate}Hz {sample_width * 8}bit"
        )

    samples = struct.unpack(f"<{len(raw) // 2}h", raw)
    if channels == 2:
        samples = tuple(
            (samples[i] + samples[i + 1]) // 2 for i in range(0, len(samples), 2)
        )

    frames = []
    for offset in range(0, len(samples), SAMPLES_PER_FRAME):
        chunk = list(samples[offset : offset + SAMPLES_PER_FRAME])
        if len(chunk) < SAMPLES_PER_FRAME:
            chunk.extend([0] * (SAMPLES_PER_FRAME - len(chunk)))
        frames.append(bytes(encode_pcmu(sample) for sample in chunk))
    return frames or [bytes([0xFF] * SAMPLES_PER_FRAME)]


def make_packet(seq, timestamp, ssrc, payload):
    return (
        struct.pack(
            "!BBHII",
            0x80,
            PAYLOAD_TYPE_PCMU,
            seq & 0xFFFF,
            timestamp & 0xFFFFFFFF,
            ssrc & 0xFFFFFFFF,
        )
        + payload
    )


def even_ports(port_min, port_count):
    first = port_min if port_min % 2 == 0 else port_min + 1
    return [first + index * 2 for index in range(port_count)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wav", required=True)
    parser.add_argument("--target-ip", default="127.0.0.1")
    parser.add_argument("--port-min", type=int, default=40000)
    parser.add_argument("--port-count", type=int, default=512)
    parser.add_argument("--duration", type=float, default=30.0)
    parser.add_argument("--pps", type=int, default=20000)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)

    frames = load_pcmu_frames(args.wav)
    ports = even_ports(args.port_min, args.port_count)
    if args.pps <= 0 or args.port_count <= 0:
        raise SystemExit("pps and port-count must be positive")
    if ports[-1] > 65535:
        raise SystemExit("port range exceeds UDP port limit")

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    interval = 1.0 / args.pps
    deadline = time.monotonic() + args.duration
    next_send = time.monotonic()
    seq = random.randint(0, 65535)
    timestamp = random.randint(0, 2**32 - 1)
    ssrc = random.randint(1, 2**32 - 1)
    sent = 0
    started_at = time.monotonic()

    print(
        f"RTP range sender: {args.pps} pps for {args.duration:.1f}s "
        f"across {len(ports)} ports {ports[0]}..{ports[-1]}",
        flush=True,
    )

    try:
        while not STOP_REQUESTED and time.monotonic() < deadline:
            frame = frames[sent % len(frames)]
            port = ports[sent % len(ports)]
            packet = make_packet(seq, timestamp, ssrc, frame)
            sock.sendto(packet, (args.target_ip, port))
            sent += 1
            seq = (seq + 1) & 0xFFFF
            timestamp = (timestamp + SAMPLES_PER_FRAME) & 0xFFFFFFFF
            next_send += interval
            sleep_for = next_send - time.monotonic()
            if sleep_for > 0:
                time.sleep(sleep_for)
    finally:
        sock.close()

    elapsed = max(time.monotonic() - started_at, 0.001)
    print(f"RTP sent: packets={sent} avg_pps={sent / elapsed:.1f}", flush=True)
    sys.exit(0)


if __name__ == "__main__":
    main()
