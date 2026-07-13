"""Data models and validation for the VOS-RS concurrency benchmark."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any


class Scenario(str, Enum):
    SIGNALING = "signaling"
    MEDIA_RELAY = "media_relay"
    RECORDING = "recording"


@dataclass(frozen=True)
class BenchmarkConfig:
    scenario: Scenario
    total: int
    cps: int
    duration: int
    sustain: int
    concurrent: int
    edge_host: str
    edge_port: int
    gateway_port: int
    caller_port: int
    manage_url: str
    manage_token: str
    edge_binary: Path
    edge_config: Path
    sipp_binary: str
    output_dir: Path
    cooldown: int
    rtp_port_min: int
    media_pps: int

    @property
    def ramp_seconds(self) -> float:
        return self.total / self.cps

    @property
    def expected_sustain_seconds(self) -> float:
        return self.duration - self.ramp_seconds

    def validate(self) -> None:
        positive = (self.total, self.cps, self.duration, self.concurrent)
        if any(value <= 0 for value in positive):
            raise ValueError("total, cps, duration and concurrent must be positive")
        if self.concurrent > self.total:
            raise ValueError("concurrent cannot exceed total calls")
        if self.expected_sustain_seconds < self.sustain:
            required = self.ramp_seconds + self.sustain
            raise ValueError(
                f"duration must be at least {required:.1f}s to sustain target concurrency "
                f"for {self.sustain}s"
            )
        for port in (self.edge_port, self.gateway_port, self.caller_port, self.rtp_port_min):
            if not 1 <= port <= 65535:
                raise ValueError(f"invalid port: {port}")


@dataclass
class Sample:
    elapsed_seconds: float
    cpu_percent: float | None = None
    rss_mb: float | None = None
    threads: int | None = None
    file_descriptors: int | None = None
    active_calls: int | None = None
    media: dict[str, Any] = field(default_factory=dict)


@dataclass
class BenchmarkResult:
    scenario: str
    started_at: str
    duration_seconds: float
    requested_calls: int
    target_cps: int
    target_concurrent: int
    calls_completed: int
    calls_failed: int
    success_rate: float
    calls_peak: int
    calls_average: float
    sustained_seconds: float
    cpu_average: float
    cpu_peak: float
    memory_average_mb: float
    memory_peak_mb: float
    media_delta: dict[str, Any]
    status: str
    failures: list[str]
    artifacts: dict[str, str]

    def as_dict(self) -> dict[str, Any]:
        return asdict(self)
