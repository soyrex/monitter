# GPUI Fast POC benchmark protocol

## Question and scope

Compare two packaged GPUI desktop builds on the same Mac under the same disconnected synthetic workload. The treatment is the source/build change under review; the reference is its parent or pre-change build. This is a short, controlled performance comparison, not a claim about production conversations, provider latency, battery life, or all supported hardware.

Use the existing profiling fixture and collector. The fixture creates synthetic messages and tasks, a disconnected dock workspace, and (for selected scenarios) a synthetic incognito browser. It disables sending, voice, and attachments, uses in-memory state, and does not load the user's profile or contact a backend. `scripts/gpui-performance-session.py` copies the exact app to a uniquely identified temporary bundle and launches only that copy. Do not replace this with a normal app launch or a live profile.

## Required new packages and evidence

All earlier batches lack measured-window visibility diagnostics and are excluded from the current performance comparison. This includes earlier compiler/input-guarded batches. Preserve their raw files and historical interpretations; do not mix them with new eligible observations. No speedup is proven by those batches.

Package both reference and treatment again with the identical schema-1 `windowConditions` instrumentation in `performance_scenario.rs` and `performance_metrics.rs`. Record completed build/test receipts, source inventories and exact executable hashes before collection. Use one frozen source package per side across all repetitions, matching release/profile/features and collector configuration. A successful older package cannot substitute for a newly instrumented package. See [visibility-audit.md](visibility-audit.md).

## Predeclared workload

For the initial Fast POC, use `streaming-body` and `pipeline-updates` as the primary quantitative workloads, with `idle` as a secondary baseline. Each run uses 10,000 history rows per pane, motion mode `off`, a 15-second warmup and a 60-second measured window. These scenarios animate themselves: streaming-body updates synthetic response text, while pipeline-updates changes synthetic task publications across four chat panes. They require no native typing or scrolling during measurement. The current replacement comparison requires five eligible paired repetitions per scenario/build. The original plan allowed three pairs before expanding for noise; the replacement selector defaults to five. Keep the window at the fixture's default 1200×820 points, theme dark, and the same physical display, refresh rate, power mode, and foreground state for both builds. Do not change settings between matched runs.

Treat `static` typing and scrolling as a native qualitative correctness check, not a timed scripted workload: CUA/native input timing is inconsistent, so do not combine it with the autonomous measured scenarios. If the change targets another path, predeclare any additional scenario before collecting results. Avoid interpreting scenario names as a native input replay: the session collector never generates keystrokes or scrolls.

Use packaged `release` builds with `performanceProfiling: true` and `visualExport: false`. The collector embeds the `--performance-profiling` build feature in its observation. Do not compare debug builds, visual-export builds, different feature sets, different row/pane counts, or different measurement durations. Keep the same machine, OS version, display arrangement, app window size, motion setting, and attached peripherals. Record these conditions alongside the result.

## Run order and repetition

Collect five eligible paired observations per build and scenario for the replacement comparison. Alternate paired order to reduce time-of-day and thermal drift (for example, reference then treatment in one pair, treatment then reference in the next). Allow the Mac to settle between launches and keep unrelated foreground work closed. Do not interact with autonomous workloads during warmup or measurement. Perform static typing/scrolling separately as qualitative verification.

Reserve approximately **40 minutes of quiet foreground time** for a full five-pair, three-scenario batch: 30 runs × (15-second warmup + 60-second observation) already totals 37.5 minutes, before startup, cleanup and settling pauses. Twenty minutes is insufficient. Excluded runs require additional time. Keep the fixture frontmost and visible; do not switch apps/Spaces, cover or minimize it, lock the Mac, sleep the display, type, scroll, click, or use UI automation during a run. Initial launch activation is performed by the existing fixture; the diagnostic neither forces focus nor restores it. If foreground/visibility is lost, retain the failed run and stop scheduling until conditions are corrected.

Use `scripts/gpui-fast-benchmark.py` for the guarded batch after preparing identical `machine-conditions.json` files under `NEW_BATCH/reference` and `NEW_BATCH/treatment`. The batch root may contain those sidecars, but must have no existing schedule or run folders. Example with placeholders for the newly verified executables:

```sh
python3 scripts/gpui-fast-benchmark.py \
  --reference /path/to/new-reference/Monitter.app/Contents/MacOS/monitter-gpui-desktop \
  --treatment /path/to/new-treatment/Monitter.app/Contents/MacOS/monitter-gpui-desktop \
  --output /path/to/NEW_BATCH --repetitions 5
```

