#!/usr/bin/env python3
"""Select complete, compiler-guarded, zero-input GPUI Fast benchmark pairs."""

import argparse
import datetime as dt
import json
import math
from pathlib import Path
import re
import shutil
import sys


SCENARIOS = ("idle", "pipeline-updates", "streaming-body")
SIDES = ("reference", "treatment")
SETTINGS = ("scenario", "motionMode", "historyRowsPerPane", "chatPanes",
            "warmupSeconds", "observationSeconds", "inputSource")
RUN_NAME = re.compile(r"^(idle|pipeline-updates|streaming-body)-(\d+)$")
REQUIRED_FILES = (
    "session.json", "metrics.json", "os-samples.json", "collection-receipt.json",
    "environment-guard.json", "build.json",
)


class SelectionError(Exception):
    pass


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise SelectionError(f"cannot read JSON {path}: {error}") from error


def parse_utc(value, label):
    if not isinstance(value, str):
        raise SelectionError(f"{label} is missing")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise SelectionError(f"{label} is not an ISO timestamp") from error
    if parsed.tzinfo is None:
        raise SelectionError(f"{label} must include a timezone")
    return parsed.timestamp() * 1000


def number(value):
    return (isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value))


def discover_roots(artifact_root, explicit_roots):
    if explicit_roots:
        roots = list(dict.fromkeys(path.resolve(strict=True) for path in explicit_roots))
    else:
        names = ("guarded-five-pair", "guarded-continuation", "guarded-extra")
        roots = [(artifact_root / name).resolve() for name in names
                 if (artifact_root / name).is_dir()]
    if not roots:
        raise SelectionError("no guarded source roots found")
    return roots


def run_candidates(roots):
    found = []
    for root in roots:
        for side in SIDES:
            side_root = root / side
            if not side_root.is_dir():
                continue
            for run_dir in sorted(side_root.iterdir(), key=lambda path: path.name):
                if not run_dir.is_dir():
                    continue
                match = RUN_NAME.fullmatch(run_dir.name)
                if not match:
                    continue
                scenario, repetition_text = match.groups()
                found.append({
                    "scenario": scenario,
                    "repetition": int(repetition_text),
                    "side": side,
                    "path": run_dir.resolve(),
                    "sourceRoot": root,
                })
    return found


def inspect_window_conditions(metrics, scenario):
    conditions = metrics.get("windowConditions")
    if not isinstance(conditions, dict) or type(conditions.get("schemaVersion")) is not int or conditions["schemaVersion"] != 1:
        return ["window conditions diagnostic is missing or not schema v1"]
    reasons = []
    duration = conditions.get("durationSeconds")
    measured = metrics.get("measuredSeconds")
    if not number(duration) or duration <= 0 or not number(measured) or abs(duration - measured) > 0.25:
        reasons.append("window conditions duration does not cover the measured duration within 0.25 seconds")
    if conditions.get("fullyActiveVisible") is not True:
        reasons.append("window was not fully active and visible throughout measurement")
    for field in ("activeSeconds", "visibleSeconds"):
        value = conditions.get(field)
        if not number(value) or value < 0 or not number(duration) or abs(value-duration) > 0.25:
            reasons.append(f"window conditions {field} does not match diagnostic duration")
    transitions = conditions.get("transitions")
    if not isinstance(transitions, list) or not transitions:
        reasons.append("window conditions have no initial state or transition timeline")
    else:
        previous = 0.0
        for index, transition in enumerate(transitions):
            if not isinstance(transition, dict):
                reasons.append("window conditions transition is malformed")
                continue
            elapsed = transition.get("elapsedSeconds")
            if not number(elapsed) or elapsed < previous or not number(duration) or elapsed > duration or (index == 0 and elapsed != 0):
                reasons.append("window conditions transition has invalid elapsed time")
            elif number(elapsed):
                previous = elapsed
            if transition.get("active") is not True or transition.get("visible") is not True:
                reasons.append("window conditions timeline includes inactive or hidden state")
    updates = conditions.get("workloadUpdateCallbacks")
    if type(updates) is not int or updates < 0:
        reasons.append("window conditions workload callback count is missing or invalid")
    elif scenario in ("streaming-body", "pipeline-updates") and updates == 0:
        reasons.append("autonomous workload has no measured update callbacks")
    draw = metrics.get("drawMs")
    count = draw.get("count") if isinstance(draw, dict) else None
    if type(count) is not int or count <= 0:
        reasons.append("measured window has no valid draw samples")
    return reasons


