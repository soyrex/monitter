# GPUI Fast POC results — provisional

**No adoption recommendation. The POC is not complete.** Earlier benchmark batches lack measured-window activation and visibility evidence and cannot establish a Fast performance benefit. This applies to the first round and subsequent compiler/input-guarded batches. Some successful autonomous observations have zero draw samples; compiler-free execution, zero input samples and successful receipts do not establish equivalent rendered work. Previously reported improvements, regressions and noise-gate outcomes are historical observations, not a valid adoption basis.

The first-round report, including its superseded tables and interpretations, is preserved at `artifacts/gpui-fast-poc/historical-results-first-round.md`. Raw observations and comparisons remain preserved. See [visibility-audit.md](visibility-audit.md) for the concrete source evidence and the limits of retrospective inference, and [benchmark-protocol.md](benchmark-protocol.md) for current collection requirements.

## Historical build and correctness evidence

Both worktrees originate at `1daa6e564174949c81346609dbf23fea4798cb93`. The candidate vendors Fast v0.1.9, upstream commit `37f3959732d13404989e58e89c4e37ec9489e3f5`. Migration source is recorded in commit `0dc6d6b`; that commit does not itself establish diagnostic-build or runtime verification.

| Historical identity | Reference | Candidate |
| --- | --- | --- |
| Release package | `build-1792490452835` | `build-1792493535394` |
| Executable SHA-256 | `c163ac8cd575a9f5c2820285c4aca9a7490df1dcf1e993e5a7c4995863e453d5` | `c506f602faff2c2d8ca24612de3bee54bd9a587919c64279d02c0e09e514bc59` |
| Profiling / visual export | Enabled / disabled | Enabled / disabled |
| Focused native cohort | 33 passed, exit 0 | 33 passed, exit 0 |

These identities describe the earlier packages, which lack the new window diagnostic. Both package metadata files mark source dirty. The candidate's saved inventory contains 1,543 source-input hashes, unchanged in the historical post-release check. These facts are retained correctness/provenance evidence, not performance equivalence evidence.

The focused 33/33 cohort is not the full native suite. Candidate-only assertions were adapted for valid clean-descendant reuse while preserving checks of painted text, color, bounds and clicks. A substantive retained-pane visibility regression required a frame/range-scoped paint observation fix. Collapse/remount/unmount replay checks pass in that historical cohort; the actual `NativePanel::on_removed` lifecycle hook was not invoked. Native preview observations and remaining browser, accessibility, drag and GPU limits are recorded in [verification-matrix.md](verification-matrix.md).

## Diagnostic verification pending

Identical fixture-only activation/visibility observers and measurement-boundary hooks are implemented in both worktrees. New `windowConditions` reports initial state, transitions, active/visible durations and workload callback/publication counts. Selection now rejects old or missing diagnostics, inactive/hidden intervals, incomplete diagnostic duration, zero draw samples, and autonomous runs without recorded update callbacks. This establishes a stronger eligibility rule; it does not establish a performance result.

Both diagnostic releases completed successfully: baseline run `47c415d5` (442.29 seconds), package `build-1792503210909`, SHA-256 `b4c479803a9583e4587431c8d1a5f19e29becf43e0f603ef5ae75b975758b38a`; candidate run `d5898af1` (394.54 seconds), package `build-1792503672608`, SHA-256 `6ae49d481244ae5cd422534777c60b1098fe0d38a993219d07062c7aa339594c`. Strict/deep signature verification passed for both. Release metadata confirms profiling enabled and visual export disabled. All 1,543 candidate source inputs match the saved pre-build inventory.

Focused diagnostic native tests passed 2/2 with exit 0 (run `fb96f7dd`, 162.42 seconds). The Python driver, selector and evaluator passed 12 focused tests. A separate 45-second candidate smoke recorded actual minimize/restore transitions and retained `fullyActiveVisible: false`; the window was inactive throughout, so this run is correctly ineligible. It overlapped compilation and included deliberate UI interaction: it proves diagnostic behavior, not performance. New eligible matched measurements remain required.

## NSH architectural requirement

The user identified Native Shell compatibility as an additional decision criterion. NSH's [GPUI backend documentation](https://nsh.dev/docs/gpui/) supports embedding `AppView` in an existing GPUI element tree. Its [GPUI workspace manifest](https://github.com/longbridge/nsh/blob/main/crates/gpui/Cargo.toml), inspected 2026-10-20, requests published `gpui-fast` and `gpui-fast-platform` 0.1.9. That aligns with the upstream Fast release used here and is a concrete architectural reason to evaluate adoption independently of benchmark gains.

Compatibility is not yet demonstrated: this POC exposes vendored package `gpui` 0.2.2 through `gpui-pre` compatibility wrappers, while NSH requests published `gpui-fast` packages. Embedding must resolve both consumers to one Rust GPUI type identity and preserve Monitter's patches. NSH also requests `lb-wry` 0.53.3, whereas this POC retains Wry 0.55.1. Neither exact dependency resolution nor a mounted NSH response has been verified. Treat this as dependency alignment, not a drop-in integration claim. The existing NSH/OpenUI ideas retain the integration scope; no NSH implementation is included in this performance treatment.


## Mechanism and maintenance burden

The migration changes retained-subtree replay and rendering beneath the application. That can reduce repeated work without reducing the number of application chrome renders: render invocation, invalidation, retained paint reuse and platform presentation are separate boundaries. The source investigation in [render-invalidation-analysis.md](render-invalidation-analysis.md) records the invalidation mechanisms; it is an explanation to test, not evidence of measured benefit. Low chrome/draw counts in an occluded window cannot distinguish a Fast improvement from suppressed presentation.

Fast is not a manifest-only upgrade. Local compatibility wrappers keep Kit/Base/Component 0.7.0 and Wry 0.55.1 on one actual core, while a vendored fork reconciles custom masks/fades, transforms, blur/backdrop rendering, inert input, native drag, capture, accessibility, profiling and replay-aware visibility. [patch-ledger.md](patch-ledger.md) records that continuing maintenance burden. Adoption requires repeatable useful benefit plus adequate correctness coverage to justify it.

Ideas `15c5a987` and `20b17186` are parked follow-ups. They are not current optimizations or part of the measured migration treatment.

The canonical checkout, installed applications, production profiles and live backend remain protected. No production merge, installation or backend restart is established or requested by this report. Preserve the isolated migration and baseline while the diagnostic build, focused verification and eligible matched measurements remain unfinished.
