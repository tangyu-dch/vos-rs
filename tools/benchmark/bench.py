#!/usr/bin/env python3
"""VOS-RS sustained concurrency benchmark runner."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from enum import Enum
from pathlib import Path
from typing import IO, Sequence, Any


# =========================================================================
# MODELS & SCHEMAS (originally models.py)
# =========================================================================

class Scenario(str, Enum):
    SIGNALING = "signaling"
    MEDIA_RELAY = "media_relay"
    RECORDING = "recording"


SCENARIO_LABELS = {
    Scenario.SIGNALING.value: "纯信令",
    Scenario.MEDIA_RELAY.value: "RTP 媒体中继",
    Scenario.RECORDING.value: "RTP 媒体中继与录音",
}

STATUS_LABELS = {"PASS": "通过", "FAIL": "失败"}

RESULT_FIELD_LABELS = {
    "scenario": "测试场景",
    "started_at": "开始时间",
    "duration_seconds": "总耗时（秒）",
    "requested_calls": "请求呼叫数",
    "target_cps": "目标每秒呼叫数",
    "target_concurrent": "目标并发数",
    "calls_completed": "成功呼叫数",
    "calls_failed": "失败呼叫数",
    "success_rate": "成功率（百分比）",
    "calls_peak": "峰值并发数",
    "calls_average": "平均并发数",
    "sustained_seconds": "持续达标时间（秒）",
    "cpu_average": "平均 CPU（百分比）",
    "cpu_peak": "峰值 CPU（百分比）",
    "memory_average_mb": "平均内存（MB）",
    "memory_peak_mb": "峰值内存（MB）",
    "media_delta": "媒体指标",
    "status": "测试状态",
    "failures": "失败原因",
    "artifacts": "产物文件",
}

SAMPLE_FIELD_LABELS = {
    "elapsed_seconds": "经过时间（秒）",
    "cpu_percent": "CPU（百分比）",
    "rss_mb": "常驻内存（MB）",
    "threads": "线程数",
    "file_descriptors": "文件描述符数",
    "active_calls": "活动通话数",
    "media": "媒体指标",
}

MEDIA_METRIC_LABELS = {
    "received_packets": "接收 RTP 包数",
    "forwarded_packets": "转发 RTP 包数",
    "dropped_invalid_packets": "无效包丢弃数",
    "dropped_no_target_packets": "无目标包丢弃数",
    "send_errors": "发送错误数",
    "learned_source_updates": "源地址学习次数",
    "dropped_spoofed_packets": "防欺骗丢弃数",
    "rtcp_quality_alerts": "RTCP 质量告警数",
    "recorded_packets": "录音处理包数",
    "recording_dropped_packets": "录音队列丢包数",
    "recording_errors": "录音错误数",
    "recording_queue_depth": "录音队列深度",
    "recording_queue_capacity": "录音队列容量",
    "recording_workers": "录音工作线程数",
    "dtmf_events": "DTMF 事件数",
    "fast_path_packets": "快路径转发包数",
    "rtcp_quality": "RTCP 质量统计",
    "reports": "报告数",
    "sender_reports": "发送方报告数",
    "receiver_reports": "接收方报告数",
    "report_blocks": "报告块数",
    "last_fraction_lost": "最近丢包比例",
    "max_fraction_lost": "最大丢包比例",
    "last_cumulative_lost": "最近累计丢包数",
    "max_cumulative_lost": "最大累计丢包数",
    "last_jitter": "最近抖动",
    "max_jitter": "最大抖动",
    "last_sender_report": "最近发送方报告",
    "delay_since_last_sender_report": "距最近发送方报告的延迟",
    "last_rtt_ms": "最近往返时延（毫秒）",
    "max_rtt_ms": "最大往返时延（毫秒）",
    "rtcp_window": "RTCP 统计窗口",
    "started_at_unix_ms": "窗口开始时间（Unix 毫秒）",
    "samples": "样本数",
    "average_fraction_lost": "平均丢包比例",
    "average_jitter": "平均抖动",
    "average_rtt_ms": "平均往返时延（毫秒）",
    "r_factor_x100": "R 因子（乘以 100）",
    "mos_x100": "MOS（乘以 100）",
    "total_fraction_lost": "丢包比例合计",
    "total_jitter": "抖动合计",
    "total_rtt_ms": "往返时延合计（毫秒）",
    "rtt_samples": "往返时延样本数",
    "rtcp_quality_degraded": "RTCP 质量是否下降",
}


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
            raise ValueError("呼叫总数、CPS、通话时长和并发数必须大于零")
        if self.concurrent > self.total:
            raise ValueError("并发数不能超过呼叫总数")
        if self.expected_sustain_seconds < self.sustain:
            required = self.ramp_seconds + self.sustain
            raise ValueError(
                f"通话时长至少需要 {required:.1f} 秒，才能让目标并发持续 "
                f"{self.sustain} 秒"
            )
        for port in (self.edge_port, self.gateway_port, self.caller_port, self.rtp_port_min):
            if not 1 <= port <= 65535:
                raise ValueError(f"端口无效：{port}")


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


# =========================================================================
# MONITOR & SAMPLER (originally monitor.py)
# =========================================================================

def fetch_json(url: str, token: str, timeout: float = 1.0) -> Any:
    request = urllib.request.Request(url, headers={"X-VOS-Token": token})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_for_manage_api(base_url: str, token: str, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            fetch_json(f"{base_url}/manage/active-calls", token)
            return
        except (OSError, ValueError, urllib.error.URLError):
            time.sleep(0.25)
    raise TimeoutError(f"管理 API 未在限定时间内就绪：{base_url}")


def process_stats(pid: int) -> tuple[float | None, float | None, int | None, int | None]:
    command = ["ps", "-o", "%cpu=,rss=", "-p", str(pid)]
    output = subprocess.check_output(command, text=True, stderr=subprocess.DEVNULL).strip()
    cpu_raw, rss_raw = output.split()[:2]
    threads = _linux_threads(pid)
    descriptors = _file_descriptor_count(pid)
    return float(cpu_raw), int(rss_raw) / 1024.0, threads, descriptors


def _linux_threads(pid: int) -> int | None:
    status = Path(f"/proc/{pid}/status")
    if not status.exists():
        return None
    for line in status.read_text(encoding="utf-8").splitlines():
        if line.startswith("Threads:"):
            return int(line.split()[1])
    return None


def _file_descriptor_count(pid: int) -> int | None:
    fd_dir = Path(f"/proc/{pid}/fd")
    if fd_dir.exists():
        return len(list(fd_dir.iterdir()))
    try:
        output = subprocess.check_output(
            ["lsof", "-p", str(pid)], text=True, stderr=subprocess.DEVNULL
        )
        return max(0, len(output.splitlines()) - 1)
    except (OSError, subprocess.SubprocessError):
        return None


class ResourceMonitor:
    def __init__(self, pid: int, manage_url: str, token: str, interval: float = 1.0):
        self.pid = pid
        self.manage_url = manage_url.rstrip("/")
        self.token = token
        self.interval = interval
        self.samples: list[Sample] = []
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._started = 0.0

    def start(self) -> None:
        self._started = time.monotonic()
        self._thread = threading.Thread(target=self._run, name="benchmark-monitor", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=self.interval + 2)

    def _run(self) -> None:
        while not self._stop.is_set():
            sample = Sample(elapsed_seconds=time.monotonic() - self._started)
            try:
                sample.cpu_percent, sample.rss_mb, sample.threads, sample.file_descriptors = process_stats(self.pid)
            except (OSError, ValueError, subprocess.SubprocessError):
                pass
            try:
                calls = fetch_json(f"{self.manage_url}/manage/active-calls/count", self.token)
                sample.active_calls = calls if isinstance(calls, int) else None
            except (OSError, ValueError, urllib.error.URLError):
                pass
            try:
                media = fetch_json(f"{self.manage_url}/manage/media-metrics", self.token)
                sample.media = media if isinstance(media, dict) else {}
            except (OSError, ValueError, urllib.error.URLError):
                pass
            self.samples.append(sample)
            self._stop.wait(self.interval)


# =========================================================================
# PROCESS LIFECYCLE MANAGEMENT (originally process.py)
# =========================================================================

class ManagedProcess:
    def __init__(self, name: str, command: Sequence[str], log_path: Path, env=None):
        self.name = name
        self.command = list(command)
        self.log_path = log_path
        self.env = env
        self.process: subprocess.Popen[str] | None = None
        self.log_file: IO[str] | None = None

    @property
    def pid(self) -> int | None:
        return self.process.pid if self.process else None

    def start(self) -> "ManagedProcess":
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        self.log_file = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            self.command,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
            env=self.env,
            start_new_session=True,
        )
        return self

    def is_running(self) -> bool:
        return self.process is not None and self.process.poll() is None

    def wait(self, timeout: float | None = None) -> int:
        if self.process is None:
            raise RuntimeError(f"进程尚未启动：{self.name}")
        return self.process.wait(timeout=timeout)

    def stop(self, grace_seconds: float = 5.0) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except OSError:
                try:
                    self.process.terminate()
                except OSError:
                    pass
            deadline = time.monotonic() + grace_seconds
            while self.process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.1)
            if self.process.poll() is None:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except OSError:
                    try:
                        self.process.kill()
                    except OSError:
                        pass
        self.process.wait()
        if self.log_file:
            self.log_file.close()
            self.log_file = None

    def __enter__(self) -> "ManagedProcess":
        return self.start()

    def __exit__(self, _type, _value, _traceback) -> None:
        self.stop()


# =========================================================================
# REPORT GENERATOR (originally report.py)
# =========================================================================

def numeric_delta(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    delta: dict[str, Any] = {}
    for key, value in after.items():
        previous = before.get(key)
        if isinstance(value, (int, float)) and isinstance(previous, (int, float)):
            delta[key] = value - previous
    return delta


def peak_numeric_metrics(samples: list[Sample]) -> dict[str, int | float]:
    """Return the highest observed value for each top-level numeric media metric."""
    peak: dict[str, int | float] = {}
    for sample in samples:
        for key, value in sample.media.items():
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                continue
            peak[key] = max(peak.get(key, value), value)
    return peak


def summarize_samples(samples: list[Sample], target: int) -> dict[str, float | int]:
    cpu = [sample.cpu_percent for sample in samples if sample.cpu_percent is not None]
    memory = [sample.rss_mb for sample in samples if sample.rss_mb is not None]
    calls = [sample.active_calls for sample in samples if sample.active_calls is not None]
    sustained = sum(1 for value in calls if value >= target) if calls else 0
    return {
        "cpu_average": statistics.fmean(cpu) if cpu else 0.0,
        "cpu_peak": max(cpu, default=0.0),
        "memory_average_mb": statistics.fmean(memory) if memory else 0.0,
        "memory_peak_mb": max(memory, default=0.0),
        "calls_average": statistics.fmean(calls) if calls else 0.0,
        "calls_peak": max(calls, default=0),
        "sustained_seconds": sustained,
    }


def evaluate(
    config: BenchmarkConfig,
    completed: int,
    _failed: int,
    summary: dict,
    media_delta: dict[str, Any] | None = None,
) -> list[str]:
    failures: list[str] = []
    success_rate = completed / config.total * 100 if config.total else 0.0
    if success_rate < 99.0:
        failures.append(f"呼叫成功率 {success_rate:.2f}% 低于 99.0%")
    if summary["calls_peak"] < config.concurrent * 0.95:
        failures.append("峰值并发未达到目标值的 95%")
    if summary["sustained_seconds"] < config.sustain * 0.9:
        failures.append("目标并发的持续时间不足")
    # The configured 99% success-rate SLO already accounts for failed SIPp calls.
    # Treating any non-zero failure as fatal made the threshold contradictory.
    media_delta = media_delta or {}
    if config.scenario != Scenario.SIGNALING:
        if media_delta.get("received_packets", 0) <= 0:
            failures.append("没有 RTP 数据包到达媒体中继")
        if media_delta.get("forwarded_packets", 0) <= 0:
            failures.append("媒体中继没有转发 RTP 数据包")
        if media_delta.get("send_errors", 0) > 0:
            failures.append("媒体中继报告 UDP 发送错误")
    if config.scenario == Scenario.RECORDING:
        if media_delta.get("recorded_packets", 0) <= 0:
            failures.append("录音工作线程没有处理 RTP 数据包")
        if media_delta.get("recording_dropped_packets", 0) > 0:
            failures.append("录音队列发生 RTP 丢包")
        if media_delta.get("recording_errors", 0) > 0:
            failures.append("录音工作线程报告错误")
    return failures


def localized_media(metrics: dict[str, Any]) -> dict[str, Any]:
    localized: dict[str, Any] = {}
    for key, value in metrics.items():
        label = MEDIA_METRIC_LABELS.get(key, key)
        localized[label] = localized_media(value) if isinstance(value, dict) else value
    return localized


def localized_result(result: BenchmarkResult) -> dict[str, Any]:
    values = result.as_dict()
    values["scenario"] = SCENARIO_LABELS.get(result.scenario, result.scenario)
    values["status"] = STATUS_LABELS.get(result.status, result.status)
    values["media_delta"] = localized_media(result.media_delta)
    values["artifacts"] = {
        "主叫日志": result.artifacts.get("caller_log", ""),
        "SIP 状态码统计": result.artifacts.get("status_counts", "{}"),
    }
    return {RESULT_FIELD_LABELS[key]: value for key, value in values.items()}


def write_reports(run_dir: Path, config: BenchmarkConfig, result: BenchmarkResult, samples: list[Sample]) -> None:
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "result.json").write_text(
        json.dumps(localized_result(result), ensure_ascii=False, indent=2), encoding="utf-8"
    )
    with (run_dir / "samples.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(SAMPLE_FIELD_LABELS.values()))
        writer.writeheader()
        for sample in samples:
            row = {
                SAMPLE_FIELD_LABELS[key]: value for key, value in sample.__dict__.items()
            }
            row[SAMPLE_FIELD_LABELS["media"]] = json.dumps(
                localized_media(sample.media), ensure_ascii=False
            )
            writer.writerow(row)
    report = _markdown_report(config, result)
    (run_dir / "report.md").write_text(report, encoding="utf-8")


def write_run_summary(run_dir: Path, results: list[BenchmarkResult]) -> None:
    with (run_dir / "results.jsonl").open("w", encoding="utf-8") as handle:
        for result in results:
            handle.write(json.dumps(localized_result(result), ensure_ascii=False) + "\n")
    fields = list(RESULT_FIELD_LABELS.values())
    with (run_dir / "summary.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for result in results:
            row = localized_result(result)
            row[RESULT_FIELD_LABELS["media_delta"]] = json.dumps(
                row[RESULT_FIELD_LABELS["media_delta"]], ensure_ascii=False
            )
            row[RESULT_FIELD_LABELS["failures"]] = json.dumps(
                row[RESULT_FIELD_LABELS["failures"]], ensure_ascii=False
            )
            row[RESULT_FIELD_LABELS["artifacts"]] = json.dumps(
                row[RESULT_FIELD_LABELS["artifacts"]], ensure_ascii=False
            )
            writer.writerow(row)


def _markdown_report(config: BenchmarkConfig, result: BenchmarkResult) -> str:
    return f"""# VOS-RS 并发压测报告

