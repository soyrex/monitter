#!/usr/bin/env python3
"""Compare repeated GPUI profiling sessions without making significance claims."""

import argparse
import json
import math
from pathlib import Path
import statistics
import sys


SUMMARY_FIELDS = (
    "inputToPlatformSubmitMs",
    "drawMs",
    "dirtyToPlatformSubmitMs",
    "animationFrameIntervalMs",
    "inputEventsPerFrame",
)
COUNTER_FIELDS = (
    "transcriptPreparations",
    "transcriptRowsPrepared",
    "chatChromeRenders",
    "detailBodyConstructions",
    "midDrawInputEventsExcluded",
    "outlineShows",
    "outlineHides",
    "outlineGeometryPaints",
)
MATCH_FIELDS = (
    "scenario",
    "motionMode",
    "historyRowsPerPane",
    "chatPanes",
    "warmupSeconds",
    "observationSeconds",
    "inputSource",
)


class CompareError(Exception):
    pass


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise CompareError(f"cannot read valid JSON from {path}: {error}") from error


def run_directories(root):
    root = root.resolve(strict=True)
    candidates = [root] if (root / "metrics.json").is_file() else []
    candidates.extend(path.parent for path in root.rglob("metrics.json"))
    unique = sorted(set(candidates))
    if not unique:
        raise CompareError(f"no metrics.json sessions found under {root}")
    return unique


def require(condition, message):
    if not condition:
        raise CompareError(message)


