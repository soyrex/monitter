#!/usr/bin/env python3
"""Validate the checked-in, explicitly synthetic Jev evaluation fixtures."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "monitter-routing-evaluation-v2"
QUESTION_SCHEMA = "monitter-jev-questions-v1"
POLICY = "monitter-model-routing-v1"
TASK_KINDS = {"answer", "investigate", "localized_edit", "bug_fix", "refactor", "architecture", "production_sensitive"}
TIERS = {"fast", "balanced", "strong", "frontier"}
REASONING = {"low", "medium", "high", "xhigh"}
EXECUTION = {"answer", "inspect", "edit"}
PERMISSIONS = {"read_only", "workspace_write", "shell_and_tests", "human_review_required"}


def read(path):
    with path.open(encoding="utf-8") as source:
        return json.load(source)


corpus = read(ROOT / "src-tauri/evaluation/jev-routing-corpus-v2.json")
outcomes = read(ROOT / "src-tauri/evaluation/jev-routing-replay-v2.json")
assert corpus["schemaVersion"] == SCHEMA
assert corpus["provenance"] == "synthetic"
assert corpus["questionSchemaVersion"] == QUESTION_SCHEMA
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
assert len(outcomes) == len(corpus["cases"]) * 2
assert all(outcome["costUsd"] is None and outcome["latencyMs"] is None for outcome in outcomes)
assert all(outcome["taskCompleted"] is None and outcome["tests"] is None for outcome in outcomes)
assert all(outcome["evidenceIds"] and outcome["repeatId"] for outcome in outcomes)
assert len({(outcome["routeId"], outcome["caseId"], outcome["repeatId"]) for outcome in outcomes}) == len(outcomes)

decision_count = 0
for outcome in outcomes:
    decision = outcome["classifierDecision"]
    if decision is None:
        assert outcome["routeId"] != "jev_route"
        assert outcome["routingPolicyVersion"] == "fixed-strong-baseline-v1"
        assert outcome["questionSchemaVersion"] == "not_applicable"
        continue
    decision_count += 1
    assert outcome["routeId"] == "jev_route"
    assert outcome["routingPolicyVersion"] == POLICY
    assert outcome["questionSchemaVersion"] == QUESTION_SCHEMA
    assert outcome["classifierModel"] and outcome["classifierModelVersion"]
    options = {
        "task_kind": TASK_KINDS,
        "model_tier": TIERS,
        "reasoning_level": REASONING,
        "execution_mode": EXECUTION,
        "permission_tier": PERMISSIONS,
    }
    probabilities = decision["question_probabilities"]
    confidences = decision["question_confidences"]
    for question, known in options.items():
        distribution = probabilities[question]
        assert set(distribution) == known
        assert abs(sum(distribution.values()) - 1.0) < 0.01
        assert all(0 <= value <= 1 for value in distribution.values())
        selected = decision[question]
        assert abs(distribution[selected] - confidences[question]) < 0.01
    assert abs(min(confidences.values()) - decision["confidence"]) < 0.01

assert decision_count == len(corpus["cases"])
print(f"Validated {len(corpus['cases'])} synthetic labeled cases, {decision_count} captured-decision replays, and {len(outcomes)} synthetic outcomes; no measured results are present.")
