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
| `/Users/alex/code/monitter-macos-responsiveness/artifacts/sqlite-candidate-OaYQpq` | 22 MB | Protected signed candidate and manifest, not a build cache. Keep. |
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
