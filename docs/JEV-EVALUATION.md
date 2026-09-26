# Jev routing evaluation

`harness eval` is an offline replay. It never reads credentials, contacts TypeSafe, starts a coding agent, or reads local transcripts. By default it loads `src-tauri/evaluation/jev-routing-corpus-v1.json` and `src-tauri/evaluation/jev-routing-replay-v1.json` and prints a report whose provenance is `synthetic`. The checked-in replay intentionally leaves cost and latency absent; the report preserves those as `null` and excludes them from averages.

Use `harness eval --corpus <corpus.json> --outcomes <outcomes.json>` to compare supplied outcomes. Both files use schema `monitter-routing-evaluation-v1`. The corpus has an explicit `provenance` (`synthetic` or `measured`) and labeled cases with a bounded prompt and expected minimum model tier. Outcomes are a JSON array containing a unique `runId`, known `caseId`, comparison `routeId`, provider and model names and optional versions, provenance, optional task completion and test evidence, optional cost, and optional latency. The importer limits corpus size, text lengths, run counts, numeric ranges, duplicate IDs, and test-count consistency. One report cannot mix synthetic and measured outcomes.

An imported `measured` label is caller supplied. Monitter does not collect it or verify a remote run; the person preparing the file is responsible for recording actual observed evidence. The report includes provider/model versions and separate observation counts. Missing measurements remain `null`; no cost, latency, completion, or test result is inferred from a missing field. Keep an imported file's provenance `synthetic` when any values are illustrative.

Example measured outcome shape (values below are placeholders, not real measurements):

```json
[
  {
    "schemaVersion": "monitter-routing-evaluation-v1",
    "provenance": "measured",
    "runId": "your-unique-run-id",
    "caseId": "answer-explain-retry",
    "routeId": "jev_route",
    "provider": "your-provider",
    "providerVersion": "your-provider-version",
    "model": "your-model",
    "modelVersion": "your-model-version",
    "taskCompleted": true,
    "tests": null,
    "costUsd": null,
    "latencyMs": null
  }
]
```

Route traces carry `policyVersion` (`monitter-model-routing-v1`) and `questionSchemaVersion` (`monitter-jev-questions-v1`). `classifierEvidence.model` records the requested/returned model id and `classifierEvidence.modelVersion` retains an explicit version when TypeSafe supplies one; the native applied policy is retained in `autoRoutePolicy`. New live TypeSafe Choice responses must include a probability for every declared option. Each probability must be finite and within 0..1, and each distribution must sum to 1 within 0.025. These per-question distributions are kept beside the confidence values for calibration work. Old traces without probabilities or version fields remain deserializable and are marked `legacy-unspecified`.

The routing confidence floor remains 0.30. Evaluation data is intended to inform a future deliberate policy review; this change does not alter routing thresholds or permission authority.

Validate the checked-in offline fixtures without a Rust build:

```sh
python3 scripts/check-jev-evaluation-fixtures.py
```
