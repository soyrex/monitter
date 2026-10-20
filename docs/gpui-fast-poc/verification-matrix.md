# GPUI Fast POC verification matrix

Both worktrees start from snapshot `1daa6e564174949c81346609dbf23fea4798cb93`.
The candidate substitutes Fast v0.1.9 (`37f3959732d13404989e58e89c4e37ec9489e3f5`)
and carries the extensions recorded in [patch-ledger.md](patch-ledger.md).
This is an isolated POC; no production merge, installation or backend restart
is authorized or performed as part of it.

## Current status

**Provisional; POC incomplete; no adoption recommendation.** All earlier benchmark batches lack measured-window visibility/activation evidence and cannot establish performance benefit. Migration source is recorded in `0dc6d6b`. Identical diagnostics are implemented in both worktrees but require new build/test receipts. At this update baseline request `47c415d5` is running, candidate request `48ec6032` is ready for execution, and focused test request `83b99ccf` is planned. The historical 33/33 passes below do not verify the new diagnostics. See [visibility-audit.md](visibility-audit.md).

## Historical build and source identity

| Evidence | Reference | Candidate |
| --- | --- | --- |
| Release package | `build-1792490452835` | `build-1792493535394` |
| Executable SHA-256 | `c163ac8cd575a9f5c2820285c4aca9a7490df1dcf1e993e5a7c4995863e453d5` | `c506f602faff2c2d8ca24612de3bee54bd9a587919c64279d02c0e09e514bc59` |
| Release receipt | Exit 0, profiling enabled, visual export disabled | Exit 0, profiling enabled, visual export disabled |
| Focused native cohort | 33 passed, 0 failed | 33 passed, 0 failed (`candidate-native-03`) |
| Source state | Snapshot tracked sources; local toolchain symlink makes package dirty marker true | Dirty POC by design; 1,543 input hashes preserved in `candidate-source-before-release.json`, unchanged after release |

Build logs, command receipts and exact selected test names are in
`artifacts/gpui-fast-poc/builds/`. Both builds used the coordinated shared Cargo
target and the existing toolchain/mobile assets. Mobile asset freshness was not
tested. Independent packages prevent later shared-target output replacement from
changing the measured executables. Each benchmark session validates its source
hash against its adjacent build metadata.

## Correctness coverage

The 33-test cohort covers selected composer Enter/IME, shortcuts, transcript
scroll/reflow, pane cache/mode/movement, modal cache replay, tab drag and resize
release cases. It is not the entire native suite. Candidate replay fixtures were
adapted to Fast's valid clean-descendant reuse while adding painted text,
bounds/color, hitbox and hidden/unmounted observation checks; the baseline
fixtures were not edited. See the ledger for the distinction between those two
count assumptions and the substantive visibility bug fixed during the POC.

| Area | Observed evidence | Remaining limit |
| --- | --- | --- |
| Composer and transcript | Focused native cohort passes in both builds. Candidate preview: native typing and selection, draft retained while scrolling history, return to bottom. | Disconnected synthetic history; no live provider turn or real profile exercised. |
| Stream invalidation | Selected transcript/scroll and reflow checks pass. Autonomous streaming-body and pipeline-updates form the benchmark workloads. | Timed runs do not inject user keystrokes or scrolling. |
| Dock and layout | Pane movement/mode/cache/resize-release cohort passes. Candidate window zoom/reflow observed without losing content. | Sidebar toggle had no observable effect in the disconnected fixture; sidebar resize/equalise behavior not established. |
| Modal and event replay | Candidate fixture checks painted text and actual clicks through prefix growth/shrink; notification/style/bounds/refresh checks pass. Preview menu dismisses with Escape. | Broader modal/accessibility interaction combinations not exhaustively tested. |
| Pane visibility | Replay-aware observation fix passes the existing content-cache test, plus collapse/remount/unmount and later replay checks. Independent source review found the observer frame/range scoped. | Unmount omits the panel from the fixture tree; actual `NativePanel::on_removed` lifecycle hook was not invoked. Collapse/zoom/displacement lifecycle combinations are not fully exercised live. |
| Drawing and fades | Scroll-chrome checks pass; edge fade visible in candidate preview. Fade mask is present during prepaint to block incompatible tile promotion. | New Fast core fade regression is source-only: not compiled or run. No exhaustive pixel-equivalence proof across masks, blur, shadows and GPU backends. |
| Native browser | Candidate browser-adjacent paint and alignment retained after zoom/reflow. | Remote browser keyboard input, navigation and full WKWebView interaction remain unverified. |
| macOS input and drag | Selected native tab/pane drag and resize-release fixtures pass. | Actual OS drag/drop and cross-window native drag completion remain unverified. |
| Accessibility | Source ports retain Fast's accessibility recording/replay and owned listener dispatch. | No live accessibility-tree traversal or assistive-technology test. |
| Other platforms | Platform wrappers and required shader/structure compatibility carried in source. | No Linux, Windows, iOS, Android or Web runtime/build claim. |

Native preview observations above were made by the coordinating parent agent
against the isolated candidate fixture, separately from timed autonomous runs.
No production profile, installed app, live backend session or mobile/shared
client was changed.

## Performance and decision

All earlier benchmark batches, including compiler/input-guarded repetitions, lack the new measured-window activation/visibility diagnostics. Successful collectors and OS samplers do not prove an active, visible workload. Some autonomous runs have no drawn frames. No earlier timing table, noise gate, apparent benefit or apparent regression establishes an adoption case.

The selector now requires schema-1 `windowConditions`, continuous active/visible state, complete measured duration, positive draw samples and positive autonomous workload callback counts. It does not invent a minimum performance-dependent sample count. Source implementation and synthetic selector checks are separate from native diagnostic verification and actual eligible observations. The new diagnostic Rust tests and release packages remain pending the coordinated receipts described above.

Follow [benchmark-protocol.md](benchmark-protocol.md) for current conditions, exclusions and repetition requirements. The earlier first-round report is archived at `artifacts/gpui-fast-poc/historical-results-first-round.md`; raw evidence is retained. Current conclusions belong in [results.md](results.md). Ideas `15c5a987` and `20b17186` remain parked follow-ups, not current optimizations. Do not call this POC complete or adopt the migration until the remaining evidence is collected and reviewed.
