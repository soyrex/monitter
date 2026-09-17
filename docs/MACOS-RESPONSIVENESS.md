# macOS responsiveness investigation

## Evidence and scope

The 2026-09-17 installed-app sample showed native CPU around 82–95% during a
ten-second observation. State contained approximately 3,983 messages, 53,008
events, a 162 MB event journal, and a 9.8 MB core JSON file. No conversation text
or credentials are included in this report.

The sampled provider-reader stacks repeatedly entered full historical event
hashing, full core-state serialization, and the global state mutex. The main
thread was mostly idle during that particular sample: it did not capture an
actual beachball, so this is evidence of expensive work and contention, not
proof of every reported freeze.

Source review additionally found that the AppKit key-down monitor cloned the
entire snapshot on **every keypress**, only to read the shortcut mode. It could
also wait behind a streaming disk write on the state mutex. This work is now
replaced by an atomic shortcut-mode flag; unrelated keypresses return early.
The flag follows durable settings commits, and the Escape shield remains
disabled outside Vim mode.

Other system load was substantial (including `fseventsd` and WindowServer),
and free disk space was low. Those processes and the running app were not
stopped. The installed executable resembles the debug artifact in size, but
its hash did not match the recorded artifacts; its exact build profile is
not proven.

## Implementation

- Serialize state writers separately from the immutable committed-state view.
  Snapshot readers do not wait for candidate construction or persistence.
- Persist state in a private SQLite database using individual entity rows,
  ordered collections, indexed task/timestamp fields, WAL, and `synchronous=FULL`.
  The serialized writer applies only changed, inserted, deleted, or reordered
  rows in one transaction; retained history is neither reserialized nor updated
  merely because another record changed. User sends, approvals, completions,
  and errors are still acknowledged only after that transaction commits.
- Share immutable event records and detail strings across in-memory snapshot clones. The
  JSON wire shape remains unchanged.
- Run blocking native commands, including the external folder chooser, on
  worker threads. Keep native key handling independent of transcript locks.

Two frontend experiments were rejected rather than shipped: a 75 ms Markdown
timer added delay to updates already coalesced at 100–125 ms, and an exact
recursive transcript comparison benchmarked slower than the existing JSON
comparison. Reader-held transcripts, live approvals/errors, scrolling, and
Markdown behavior remain unchanged. Duplicate TypeScript/parser declarations
at the branch baseline were repaired to allow frontend validation.

## Repeatable checks

Run from this worktree, with a private target directory or an explicitly chosen
shared build cache. Do not run `npm run build` for isolated QA: it also publishes
the LAN web bundle. Use `node node_modules/vite/bin/vite.js build` instead.

```sh
cd src-tauri
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test --lib --offline
TAURI_CONFIG='{"bundle":{"resources":[]}}' cargo test --lib --offline \
  store::tests::benchmark_save_update_against_checkpoint_history \
  -- --ignored --exact --nocapture --test-threads=1
```

The ignored benchmark compares full checkpoints with incremental updates on
separate temporary synthetic histories. It does not contact a model, run a
user task, or read live app data. Wall-clock numbers are reported, not enforced
as fragile machine-independent assertions.

Pre-SQLite JSON-storage baseline: initial debug-test result (4,096 events with 4 KiB detail each, 1,000 messages
with 2 KiB text each, 20 updates, same loaded Mac):

| Persistence path | Median | p95 |
| --- | ---: | ---: |
| Full checkpoint per update | 526.9 ms | 673.7 ms |
| Numbered-JSON incremental prototype | 16.6 ms | 34.9 ms |

This measures storage calls, not end-to-end typing latency or a release-build
speedup. Deterministic service tests separately verify that readers progress
while a writer is paused, rejected candidates do not change visible revisions,
concurrent writers do not lose updates, and shortcut flags do not acquire the
transcript mutex.

The subsequent full native Service benchmark used 53,000 distinct 4 KiB event
details and 4,000 2 KiB messages, with 20 mutations in a debug build:

| Operation | Median | p95 |
| --- | ---: | ---: |
| Candidate construction + durable mutation + publication | 20.6 ms | 28.2 ms |
| Unchanged-revision UI read | 7.3 microseconds | 13.5 microseconds |
| Full compact UI projection | 42.8 ms | 47.2 ms |