def inspect_run(candidate, max_guard_gap_seconds):
    path = candidate["path"]
    reasons = []
    data = {}
    for name in REQUIRED_FILES:
        try:
            data[name] = read_json(path / name)
        except SelectionError as error:
            reasons.append(str(error))
    side_conditions_path = candidate["sourceRoot"] / candidate["side"] / "machine-conditions.json"
    try:
        data["machineConditions"] = read_json(side_conditions_path)
    except SelectionError as error:
        reasons.append(str(error))
    if reasons:
        return candidate_record(candidate, reasons, None)

    session = data["session.json"]
    metrics = data["metrics.json"]
    os_data = data["os-samples.json"]
    receipt = data["collection-receipt.json"]
    guard = data["environment-guard.json"]
    build = data["build.json"]

    reasons.extend(inspect_window_conditions(metrics, candidate["scenario"]))
    source_build = session.get("build")
    if not isinstance(source_build, dict):
        reasons.append("session source build metadata is missing")
        source_build = {}
    for field in ("profile", "visualExport", "performanceProfiling", "lanAssets", "sourceCommit", "sourceDirty", "builtAt"):
        if field not in build or field not in source_build or build[field] != source_build[field]:
            reasons.append(f"fixture and source build metadata differ or lack {field}")
    for session_field, metadata in (("sha256", build), ("sourceSha256", source_build)):
        value = session.get(session_field)
        if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value) or value != metadata.get("desktopSha256"):
            reasons.append(f"session {session_field} does not match verified build metadata")
    if metrics.get("profilerEnabled") is not True:
        reasons.append("metrics profiler is not enabled")
    if any(field not in session for field in SETTINGS):
        reasons.append("session comparison settings are incomplete")
    for session_field, metrics_field in (("warmupSeconds", "warmupSeconds"), ("observationSeconds", "requestedSeconds")):
        if not number(session.get(session_field)) or session[session_field] <= 0 or session[session_field] != metrics.get(metrics_field):
            reasons.append(f"session {session_field} differs from metrics {metrics_field}")

    if session.get("scenario") != candidate["scenario"]:
        reasons.append("session scenario does not match run folder")
    if not isinstance(session.get("pid"), int) or isinstance(session.get("pid"), bool):
        reasons.append("session PID is missing or invalid")
    else:
        pid = session["pid"]
        if metrics.get("pid") != pid:
            reasons.append("session and metrics PIDs differ")
        if os_data.get("pid") != pid:
            reasons.append("session and OS sample PIDs differ")
        if receipt.get("fixturePid") != pid:
            reasons.append("session and collection receipt PIDs differ")

    if receipt.get("collectorExitCode") != 0 or receipt.get("samplerExitCode") != 0:
        reasons.append("collector or OS sampler did not exit successfully")
    if metrics.get("schemaVersion") != 1:
        reasons.append("unsupported metrics schema")
    if metrics.get("visualExport") is not False or metrics.get("debugAssertions") is not False:
        reasons.append("metrics are not release non-visual-export observations")
    if build.get("profile") != "release" or build.get("visualExport") is not False \
            or build.get("performanceProfiling") is not True:
        reasons.append("build is not a profiling release without visual export")
    if os_data.get("schemaVersion") != 2 or os_data.get("complete") is not True:
        reasons.append("OS samples are incomplete or not schema v2")

    histogram = metrics.get("inputToPlatformSubmitMs")
    count = histogram.get("count") if isinstance(histogram, dict) else None
    if not isinstance(count, int) or isinstance(count, bool) or count < 0:
        reasons.append("input-latency histogram count is missing or invalid")
    elif count != 0:
        reasons.append(f"input-latency histogram has {count} samples (requires zero)")

    try:
        runtime_start = parse_utc(session.get("startedAtUtc"), "session.startedAtUtc")
        sampler_finish = parse_utc(os_data.get("finishedAtUtc"), "os-samples.finishedAtUtc")
    except SelectionError as error:
        reasons.append(str(error))
        runtime_start = sampler_finish = None
    start_ms = metrics.get("measurementStartedAtUnixMs")
    finish_ms = metrics.get("measurementFinishedAtUnixMs")
    if not number(start_ms) or not number(finish_ms) or finish_ms <= start_ms:
        reasons.append("metrics lack a valid measured time window")
    elif not number(metrics.get("measuredSeconds")) or metrics["measuredSeconds"] < 60:
        reasons.append("measured duration is missing or shorter than 60 seconds")

    runtime_finish = finish_ms if number(finish_ms) else None
    if runtime_start is not None and runtime_finish is not None:
        if runtime_finish <= runtime_start:
            reasons.append("metrics finish does not follow session start")
        if sampler_finish is None or sampler_finish < runtime_finish:
            reasons.append("OS sampler did not cover the measured window")
        if not number(start_ms) or start_ms < runtime_start:
            reasons.append("metrics window starts before the fixture session")

    if guard.get("schemaVersion") != 1 or guard.get("compilerOverlap") is not False:
        reasons.append("environment guard is missing or reports compiler overlap")
    observations = guard.get("observations")
    parsed_observations = []
    if not isinstance(observations, list) or not observations:
        reasons.append("environment guard has no observations")
    else:
        for observation in observations:
            timestamp = observation.get("unixMs") if isinstance(observation, dict) else None
            compilers = observation.get("compilers") if isinstance(observation, dict) else None
            if not number(timestamp) or not isinstance(compilers, list):
                reasons.append("environment guard has malformed observation")
                continue
            parsed_observations.append((timestamp, compilers))
        parsed_observations.sort(key=lambda item: item[0])
        if any(compilers for _, compilers in parsed_observations):
            reasons.append("environment guard observed compiler processes")
        if runtime_start is not None and runtime_finish is not None and parsed_observations:
            times = [timestamp for timestamp, _ in parsed_observations]
            if times[0] > runtime_start or times[-1] < runtime_finish:
                reasons.append("environment guard observations do not cover fixture runtime")
            max_gap = max((right - left for left, right in zip(times, times[1:])), default=0)
            if max_gap > max_guard_gap_seconds * 1000:
                reasons.append(
                    f"environment guard sampling gap {max_gap / 1000:.3f}s exceeds "
                    f"{max_guard_gap_seconds:.3f}s"
                )

    record = candidate_record(candidate, reasons, data.get("machineConditions"), count)
    record["settings"] = {field: session.get(field) for field in SETTINGS}
    record["sourceBuild"] = source_build
    record["fixtureSha256"] = session.get("sha256")
    record["windowConditionsSchemaVersion"] = metrics.get("windowConditions", {}).get("schemaVersion") if isinstance(metrics.get("windowConditions"), dict) else None
    return record


