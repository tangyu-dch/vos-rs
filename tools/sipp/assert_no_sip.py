#!/usr/bin/env python3
"""Fail when a SIP datagram reaches an endpoint during a guarded interval."""

import argparse
import socket
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("host")
    parser.add_argument("port", type=int)
    parser.add_argument("--timeout", type=float, default=3.0)
    parser.add_argument("--output")
    args = parser.parse_args()

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind((args.host, args.port))
    sock.settimeout(args.timeout)
    try:
        payload, peer = sock.recvfrom(65535)
    except socket.timeout:
        return 0
    finally:
        sock.close()

    text = payload.decode("utf-8", errors="replace")
    if args.output:
        with open(args.output, "w", encoding="utf-8") as output:
            output.write(f"peer={peer[0]}:{peer[1]}\n{text}")
    print(f"unexpected SIP datagram from {peer[0]}:{peer[1]}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