These are in-process timings, not IPC, browser rendering, or a matched
before/after installed-app result. A pre-SQLite native baseline was recorded
as 418 passed, 0 failed, and 10 intentionally ignored (92.93 seconds). An
earlier parallel run hit an unrelated OpenCode inspection timeout; the
legacy-null metadata migration failure found in that earlier work was fixed
and covered. This historical result is not a claim about the current SQLite
integration; coordinated native validation remains to be reported.

The pre-SQLite production frontend build and the performance, activity-grouping,
and snapshot-index regression scripts were recorded as passing. An isolated,
separately identified, ad-hoc-signed **debug** native bundle was built and
launched successfully with its own `com.monitter.responsiveness.20260917` data
directory. It is not installed as the user's app and is not a release acceptance
result.

### GUI evidence still incomplete

`node scripts/gui-latency-preview.mjs` serves an explicitly synthetic, loopback
fixture with 4,000 messages and four streams. An initial Chrome run exercised
typing, chat switching and scrollback, but did **not** meet the proposed target:
reader/composer event-to-two-animation-frame proxy p95 was about 163 ms and
one chat switch was about 380 ms. Frame gaps included multi-second outliers.
Concurrent compilation, substantial system load and browser-control timeouts
confound that run; these numbers cannot establish an app-only cause or a native
WebKit result. The fixture's metrics are proxies, not physical paint latency.
Do not claim the GUI/beachball goal is complete from the storage results.

### Storage direction under evaluation

The numbered-JSON update journal measured above was a pre-SQLite prototype and
is not the deployed persistence format. The current implementation uses bundled
SQLite with individual entity rows, indexes, WAL and durable transactions. It
does not retain a numbered-patch fallback. The independent writer/read
separation, immutable history and native worker/key handling fixes remain.

The standalone `scripts/sqlite-latency-benchmark` prototype uses bundled SQLite
3.53.2 via rusqlite 0.40.2. With 53,000 4 KiB events and 4,000 2 KiB messages,
25 WAL/FULL row-level update+insert transactions measured debug median 0.241 ms,
p95 1.646 ms (release 0.217/0.568 ms); import took about 9 seconds and reopened
integrity/count checks passed. This does **not** include Service copying,
diffing, IPC or rendering and is not directly comparable to the Service table.
The indexed latest-20 benchmark selects IDs only, not full message payloads.

SQLite is integrated into the native Store source, but it has not been deployed
or installed over user data. The Store uses a strict application/schema check,
row-level differential transactions, WAL checkpoints for migration installs,
and fail-closed recovery. Migration first records an intent with exact legacy
source hashes, creates exact private source backups, verifies the imported
database, installs it atomically, then replaces `state.json` with a downgrade
guard only after verification. A private lease prevents two current binaries
from writing the data directory concurrently.

The lease is not understood by an already-running old binary. Before any
migration, the old app must be verified stopped; otherwise it can write legacy
JSON after the final source verification and before the downgrade guard is
installed. There is no safe downgrade after new SQLite writes without an
explicit legacy export, or restoring a complete pre-upgrade backup. No
deployment, install, or live-data migration has been performed for this work.

The later frontend candidate changes only the immutable AppSurface snapshot
binding to `$state.raw`, retaining reactive form/composer state. The focused
installed-Svelte benchmark demonstrated substantially less indexing overhead;
this is a microbenchmark, not GUI acceptance. Its production Vite rebuild passed.
A subsequent installed-Svelte run measured 1,629 ms for deep-state indexing and
94.5 ms for raw-state indexing across the same 25-snapshot workload (94.2% less
work in that microbenchmark). Native GUI acceptance is still required.

### Resumed SQLite validation

The integrated Store and migration implementation passed 429 native tests with
0 failures and 9 intentionally ignored (97.99 seconds, serial debug run).
Coverage includes exact legacy backups, committed journal prefixes, interrupted
installation recovery, corruption/symlink rejection, transaction rollback,
usage accounting, and readers progressing during a blocked terminal-error write.
Later changes must be rerun, particularly integration with the main checkout.

The integrated branch preserves the main checkout's pending native subagent,
goal, archived-agent-name and appearance work. SQLite now atomically round-trips
the subagent session list and inline transcript map too. Its combined native suite
passed **444 tests, 0 failures, 9 ignored** (84.22 seconds, serial debug run).
A preceding run had one unchanged PTY interrupt test timeout; its focused retry
and the full rerun passed. Frontend performance, activity-grouping and snapshot
index checks also passed, and integration did not change the already-built frontend.
The original dirty checkout still matches its captured tracked-file snapshot.
After adding the subprocess abrupt-exit WAL recovery test, the complete suite
passed **446 tests, 0 failures, 9 ignored** (66.63 seconds, serial debug run).
That test verifies committed event/usage recovery, rollback of an unfinished
write, and OS release of the profile lease after a child exits without Drop.

