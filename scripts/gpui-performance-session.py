#!/usr/bin/env python3
"""Observe one explicitly isolated native fixture; never generate UI input.

Use a build with --performance-profiling. Type/scroll through normal native
input while it is open. Only the child started here is terminated afterward.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import time
import uuid


def cpu_seconds(pid):
    result = subprocess.run(
        ["/bin/ps", "-p", str(pid), "-o", "time="],
        capture_output=True, text=True, check=False,
    )
    value = result.stdout.strip()
    if not value:
        return None
    parts = value.split(":")
    total = 0.0
    for part in parts:
        total = total * 60 + float(part)
    return total


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def isolate_app(binary, output, fixture_env):
    """Give native UI tools an unambiguous app that always opens the fixture.

    Launch Services may reopen a selected app after an observation ends. The
    copied bundle's environment keeps that reopening away from live startup.
    """
    source_app = binary.parents[2]
    if source_app.suffix != ".app" or binary.parent.name != "MacOS":
        raise SystemExit("a macOS app bundle is required for native observations")
    app = output / "Monitter Performance.app"
    shutil.copytree(source_app, app)
    plist_path = app / "Contents" / "Info.plist"
    with plist_path.open("rb") as stream:
        info = plistlib.load(stream)
    identifier = "com.monitter.gpui.performance-fixture." + uuid.uuid4().hex[:12]
    info.update({
        "CFBundleIdentifier": identifier,
        "CFBundleName": "Monitter Performance",
        "CFBundleDisplayName": "Monitter Performance",
        "LSEnvironment": fixture_env,
    })
    with plist_path.open("wb") as stream:
        plistlib.dump(info, stream)
    subprocess.run(["/usr/bin/codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    return app / "Contents" / "MacOS" / binary.name, identifier


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--scenario", choices=["idle", "static", "streaming-body", "pipeline-updates", "browser-adjacent"], default="static")
    parser.add_argument("--rows", type=int, default=10_000)
    parser.add_argument("--seconds", type=int, default=45)
    parser.add_argument("--warmup", type=int, default=8)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if not os.access(binary, os.X_OK):
        parser.error("binary must be executable")
    if not 1 <= args.seconds <= 600 or not 1 <= args.warmup <= 300:
        parser.error("seconds must be 1..600 and warmup 1..300")
    if not 1_000 <= args.rows <= 100_000:
        parser.error("rows must be 1000..100000")
    # Normal builds ignore the fixture environment. Refuse them before launch
    # so a mistaken executable can never connect this runner to the live profile.
    fingerprint = digest(binary)
    manifest_path = binary.parents[3] / "build.json"
    try:
        build = json.loads(manifest_path.read_text())
    except (OSError, ValueError):
        parser.error("a packaged profiling build with adjacent build.json is required")
    if build.get("performanceProfiling") is not True or build.get("desktopSha256") != fingerprint:
        parser.error("build.json must verify this exact executable as a profiling build")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    metrics = output / "metrics.json"
    fixture_env = {
        "MONITTER_GPUI_PERF_SCENARIO": args.scenario,
        "MONITTER_GPUI_PERF_HISTORY_ROWS": str(args.rows),
        "MONITTER_GPUI_PERF_SECONDS": str(args.seconds),
        "MONITTER_GPUI_PERF_WARMUP_SECONDS": str(args.warmup),
        "MONITTER_GPUI_PERF_OUTPUT": str(metrics),
    }
    fixture_binary, identifier = isolate_app(binary, output, fixture_env)
    fixture_fingerprint = digest(fixture_binary)
    env = dict(os.environ)
    env.update(fixture_env)
    fixture_build = dict(build, app=str(fixture_binary.parents[2]),
                         desktopSha256=fixture_fingerprint, isolatedFixture=True)
    (output / "build.json").write_text(json.dumps(fixture_build, indent=2) + "\n")
    metadata = {
        "binary": str(fixture_binary),
        "sha256": fixture_fingerprint,
        "bundleId": identifier,
        "sourceBinary": str(binary),
        "sourceSha256": fingerprint,
        "scenario": args.scenario,
        "historyRowsPerPane": args.rows,
        "chatPanes": 4 if args.scenario == "pipeline-updates" else 1,
        "build": build,
        "startedAtUtc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "warmupSeconds": args.warmup,
        "observationSeconds": args.seconds,
        "inputSource": "external native input; this script generates none",
    }
    with (output / "native.log").open("w") as log:
        child = subprocess.Popen([str(fixture_binary)], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        metadata["pid"] = child.pid
        (output / "session.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(json.dumps({"pid": child.pid, "output": str(output), "scenario": args.scenario,
                          "app": str(fixture_binary.parents[2]), "bundleId": identifier}), flush=True)
        samples = []
        start = time.monotonic()
        deadline = start + args.warmup + args.seconds + 30
        completed = False
        try:
            while child.poll() is None and time.monotonic() < deadline:
                samples.append({"elapsedSeconds": time.monotonic() - start, "unixSeconds": time.time(),
                                "cpuSeconds": cpu_seconds(child.pid)})
                if metrics.exists():
                    try:
                        observation = json.loads(metrics.read_text())
                        if observation.get("pid") != child.pid:
                            raise RuntimeError("metrics came from a different process; observation rejected")
                        completed = True
                        break
                    except (json.JSONDecodeError, OSError):
                        pass
                time.sleep(1)
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            (output / "cpu-samples.json").write_text(json.dumps(samples, indent=2) + "\n")
        if not completed:
            raise SystemExit(f"No complete observation; inspect {output / 'native.log'}")
        print(f"Observation saved: {metrics}", flush=True)


if __name__ == "__main__":
    main()
