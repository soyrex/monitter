# Fixture visibility audit

Successful collection, zero input samples, and compiler-free execution do not establish equivalent foreground rendering. The previous metrics contained no activation/occlusion timeline. Some successful sixty-second streaming/pipeline observations have zero draw samples; this does not prove either a performance improvement or a specific cause.

Both platform implementations stop the display link when AppKit no longer reports `NSWindowOcclusionStateVisible`: reference `desktop-gpui/vendor/gpui-pre-macos/src/window.rs`, `window_did_change_occlusion_state`; treatment `desktop-gpui/vendor/gpui-fast/crates/gpui_macos/src/window.rs`, the same function. This covers full occlusion, minimization, hiding, Space switches and display sleep. Backgrounding and occlusion are distinct. A partially covered window may remain visible.

The synthetic streaming timer runs approximately every 200ms and the pipeline timer every 100ms, subject to scheduling and publication backpressure. Their loops do not require an active or visible window. The profiler waits by wall time and can finish successfully while no frames are presented. Existing native logs only report saving the observation and cannot retrospectively recover visibility.

## Identical diagnostic in both builds

`performance_scenario.rs` installs retained activation and visibility observers using the existing public GPUI API. `performance_metrics.rs` begins and ends an observation at its profiler snapshot boundaries. The new `windowConditions` object records:

- initial active/visible state and timestamped transitions relative to observation start;
- total diagnostic duration, active seconds, and visible seconds;
- sticky `fullyActiveVisible`, false if either state was false initially or at any observed transition;
- measured-window `workloadUpdateCallbacks`: streaming fixture callbacks delivered, or pipeline publications committed. Idle should have zero. A delivered streaming callback does not prove a streaming body existed or changed on screen.

No timer, frame request, focus activation, or workload scheduling was added by this diagnostic. Observation state exists only in the profiling fixture module and is populated only during the explicit measurement. Pipeline/streaming counter increments use fixture-only paths. Observation boundaries surround the existing profiler window; small snapshot/counter bookkeeping differences remain within the same UI-thread turn.

Require `fullyActiveVisible: true` and demonstrated scenario-appropriate workload progress before comparing new autonomous runs. Do not invent a minimum update threshold from observed performance results: timers, pipeline backpressure, and rendering have different expected counts. Record and inspect the counts and draw samples. These diagnostics do not establish physical display scanout, exact partial-cover geometry, or content correctness. They cannot repair old observations lacking the fields.

Two pure accumulator tests cover hidden intervals retained after reactivation, separate active/visible duration, and a fully active visible interval. They require the parent's coordinated Rust test/build run; no compilation was performed while adding them.
