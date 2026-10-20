# GPUI Fast POC patch ledger

The candidate vendors `longbridge/gpui-fast` tag `v0.1.9`, commit
`37f3959732d13404989e58e89c4e37ec9489e3f5`. Its source is under
`desktop-gpui/vendor/gpui-fast`. The original patched pre0.3.7 sources remain
untouched as baseline and rollback evidence, outside the active dependency graph.

## Dependency strategy

- GPUI Kit, patched GPUI Base, patched GPUI Component and patched GPUI Wry stay
  at 0.7.0. Wry stays exactly 0.55.1. Application imports stay unchanged.
- Fast's compatibility packages normally identify as pre0.3.9. Their local
  package versions are 0.3.7 so Kit's exact requirements select them. Each
  wrapper only reexports the corresponding package in the one Fast workspace.
- Additional pre0.3.7 macOS and Apple wrappers preserve the application's direct
  frame-capture feature dependencies. Platform selection goes through Fast's
  platform package, so it cannot instantiate the original pre core.
- The optimized dev-profile package changes from the wrapper `gpui-pre` to the
  actual core `gpui`. Release optimization settings are unchanged.
- Fast Web pins Unicode Properties exactly 0.1.3. The original lock held 0.1.4;
  `cargo update -p unicode-properties --precise 0.1.3` resolved that incompatibility.
  Other lock changes replace pre implementation packages with Fast workspace
  packages and Fast's pinned upstream git dependencies. They are source fetches,
  without installation, compilation or paid resources.
- `cargo metadata --locked` resolved the complete graph. It contains one actual
  core, `gpui` 0.2.2 at `vendor/gpui-fast/crates/gpui`, one Wry 0.55.1, and local
  pre0.3.7 wrappers for core, platform, macOS, Apple, macros, sum-tree and Web.
  `gpui-pre-reqwest` 0.12.15 remains only as the existing WASM assets dependency;
  it is an HTTP library and introduces no second core.

## Patch disposition

Patches were compared against cached registry pre0.3.7 and three-way merged into
pinned Fast source. Fast rendering, layout, dependencies, composition, retained
subtree and scroll-layer hooks are preserved. Downstream public APIs are explicit
Monitter compatibility exceptions to Fast's upstream public API policy. This
vendored downstream tree is not an unmodified upstream release.

| Baseline capability | Candidate disposition | Files and integration |
| --- | --- | --- |
| Typed cross-window drag completion, internal payload and native drag state | Ported | Core `app.rs`, `app/context.rs`, `interactive.rs`, `platform.rs`, `window.rs`; macOS `window.rs`. Fast's platform/frame-signal additions remain. |
| Global window bounds and screen coordinate conversion | Ported | Core platform/window and macOS window APIs. |
| Frame capture without enabling all test support | Ported | Core, Apple and macOS `frame-capture` features; wrapper forwarding; existing render-to-image hooks. |
| macOS system material behavior | Ported | AppKit retains ownership of the material layer tree; the old private-layer mutation callback remains removed. |
| Vertical/horizontal edge fades and rounded content masks | Ported with explicit layer fallback | Core content masks and scroll-edge-fade element; Metal shaders and path vertices. Fast content-mask literals initialize extension fields, retained context compares the full mask, and primitive translation moves extended anchors. The fade wrapper installs conservative bounded ramps in prepaint, before child overflow geometry and layer promotion are decided; paint computes actual edge visibility from the updated handle. Containers inheriting a nonrectangular/effect mask bypass scroll layers and draw the direct scene. Retained views and layout remain enabled. |
| Backdrop blur, blend modes and ordered scene barriers | Ported | Core scene/color/style, Metal pipeline and shaders. Fast normal, grouped-path and rounded-composition encoders share ordered backdrop handling in `gpui_apple/src/fast/monitter.rs`. Rounded clip bindings are restored after encoder restarts. Backdrop quads stay out of ordinary cached scroll tiles. |
| Renderer data layout for other platforms | Ported, untested | Existing WGPU/Windows shader and renderer modifications were three-way imported onto the Fast implementations. This POC validates macOS only. |
| Hidden/inert content input behavior | Ported and added to Fast context | Core listener/focus/hitbox paths and deferred draws preserve the baseline guards. Inertness participates in retained-context matching; deferred splice copies preserve the flag. Inert containers bypass scroll layers. |
| Baseline retained-cache generation repair | Superseded | The old pre `ViewElementState`/frame-generation cache implementation is replaced by Fast's retained subtree machinery. Restoring deleted old cache bodies alongside Fast would produce two inconsistent replay systems. The inactive compatibility counter in Window remains harmless baseline instrumentation. |
| Baseline accessibility subtree replay | Superseded | Fast's AX operation log, listener replay and last-tree handling remain authoritative. The old pre node/action-history replay and indices are excluded rather than mixed with Fast replay indices. |
| Image handling, SVG behavior and style helpers | Ported | Baseline modified core image/SVG/style modules merged with Fast hooks. |
| Patched GPUI Base/Component/Wry | Preserved | Existing vendored sources and manifests remain the dependencies, apart from resolution of their pre dependencies through the compatibility aliases. |

