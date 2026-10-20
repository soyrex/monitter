# Render invalidation investigation

This is a read-only source investigation of the frozen POC packages. No Rust
changes, rebuilds or UI interactions were performed during the fresh five-pair
benchmark. The earlier three-pair counters motivate the investigation; they do
not establish causality, and the fresh results must be evaluated separately.

## Pipeline: confirmed global invalidation mechanism

The explicit content cache is still present at
`desktop-gpui/src/docking.rs:468`: `self.spec.content.clone().cached(...)`.
The migration did not simply remove that cache.

The following source path explains a concrete invalidation fan-out:

1. `desktop-gpui/src/performance_scenario.rs:790` schedules pipeline publication
   every 100 ms. `commit_publication` calls `activity_effects::sync` at line 390,
   including publications that change message bodies or `updatedAt` without
   changing the activity presentation.
2. `desktop-gpui/src/activity_effects.rs:69` unconditionally obtains
   `global_mut::<Effects>()` to reset the snapshot-source weak pointer. This
   happens before the unchanged-presentation early return at line 79. Obtaining
   the mutable global therefore occurs even when the function returns `false`.
3. `desktop-gpui/vendor/gpui-fast/crates/gpui/src/app.rs:2424` makes
   `global_mut` push `NotifyGlobalObservers`; `push_effect` at line 1980 stamps
   the global dependency change. This is not an equality check on the field.
4. Cached views include global changes in their invalidation predicate at
   `desktop-gpui/vendor/gpui-fast/crates/gpui/src/fast/dependencies.rs:545`.
   The retained dirty-view walk uses that predicate at
   `desktop-gpui/vendor/gpui-fast/crates/gpui/src/fast/retained.rs:1121`.
5. Chat chrome reads the same global through `is_naming` and
   `for_task_in_window` at `desktop-gpui/src/chat.rs:6648` and line 6839.
   NativePanel decorations also read it at `desktop-gpui/src/docking.rs:455`.

Consequently, a logically unchanged activity sync can invalidate every cached
chat reader under Fast. Four panes at approximately ten publications per second
give about 2,400 chrome renders per minute, consistent with the earlier
candidate range of 2,328–2,428. This numerical agreement supports the mechanism;
its share of total CPU/draw time has not been isolated experimentally.

The baseline cache predicate at
`desktop-gpui/vendor/gpui-pre/src/view.rs:485` validates explicit dirtiness and
render context rather than tracking arbitrary global reads. Fast's broader
dependency tracking is intentional correctness machinery; suppressing global
invalidation generally would be unsafe.

## Idle: cursor hypothesis, not a confirmed regression cause

The fixture explicitly focuses the first composer at
`desktop-gpui/src/performance_scenario.rs:718`. Its cursor model toggles and
notifies every 500 ms in
`desktop-gpui/vendor/gpui-base/src/input/base/blink_cursor.rs:59`; the input
reads that model for caret visibility in
`desktop-gpui/vendor/gpui-base/src/input/base/state.rs:3416`. Approximately 120
ticks per minute is consistent with the earlier candidate idle count of 115
chrome renders, but counts alone do not prove which invalidation caused them.

There is also a baseline correctness question. The optimized `App::notify`
path in `desktop-gpui/vendor/gpui-pre/src/app.rs:2905` invalidates windows
directly and bypasses observers when a model has live window invalidators.
The cursor is a model rather than a rendered view. Its ID may have no dispatch
tree ancestor path, while baseline cached replay only checks explicit view
dirtiness and context. Fast additionally detects changed descendant model
dependencies when evaluating cached replay at
`desktop-gpui/vendor/gpui-fast/crates/gpui/src/fast/retained.rs:590` and rebuilds
the cached view when reuse fails at line 1686. Fast may therefore be preserving
correct caret updates while rebuilding more chrome. Actual caret paint and
focus cadence have not been compared; do not interpret baseline's lower count
as proof of equivalent visible behavior.

## Bounded next steps

- After measurement, compare caret visibility and focus in both frozen
  packages over several blink periods. A focused native regression should then
  exercise a notified non-view model inside a cached parent and check visible
  output as well as parent/child render counts.
- For pipeline, make unchanged activity synchronization avoid mutable global
  access. Snapshot-source identity is bookkeeping and can be separated from
  render-observable Effects state. `sync_cached` also writes this bookkeeping
  on its unchanged path (`activity_effects.rs:117`); handle both entry points.
  Preserve real title, running-status, motion, palette, naming and visibility
  invalidation. Verify no-op and real-change behavior before remeasuring.
- Do not disable retention, weaken paint/hitbox assertions, or force render
  count parity. A pipeline-only fix would not resolve the idle hypothesis.

These findings support keeping the POC isolated until fresh measurements and
remaining correctness checks justify a decision. They do not require an
unbounded performance rewrite to conclude this POC with a no-adoption result.
