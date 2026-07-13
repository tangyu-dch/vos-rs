"""SIPp command construction and summary parsing."""

from __future__ import annotations

import re
from pathlib import Path

from models import BenchmarkConfig


def gateway_command(config: BenchmarkConfig, scenario_file: Path) -> list[str]:
    return [
        config.sipp_binary,
        f"{config.edge_host}:{config.edge_port}",
        "-sf", str(scenario_file),
        "-i", config.edge_host,
        "-p", str(config.gateway_port),
        "-m", str(config.total),
        "-aa", "-nostdin",
        "-timeout", f"{config.duration + 30}s",
    ]


def caller_command(config: BenchmarkConfig, scenario_file: Path) -> list[str]:
    return [
        config.sipp_binary,
        f"{config.edge_host}:{config.edge_port}",
        "-sf", str(scenario_file),
        "-i", config.edge_host,
        "-p", str(config.caller_port),
        # Avoid production numeric billing prefixes for synthetic calls.
        "-s", "benchmark",
        "-m", str(config.total),
        "-r", str(config.cps),
        "-l", str(config.concurrent),
        "-d", str(config.duration * 1000),
        "-aa", "-nostdin", "-trace_err",
        "-timeout", f"{config.duration + int(config.ramp_seconds) + 30}s",
    ]


def rtp_command(config: BenchmarkConfig, sender: Path, wav: Path) -> list[str]:
    port_count = min(config.total * 2, (65534 - config.rtp_port_min) // 2)
    return [
        "python3", "-u", str(sender),
        "--wav", str(wav),
        "--target-ip", config.edge_host,
        "--port-min", str(config.rtp_port_min),
        "--port-count", str(max(1, port_count)),
        "--duration", str(config.duration + int(config.ramp_seconds)),
        "--pps", str(config.media_pps),
    ]


def parse_sipp_summary(log_path: Path) -> tuple[int, int, dict[str, int]]:
    text = log_path.read_text(encoding="utf-8", errors="replace") if log_path.exists() else ""
    completed = _last_table_value(text, "Successful call")
    failed = _last_table_value(text, "Failed call")
    status_counts: dict[str, int] = {}
    for status in re.findall(r"SIP/2\.0\s+(\d{3})", text):
        status_counts[status] = status_counts.get(status, 0) + 1
    return completed, failed, status_counts


def _last_table_value(text: str, label: str) -> int:
    values: list[int] = []
    for line in text.splitlines():
        if label not in line:
            continue
        columns = [column.strip() for column in line.split("|")]
        for column in reversed(columns):
            if column.isdigit():
                values.append(int(column))
                break
    return values[-1] if values else 0