## Runtime flags and measurement boundaries

Fast reads these flags when creating each window:

- Default/unset `GPUI_VIEW_RETENTION`: retention enabled. Only the exact string
  `0` disables it. `GPUI_VIEW_RETENTION=0` also prevents scroll layers from being
  active, giving a diagnostic control within the same candidate binary.
- Default/unset `GPUI_SCROLL_LAYERS`: enabled on supported platforms. Only `0`
  disables them; `GPUI_SCROLL_LAYERS=0` leaves view/layout retention available.
- Extended viewport mask and inert-content fallbacks are per-container
  compatibility guards. They do not turn off retention for the whole window.

The historical performance collector used unchanged histogram boundaries. New
identical fixture-only window-condition diagnostics are now implemented in both
worktrees, with activation/visibility transitions, durations and workload counters
bracketing the measurement. These additions await coordinated build/test receipts;
they do not force focus, request frames or change workload scheduling. See
[visibility-audit.md](visibility-audit.md). Source comparison
of baseline and Fast `WindowProfiler` found unchanged equations for the four
exported histograms:

| Export | Boundary |
| --- | --- |
| `inputToPlatformSubmitMs` | Oldest pending valid dispatched input to completion of platform submission. |
| `drawMs` | Window begin-draw to end-draw. Cached/retained work executes inside the same draw boundary. |
| `dirtyToPlatformSubmitMs` | First valid dirty timestamp of the submitted drawn frame to platform submission completion. |
| `animationFrameIntervalMs` | Submission-to-submission interval when the preceding submission scheduled animation and the window remains active. |

Fast adds per-refresh and view-phase statistics around these measurements. It
retains the same aggregate histogram equations and validity/power-interruption
filters used by the existing collector. `Window::present` still brackets platform
submission, now via Fast's composition helper. This is a source audit; runtime
comparability and performance still require the coordinated build and workload.
Physical display scanout and OS event queue time remain outside the measurements.

## Verification state — provisional

Migration source is recorded in `0dc6d6b`. This is an isolated POC, not a completed adoption case. The earlier reference/candidate profiling releases and focused native cohorts succeeded (33/33 each). Historical package identities and coverage limits are preserved in [verification-matrix.md](verification-matrix.md) and [results.md](results.md). The historical candidate executable SHA-256 is `c506f602faff2c2d8ca24612de3bee54bd9a587919c64279d02c0e09e514bc59`; its 1,543-input inventory was unchanged in the historical post-release check.

All earlier benchmark batches lack measured-window activation/visibility evidence and cannot establish Fast performance benefit. Zero input samples and compiler-free guards do not prove equivalent rendered work. Their old tables and interpretation are archived under `artifacts/gpui-fast-poc/historical-results-first-round.md`; raw evidence is retained.

Identical fixture diagnostics are implemented in both worktrees. At this update baseline build request `47c415d5` is running, candidate request `48ec6032` is ready for execution, and focused test request `83b99ccf` is planned. New diagnostic source is not verified by the older build receipts. Eligible matched measurements and their interpretation remain unfinished. No adoption recommendation is made.

Historical candidate preview observations include composer typing/selection, retained draft while scrolling, return to bottom, menu Escape, zoom/reflow, visible fade and browser-adjacent paint/alignment. Live accessibility traversal, remote browser input/navigation, actual OS drag/drop, actual removal-hook lifecycle, exhaustive GPU pixel equivalence and the added core fade regression remain unverified. Fast's upstream-policy script cannot compare absent import commit `89ed5c77614b643c5ed1601900ce22b3c2c5cb8e`; that is not a passing check.