The first full-path SQLite benchmark exposed an expensive per-update history
hash-index rebuild: mutation median 166.9 ms, p95 185.3 ms. An immutable-prefix
fast path reduced a subsequent run to median 29.3 ms, p95 66.6 ms. This led to a
stable-order differential path for content edits as well as appends, retaining
SQLite primary-key rejection of duplicate appended IDs. A serial rerun after
that optimization measured mutation median 11.15 ms / p95 11.61 ms, unchanged
UI reads 5.79 / 8.96 microseconds, and full UI projection 35.10 / 35.46 ms.
It used the same 53,000-event/4,000-message debug fixture and 20 updates, without
concurrent compilation. This is the pre-main-feature-integration benchmark and
does not include IPC or rendering; these are not GUI timings.
A post-integration rerun with the same fixture measured mutation median
12.25 ms / p95 13.16 ms, cached UI reads 7.92 / 9.58 microseconds, and full UI
projection 36.48 / 37.60 ms (21.52-second run, no concurrent build).

The isolated optimized WebKit test bundle built successfully and its ad-hoc
signature verified. Its distinct profile was initialized by the built companion
before first launch; an offline integrity check returned `ok`, with only one
host row and two metadata rows, no agents/history. This is not the installed
user profile. The loopback synthetic page uses browser-mode rendering inside
WebKit and mocked snapshots; it deliberately has no native IPC permission.
An initial fixture startup attempted a native escape-shield command and was
correctly denied by ACL. The fixture marker was corrected without broadening
permissions. Native backend and rendering tests remain separate, not a real
provider-to-SQLite-to-IPC end-to-end performance result.

The first WebKit fixture run (4,000 messages, four streams at 100 ms, token-style
chunks) measured 87 reader/composer events at p50 35 ms / p95 102 ms / max 103 ms.
Two fresh chat switches while streaming measured 227 ms and 278 ms; these miss
the 100 ms target. Scrolling up exposed the jump-to-latest control with new
updates waiting. The old frame-gap display retained only its latest 240 samples,
so it cannot establish a whole-run maximum; the fixture now retains up to
10,000 frames and explicitly reports whether Long Tasks observation is supported.
System load included about 8.3 GB swap and a busy FSEvents process. Subsequent
opt-in phase markers diagnose chat selection, transcript mount, and the first
virtualizer change without claiming that a callback is a completed screen paint.
Three instrumented chat switches measured control median 138 ms / max 222 ms,
selection-to-first-virtualizer-change median 11 ms / max 14 ms, and selection-to-
mount median 37 ms / max 53 ms. Later pane splitting exposed a stale diagnostic
start timestamp; the helper now expires starts after 5 seconds and consumes the
start after mount. The bogus later pane attribution is excluded from these
selection measurements; these probes do not causally attribute multi-pane work.

A subsequent two-pane/four-stream run retained the same grown message bodies.
Its final 100 reader/composer samples were median 30 ms / p95 115 ms / max 120 ms;
the measured visible frame-gap maximum was 282 ms over 2,809 frames. No
multi-second gap appeared in that bounded run, but it still misses the p95 goal
and is not proof the user's original beachball is resolved. The held right
reader stayed visually on stream ticks 1049–1101 while the following left pane
advanced from tick 1494 to 1684. Clicking Jump to latest released the queued
right-pane updates, and both panes then reached tick 1713. All synthetic streams
were stopped after the check; no provider/model requests were made.

Alex explicitly chose a completely blank profile, including no carried-over
agents, chats, or preferences. The old profile must remain separately recoverable
and CLI authentication/configuration must not change. The new `monitter-state
init-empty-profile` helper refuses existing directories and validates an empty
reopened SQLite profile. This rollout does not require importing the old history.
Run that companion CLI before the new profile's first GUI launch: the GUI itself
creates its profile directory, and the helper deliberately refuses existing
directories. On macOS its exact destination is `Library/Application Support/`
plus the bundle identifier. A new identifier also isolates WebKit preferences
for test runs. Resetting the installed identifier requires separately preserving
its existing WebKit/client settings as well as its backend profile while stopped.
Service startup creates one hidden internal housekeeping agent; this is not an
imported user agent and does not automatically start a model turn.

