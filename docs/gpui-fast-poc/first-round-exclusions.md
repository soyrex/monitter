# First-round benchmark exclusions

The initial 18-session comparison (`artifacts/gpui-fast-poc/reference` and
`artifacts/gpui-fast-poc/treatment`) is excluded in full from the performance
comparison. Keep its raw artifacts for audit only. Do not combine any of these
runs with the guarded rerun.

Two compilation intervals overlapped fixture processes. Times are UTC; fixture
runtime uses `session.json.startedAtUtc` through the OS sampler's
`finishedAtUtc`.

| Compile interval | Overlapping run | Fixture overlap |
| --- | --- | --- |
| 11:06:29.554–11:09:09.933 | treatment/idle-01 | 11:06:29.554–11:06:36.853 |
| 11:06:29.554–11:09:09.933 | reference/streaming-body-02 | 11:08:11.503–11:09:09.933 |
| 11:06:29.554–11:09:09.933 | treatment/streaming-body-02 | 11:06:42.466–11:08:05.936 |
| 11:09:35.903–11:11:42.800 | reference/pipeline-updates-02 | 11:11:12.105–11:11:42.800 |
| 11:09:35.903–11:11:42.800 | treatment/pipeline-updates-02 | 11:09:40.722–11:11:06.547 |

The autonomous workload collector generates no keystrokes or pointer input,
but first-round input-latency histogram counts were nonzero in several runs.
Most notably, reference `streaming-body-01` recorded 161 samples and reference
`pipeline-updates-01` recorded 148; their matched treatment runs recorded 13 and
7. Other first-round sessions recorded 0–6 samples. The native logs for the two
high-count reference runs do not identify focus, visibility, or input events.
The pipeline-updates pair logs contain AppKit
`IMKCFRunLoopWakeUpReliable` errors, but those do not establish that user input
caused the samples. GPUI's aggregate export omits each platform-input event
kind, so the cause cannot be determined from retained artifacts.

These samples mean the input-latency histogram is not a pure workload measure.
Because external platform input can also perturb draw timing, treat the affected
pairs as contaminated. Together with compile overlap and fewer than three
uncontaminated matched pairs per scenario, the complete first round is
inconclusive and supplies no performance claim. A new guarded, paired round is
required. The guarded rerun is stored separately under
`artifacts/gpui-fast-poc/guarded-five-pair/`.