- 测试场景：`{SCENARIO_LABELS.get(result.scenario, result.scenario)}`
- 测试状态：**{STATUS_LABELS.get(result.status, result.status)}**
- 请求呼叫数：{result.requested_calls}
- 目标每秒呼叫数：{result.target_cps}
- 目标并发数：{result.target_concurrent}
- 并发爬升时间：{config.ramp_seconds:.2f} 秒
- 预期持续时间：{config.expected_sustain_seconds:.2f} 秒

| 指标 | 数值 |
|---|---:|
| 成功呼叫数 | {result.calls_completed} |
| 失败呼叫数 | {result.calls_failed} |
| 呼叫成功率 | {result.success_rate:.2f}% |
| 峰值并发数 | {result.calls_peak} |
| 平均并发数 | {result.calls_average:.2f} |
| 持续达标时间 | {result.sustained_seconds:.0f} 秒 |
| 平均 / 峰值 CPU | {result.cpu_average:.2f}% / {result.cpu_peak:.2f}% |
| 平均 / 峰值内存 | {result.memory_average_mb:.2f} / {result.memory_peak_mb:.2f} MB |
| RTP 接收 / 转发包数 | {result.media_delta.get('received_packets', 0)} / {result.media_delta.get('forwarded_packets', 0)} |
| 录音处理 / 录音丢包数 | {result.media_delta.get('recorded_packets', 0)} / {result.media_delta.get('recording_dropped_packets', 0)} |