def numeric(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def load_conditions(root):
    path = root.resolve(strict=True) / "machine-conditions.json"
    conditions = read_json(path)
    require(isinstance(conditions, dict) and conditions.get("schemaVersion") == 1,
            f"{path}: expected machine-conditions schemaVersion 1")
    machine = conditions.get("machine")
    require(isinstance(machine, dict), f"{path}: machine object is required")
    for field in ("system", "architecture", "platformRelease"):
        require(isinstance(machine.get(field), str) and machine[field],
                f"{path}: machine.{field} must be a non-empty string")
    for field in ("macVersion", "cpuModel"):
        require(machine.get(field) is None or isinstance(machine[field], str),
                f"{path}: machine.{field} must be a string or null")
    memory = machine.get("physicalMemoryBytes")
    require(memory is None or (isinstance(memory, int) and not isinstance(memory, bool) and memory > 0),
            f"{path}: machine.physicalMemoryBytes must be a positive integer or null")
    display = conditions.get("display")
    require(isinstance(display, dict), f"{path}: display object is required")
    viewport = display.get("fixtureViewportPoints")
    require(isinstance(viewport, dict) and viewport.get("width") == 1200
            and viewport.get("height") == 820,
            f"{path}: fixtureViewportPoints must match the 1200x820 fixture window")
    require(display.get("theme") == "dark", f"{path}: theme must be dark")
    scale = display.get("scale")
    require(scale is None or (numeric(scale) and scale > 0),
            f"{path}: display.scale must be positive or null")
    power = conditions.get("power")
    require(isinstance(power, dict) and power.get("source") in {"AC", "battery", "unknown"},
            f"{path}: power.source must be AC, battery, or unknown")
    require(power.get("mode") is None or isinstance(power["mode"], str),
            f"{path}: power.mode must be a string or null")
    return conditions


def load_run(path):
    session = read_json(path / "session.json")
    metrics = read_json(path / "metrics.json")
    build = session.get("build")
    require(isinstance(build, dict), f"{path}: session.json has no build metadata")
    require(metrics.get("schemaVersion") == 1, f"{path}: unsupported metrics schema")
    require(metrics.get("profilerEnabled") is True, f"{path}: profiler is not enabled")
    require(metrics.get("debugAssertions") is False, f"{path}: debug result refused")
    require(metrics.get("visualExport") is False, f"{path}: visual-export result refused")
    require(build.get("performanceProfiling") is True, f"{path}: build is not a profiling build")
    require(build.get("visualExport") is False, f"{path}: visual-export build refused")
    require(build.get("profile") == "release", f"{path}: expected a release build")
    require(session.get("sourceSha256") == build.get("desktopSha256"),
            f"{path}: session source hash does not match build.json metadata")
    require(session.get("pid") == metrics.get("pid"), f"{path}: session and metrics PIDs differ")
    require(numeric(metrics.get("requestedSeconds")) and metrics["requestedSeconds"] > 0
            and numeric(metrics.get("measuredSeconds")) and metrics["measuredSeconds"] > 0,
            f"{path}: observation duration is missing or zero")

    for field in MATCH_FIELDS:
        require(field in session, f"{path}: session missing comparison field {field}")
    key = tuple(session[field] for field in MATCH_FIELDS)

    values = {}
    for field in SUMMARY_FIELDS:
        summary = metrics.get(field)
        count = summary.get("count") if isinstance(summary, dict) else None
        if not numeric(count) or count < 1:
            for quantile in ("p50", "p95", "p99"):
                values[f"{field}.{quantile}"] = None
            continue
        for quantile in ("p50", "p95", "p99"):
            value = summary.get(quantile)
            values[f"{field}.{quantile}"] = value if numeric(value) else None
    for field in COUNTER_FIELDS:
        value = metrics.get(field)
        values[field] = value if numeric(value) else None

    cpu_path = path / "cpu-samples.json"
    cpu_rate = None
    cpu_seconds = None
    cpu_wall_seconds = None
    if cpu_path.is_file():
        samples = read_json(cpu_path)
        if isinstance(samples, list):
            valid = [sample for sample in samples if isinstance(sample, dict)
                     and numeric(sample.get("elapsedSeconds"))
                     and numeric(sample.get("cpuSeconds"))]
            if len(valid) >= 2:
                cpu_wall_seconds = valid[-1]["elapsedSeconds"] - valid[0]["elapsedSeconds"]
                cpu_seconds = valid[-1]["cpuSeconds"] - valid[0]["cpuSeconds"]
                if cpu_wall_seconds > 0 and cpu_seconds >= 0:
                    cpu_rate = cpu_seconds / cpu_wall_seconds
    values["processCpuSecondsPerWallSecond"] = cpu_rate
    values["processCpuSecondsObserved"] = cpu_seconds
    values["processCpuSampleWallSeconds"] = cpu_wall_seconds

    os_sample_path = path / "os-samples.json"
    os_samples_present = os_sample_path.is_file()
    if os_samples_present:
        os_data = read_json(os_sample_path)
        require(os_data.get("schemaVersion") == 2, f"{path}: OS samples must use schemaVersion 2 with explicit Mach units")
        require(os_data.get("pid") == session.get("pid"), f"{path}: OS sample PID differs from session")
        require(os_data.get("complete") is True, f"{path}: OS sample is incomplete")
        require(numeric(os_data.get("sampleIntervalSeconds"))
                and os_data["sampleIntervalSeconds"] > 0,
                f"{path}: OS sample interval is missing or invalid")
        require(os_data.get("cpuTimeUnit") == "mach_absolute_time_ticks",
                f"{path}: OS CPU samples are not tagged as Mach timebase ticks")
        timebase = os_data.get("machTimebase")
        require(isinstance(timebase, dict)
                and isinstance(timebase.get("numer"), int) and timebase["numer"] > 0
                and isinstance(timebase.get("denom"), int) and timebase["denom"] > 0
                and numeric(timebase.get("nanosecondsPerTick"))
                and timebase["nanosecondsPerTick"] == timebase["numer"] / timebase["denom"],
                f"{path}: OS sample has missing or inconsistent Mach timebase")
        samples = os_data.get("samples")
        require(isinstance(samples, list) and len(samples) >= 2,
                f"{path}: OS sample requires at least two records")
        identity = os_data.get("processIdentity")
        require(isinstance(identity, dict) and identity.get("uuid")
                and identity.get("procStartAbstime"),
                f"{path}: OS sample has no process identity")
        metrics_start_ms = metrics.get("measurementStartedAtUnixMs")
        metrics_finish_ms = metrics.get("measurementFinishedAtUnixMs")
        require(numeric(metrics_start_ms) and numeric(metrics_finish_ms)
                and metrics_finish_ms > metrics_start_ms,
                f"{path}: metrics lack a valid measured time window")
        for field in ("unixTimeNs", "userTimeTicks", "systemTimeTicks", "packageIdleWakeups",
                      "interruptWakeups", "pageIns", "residentBytes", "physicalFootprintBytes"):
            require(all(isinstance(sample, dict) and numeric(sample.get(field)) for sample in samples),
                    f"{path}: OS sample records are missing {field}")
        window_start_ns = metrics_start_ms * 1_000_000
        window_finish_ns = metrics_finish_ms * 1_000_000
        window_samples = [sample for sample in samples
                          if window_start_ns <= sample["unixTimeNs"] <= window_finish_ns]
        require(len(window_samples) >= 2,
                f"{path}: fewer than two OS samples fall inside the measured metrics window")
        first, last = window_samples[0], window_samples[-1]
        elapsed = (last["unixTimeNs"] - first["unixTimeNs"]) / 1_000_000_000
        cpu_ticks = ((last["userTimeTicks"] + last["systemTimeTicks"])
                     - (first["userTimeTicks"] + first["systemTimeTicks"]))
        cpu = cpu_ticks * timebase["numer"] / timebase["denom"] / 1_000_000_000
        require(elapsed > 0 and cpu >= 0, f"{path}: invalid OS sample time or CPU delta")
        values.update({
            "osSampledProcessCpuSeconds": cpu,
            "osSampledProcessCpuSecondsPerWallSecond": cpu / elapsed,
            "osSampleWindowWallSeconds": elapsed,
            "osSamplePeakResidentBytes": max(sample["residentBytes"] for sample in window_samples),
            "osSamplePeakPhysicalFootprintBytes": max(sample["physicalFootprintBytes"] for sample in window_samples),
            "osSamplePeakWiredBytes": max((sample.get("wiredBytes") or 0) for sample in window_samples),
            "osSamplePackageIdleWakeupDelta": last["packageIdleWakeups"] - first["packageIdleWakeups"],
            "osSampleInterruptWakeupDelta": last["interruptWakeups"] - first["interruptWakeups"],
            "osSamplePageInDelta": last["pageIns"] - first["pageIns"],
            "osSampleCount": len(window_samples),
            "osSampleIntervalSeconds": os_data.get("sampleIntervalSeconds"),
            "osSampleWindowStartUnixNs": first["unixTimeNs"],
            "osSampleWindowFinishUnixNs": last["unixTimeNs"],
        })

    return {
        "path": str(path),
        "startedAtUtc": session.get("startedAtUtc"),
        "key": key,
        "settings": dict(zip(MATCH_FIELDS, key)),
        "sourceCommit": build.get("sourceCommit"),
        "sourceDirty": build.get("sourceDirty"),
        "osSamplesPresent": os_samples_present,
        "values": values,
    }


def median_absolute_deviation(values):
    center = statistics.median(values)
    return statistics.median(abs(value - center) for value in values)


def summarize(reference_runs, treatment_runs, conditions):
    keys = {run["key"] for run in reference_runs} | {run["key"] for run in treatment_runs}
    reference_keys = {run["key"] for run in reference_runs}
    treatment_keys = {run["key"] for run in treatment_runs}
    require(reference_keys == treatment_keys,
            "reference and treatment scenario/settings groups differ; refusing unmatched comparison")
    groups = []
    for key in sorted(keys, key=str):
        refs = [run for run in reference_runs if run["key"] == key]
        treatments = [run for run in treatment_runs if run["key"] == key]
        os_sample_flags = {run["osSamplesPresent"] for run in refs + treatments}
        require(len(os_sample_flags) == 1,
                "OS sampling is present for only some runs in a matched group; refusing mixed comparison")
        metric_names = sorted(set.intersection(
            *(set(run["values"]) for run in refs + treatments)
        ))
        metrics = {}
        for name in metric_names:
            left = [run["values"][name] for run in refs if numeric(run["values"].get(name))]
            right = [run["values"][name] for run in treatments if numeric(run["values"].get(name))]
            if not left or not right:
                metrics[name] = {"available": False}
                continue
            left_median = statistics.median(left)
            right_median = statistics.median(right)
            delta = right_median - left_median
            metrics[name] = {
                "available": True,
                "reference": {"n": len(left), "median": left_median, "min": min(left),
                              "max": max(left), "mad": median_absolute_deviation(left)},
                "treatment": {"n": len(right), "median": right_median, "min": min(right),
                              "max": max(right), "mad": median_absolute_deviation(right)},
                "deltaTreatmentMinusReference": delta,
                "deltaPercent": delta / abs(left_median) * 100 if left_median else None,
            }
        enough = len(refs) >= 3 and len(treatments) >= 3
        groups.append({
            "settings": dict(zip(MATCH_FIELDS, key)),
            "repeatabilityStatus": "descriptive_only_fewer_than_3_runs" if not enough
                                   else "descriptive_repeated_runs_not_significance_test",
            "referenceRunCount": len(refs),
            "treatmentRunCount": len(treatments),
            "osSamplesIncluded": os_sample_flags.pop(),
            "referenceRuns": refs,
            "treatmentRuns": treatments,
            "metrics": metrics,
        })
    return {
        "schemaVersion": 1,
        "machineConditions": conditions,
        "claimLimit": "descriptive comparison; no statistical significance or causal claim",
        "memory": "optional OS sample reports sampled peak resident/physical-footprint bytes; not allocation or exact peak measurement",
        "wakeups": "optional OS sample reports package idle-wakeup and interrupt-wakeup counter deltas; these are distinct counters, not all process wakeups",
        "groups": groups,
    }


def fmt(value):
    return "n/a" if value is None else f"{value:.3f}"


def print_report(report):
    for group in report["groups"]:
        settings = group["settings"]
        print(f"{settings['scenario']} rows={settings['historyRowsPerPane']} "
              f"motion={settings['motionMode']} panes={settings['chatPanes']}: "
              f"reference n={group['referenceRunCount']}, treatment n={group['treatmentRunCount']} "
              f"({group['repeatabilityStatus']})")
        for name, item in group["metrics"].items():
            if not item["available"]:
                continue
            left = item["reference"]
            right = item["treatment"]
            print(f"  {name}: median {fmt(left['median'])} [{fmt(left['min'])}, {fmt(left['max'])}]"
                  f" -> {fmt(right['median'])} [{fmt(right['min'])}, {fmt(right['max'])}]; "
                  f"delta {fmt(item['deltaTreatmentMinusReference'])}"
                  f" ({fmt(item['deltaPercent'])}%)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path, help="directory containing reference session folders")
    parser.add_argument("treatment", type=Path, help="directory containing treatment session folders")
    parser.add_argument("--json", action="store_true", help="emit the full JSON report")
    args = parser.parse_args()
    try:
        reference_conditions = load_conditions(args.reference)
        treatment_conditions = load_conditions(args.treatment)
        require(reference_conditions == treatment_conditions,
                "machine-conditions.json differs between reference and treatment roots")
        reference = [load_run(path) for path in run_directories(args.reference)]
        treatment = [load_run(path) for path in run_directories(args.treatment)]
        report = summarize(reference, treatment, reference_conditions)
    except (CompareError, OSError) as error:
        parser.error(str(error))
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_report(report)


if __name__ == "__main__":
    main()