def candidate_record(candidate, reasons, machine_conditions, input_count=None):
    return {
        "scenario": candidate["scenario"],
        "repetition": candidate["repetition"],
        "side": candidate["side"],
        "sourcePath": str(candidate["path"]),
        "sourceRoot": str(candidate["sourceRoot"]),
        "inputLatencySampleCount": input_count,
        "eligible": not reasons,
        "exclusionReasons": reasons,
        "machineConditions": machine_conditions,
    }


def make_selection(candidates, minimum_pairs):
    inspected = [inspect_run(item, 2.5) for item in candidates]
    by_key = {}
    for record in inspected:
        by_key.setdefault((record["scenario"], record["repetition"]), {}).setdefault(
            record["side"], []).append(record)

    pairs_by_scenario = {scenario: [] for scenario in SCENARIOS}
    all_pair_records = []
    for (scenario, repetition), sides in sorted(by_key.items()):
        references = sides.get("reference", [])
        treatments = sides.get("treatment", [])
        reasons = []
        if len(references) != 1:
            reasons.append(f"expected one reference run, found {len(references)}")
        if len(treatments) != 1:
            reasons.append(f"expected one treatment run, found {len(treatments)}")
        if len(references) == 1 and len(treatments) == 1:
            ref, treatment = references[0], treatments[0]
            reasons.extend(f"reference: {reason}" for reason in ref["exclusionReasons"])
            reasons.extend(f"treatment: {reason}" for reason in treatment["exclusionReasons"])
            if ref.get("windowConditionsSchemaVersion") != treatment.get("windowConditionsSchemaVersion"):
                reasons.append("reference and treatment window diagnostic versions differ")
            for field in ("profile", "visualExport", "performanceProfiling", "lanAssets"):
                if ref.get("sourceBuild", {}).get(field) != treatment.get("sourceBuild", {}).get(field):
                    reasons.append(f"reference and treatment build feature {field} differs")
            if ref.get("settings") != treatment.get("settings"):
                reasons.append("reference and treatment settings differ")
            if ref["machineConditions"] != treatment["machineConditions"]:
                reasons.append("reference and treatment machine conditions differ")
        pair = {
            "scenario": scenario,
            "repetition": repetition,
            "eligible": not reasons,
            "referencePath": references[0]["sourcePath"] if len(references) == 1 else None,
            "treatmentPath": treatments[0]["sourcePath"] if len(treatments) == 1 else None,
            "exclusionReasons": reasons,
        }
        all_pair_records.append(pair)
        if pair["eligible"]:
            pairs_by_scenario[scenario].append(pair)

    # Reject mixed source builds within a side before choosing earliest repetitions.
    # Fixture hashes vary because each isolated app is signed independently.
    for side in SIDES:
        builds = [record.get("sourceBuild") for record in inspected
                  if record["side"] == side and record["eligible"]]
        if builds and any(build != builds[0] for build in builds[1:]):
            raise SelectionError(f"eligible {side} runs contain different source builds")
    configurations = [record.get("settings") for record in inspected if record["eligible"]]
    for scenario in SCENARIOS:
        settings = [item for item in configurations if item and item["scenario"] == scenario]
        if settings and any(item != settings[0] for item in settings[1:]):
            raise SelectionError(f"eligible {scenario} runs contain different settings")

    selected = {}
    for scenario in SCENARIOS:
        selected[scenario] = sorted(
            pairs_by_scenario[scenario], key=lambda pair: pair["repetition"]
        )[:minimum_pairs]
    enough = all(len(selected[scenario]) >= minimum_pairs for scenario in SCENARIOS)
    return inspected, all_pair_records, selected, enough


