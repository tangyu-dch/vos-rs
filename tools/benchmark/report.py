"""Result aggregation and machine/human readable report output."""

from __future__ import annotations

import csv
import json
import statistics
from pathlib import Path
from typing import Any

from models import BenchmarkConfig, BenchmarkResult, Sample, Scenario


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
    failed: int,
    summary: dict,
    media_delta: dict[str, Any] | None = None,
) -> list[str]:
    failures: list[str] = []
    success_rate = completed / config.total * 100 if config.total else 0.0
    if success_rate < 99.0:
        failures.append(f"success rate {success_rate:.2f}% is below 99.0%")
    if summary["calls_peak"] < config.concurrent * 0.95:
        failures.append("peak concurrent calls did not reach 95% of target")
    if summary["sustained_seconds"] < config.sustain * 0.9:
        failures.append("target concurrency was not sustained long enough")
    if failed:
        failures.append(f"SIPp reported {failed} failed calls")
    media_delta = media_delta or {}
    if config.scenario != Scenario.SIGNALING:
        if media_delta.get("received_packets", 0) <= 0:
            failures.append("no RTP packets reached the media relay")
        if media_delta.get("forwarded_packets", 0) <= 0:
            failures.append("no RTP packets were forwarded by the media relay")
        if media_delta.get("send_errors", 0) > 0:
            failures.append("media relay reported UDP send errors")
    if config.scenario == Scenario.RECORDING:
        if media_delta.get("recorded_packets", 0) <= 0:
            failures.append("recording workers did not process RTP packets")
        if media_delta.get("recording_dropped_packets", 0) > 0:
            failures.append("recording queue dropped RTP packets")
        if media_delta.get("recording_errors", 0) > 0:
            failures.append("recording workers reported errors")
    return failures


def write_reports(run_dir: Path, config: BenchmarkConfig, result: BenchmarkResult, samples: list[Sample]) -> None:
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "result.json").write_text(
        json.dumps(result.as_dict(), ensure_ascii=False, indent=2), encoding="utf-8"
    )
    with (run_dir / "samples.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(Sample.__dataclass_fields__))
        writer.writeheader()
        for sample in samples:
            row = sample.__dict__.copy()
            row["media"] = json.dumps(row["media"], ensure_ascii=False)
            writer.writerow(row)
    report = _markdown_report(config, result)
    (run_dir / "report.md").write_text(report, encoding="utf-8")


def write_run_summary(run_dir: Path, results: list[BenchmarkResult]) -> None:
    with (run_dir / "results.jsonl").open("w", encoding="utf-8") as handle:
        for result in results:
            handle.write(json.dumps(result.as_dict(), ensure_ascii=False) + "\n")
    fields = list(BenchmarkResult.__dataclass_fields__)
    with (run_dir / "summary.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for result in results:
            row = result.as_dict()
            row["media_delta"] = json.dumps(row["media_delta"], ensure_ascii=False)
            row["failures"] = json.dumps(row["failures"], ensure_ascii=False)
            row["artifacts"] = json.dumps(row["artifacts"], ensure_ascii=False)
            writer.writerow(row)


def _markdown_report(config: BenchmarkConfig, result: BenchmarkResult) -> str:
    return f"""# VOS-RS Concurrency Benchmark

- Scenario: `{result.scenario}`
- Status: **{result.status}**
- Requested calls: {result.requested_calls}
- Target CPS: {result.target_cps}
- Target concurrent: {result.target_concurrent}
- Ramp: {config.ramp_seconds:.2f}s
- Expected sustained period: {config.expected_sustain_seconds:.2f}s

| Metric | Value |
|---|---:|
| Completed | {result.calls_completed} |
| Failed | {result.calls_failed} |
| Success rate | {result.success_rate:.2f}% |
| Peak concurrent | {result.calls_peak} |
| Average concurrent | {result.calls_average:.2f} |
| Sustained seconds | {result.sustained_seconds:.0f} |
| CPU average / peak | {result.cpu_average:.2f}% / {result.cpu_peak:.2f}% |
| RSS average / peak | {result.memory_average_mb:.2f} / {result.memory_peak_mb:.2f} MB |
| RTP received / forwarded | {result.media_delta.get('received_packets', 0)} / {result.media_delta.get('forwarded_packets', 0)} |
| Recorded / recording dropped | {result.media_delta.get('recorded_packets', 0)} / {result.media_delta.get('recording_dropped_packets', 0)} |

## Failures

{chr(10).join(f'- {failure}' for failure in result.failures) if result.failures else '- None'}
"""
