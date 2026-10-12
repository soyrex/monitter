# GPUI motion validation

Worktree: `/Users/alex/code/monitter-gpui-motion-plan`; branch `plan/gpui-motion-20261012`.

Implementation and isolated validation completed on 13 October 2026. This file records the checks for [the animation plan](GPUI_MOTION_PLAN.md), including hover/focus background emphasis, together with the remaining coverage limits. The installed app and live backend were not validation targets for this isolated branch.

## Build and regression checks

The shared native toolchain and build cache are used without changing the primary checkout:

```sh
CARGO_TARGET_DIR=/Users/alex/code/monitter-gpui/.cargo-target/gpui \
  node scripts/gpui-toolchain.mjs cargo test \
  --manifest-path desktop-gpui/Cargo.toml --locked --jobs 2 \
  --lib --bin monitter-gpui-desktop \
  --features visual-export,performance-profiling -- --test-threads=1

CARGO_TARGET_DIR=/Users/alex/code/monitter-gpui/.cargo-target/gpui \
  npm run build:gpui -- --release --performance-profiling --jobs 4
```

Regression coverage includes interrupted/reversed geometry, minimum pane sizes, stable dock identities and focus, drag takeover, hidden/reduced-motion settlement, stale callback fencing, copy expiry, initial/history completion suppression, and reuse of prepared transcript content during decorative frames.

- Native library: 54 tests passed, including task-status projection reuse for body-only publications.
- Native app: 434 tests passed in the final full run; one existing preparation-key assertion still expected width changes to invalidate content. After updating it for the new semantic boundary (and retaining compact-mode invalidation), its focused rerun passed: 435 app tests in total. The suite covers the new real 10,000-row reflow fixture, startup topology and canceled-sidebar-transition persistence. Focus and global Reduce Motion changes may invalidate styles once; the hover/cache test isolates the following five animation frames. Existing pane-header tests also caught and verified removal of a new assumption that every retained pane host has a component-root dialog state.
- Vendor geometry and policy: 11 focused tests passed (three Equalise geometry, one constrained resize, seven Reduce Motion).
- Fixture startup: the native suite includes assembly of three chat groups and a browser surrogate before first paint. It reproduces and prevents the initial preview's startup failure, where the fixture incorrectly required a focused group before layout. Split anchors now resolve from retained topology.
- Python profiling runner: syntax parsed successfully; `git diff --check` passed.

This totals 500 distinct passed tests across app, library and focused vendor checks. Vendor tests used an offline temporary package with the actual vendored `src` symlinked, the desktop lockfile copied, and absolute patches for the pinned GPUI crates. Unused benchmark/platform/client dev dependencies were omitted from that temporary manifest; production sources and tracked manifests/locks were unchanged. A pre-existing test-only missing `MouseButton` import was fixed. Final test logs are `/tmp/monitter-base-motion-tests.log`, `/tmp/monitter-motion-tests-complete.log`, `/tmp/monitter-motion-key-test.log`, and `/tmp/monitter-motion-reflow-agent-test.log`.

## Native observations

The profiling runner copies the exact signed app into a uniquely identified fixture bundle, verifies its binary hash and profiling manifest, and starts synthetic in-memory state without a backend. Native input is supplied through the app UI; the runner generates none. It terminates only its own child process.

```sh
python3 scripts/gpui-performance-session.py \
  '<packaged app>/Contents/MacOS/monitter-gpui-desktop' \
  --scenario motion --motion-mode on --rows 10000 --seconds 90 --warmup 3 \
  --output 'artifacts/gpui/motion-validation/<observation>'
```

Input timing runs from GPUI event dispatch to platform submission; it excludes operating-system queue delay and physical display scanout. Short manually driven observations establish behavior and counter changes, not a statistically controlled performance guarantee.

### Browser beside active conversation

An optimized 90-second observation used 10,000 synthetic history rows, active sparkles, composer typing, individual key events, and focus changes to/from the browser toolbar. Typed text was verified through native accessibility state and a screenshot. Data: `artifacts/gpui/motion-validation/browser-adjacent/`.

| Measure | Result |
| --- | --- |
| Draw time p50 / p95 | 1.75 / 2.96 ms |
| Input to platform submission p50 / p95 | 11.13 / 29.33 ms (28 input-bearing frames) |
| Transcript preparations / rows prepared | 0 / 0 |
| Browser bounds writes / show/hide transitions | 0 / 0 |
| Missing visible-pane observations | 0 |
| Active outline show/hide transitions | 0 / 0 |
| Mean process CPU during observation | 16.3% of one core |

The input maximum was 158.7 ms during a burst containing 116 events in one input-bearing frame; this short mixed workload does not establish a typing-latency percentile for ordinary human input. Screenshot snapshots showed the static browser page intact. Zero bounds and visibility mutations support browser stability more directly than static screenshots alone. Copy confirmation was not visually established through the collapsed hover-only action strip in this observation.

