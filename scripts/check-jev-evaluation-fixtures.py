#!/usr/bin/env python3
"""Validate the checked-in, explicitly synthetic Jev evaluation fixtures."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "monitter-routing-evaluation-v1"


def read(path):
    with path.open(encoding="utf-8") as source:
        return json.load(source)


corpus = read(ROOT / "src-tauri/evaluation/jev-routing-corpus-v1.json")
outcomes = read(ROOT / "src-tauri/evaluation/jev-routing-replay-v1.json")
assert corpus["schemaVersion"] == SCHEMA
assert corpus["provenance"] == "synthetic"
assert 1 <= len(corpus["cases"]) <= 500
case_ids = [case["id"] for case in corpus["cases"]]
assert len(case_ids) == len(set(case_ids))
assert {case["label"] for case in corpus["cases"]} >= {
    "explanation", "investigation", "localized_edit", "bug_fix", "refactor",
    "architecture", "production_sensitive",
}
assert all(case["expectedMinimumTier"] in {"fast", "balanced", "strong", "frontier"} for case in corpus["cases"])
assert all(outcome["schemaVersion"] == SCHEMA and outcome["provenance"] == "synthetic" for outcome in outcomes)
assert all(outcome["caseId"] in set(case_ids) for outcome in outcomes)
assert len({outcome["runId"] for outcome in outcomes}) == len(outcomes)
assert all(outcome["costUsd"] is None and outcome["latencyMs"] is None for outcome in outcomes)
print(f"Validated {len(corpus['cases'])} synthetic labeled cases and {len(outcomes)} synthetic replay outcomes.")