def link_selected_runs(output, selected, minimum_pairs):
    roles = {side: [] for side in SIDES}
    conditions = None
    for scenario in SCENARIOS:
        for pair in selected[scenario][:minimum_pairs]:
            for side, key in (("reference", "referencePath"), ("treatment", "treatmentPath")):
                source = Path(pair[key])
                destination = output / side / f"{scenario}-{pair['repetition']:02d}"
                destination.mkdir(parents=True)
                for item in sorted(source.iterdir(), key=lambda path: path.name):
                    if item.is_file():
                        (destination / item.name).symlink_to(item.resolve())
                (destination / "source-run").symlink_to(source, target_is_directory=True)
                roles[side].append(str(source))
                conditions_path = source.parent / "machine-conditions.json"
                current_conditions = read_json(conditions_path)
                if conditions is None:
                    conditions = current_conditions
                elif conditions != current_conditions:
                    raise SelectionError("selected runs have different machine-conditions sidecars")
    if conditions is None:
        raise SelectionError("selected runs have no machine-conditions sidecars")
    for side in SIDES:
        (output / side / "machine-conditions.json").write_text(
            json.dumps(conditions, indent=2, sort_keys=True) + "\n"
        )
    return roles


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-root", type=Path,
                        default=Path("artifacts/gpui-fast-poc"),
                        help="contains guarded-five-pair, guarded-continuation, guarded-extra")
    parser.add_argument("--source-root", type=Path, action="append", default=[],
                        help="guarded source root; may be repeated (default discovers known roots)")
    parser.add_argument("--output", type=Path, required=True,
                        help="new manifest/analysis root; existing paths are refused")
    parser.add_argument("--pairs", type=int, default=5,
                        help="minimum selected matched pairs per scenario (default 5)")
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error("--pairs must be positive")
    output = args.output.resolve()
    if output.exists():
        parser.error(f"refusing to overwrite existing output: {output}")
    try:
        roots = discover_roots(args.artifact_root.resolve(), args.source_root)
        candidates = run_candidates(roots)
        inspected, pair_records, selected, enough = make_selection(candidates, args.pairs)
        conditions = None
        if enough:
            for scenario in SCENARIOS:
                for pair in selected[scenario][:args.pairs]:
                    for side in SIDES:
                        source = Path(pair[f"{side}Path"])
                        current = read_json(source.parent / "machine-conditions.json")
                        if conditions is None:
                            conditions = current
                        elif conditions != current:
                            raise SelectionError("selected runs have different machine-conditions sidecars")
        output.mkdir(parents=True, exist_ok=False)
        linked = {}
        if enough:
            linked = link_selected_runs(output, selected, args.pairs)
        manifest = {
            "schemaVersion": 1,
            "status": "ready_for_descriptive_comparison" if enough else "insufficient_clean_pairs",
            "minimumPairsPerScenario": args.pairs,
            "selectedPairCountByScenario": {scenario: len(selected[scenario]) for scenario in SCENARIOS},
            "selectedPairs": {
                scenario: selected[scenario][:args.pairs] if enough else []
                for scenario in SCENARIOS
            },
            "pairs": pair_records,
            "runs": inspected,
            "analysisRoots": linked if enough else {},
            "selectionRule": (
            "earliest numeric repetitions with exactly one eligible reference and treatment "
                "run, successful collector/sampler receipts, matching PIDs, complete OS samples, "
                "compiler-free guard observations covering the whole fixture runtime, identical "
                "machine conditions, schema-v1 fully-active-visible diagnostics covering measurement, "
                "positive autonomous update callbacks and draw samples, and zero inputToPlatformSubmitMs samples; compiler guard "
                "observations cover session start through the measured-window finish"
            ),
            "maximumEnvironmentGuardGapSeconds": 2.5,
            "maximumWindowConditionsDurationDifferenceSeconds": 0.25,
            "requiredWindowConditionsSchemaVersion": 1,
            "inputCountLimit": (
                "Zero qualifying GPUI input-latency samples is a conservative selection filter; "
                "it does not prove that no OS input or other host load occurred. Nonzero input "
                "event kinds are not preserved in metrics.json."
            ),
            "claimLimit": (
                "A ready selection only supplies eligible raw runs to the descriptive comparator; "
                "it is not a significance, causal, or adoption decision."
            ),
        }
        manifest_path = output / "selection-manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    except (SelectionError, OSError) as error:
        parser.error(str(error))
    print(json.dumps({
        "status": manifest["status"],
        "selectedPairCountByScenario": manifest["selectedPairCountByScenario"],
        "manifest": str(manifest_path),
        "analysisRootsCreated": enough,
    }, indent=2))
    if not enough:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