### Layout reflow baseline

The first successful `motion` observation used three conversations of 10,000 rows each, a static native browser, and 90 seconds after an eight-second warmup. Native controls exercised expanded/compact/hidden rail widths and Equalise; typed composer text and the final equalised layout were verified. Data: `artifacts/gpui/motion-validation/layout-before/`.

This exposed 57 width-triggered transcript preparations, visiting 570,000 rows. Draw p50/p95 was 4.06/6.67 ms, with a 65.50 ms maximum. The browser had 25 changed-bounds writes during actual geometry changes, zero visibility transitions, and zero missing-visible observations. The settled rail reported 24 callbacks. Only four input-bearing frames fell inside this measurement, so input percentiles are not a useful comparison.

The implementation now separates scalar width from semantic transcript preparation. A retained geometry value is updated from the actual viewport allocation before the virtual list lays out visible rows. Bubble proportions, Markdown wrapping and portrait attachment placement use that current width. The virtual list still invalidates its stored height estimates on width changes; this optimization removes history content preparation, not all work proportional to the number of stored height entries.

### Final optimized motion on/off observations

Both final observations used the same optimized source commit `6cc751c8c700dac61b402240e0726e97e23db636`, 30,000 history rows across three chats, and a native browser. Each ran for 90 seconds after a three-second warmup. No compiler ran during either observation. Native actions followed the same sequence: type `motion test` in the first composer; compact, expand, hide and expand the rail; equalise panes; type `next` and `third` in the other composers; focus the browser address field and send Left/Right; return to the first composer and append ` ok`. Final drafts, equalised geometry and the intact browser page were verified through native accessibility state and screenshots.

| Measure | Motion On | Motion Off |
| --- | ---: | ---: |
| Draw p50 / p95 | 4.17 / 7.63 ms | 4.34 / 8.30 ms |
| Maximum draw | 16.42 ms | 13.30 ms |
| Input-to-submit p50 / p95 | 17.10 / 31.49 ms | 10.87 / 22.43 ms |
| Input-bearing frames | 42 | 37 |
| Transcript preparations / rows prepared | 0 / 0 | 0 / 0 |
| Browser bounds writes during layout changes | 54 | 8 |
| Browser show/hide transitions | 0 | 0 |
| Missing visible-pane observations | 0 | 0 |
| Outline show/hide transitions | 0 | 0 |
| Settled rail callback counter | 50 | 0 |
| Mean CPU, percent of one core | 38.17% | 2.38% |
| Total draws | 5,432 | 229 |

Data directories: `artifacts/gpui/motion-validation/layout-after-on/` and `layout-after-off/`. The rail callback count remained unchanged after settling while typing and focus changes continued. Motion On includes the application's existing continuously running sparkle effects; Motion Off disables those too. The CPU and input differences therefore do not isolate the added transitions. The larger draw count with motion enabled also changes the sampled distribution. These are short manual observations, not repeat-run evidence of equivalent input latency. The clear measured improvement is elimination of width-triggered history preparation; the earlier 65.50 ms maximum draw did not recur in this sample. This does not establish a universal latency bound or satisfy the broader proposed benchmark matrix.

The final packaged build is `artifacts/gpui/release/build-1791843562229/Monitter GPUI.app`, with adjacent server app and `build.json`. Desktop executable SHA-256: `95ff6974b80de175c9c008ca0ec3627fe208250dcab31f85bc93420c900d163e`. The build completed in 1m 47s and both bundles passed packaging signature verification. `sourceDirty` is true because the worktree contains its local untracked `.toolchain` symlink; implementation sources were committed before building. Each observation used an independently signed fixture copy with its own recorded hash and bundle identity.

## Remaining coverage and integration

- All nine planned interactions are implemented, with 500 distinct regression checks passing. This branch has not been merged or installed.
- The native motion fixture exercises the actual retained chat/dock/browser components and equalisation. Its synthetic rail uses the shared interpolation primitive; it does not instantiate the full production Desktop/sidebar/preferences lifecycle.
- Native regression tests cover hover/focus fades, reversal, hidden settlement, Reduce Motion policy changes, copy expiry, completion suppression and attachment entrance state. Live OS accessibility settings were not changed during validation.
- End-to-end production preferences entry, actual attachment storage, Copy feedback through the hover strip, IME composition and the full light/dark/theme/scale matrix remain unverified visually. Existing user data, profile preferences, backend sessions and the installed application were preserved.
- The worktree begins with a frozen snapshot of pre-existing main-checkout edits. Future integration should apply the animation commits, not replay that whole snapshot onto main.