## Handoff status

Goal remains incomplete: GUI acceptance has not been matched to the
installed-app workload, and browser/WebKit fixture runs still missed the p95 target. The
SQLite implementation and integration with the captured pending workspace work
have passed the native tests above; GUI acceptance and rollout remain incomplete.
The live app, user sessions, and main dirty checkout were not replaced or
restarted. Complete the recoverable blank-profile cutover preparation, final UI
verification, and explicit installation/restart reporting first.

### Packaged blank-profile candidate

Source commit `9684a3b` produced the optimized real-UI candidate, built with its
normal bundled LAN web assets and a separate test bundle identity. The native
build completed in 5m24s after the final Vite build (2m25s). Ad-hoc signing and
`codesign --verify --deep --strict` passed. The protected deliverable is:

`/Users/alex/code/monitter-macos-responsiveness/artifacts/sqlite-candidate-OaYQpq/Monitter SQLite Candidate.app`

Its adjacent `manifest.json` records the source commit, executable SHA-256,
signature check, and profile identity. Do not include this 22 MB deliverable
directory or its profile in cache cleanup. It was launched and visibly verified
at `tauri://localhost` (not the synthetic page): **Connected 0 agents · 0 running**,
**Create first agent**, and no tasks in any status section. Its identity is
`com.monitter.sqlite-candidate.20260917`; its separately initialized profile is
`/Users/alex/Library/Application Support/com.monitter.sqlite-candidate.20260917`.
The one internal housekeeping agent is hidden, as designed.

The candidate is open for a trial. `/Applications/Monitter.app` and the older
user-used `Monitter Performance Test.app` were not replaced or stopped. No old
profile, CLI login, or CLI configuration was reset. A real installation/blank-
profile cutover still needs an explicitly coordinated quit/restart and protected
old-profile/client-settings backup. Passing synthetic checks and native tests
does not establish that real provider streaming can no longer beachball.

### Native stack sample and immutable-message formatting

A read-only native WebContent sample was collected from the sole new WebContent
process appearing with the isolated test-app launch (PID 70255, launch time
22:26:42, app PID 70246). This attribution is launch correlation, not verified
process-coalition metadata. The retained reports and context are in
`artifacts/webkit-profile-BU6cA8/` (about 3 MB; diagnostic evidence, not build cache).
The three-second idle report had 1,136 of 1,263 main-thread observations waiting
in `mach_msg`. In the 15-second streaming/switch report, 3,567 of 6,262 observations
were waiting and 1,993 were in timer callbacks. A timer/microtask JavaScript chain
contained 903 observations; one direct `Date.toLocaleString` branch contained
75, with nested Intl/ICU formatter construction. These are sampled stack states,
not CPU percentages, and nested/recursive counts must not be added together.
No single runaway layout stack or multi-second beachball was demonstrated.

The new read-only `scripts/macos-responsiveness-sample.mjs` records selected PID
CPU/RSS and deltas of cumulative VM counters. It uses shell-free system tools,
bounded duration/arguments, reports missing/reset counters as unavailable, and
does not change processes or files. Its parser/self-tests pass. A 15.010-second
sample from 20:28:24.933Z to 20:28:39.943Z recorded 132 swap-in pages (2,162,688
bytes) and zero swap-outs/pageouts/compressions. Thus active swap thrashing was
not evident in that window despite about 8.2 GB total swap used. Rolling CPU
maxima were 54.6% for test WebContent, 2.2% for the test app, 100.6% for fseventsd,
and 42.4% for WindowServer. Fseventsd had about 6.3 GiB RSS. The latter background
load remains a confound, not an established explanation for Monitter's stalls;
no system service was stopped or reconfigured.

The stack evidence led to a reproducible Svelte dependency issue. MessageMeta's
Date derived directly from a prop getter backed by a replaced message object.
Even unchanged `createdAt` values rebuilt Date/Intl formatting on each snapshot.
Markdown had the same pattern: its parse/sanitize derived directly from the
message-backed text getter, so unchanged history was reparsed and resanitized.
Both now have a primitive derived boundary before the expensive operation.

