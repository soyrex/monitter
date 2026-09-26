# Offline Jev routing evaluation

`harness eval` is an offline replay and evidence summarizer. It does not call
Jev, Codex, or another provider. The checked-in corpus and replay are
`synthetic`; their classifier decisions exercise replay behavior but make no
claim about live classifier quality or agent outcomes.

The corpus labels each case with an expected minimum tier. The v2 replay
records the typed classifier decision, its complete question probability
distributions, classifier model/version, routing-policy version,
question-schema version, repeat ID, and opaque evidence IDs. Replay invokes
the same native eligibility function used by new-chat auto-routing. The report
shows the selected tier only when that policy is eligible; otherwise it marks
the decision as abstained/recommended. Eligible decisions are counted as
underallocated, meeting the expected minimum, or above it. This is a review
label comparison, not an execution permission or a claim of task quality.
New TypeSafe Choice responses require a complete finite probability
distribution for each question; older stored routing traces without these
distributions remain deserializable for compatibility but cannot be imported
as v2 replay decisions.

Outcome quality and performance fields (`taskCompleted`, `tests`, `costUsd`,
and `latencyMs`) are optional observations. Missing values remain null and do
not become successful completions or zero cost/latency. Route summaries report
observation counts. Paired comparisons match only identical `caseId` and
`repeatId` values across routes; the report shows unmatched counts, excludes
unmatched rows from paired means, and defines each delta as left route minus
right route. Synthetic fixture outcomes have all quality, cost, and latency
fields unset.

## Importing outcomes

Prepare a v2 JSON corpus and outcomes file from deliberately collected,
authorized observations, then run:

```sh
harness eval --corpus ./corpus.json --outcomes ./outcomes.json
```

Files declare `provenance` as `synthetic` or `measured`; one report cannot mix
the two. Outcome entries must use a supported `schemaVersion`, identify a
policy and question-schema version, include one or more opaque `evidenceIds`,
and provide a stable `repeatId`. A `jev_route` outcome must include the typed
`classifierDecision`, complete finite probability distributions, and
`classifierModel` plus `classifierModelVersion`. Non-classifier baselines use
`routingPolicyVersion: "fixed-strong-baseline-v1"`,
`questionSchemaVersion: "not_applicable"`, and a null decision. Unknown case
IDs, duplicate route/case/repeat observations, malformed distributions,
unsupported versions, inconsistent test counts, and out-of-range values are
rejected. Corpus and outcome input files are bounded to 2 MiB and 8 MiB,
respectively, before their bytes are read into memory.

Evidence IDs are caller-supplied references only. Do not include prompts,
transcripts, secrets, or credentials in this evaluation format. This change
adds an import path; it does not collect live runs or initiate billable calls.

Validate the checked-in fixtures without a Rust build:

```sh
python3 scripts/check-jev-evaluation-fixtures.py
```
