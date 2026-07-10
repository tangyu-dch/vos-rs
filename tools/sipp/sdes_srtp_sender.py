#!/usr/bin/env python3
"""Send PCMU RTP protected with the SDES-SRTP profile used by rtp-core."""

import argparse
import base64
import hashlib
import hmac
import random
import signal
import socket
import struct
import time
import wave

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

STOP_REQUESTED = False


def stop(_signum, _frame):
    global STOP_REQUESTED
    STOP_REQUESTED = True


def pcmu(sample):
    sign = 0x80 if sample < 0 else 0
    sample = min(abs(sample) + 0x84, 32635)
    exponent = 7
    mask = 0x4000
    while exponent and not sample & mask:
        exponent -= 1
        mask >>= 1
    return (~(sign | exponent << 4 | (sample >> (exponent + 3) & 0x0F))) & 0xFF


def frames(path):
    with wave.open(path, "rb") as wav:
        if (wav.getframerate(), wav.getsampwidth()) != (8000, 2):
            raise ValueError("WAV must be 8kHz 16-bit PCM")
        raw = wav.readframes(wav.getnframes())
        channels = wav.getnchannels()
    samples = struct.unpack(f"<{len(raw) // 2}h", raw)
    if channels == 2:
        samples = [(samples[i] + samples[i + 1]) // 2 for i in range(0, len(samples), 2)]
    result = []
    for offset in range(0, len(samples), 160):
        chunk = list(samples[offset : offset + 160])
        chunk.extend([0] * (160 - len(chunk)))
        result.append(bytes(pcmu(sample) for sample in chunk))
    return result or [bytes([0xFF] * 160)]


def protect(packet, key, salt, ssrc, index):
    session_salt = bytearray(salt)
    session_salt[4:8] = bytes(a ^ b for a, b in zip(session_salt[4:8], ssrc.to_bytes(4, "big")))
    index_bytes = index.to_bytes(8, "big")
    session_salt[8:14] = bytes(a ^ b for a, b in zip(session_salt[8:14], index_bytes[2:]))
    iv = bytes(session_salt) + b"\x00\x00"
    cipher = Cipher(algorithms.AES(key), modes.CTR(iv)).encryptor()
    encrypted = packet[:12] + cipher.update(packet[12:])
    tag = hmac.new(key, encrypted, hashlib.sha1).digest()[:10]
    return encrypted + tag


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wav", required=True)
    parser.add_argument("--target-ip", default="127.0.0.1")
    parser.add_argument("--port-min", type=int, default=40000)
    parser.add_argument("--port-count", type=int, default=1)
    parser.add_argument("--pps", type=int, default=50)
    parser.add_argument("--duration", type=float, default=6)
    parser.add_argument("--key", required=True, help="base64-encoded 30-byte master key and salt")
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    material = base64.b64decode(args.key)
    if len(material) != 30:
        raise SystemExit("--key must decode to exactly 30 bytes")
    key, salt = material[:16], material[16:]
    port = args.port_min if args.port_min % 2 == 0 else args.port_min + 1
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    ssrc = random.randint(1, 2**32 - 1)
    sequence = random.randint(0, 65535)
    timestamp = random.randint(0, 2**32 - 1)
    started = time.monotonic()
    sent = 0
    source = frames(args.wav)
    deadline = started + args.duration
    while not STOP_REQUESTED and time.monotonic() < deadline:
        payload = source[sent % len(source)]
        plain = struct.pack("!BBHII", 0x80, 0, sequence, timestamp, ssrc) + payload
        sock.sendto(protect(plain, key, salt, ssrc, sent), (args.target_ip, port))
        sent += 1
        sequence = (sequence + 1) & 0xFFFF
        timestamp = (timestamp + 160) & 0xFFFFFFFF
        time.sleep(1 / args.pps)
    sock.close()
    print(f"SRTP sent: packets={sent} avg_pps={sent / max(time.monotonic() - started, 0.001):.1f}")


if __name__ == "__main__":
    main()