## 失败原因

{chr(10).join(f'- {failure}' for failure in result.failures) if result.failures else '- 无'}
"""


# =========================================================================
# SIPP ARGS PARSER & COMMANDS (originally sipp.py)
# =========================================================================

def gateway_command(config: BenchmarkConfig, scenario_file: Path) -> list[str]:
    return [
        config.sipp_binary,
        f"{config.edge_host}:{config.edge_port}",
        "-sf", str(scenario_file),
        "-i", config.edge_host,
        "-p", str(config.gateway_port),
        "-m", str(config.total),
        "-buff_size", "4194304",
        "-max_recv_loops", "1000",
        "-timer_resol", "1",
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
        "-buff_size", "4194304",
        "-max_recv_loops", "1000",
        "-timer_resol", "1",
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


# =========================================================================
# MAIN BENCHMARK RUNNER CORE LOGIC
# =========================================================================

ROOT = Path(__file__).resolve().parents[2]

CONFIG_BY_SCENARIO = {
    Scenario.SIGNALING: ROOT / "tools/sipp/configs/performance.yaml",
    Scenario.MEDIA_RELAY: ROOT / "tools/sipp/configs/performance.yaml",
    Scenario.RECORDING: ROOT / "tools/sipp/configs/full_flow.yaml",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=[item.value for item in Scenario] + ["all"], default="all")
    parser.add_argument("--total", type=int, default=500)
    parser.add_argument("--cps", type=int, default=100)
    parser.add_argument("--duration", type=int, default=35)
    parser.add_argument("--sustain", type=int, default=30)
    parser.add_argument("--concurrent", type=int)
    parser.add_argument("--edge-binary", type=Path, default=ROOT / "target/release/sip-edge")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--sipp", default=os.environ.get("SIPP_BIN", "sipp"))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/benchmark")
    parser.add_argument("--manage-url", default="http://127.0.0.1:5182")
    parser.add_argument("--manage-token")
    parser.add_argument("--cooldown", type=int, default=5)
    parser.add_argument("--media-pps", type=int, default=0)
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def build_config(args: argparse.Namespace, scenario: Scenario, run_dir: Path) -> BenchmarkConfig:
    config = BenchmarkConfig(
        scenario=scenario,
        total=args.total,
        cps=args.cps,
        duration=args.duration,
        sustain=args.sustain,
        concurrent=args.concurrent or args.total,
        edge_host="127.0.0.1",
        edge_port=5160,
        gateway_port=5170,
        caller_port=5164,
        manage_url=args.manage_url,
        manage_token=args.manage_token or (
            "sipp-test-secret" if scenario == Scenario.RECORDING else "sipp-performance-secret"
        ),
        edge_binary=args.edge_binary,
        edge_config=args.config or CONFIG_BY_SCENARIO[scenario],
        sipp_binary=args.sipp,
        output_dir=run_dir,
        cooldown=args.cooldown,
        rtp_port_min=40000,
        media_pps=args.media_pps or args.total * 100,
    )
    config.validate()
    return config


def preflight(config: BenchmarkConfig) -> None:
    if not config.edge_binary.is_file():
        raise FileNotFoundError(f"找不到 sip-edge 可执行文件：{config.edge_binary}")
    if not config.edge_config.is_file():
        raise FileNotFoundError(f"找不到配置文件：{config.edge_config}")
    if shutil.which(config.sipp_binary) is None:
        raise FileNotFoundError(f"找不到 SIPp：{config.sipp_binary}")
    for port in (config.edge_port, config.gateway_port, config.caller_port):
        probe = subprocess.run(
            ["lsof", "-nP", f"-iUDP:{port}"], capture_output=True, text=True, check=False
        ) if shutil.which("lsof") else None
        if probe and probe.returncode == 0 and probe.stdout.strip():
            raise RuntimeError(f"UDP 端口已被占用：{port}")


def run_scenario(config: BenchmarkConfig, dry_run: bool) -> BenchmarkResult | None:
    scenario_dir = config.output_dir
    scenario_dir.mkdir(parents=True, exist_ok=True)
    gateway_xml = ROOT / "tools/sipp/scenarios/gateway_longcall.xml"
    caller_xml = ROOT / "tools/benchmark/scenarios/caller_concurrency.xml"
    edge_log = scenario_dir / "sip-edge.log"
    gateway_log = scenario_dir / "gateway.log"
    caller_log = scenario_dir / "caller.log"
    env = os.environ.copy()
    env["VOS_RS_CONFIG_FILE"] = str(config.edge_config)
    env.setdefault("RUST_LOG", "warn")
    commands = {
        "edge": [str(config.edge_binary)],
        "gateway": gateway_command(config, gateway_xml),
        "caller": caller_command(config, caller_xml),
    }
    if config.scenario != Scenario.SIGNALING:
        commands["rtp"] = rtp_command(
            config, ROOT / "tools/sipp/rtp_range_sender.py", ROOT / "tools/sipp/test_speech.wav"
        )
    if dry_run:
        print(json.dumps(commands, ensure_ascii=False, indent=2))
        return None

    started_wall = datetime.now(timezone.utc).isoformat()
    started = time.monotonic()
    processes: list[ManagedProcess] = []
    monitor: ResourceMonitor | None = None
    media_before: dict = {}
    try:
        edge = ManagedProcess("sip-edge", commands["edge"], edge_log, env).start()
        processes.append(edge)
        wait_for_manage_api(config.manage_url, config.manage_token, 15)
        media_before = fetch_json(f"{config.manage_url}/manage/media-metrics", config.manage_token)
        monitor = ResourceMonitor(edge.pid or 0, config.manage_url, config.manage_token)
        monitor.start()
        gateway = ManagedProcess("gateway", commands["gateway"], gateway_log).start()
        processes.append(gateway)
        time.sleep(1)
        if "rtp" in commands:
            rtp = ManagedProcess("rtp", commands["rtp"], scenario_dir / "rtp.log").start()
            processes.append(rtp)
        caller = ManagedProcess("caller", commands["caller"], caller_log).start()
        processes.append(caller)
        timeout = config.duration + config.ramp_seconds + 45
        caller.wait(timeout=timeout)
        time.sleep(2)
    finally:
        if monitor:
            monitor.stop()
        for process in reversed(processes):
            process.stop()

    elapsed = time.monotonic() - started
    samples = monitor.samples if monitor else []
    media_after = peak_numeric_metrics(samples)
    media_delta = numeric_delta(media_before, media_after)
    completed, failed, status_counts = parse_sipp_summary(caller_log)
    summary = summarize_samples(samples, config.concurrent)
    failures = evaluate(config, completed, failed, summary, media_delta)
    success_rate = completed / config.total * 100 if config.total else 0.0
    result = BenchmarkResult(
        scenario=config.scenario.value,
        started_at=started_wall,
        duration_seconds=elapsed,
        requested_calls=config.total,
        target_cps=config.cps,
        target_concurrent=config.concurrent,
        calls_completed=completed,
        calls_failed=failed,
        success_rate=success_rate,
        calls_peak=int(summary["calls_peak"]),
        calls_average=float(summary["calls_average"]),
        sustained_seconds=float(summary["sustained_seconds"]),
        cpu_average=float(summary["cpu_average"]),
        cpu_peak=float(summary["cpu_peak"]),
        memory_average_mb=float(summary["memory_average_mb"]),
        memory_peak_mb=float(summary["memory_peak_mb"]),
        media_delta=media_delta,
        status="PASS" if not failures else "FAIL",
        failures=failures,
        artifacts={"caller_log": str(caller_log), "status_counts": json.dumps(status_counts)},
    )
    write_reports(scenario_dir, config, result, samples)
    return result


def write_metadata(run_dir: Path, configs: list[BenchmarkConfig]) -> None:
    metadata = {
        "创建时间": datetime.now(timezone.utc).isoformat(),
        "操作系统": platform.platform(),
        "Python 版本": platform.python_version(),
        "CPU 核心数": os.cpu_count(),
        "Git 提交": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "工作区存在未提交修改": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
        "场景配置": {
            SCENARIO_LABELS[item.scenario.value]: {
                "配置路径": str(item.edge_config),
                "配置 SHA256": hashlib.sha256(item.edge_config.read_bytes()).hexdigest(),
            }
            for item in configs
        },
    }
    (run_dir / "metadata.json").write_text(
        json.dumps(metadata, ensure_ascii=False, indent=2), encoding="utf-8"
    )


def main() -> int:
    args = parse_args()
    scenarios = list(Scenario) if args.scenario == "all" else [Scenario(args.scenario)]
    run_id = datetime.now().strftime("%Y%m%d-%H%M%S")
    run_dir = args.output_dir / run_id
    configs = [build_config(args, scenario, run_dir / scenario.value) for scenario in scenarios]
    if not args.dry_run:
        for config in configs:
            preflight(config)
        run_dir.mkdir(parents=True, exist_ok=True)
        write_metadata(run_dir, configs)
    results = []
    for index, config in enumerate(configs):
        print(f"[{index + 1}/{len(configs)}] 测试场景：{SCENARIO_LABELS[config.scenario.value]}")
        result = run_scenario(config, args.dry_run)
        if result:
            results.append(result)
            print(
                f"  {STATUS_LABELS[result.status]}：成功呼叫={result.calls_completed}，"
                f"峰值并发={result.calls_peak}"
            )
        if index + 1 < len(configs) and not args.dry_run:
            time.sleep(config.cooldown)
    if results:
        write_run_summary(run_dir, results)
    return 1 if any(result.status != "PASS" for result in results) else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (FileNotFoundError, RuntimeError, ValueError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"压测执行错误：{error}", file=sys.stderr)
        raise SystemExit(2)
