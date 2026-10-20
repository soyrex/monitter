#!/usr/bin/env python3
"""Run the existing disconnected fixture collector plus a read-only OS sampler."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenario", choices=["idle", "static", "streaming-body", "pipeline-updates", "browser-adjacent", "motion"], required=True)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--warmup", type=int, default=15)
    parser.add_argument("--rows", type=int, default=10000)
    parser.add_argument("--motion-mode", choices=["off", "on", "subtle", "system"], default="off")
    args = parser.parse_args()
    scripts = Path(__file__).resolve().parent
    output = args.output.resolve()
    if output.exists():
        parser.error("output must not already exist")
    command = [sys.executable, str(scripts / "gpui-performance-session.py"), str(args.binary.resolve()),
               "--output", str(output), "--scenario", args.scenario, "--seconds", str(args.seconds),
               "--warmup", str(args.warmup), "--rows", str(args.rows), "--motion-mode", args.motion_mode]
    collector = subprocess.Popen(command)
    session = output / "session.json"
    deadline = time.monotonic() + 60
    record = None
    while collector.poll() is None and time.monotonic() < deadline:
        try:
            record = json.loads(session.read_text())
            if isinstance(record.get('pid'), int):
                break
        except (OSError, ValueError):
            pass
        time.sleep(0.1)
    if record is None or not isinstance(record.get('pid'), int):
        # The existing collector owns its bounded fixture lifecycle; let it
        # finish rather than interrupting its cleanup or guessing a child PID.
        code = collector.wait()
        raise SystemExit(f"collector did not publish a session (exit {code})")
    sampler = subprocess.Popen([sys.executable, str(scripts / "gpui-fast-os-sample.py"),
                                str(record["pid"]), "--output", str(output / "os-samples.json")])
    collector_code = collector.wait()
    sampler_code = sampler.wait(timeout=15)
    receipt = {"collectorExitCode": collector_code, "samplerExitCode": sampler_code,
               "fixturePid": record["pid"], "output": str(output)}
    (output / "collection-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt), flush=True)
    if collector_code or sampler_code:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
