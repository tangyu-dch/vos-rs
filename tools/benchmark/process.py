"""Safe child-process lifecycle management without global pkill calls."""

from __future__ import annotations

import os
import signal
import subprocess
import time
from pathlib import Path
from typing import IO, Sequence


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
            raise RuntimeError(f"{self.name} has not started")
        return self.process.wait(timeout=timeout)

    def stop(self, grace_seconds: float = 5.0) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            deadline = time.monotonic() + grace_seconds
            while self.process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.1)
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGKILL)
        self.process.wait()
        if self.log_file:
            self.log_file.close()
            self.log_file = None

    def __enter__(self) -> "ManagedProcess":
        return self.start()

    def __exit__(self, _type, _value, _traceback) -> None:
        self.stop()
