#!/usr/bin/env python3
"""VOS-RS sustained concurrency benchmark runner."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))

from models import BenchmarkConfig, BenchmarkResult, Scenario
from monitor import ResourceMonitor, fetch_json, wait_for_manage_api
from process import ManagedProcess
from report import (
    evaluate,
    numeric_delta,
    peak_numeric_metrics,
    summarize_samples,
    write_reports,
    write_run_summary,
)
from sipp import caller_command, gateway_command, parse_sipp_summary, rtp_command

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
        raise FileNotFoundError(f"sip-edge binary not found: {config.edge_binary}")
    if not config.edge_config.is_file():
        raise FileNotFoundError(f"config not found: {config.edge_config}")
    if shutil.which(config.sipp_binary) is None:
        raise FileNotFoundError(f"SIPp not found: {config.sipp_binary}")
    for port in (config.edge_port, config.gateway_port, config.caller_port):
        probe = subprocess.run(
            ["lsof", "-nP", f"-iUDP:{port}"], capture_output=True, text=True, check=False
        ) if shutil.which("lsof") else None
        if probe and probe.returncode == 0 and probe.stdout.strip():
            raise RuntimeError(f"UDP port {port} is already in use")


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
        "created_at": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "cpu_count": os.cpu_count(),
        "git_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "git_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
        "configs": {
            item.scenario.value: {
                "path": str(item.edge_config),
                "sha256": hashlib.sha256(item.edge_config.read_bytes()).hexdigest(),
            }
            for item in configs
        },
    }
    (run_dir / "metadata.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")


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
        print(f"[{index + 1}/{len(configs)}] scenario={config.scenario.value}")
        result = run_scenario(config, args.dry_run)
        if result:
            results.append(result)
            print(f"  {result.status}: completed={result.calls_completed} peak={result.calls_peak}")
        if index + 1 < len(configs) and not args.dry_run:
            time.sleep(config.cooldown)
    if results:
        write_run_summary(run_dir, results)
    return 1 if any(result.status != "PASS" for result in results) else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (FileNotFoundError, RuntimeError, ValueError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"benchmark error: {error}", file=sys.stderr)
        raise SystemExit(2)