The source invalidation/replay investigation is in [render-invalidation-analysis.md](render-invalidation-analysis.md). It explains possible work boundaries without proving performance benefit. Ideas `15c5a987` and `20b17186` remain parked follow-ups and are not current optimizations. No production merge, installed-app replacement or backend restart occurred as part of the recorded migration.

The following entries preserve the historical migration failures and fixes. Their then-pending checks were followed by the historical receipts above; new diagnostics still require separate verification.

### Candidate check 01 follow-up

The parent-owned first candidate check reached Fast core and reported five type
errors. The downstream hover observer composition now uses Fast's boxed rare
listener storage, and accessibility dispatch calls Fast's owned `FnMut` listener
directly. All Fast polychrome sprite constructors, including renderer fixtures,
now initialize the retained image transformation and transformed bounds fields.
The analogous layer relocation path translates both bounds and conjugates the
image transform; tile hashing includes the transform and transformed bounds so
cached images invalidate when their transform changes. The existing primitive
translation regression now covers a rotated polychrome sprite.

These edits were parsed with Rustfmt only; the parent coordinates the next
compiler check. No check or test was run by the migration agent.

### Candidate check 02 follow-up

The second parent-owned candidate check passed Fast core and Apple/macOS type
checking, then reached three `gpui-base` calls using the previous two-argument
`LineWrapper::wrap_line` API. They now explicitly pass
`IndentAdjustment::SameIndent`, matching the prior wrapper's continuation-indent
policy. The editor's higher-level `WrappingIndent` handling remains unchanged.
The call-site audit found no other callers in the desktop, component or Wry
sources. These two base files are the only downstream Kit source changes added
for this API adjustment. Rustfmt parsed them; compilation remains parent-owned.

### Candidate native check 01 follow-up

Production binary checking passed; the first visual-export native test compile
then exposed 44 instances of a single macro-name ambiguity. The macOS test-only
root alias previously named the entire Kit facade as `gpui`, while Kit's glob
imports also expose its hidden `gpui` core-crate reexport. Fast's Action derive
resolves `gpui` lexically, so those different aliases conflict. The macOS test
alias now names the already-direct `gpui` dependency itself, matching Kit's
reexport identity. No macro implementation, test behavior, application API or
dependency graph changed. The existing nonmacOS alias remains behind its own cfg
because the direct core dependency is currently macOS-specific. Source parsed;
parent owns the retry and no compile was run by the migration agent.

### Candidate native check 02 follow-up

The parent-owned focused native run passed 30 of 33 baseline checks. One failure
was an observable visibility regression: Fast retains the noncached NativePanel
entity itself, so a retained frame skipped the canvas that repopulates
`PaintState.painted`. DockWorkspace's next-frame lifecycle decision could then
hide an onscreen native pane. The canvas now registers its positive-size paint
observation with `Window::observe_paint`. This narrow bookkeeping callback runs
on initial paint and when that exact paint range is reused (including splices
and scroll-layer record carries). Its frame-owned callback vector is indexed,
rebased and cleared alongside the other paint records. Removed or hidden ranges
do not replay it; observers cannot draw or mutate window state.

The existing pane visibility fixture additionally checks collapse, remount,
removal and later parent replay, clearing the painted set before each frame.
It keeps the original content-notification and content-reuse assertions.

The other two failures required a clean child Render call solely to rebuild old
frame indices after parent replay. Fast explicitly copies/rebases nested records
in `reuse_retained_prepaint` and finalizes nested paint indices in `finish_frame`;
those records are not stale and an extra render is unnecessary. Candidate tests
now assert clean-child reuse while checking actual text-decoration paint and
click hitboxes after both prefix insertion and removal. The nested-cache fixture
also checks painted bounds and inherited text color, retaining its original
child-notification, bounds/style invalidation and explicit-refresh assertions.
Test names remain stable for baseline/candidate result mapping. These are
candidate-only assertion adaptations to Fast's stronger retention behavior, not
a baseline modification. New behavior remains unverified until the parent runs
the coordinated retry; only Rustfmt parsing was done by the migration agent.