The driver delegates to `gpui-fast-session.py`, which runs the existing isolated collector and matched OS sampler and records their exit codes. The underlying collector checks the source executable against adjacent build metadata, signs a unique temporary app copy, records session/metrics/build identities and closes only its child. Never use a normal app launch or live profile. Preserve every run, receipt, guard and log, including failed attempts. The driver writes a receipt and stops subsequent scheduling when the selector's window/workload diagnostic rules reject a run; this early check does not replace full selection.

Capture machine conditions once for a matched reference/treatment comparison, save the same `machine-conditions.json` file in both comparison roots, and keep unknowns explicit. Schema version 1 is:

```json
{
  "schemaVersion": 1,
  "machine": {
    "system": "Darwin",
    "architecture": "arm64",
    "platformRelease": "...",
    "macVersion": "...",
    "cpuModel": "...",
    "physicalMemoryBytes": null
  },
  "display": {
    "fixtureViewportPoints": {"width": 1200, "height": 820},
    "theme": "dark",
    "scale": null
  },
  "power": {"source": "AC", "mode": null}
}
```

Use `null` for unknown model, memory, display scale, or power mode; `power.source` is `AC`, `battery`, or `unknown`. The comparator validates this schema and requires exact equality between both roots. It does not infer host or hardware details from build metadata.

The current comparison requires complete schema-2 OS samples for every run, not optional or mixed coverage. The session wrapper starts `gpui-fast-os-sample.py` against the exact fixture PID. It samples approximately once per second until process exit, records a UUID/start identity to detect PID reuse, sends no signals, and atomically records a complete final snapshot. The collector owns fixture cleanup. A standalone comparator can accept groups without OS samples; that weaker mode is not this protocol.

## Metrics and interpretation

Use `drawMs` p50/p95/p99 as the main latency summaries for autonomous runs. The zero-input selection rule makes `inputToPlatformSubmitMs` unavailable in eligible runs; do not present it as measured typing latency. For separate qualitative/native-input investigations, input timing starts at GPUI dispatch and ends at platform submission; it excludes OS event-queue delay and physical display scanout. Each sample is associated with a presented frame and represents the oldest pending input event. `drawMs` measures GPUI draw duration, not full display latency. `dirtyToPlatformSubmitMs` measures the interval from a dirty frame to platform submission. `animationFrameIntervalMs` describes intervals between observed presented frames; it is not a process wakeup counter.

Also report `transcriptPreparations`, `transcriptRowsPrepared`, `chatChromeRenders`, `detailBodyConstructions`, `transcriptPreparationReasons`, and `inputEventsPerFrame` as workload/behavior context. These are profiler counters over the measured window, not normalized per keystroke unless the exact number of operator inputs is recorded. A reduction in render/preparation counters is useful only if the paired runs exercised the same interaction sequence and the latency distributions remain acceptable.

The wrapper samples `/bin/ps` CPU time approximately once per second. This is process CPU accumulated since process start, so the first-to-last delta includes startup and warmup; it is a coarse whole-process measure and may miss short bursts. It is not the GPUI draw thread's CPU time and must not be presented as frame CPU. The required libproc sampler records cumulative user/system CPU, resident bytes, physical footprint, wired bytes, page-ins, and two wakeup counters. `ri_user_time` and `ri_system_time` are Mach absolute-time ticks, not nanoseconds. The sampler records `mach_timebase_info` (`numer`/`denom`), and the comparator converts tick deltas to seconds using `ticks * numer / denom / 1e9`; do not divide raw values directly by 1e9. The comparator uses only OS samples timestamped inside the GPUI metrics window, excluding startup/warmup and later samples; boundary samples outside the window are omitted. Its CPU and wakeup deltas cover the first-to-last included OS sample, and peak memory is the highest one-second sample inside that interval, not the exact instantaneous peak. Neither source reports allocations, GPU time, or energy.

Keep `packageIdleWakeups` and `interruptWakeups` distinct. Package idle wakeups count a specific macOS process accounting category; interrupt wakeups count another category. They are not a complete count of every process wakeup and must not be summed or relabeled as total wakeups. Do not infer idle wakeups from frame intervals: a sleeping process can have no presented frames while still receiving timers or other wakeups. Any idle-power conclusion needs matched idle duration, the exact process identity, and a separate OS-level energy source such as Instruments/powermetrics.

## Analysis and decision rule