The tests `node --conditions=browser scripts/message-meta-reactivity-test.mjs`
and `node --conditions=browser scripts/markdown-reactivity-test.mjs` compile the
actual component declarations and execute Svelte client reactivity without a
browser/DOM. Over 1,000 unchanged parent-message replacements, the baseline
performed 1,000 Date constructions/time/title formats and 1,000 Markdown
parse/sanitize calls. The primitive-boundary versions performed zero additional
calls. Changed timestamps/text still updated once, invalid dates stayed safe,
and the Markdown test confirms sanitizer output remains the rendered value.
The Markdown test mocks parse/sanitize to count invocations; it is not a new
sanitizer-security test. Production DOMPurify use and image/link behaviour are
unchanged. No transcript content or date formatting policy changed.

The production frontend build passed after both primitive-boundary changes
(existing ShareControl accessibility warning remains). A fresh full-app native
fixture run at about 22:40 local time used four 1,000-message chats, four 100 ms
streams, two panes, Light theme, and Motion System. It reached 589 stream ticks.
Three chat switches measured median 139 ms / maximum 147 ms; open-to-mount
maximum was 33 ms and first virtualizer change maximum 12 ms. The last 100
reader/composer events measured p95 42 ms / maximum 43 ms; maximum visible frame
gap was 144 ms. These are event-to-two-rAF proxies, not screen-paint timing.
This small sequential run does not establish a causal improvement over the
previous run, and chat switching still exceeds the 100 ms target.

Both panes accepted typing during streaming and streamed content rendered.
Scrolling the right message pane upward at about tick 568 exposed the live
"new updates waiting" jump control, but the screenshot showed a blank message
viewport despite history remaining in the accessibility tree. Clicking Jump
restored the visible stream tail. This is an unresolved scrollback rendering
observation, not a reader-position acceptance pass. The task-owned fixture app
and loopback server were stopped afterwards. The protected packaged SQLite
candidate remains unchanged and does not yet contain these frontend fixes.

#### Scrollback follow-up

The loopback full-app fixture now has an explicit scroll-geometry inspection
control and wheel-triggered readouts after two animation frames and 500 ms.
They report scroll bounds, mounted row indexes, intersecting row boxes and
spacers, without message contents. A fixture-only mask toggle supports isolating
paint from range errors if the blank viewport recurs. These diagnostics are not
loaded into the packaged app; their geometry reads make this a diagnostic run,
not a clean latency benchmark.

In the next native run, the blank viewport did not recur when scrolling within
a 5,264 px streamed row, nor within a 9,724 px row after typing during streaming.
The latter had row 999 intersecting the viewport at relative top -8,226 px.
A deeper 20-page upward scroll mounted indexes 951–966 plus the retained
970–999 tail; rows 957–960 intersected and were visibly readable. During renewed
streaming, the held right pane stayed at scrollTop 116,607 / scrollHeight 128,878
with the same intersecting row positions, while the live left pane grew from
127,971 to 128,840 px. Its pending-update control remained live. Jump released
the held history and the next scrollback remained visibly readable.

This verifies those specific reader-hold/range cases, but does not explain or
resolve the earlier single blank screenshot. No production virtualizer or mask
change was made on that evidence. The synthetic app and server were stopped
afterwards; its profile and diagnostic evidence were retained.

### Avoid unchanged document-wide appearance work

The full snapshot application path called `applyAppearance` directly and again
through its reactive effect. A separate tint/chrome effect also depended on the
snapshot. Each changed snapshot could therefore repeat palette calculation,
root CSS/dataset changes, and synchronous appearance localStorage writes even
when agents changed only transcript content.

`appearance-key.ts` now keys the renderer on its 12 consumed settings fields,
four palette-selection fields, viewer scale, surface tint, and native-runtime
mode. A separate browser-chrome key preserves system colour-scheme updates.
Caches are plain variables, not reactive dependencies. Native zoom failures
invalidate the appearance cache for a retry; embedded panes still inherit the
root's document-wide appearance. The duplicate chrome call was removed from
the tint-only CSS effect. No palette, animation, or user preference changed.

`node scripts/appearance-update-test.mjs` executes the production renderer bodies
with real colour helpers and mocked DOM/storage. The first render recorded 38
style/meta calls, 13 legacy removals, five dataset writes, and two storage writes;
1,000 fresh-but-equivalent snapshots added none. Tests cover every key input,
unrelated-setting stability, font/scale/palette changes, system-colour changes,
and a failed native zoom followed by a successful same-key retry. The paired
theme, interface-scale preference, performance-contract, and diagnostic-hook
tests pass. A direct Vite production build passed with the existing ShareControl
accessibility warning. The standalone browser scale suite did not execute its
cases because its browser executable was unavailable; no browser was installed.

