"""Resource, active-call and media metric sampling."""

from __future__ import annotations

import json
import os
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from models import Sample


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
    raise TimeoutError(f"manage API did not become ready: {base_url}")


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
                calls = fetch_json(f"{self.manage_url}/manage/active-calls", self.token)
                sample.active_calls = len(calls) if isinstance(calls, list) else None
            except (OSError, ValueError, urllib.error.URLError):
                pass
            try:
                media = fetch_json(f"{self.manage_url}/manage/media-metrics", self.token)
                sample.media = media if isinstance(media, dict) else {}
            except (OSError, ValueError, urllib.error.URLError):
                pass
            self.samples.append(sample)
            self._stop.wait(self.interval)