Run `gpui-fast-select.py --source-root NEW_BATCH --output NEW_SELECTION --pairs 5` first; repeat `--source-root` for fresh continuation batches using distinct numeric repetition IDs. Compare only the created selection roots. Use `scripts/gpui-fast-compare.py NEW_SELECTION/reference NEW_SELECTION/treatment --json` with an identical `machine-conditions.json` at each root. It validates that every included run is a matching profiling release observation and refuses debug, visual-export, malformed, settings-mismatched, or machine-mismatched data. Its `--json` output reports each metric's per-run values, median, min/max range, and treatment-minus-reference change and percent change where defined. A zero-count profiler histogram is reported unavailable, never as zero latency. Keep scenario/settings groups separate. Pass the saved comparison JSON and `NEW_SELECTION/selection-manifest.json` to `scripts/gpui-fast-evaluate.py`. It maps exact source paths and repetition IDs rather than array order, reports missing values as unavailable, and separates the median noise gate from direction consistency and range overlap. The comparator alone does not enforce visibility eligibility or the selection manifest.

Treat the comparison as descriptive. With fewer than three valid runs on either side, the report explicitly marks the result as insufficient for a repeatability claim. With three or more runs, do not call a change meaningful solely because the medians differ: inspect run ranges and distributions, check for consistent direction across paired repetitions, and repeat if ranges overlap materially or drift is visible. As a practical noise gate, require complete values and an absolute median shift strictly greater than both 5% and the larger within-build median absolute deviation. A zero reference median cannot pass the percentage gate. The evaluator permits this gate with at least three complete pairs, but the current replacement protocol still requires five eligible pairs per scenario. Gate passage alone is insufficient: inspect direction consistency and overlap before describing any likely signal; otherwise report it as inconclusive. This threshold is an operational screening rule, not a statistical significance test. Always retain raw samples and report the number of runs.

This POC does not establish a release-wide performance guarantee. The synthetic fixture does not reproduce real transcripts, extensions, active providers, or a user's background workload. Profiling instrumentation is enabled in both builds and may affect absolute timings; comparisons are valid only when both builds use the same collector and profiler configuration.

## Exact replacement selection rules

`gpui-fast-select.py` chooses the earliest five eligible numeric paired repetitions per scenario. It does not select using draw quantile, CPU, footprint, wakeup or chrome-count magnitudes. Positive draw/progress counts are workload-validity checks, with no invented minimum performance threshold.

Each side requires successful collector/sampler receipts, matching session/metrics/OS/receipt PIDs, complete schema-2 OS sampling, valid measured timestamps and at least 60 measured seconds. Source and isolated-fixture hashes must agree with their respective metadata; release/profiling flags and relevant build metadata must agree. Session warmup/requested duration must match metrics. Settings and build features must match across a pair; eligible source builds must be identical within each side across repetitions. Machine-condition sidecars must match exactly across the selected comparison.

Compiler observations must be empty, cover session start through measured-window end, and have no gap greater than 2.5 seconds. The input-latency histogram must contain exactly zero samples. This conservative input filter does not prove absence of all OS input or other host load.

Every run additionally requires:

- `windowConditions.schemaVersion` exactly integer `1`, matching both sides;
- `fullyActiveVisible: true`;
- positive finite diagnostic `durationSeconds`, within 0.25 seconds of `metrics.measuredSeconds`;
- finite nonnegative `activeSeconds` and `visibleSeconds`, each within 0.25 seconds of diagnostic duration;
- a nonempty transition timeline starting at elapsed zero, with finite ordered timestamps bounded by diagnostic duration and every state explicitly active and visible;
- a nonnegative integer `workloadUpdateCallbacks`, strictly positive for streaming-body and pipeline-updates;
- a strictly positive integer `drawMs.count` for every scenario, including idle.

Streaming counts delivered fixture update callbacks, not proof that a body existed or changed on screen; pipeline counts committed publications. Platform visibility detects full occlusion, not exact partial-cover geometry. These diagnostics strengthen equivalence evidence without proving content correctness, physical scanout, energy, or real-provider behavior.

Missing artifacts, interrupted sessions and disqualified pairs remain in the manifest with reasons. Insufficient selections produce no comparison roots. All earlier batches fail the diagnostic requirement and must remain excluded. Additional attempts replace disqualified pairs without hiding them. New packages or instrumentation changes require a fresh matched build set and comparison, rather than mixing source identities.

Compiler-gap, zero-input and visibility/progress filters were introduced after discovering contamination and missing observability. They are post-discovery validity filters, not the original performance threshold. Unknown display scale, power mode, thermal state and unrelated host load remain limitations. The outcome remains inconclusive/no proven speedup until eligible matched measurements are collected and reviewed; no statistical significance, causal or adoption conclusion follows automatically from selection or a noise-gate pass.