A subsequent native WebKit full-app synthetic run used two panes, four streams
at 100 ms, and fresh 1,000-message chats. With the existing System motion setting,
the three switch proxies had p50 150 ms / maximum 160 ms, with open-to-mount
maximum 30 ms and first-virtualizer maximum 10 ms. The last 100 reader/composer
input samples had p95 37 ms / maximum 64 ms. Visible frame-gap maximum was 154 ms.
There were 590 generated stream ticks before stopping.

Motion was then set to Off only in that test profile, verified in Controls, and
the page reloaded to reset synthetic histories before repeating the workload.
Three switch proxies had p50 128 ms / maximum 167 ms; mount maximum 34 ms and
first-virtualizer maximum 11 ms. Reader/composer p95 was 76 ms / maximum 77 ms,
with maximum frame gap 155 ms and 511 stream ticks before stopping. This small,
sequential comparison does not establish a speedup; critically, disabling motion
did not remove the >100 ms tail. Motion was restored to System. Light → Dark →
Light was also verified through the native UI after the appearance guard.
Attempted scroll actions in these full-app runs did not visibly confirm reader
detachment, so these runs add no reader-position acceptance claim.

These are event-to-two-rAF proxies, not physical paint latency. WebKit did not
support Long Tasks observation. The fixture bypasses real provider/IPC storage
delivery. No end-to-end beachball-resolution or causally measured GUI speedup
is claimed. The test app and loopback server were stopped after the run; the
protected `9684a3b` candidate was not rebuilt or altered with these later changes.

The native-path review identified a separate remaining scaling risk: providers
batch text at 100 ms, but every committed mutation still emits a tiny
`monitter:changed` event, with no service-wide coalescer across parallel runs.
Renderer refreshes coalesce, but this does not bound Tauri event evaluation.
Furthermore, compact UI snapshots cap diagnostic events but still carry all
message/channel/subagent transcript text. Rust projection runs off the UI
thread, but serialization/IPC/WebKit parsing can still grow with history. Any
future event coalescer must preserve the final update and approval/error wakeups,
and account for the existing bridge 100 ms plus AppSurface 125 ms scheduling
rather than simply stacking another delay. These are evidenced remaining risks,
not proof that either caused the user's observed beachballs.

### Isolated native transcript geometry diagnostic

`node scripts/transcript-geometry-preview.mjs` builds a small fixture in memory
and serves only `127.0.0.1:18434`; `--build-only` compiles without a server or
disk bundle. It uses the real MessagePane, TranscriptVirtualList, Markdown, and
reader buffer, with four synthetic 1,000-message chats and immutable 100 ms
updates. It does not use TaskTranscript, provider output, IPC, or persistent
workspace data. Its simplified last-message fingerprint is valid only for this
append-only synthetic workload, not a replacement for production change
detection. Icons are stubbed. This is a diagnostic, not an end-to-end acceptance
benchmark or a comparison against the earlier full-app fixture.

An initial fixture run froze at tick 1 because its own geometry-reporting effect
tracked the parent metrics state it updated. That run was discarded. Sampling
is now untracked, and runtime errors are displayed visibly. The corrected run
in the separately identified native WebKit test app reached 597 ticks, with two
panes, typing, three chat selections, and reader detachment:

| Diagnostic | Observed result |
| --- | --- |
| Typing event to two animation frames | 136 events; maximum 47 ms |
| Chat selection to two animation frames | 3 events; maximum 104 ms |
| Wheel event to two animation frames | 2 events; maximum 11 ms; too few for a useful distribution |
| Visible frame interval | Whole-run maximum 107 ms |
| Direct row measurement | 482 calls; maximum 1 ms |
| Margin measurement | 172 calls; maximum 1 ms |
| Follow-layout callback | 1,635 calls; maximum 3 ms |
| Synchronous virtualizer flush | 349 calls; maximum 1 ms |

The timer resolution was about 1 ms: reported zero is not zero work. Phases can
overlap and do not include all browser layout/paint work. The chat-start mark is
shared by both fixture panes, so its six first-virtualizer measures are not six
independent chat selections. Small-sample percentile rounding in the fixture
was corrected after this run; the table deliberately reports unambiguous
counts and maxima instead. Future quantiles use nearest rank over the last
120 samples, and cumulative phase totals are shown separately.

