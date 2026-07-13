import tempfile
import unittest
from pathlib import Path

from models import BenchmarkConfig, BenchmarkResult, Scenario, Sample
from report import (
    evaluate,
    numeric_delta,
    peak_numeric_metrics,
    summarize_samples,
    write_run_summary,
)
from sipp import parse_sipp_summary


def config(**overrides):
    values = dict(
        scenario=Scenario.SIGNALING, total=100, cps=20, duration=15, sustain=10,
        concurrent=100, edge_host="127.0.0.1", edge_port=5160, gateway_port=5170,
        caller_port=5164, manage_url="http://127.0.0.1:5182", manage_token="test",
        edge_binary=Path("sip-edge"), edge_config=Path("config.yaml"), sipp_binary="sipp",
        output_dir=Path("target"), cooldown=0, rtp_port_min=40000, media_pps=10000,
    )
    values.update(overrides)
    return BenchmarkConfig(**values)


class BenchmarkTests(unittest.TestCase):
    def test_rejects_duration_without_sustain_window(self):
        with self.assertRaises(ValueError):
            config(duration=10, sustain=10).validate()

    def test_summarizes_resource_samples(self):
        samples = [
            Sample(0, cpu_percent=10, rss_mb=100, active_calls=50),
            Sample(1, cpu_percent=30, rss_mb=120, active_calls=100),
        ]
        summary = summarize_samples(samples, 100)
        self.assertEqual(summary["cpu_average"], 20)
        self.assertEqual(summary["calls_peak"], 100)
        self.assertEqual(summary["sustained_seconds"], 1)

    def test_computes_numeric_media_delta(self):
        self.assertEqual(numeric_delta({"received": 10}, {"received": 25}), {"received": 15})

    def test_uses_peak_media_metrics_before_ports_are_released(self):
        samples = [
            Sample(0, media={"received_packets": 0}),
            Sample(1, media={"received_packets": 25}),
            Sample(2, media={"received_packets": 0}),
        ]
        self.assertEqual(peak_numeric_metrics(samples)["received_packets"], 25)

    def test_parses_sipp_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "caller.log"
            path.write_text("Successful call | 0 | 100\nFailed call | 0 | 2\n", encoding="utf-8")
            self.assertEqual(parse_sipp_summary(path)[:2], (100, 2))

    def test_evaluation_requires_peak_and_sustain(self):
        failures = evaluate(config(), 100, 0, {"calls_peak": 50, "sustained_seconds": 0})
        self.assertEqual(len(failures), 2)

    def test_recording_evaluation_requires_media_and_recording(self):
        recording = config(scenario=Scenario.RECORDING)
        failures = evaluate(
            recording,
            100,
            0,
            {"calls_peak": 100, "sustained_seconds": 10},
            {"received_packets": 10, "forwarded_packets": 10, "recorded_packets": 0},
        )
        self.assertIn("recording workers did not process RTP packets", failures)


if __name__ == "__main__":
    unittest.main()
