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

The first full-path SQLite benchmark exposed an expensive per-update history
hash-index rebuild: mutation median 166.9 ms, p95 185.3 ms. An immutable-prefix
fast path reduced a subsequent run to median 29.3 ms, p95 66.6 ms. This led to a
stable-order differential path for content edits as well as appends, retaining
SQLite primary-key rejection of duplicate appended IDs. Final benchmark results
after that last optimization remain to be recorded; these are not GUI timings.

Alex explicitly chose a completely blank profile, including no carried-over
agents, chats, or preferences. The old profile must remain separately recoverable
and CLI authentication/configuration must not change. The new `monitter-state
init-empty-profile` helper refuses existing directories and validates an empty
reopened SQLite profile. This rollout does not require importing the old history.

## Handoff status

Goal remains incomplete: native GUI acceptance has not been matched to the
installed-app workload, and the initial browser GUI run missed the target. The
SQLite implementation has passed the native tests above, but integration and
GUI acceptance remain incomplete. The live app, user sessions, and main dirty checkout were not replaced or
restarted. Complete the backup/export gate, final UI verification, integration
with main's unrelated work, and explicit installation/restart reporting first.

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