While the primary pane advanced from scrollTop 125250 to 125757, the detached
secondary remained at 124055 with a 1166 px bottom gap and a held snapshot.
The jump action released the held state. Its final geometry sample was taken
before layout settled after streaming stopped, so it is not proof of the final
bottom coordinate. The fixture and its server were then stopped; no data was
deleted and no user-used app was stopped.

This evidence does not justify removing the virtualizer's synchronous flush or
scroll-owner safeguards: measured callback maxima were small, while the
broader selection proxy still exceeded 100 ms. Opt-in geometry probes are
inactive outside `monitter-perf=1`. Their off/on/error/bounded-entry contracts
pass `node scripts/perf-phases-test.mjs`; the fixture compile and existing
performance, activity-grouping, and snapshot-index tests also pass. These new
diagnostics are not included in the protected candidate built from `9684a3b`.

## Build-cache cleanup tracking

Alex requested tracking build-cache cleanup alongside the responsiveness work.
The 2026-09-17 inventory (build sizes rechecked after the candidate build) was:

| Exact build directory | Size | Cleanup status |
| --- | ---: | --- |
| `/Users/alex/code/monitter/src-tauri/target/debug` | 5.4 GB | Shared cache; contains a running user-used test app. Preserve until its consumers are stopped and exact disposable targets are agreed. |
| `/Users/alex/code/monitter/src-tauri/target/release` | 1.1 GB | Task build cache; compilation finished. Retain through native QA and artifact handoff, then assess disposable intermediates. |
| `/Users/alex/code/monitter-macos-responsiveness/build` | 4.1 MB | Built frontend for the packaged candidate and full-app fixture; retain through QA. |
| `/Users/alex/code/monitter-macos-responsiveness/.svelte-kit` | 6.5 MB | Regenerable frontend intermediates; retain while frontend verification continues. |
| `/Users/alex/code/monitter-macos-responsiveness/artifacts/sqlite-candidate-OaYQpq` | 22 MB | Protected signed candidate and manifest, not a build cache. Keep. |
| `/Users/alex/code/monitter-macos-responsiveness/artifacts/webkit-profile-BU6cA8` | 3.0 MB | Native profiling evidence and context, not build cache. Keep. |
| `/Users/alex/Library/Caches/Monitter/builds` | 319 MB | Existing staged apps and 90 MB of packages; identify referenced deliverables before removing any exact staging directory. |
| `/Users/alex/code/monitter/artifacts` | 5.4 GB | Mixed artifacts, not a disposable cache: includes 1.4 GB install backups and 3.0 GB idle-runtime diagnostics. Preserve pending exact classification/approval. |

No cache directories were deleted for this inventory. Free disk space was about
11 GiB at the post-build check (earlier as low as 3.3 GiB; unrelated system activity changes
it). Before cleanup, remeasure sizes, identify exact no-longer-used artifacts,
and preserve the deliverable app, running apps, source worktrees, profiles,
backups, CLI auth, and shared dependencies. Prefer recoverable removal; report
actual free-space change separately, since moving files to Trash alone does not
reclaim disk space. Never clean a cache while a build is using it.
The shared debug `incremental` directory measured 1.2 GB in a later check; this
task disables incremental compilation, so that directory is not assumed owned
by this work. The synthetic WebKit test app and its loopback server have now
been stopped; its generated profile/artifact files are retained for inspection.
The same listed sizes were rechecked after the 22:40 frontend test, with 11 GiB
available. No deletion or cache cleanup was performed. The 1.2 GB incremental
directory is already included in the 5.4 GB debug total, not additional space.

## Native acceptance and rollout boundary

Use a separately identified test app/data directory before touching the live
installation. Exercise typing, chat switching, scrollback, split-pane resizing,
concurrent streaming, approvals, and completion. Target p95 interaction latency
under 100 ms and no multi-second stalls. Record the app profile, history size,
pane count, active streams, CPU/RSS, memory pressure, and before/after workload;
do not compare unrelated workloads or claim success from a browser-only test.

Back up the whole data directory, not just `state.json`. Older binaries cannot
decode the explicit new storage marker; this prevents silent loss of updates.
Downgrading requires an explicit legacy-format export or restoring a whole
pre-upgrade backup, not merely producing another new-format checkpoint. Never
launch two versions against the same app data. Do not restart active user
sessions for this validation. Installation and native acceptance must be
reported separately from source/test completion.
