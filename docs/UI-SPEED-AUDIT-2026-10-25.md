# Native UI speed audit — 2026-10-25

## Status

The performance goal remains open. The current source includes the eight scoped
improvements below, additional font/snapshot work, and latency instrumentation.
The installed signed scheduler release has GUI SHA-256 `605cdc1d2f8fee469573968ee052f59826626a591d05fcbddd712176b49ca1d4`.
One controlled launch of that installed release observed actionable paint
1,089.675 ms after the launch request. Its four workspace requests all reached
content paint on the first attempt, but readiness diagnostics invalidate the
formal workspace sample. A mixed navigation and typing capture recorded 33
input frames, with 78.709 ms p95 to platform submission; it is not a
typing-only result. The six saved draft digests and empty attachment set
survived the GUI restart, and the backend kept the same process identity.
The sub-500-ms startup and prepared-switch targets remain unverified or unmet.

## Implemented source changes

| Area | Change | Evidence still required |
| --- | --- | --- |
| Cached startup | Prepare cached workspace content after session-lock authorization, without waiting for full workspace publication. | External launch to usable restored workspace; locked-state behavior. |
| Workspace activation | Remove save completion from the workspace activation dependency. | Switch latency and persisted layout after relaunch. |
| Retained workspaces | Reuse prepared content within its native window. | Prepared and first-visit switch cohorts, draft retention. |
| Chat header | Avoid cloning whole task records for header rendering. | Release rendering and input measurements. |
| Publication updates | Scope invalidation to affected content and reuse shared snapshots. | Typing during incoming activity without broad redraw stalls. |
| Sidebar | Render visible rows through a virtual list. | Real large-sidebar navigation and scrolling. |
| Transcript | Reuse cached summary/grouping work. | Long transcript updates and scrolling. |
| Pane status | Retain controls and share their snapshot context. | Correct control updates without repeated unrelated rendering. |

The follow-up font work bypasses installed-font enumeration for bundled fonts and
shares the installed-font catalog for custom families. Typography cache keys
include the catalog revision. A missing initializer found by compilation was
corrected. The latest installed release includes this correction; the older
rollback bundle remains an artifact, rather than the running GUI.

## Observed baseline

On 2026-10-25, the installed earlier release reported these milestones for PID
22246, relative to entering `main`:

| Milestone | Time |
| --- | ---: |
| Application ready | 209.4 ms |
| Window created | 388.6 ms |
| Session lock authorized | 543.0 ms |
| Cached workspace ready | 546.0 ms |
| Cached workspace paint observed | 1192.3 ms |
| Window activated | 1193.1 ms |
| Live workspace paint observed | 1564.4 ms |

These are an existing-process diagnostic snapshot, not an external launch timing
trial. The executable SHA-256 was
`f1bf9ecb2c77076dbfa8fa7bb5bea4e49d9eda81103ed296a642bb00742c2f44`.
It predates the latest font/shared-snapshot changes and stronger readiness markers.

A three-second main-thread sample contained substantial GPUI drawing, chat-pane
rendering and publication work. Sampling stack counts are inclusive observations,
not operation counts or latency measurements. No installed-font enumeration was
visible in that sample. CPU snapshots showed ample idle capacity; machine load
does not establish the cause of the UI stalls.

## Measurement contract

- Record exact release artifact hash, source snapshot and signature verification.
- Launch through the supported native UI interface; record a fresh GUI process,
  the external request timestamp, and unchanged backend process identity.
- Require ordered draft hydration, workspace command readiness, and observed
  paint before treating startup as usable. A window appearing alone is insufficient.
- Report external launch time separately from monotonic `main`-entry milestones.
- When available, also report kernel process creation to main entry and to the
  usable workspace paint. Bind that optional timestamp to the same PID, process
  identity and executable hash; reject inconsistent clock ordering. Its absence
  must not erase otherwise valid main-entry evidence or imply a zero launch delay.
- Separate prepared workspace switches from requests requiring preparation.
  Include failed, superseded, evicted and incomplete requests in the report.
- Reject incomplete workspace logs: bind the file header to the measured PID,
  check dropped-record diagnostics, and reconcile requests with recorded UI actions.
- Report input-to-platform-submission measurements with their sample count and
  percentiles. They exclude physical display scanout and must not be described as
  complete hardware key-to-pixel latency.
- The current input histogram combines all invalidating input types and records
  the oldest pending event for each submitted frame. Run the focused typing
  interval before workspace-click measurements and label any mixed interval
  accordingly. Record excluded mid-draw events and events per frame; a zero-sample
  capture cannot demonstrate responsiveness. Check text/caret correctness and
  draft retention separately from latency.
- Preserve drafts, attachments, session-lock checks and live backend sessions.

`MONITTER_WORKSPACE_TIMING=1` enables workspace instrumentation. The optional
`MONITTER_WORKSPACE_TIMING_LOG` names a new absolute file; a bounded background
writer records content-free timing events without waiting for file I/O on the UI
thread. Logging remains disabled by default. Queue drops and pending-request
evictions are reported explicitly.

## Build and runtime verification

The previous release attempt failed with an omitted `font_catalog_revision`
initializer and a process-spawn error. Its reservation was completed as failed;
no release artifact from that attempt was accepted.

The corrected retry is registered as
`ui-speed-font-snapshot-release-retry-20261025`, using the shared incremental Cargo
target and one build job. A separate existing profiling request is available for
input latency measurement. The other task's expired deployment reservation was
recovered by its owner. The corrected release completed under reservation
`1e9c9198-2f8e-44a5-9e30-c2b21ba394e9` with observed exit 0. The optimized build
took 8m41s and produced `artifacts/gpui/release/build-1792947676302/Monitter GPUI.app`.
Strict deep signature verification passed; the GUI hash matched its metadata:
`0b9e466f9694f5104cecb3b30a225c06db435f8b81ae414825cfcde4b9335209`.
All 1,524 captured source fingerprints were unchanged through this build.

The earlier audit pass added source and measurement preparation without launching
a compiler or replacing/restarting the GUI/backend. Source diff checks passed.
The resumed release pass found all ten follow-up source fingerprints unchanged
and about 33 GiB available. Useful artifacts and warm caches were retained, with
no cleanup needed. No automated tests have been run by this resumed pass.

Disk remained at 33.06 GiB free after packaging; the 11.3 GiB incremental cache
was retained. No build process remained when the reservation was completed.

### First corrected-release diagnostic launch

A signed diagnostic copy of that artifact was launched through CUA at
`1792947802211` ms Unix time. Its only modifications were diagnostic bundle
identity/environment and the resulting ad-hoc signature. PID 35350 entered
`main` at `1792947803278` ms. The timing JSON was read before another GUI started.

| Milestone | From main entry |
| --- | ---: |
| Application ready | 188.4 ms |
| Kit initialization ready | 326.8 ms |
| Fonts ready | 335.1 ms |
| Window created | 1015.7 ms |
| Session lock authorized | 1221.2 ms |
| Cached workspace paint observed | 1287.4 ms |
| Drafts hydrated | 1510.7 ms |
| Workspace actions ready | 1511.4 ms |
| Workspace actions paint observed | 1577.4 ms |

This misses the 500 ms target. The external CUA request preceded main entry by
1,067 ms, so these internal timings are not an external launch acceptance claim.
The 681 ms fonts-to-window interval needs finer instrumentation before attribution.
The stale connecting footer was visible despite the action-readiness markers.

The first workspace request logged about 2,000 ms to content paint and the next
about 275 ms. These are diagnostic observations, not accepted switch percentiles:
an independently launched installed GUI (PID 35599) appeared at 18:04:03 during
the trial, and CUA timed out partway through the action sequence. This caused a
native-preference revision conflict. The trial is explicitly excluded from
workspace acceptance. The diagnostic was closed through its UI after reloading
the saved workspace; all seven saved draft digests and attachment digests were
unchanged. Backend PID 22235 remained running throughout.

Evidence is retained under `/tmp/monitter-ui-speed-20261025/release2/` and
`/tmp/monitter-ui-speed-20261025/runtime-release2/`. The installed normal GUI was
not replaced by this measurement pass. Startup, switching and typing targets
remain open. The startup milestones were transcribed from the observed tool
output into `runtime-release2/launch-1/startup-observation-from-tool.json` after
the shared timing file was overwritten by the other GUI. That transcription is
explicitly labelled; it is not an original timing-file receipt.

### Profiling release ready for measurement

Reservation `2d177bd8-23b2-453d-b8b3-09b0b972e62b` completed with observed exit 0.
The optimized profiling build took 19m31s and produced
`artifacts/gpui/release/build-1792949437645/Monitter GPUI.app`. Strict deep signature
verification passed and the executable hash matched the artifact metadata:
`5bab91cbb0cee157121fdc9542a5662b5f2abc9e876210d06e530aa5063e0ad1`.
All 1,524 captured Rust source hashes remained unchanged. No owned build process
remained when the reservation was completed. Free disk space went from
32.89 GiB to 32.38 GiB; the 11.7 GiB warm cache was retained and no cleanup was
needed. No automated tests were run by this pass.

This build includes the window-options, native-window root-builder and
window-open-returned milestones, input latency instrumentation, and the peer's
composer-footer readiness recovery. Its build evidence is in
`/tmp/monitter-ui-speed-20261025/profile1/`.

A signed measurement copy is staged in
`/tmp/monitter-ui-speed-20261025/runtime-profile1/launch-1/`. It preserves the
normal bundle identity and is intended for a coordinated GUI-only measurement
at `/Applications/Monitter.app`; it has not been installed or launched. Its
environment enables content-free workspace timing and the bounded performance
capture. The resulting signed-copy hash is
`4db808161b8081f3ce3d3b4bf8a72ee7fb106c221c1ffc9723f757361788eb91`.

The next trial must capture startup evidence immediately, verify the fresh GUI
PID and exact executable hash, and reject intervals with more than one GUI using
the live profile. The capture helper also records kernel process-creation time
when available, separately from the CUA request and internal main-entry markers.
Draft and attachment digests and the resident backend identity must remain
unchanged. Startup, prepared switching, typing and the recovered footer still
need runtime acceptance evidence from this artifact.

### Earlier backend bootstrap and measurement interruption

Two attempts to close the installed experimental GUI through CUA were rejected
because the user changed the window during control. No measurement app was
installed or launched. A quiet window was requested before retrying. The installed
GUI remained PID 37780 with executable hash `f1bf9ecb…`; backend PID 22235 was
preserved. The rejection receipt and verified rollback-copy receipt are under
`/tmp/monitter-ui-speed-20261025/runtime-profile1/`.

Source review found that the backend connection started only after native window
construction. The next source change starts connection and independent initial
preferences/state reads at main entry, overlapping native initialization. Worker
receipts are bounded, and the change subscription is established before the
initial state read and consumed after that state is imported. The UI receives the
same pending service facade: session-lock authorization, draft hydration and
authoritative workspace readiness still gate workspace commands.

Synthetic preview/profile modes retain the late path. Workspace timing and the
three passive performance-capture flags permit the normal early path. A transient
initial state failure schedules one full reset with bounded retries; exhausted
retries expose a user-initiated recovery control instead of waiting indefinitely
for a change from a quiet backend.

The isolated patch was reviewed and integrated into `main.rs` and the new
`startup_bootstrap.rs`; the baseline matched and the whitespace diff check passed.
The first peer compile stopped before its test could run because `async-channel`
was not declared as a direct desktop dependency. The manifest and lock entry now
reference the already-resolved exact version 2.5.0. Build request
`a463747d-d36d-4077-830c-d84fd86bb83f` then completed successfully under run
`4b96e2a0-c022-4897-ae10-3d5d9865caf0`: the runner and terminal both reported exit
0, and the optimized release completed in 13m11s. No owned compiler or runner
process remained when its reservation was released.

The resulting app is
`artifacts/gpui/release/build-1792951276790/Monitter GPUI.app`, with executable
SHA-256 `113826ec0fe688209bd663ec1c66cfbde31118fb7fe53ba238ccd39dbb59ee38`.
Strict deep signature verification passed, the metadata hash matched, and all
1,532 captured source/dependency fingerprints remained unchanged. Build evidence
is retained in `/tmp/monitter-ui-speed-20261025/profile2-early-bootstrap/`.
Free disk space went from 31.52 GiB before compilation to 28.9 GiB after packaging.
The 11.8 GiB warm build cache and rollback artifacts were retained; no cleanup or
automated tests were performed by this pass.

A separately signed measurement copy is staged under
`/tmp/monitter-ui-speed-20261025/runtime-profile2/launch-1/`, preserving the normal
bundle identifier and enabling bounded performance/workspace logging. Its hash
is `1c76c8bf78c307f056b3b8818d775231142bd6e96e05bf08fc5fdb6154921f8c`.
It has not been installed or launched. The quiet-window reply is still pending;
the startup, workspace, input and footer runtime requirements remain open.

### Additional observation from the unchanged installed app

While the profiling package was being prepared, the installed older executable
`f1bf9ecb…` was independently relaunched as PID 48774 at 18:59:36. This task did not
launch it. Its original PID-matched timing file and process/hash provenance were
copied read-only to
`/tmp/monitter-ui-speed-20261025/observed-old-gui-48774/` before another launch could
replace the shared file.

That log records application-ready at 1,246.5 ms, fonts-ready at 1,559.5 ms,
window-created at 1,764.7 ms, cached-workspace-frame at 3,301.5 ms, and
live-workspace-frame at 4,361.1 ms, all from main entry. This older executable lacks
the newer draft-hydration/action-paint markers. There is no external launch
receipt, so it supplies startup phase observations, not a usable-startup result
for the new build. The large variation before application-ready warrants tracing
that interval; these timestamps alone do not establish its cause.

A bounded read-only Sol audit traced the current dependency path (`gpui-kit`
0.7.0 through the patched GPUI alias into vendored GPUI Fast). Before the
application-ready callback it includes MacPlatform construction, keyboard-layout
and pasteboard queries, initial display enumeration, text-system/app creation,
SVG renderer construction, AppKit launch callbacks, and another display update.
Relevant entry points are `gpui_macos/src/platform.rs:228`, `:551`, `:1288` and
`gpui/src/app.rs:928`, `:249` under `desktop-gpui/vendor/gpui-fast/crates/`.

That source audit found no explicit synchronous subprocess or bulk font catalog
scan on the current main-thread path. The SVG font database and CoreText source
defer their expensive setup; Metal renderer creation occurs later. This is source
mapping, not a measured attribution for the older installed executable. A
controlled launch must first establish whether the interval is still material in
the new build. If it is, separate application-construction, asset setup, AppKit
launch-callback and display-enumeration timestamps or a startup profiler capture
can distinguish CPU work from waiting. Sampling an already-running process cannot
recover that earlier interval. No speculative source change was made from this
observation.

### 2026-10-26 resumed measurement window

The user reported that the earlier quiet-window question was not visible. It
was re-presented in chat and the user explicitly approved proceeding. The
bounded worker restart check subsequently returned exit 0; it did not install
or launch a GUI. Runtime measurement work resumed afterward.

The pane-recovery task had already installed the verified early-bootstrap app
(`113826ec…`) as GUI PID 8137. A read-only capture verified the installed hash
and preserved its original PID-matched startup log at
`/tmp/monitter-ui-speed-20261025/runtime-20261026/observed-installed-8137/`.
Backend PID 22235 retained its original 2026-10-25 17:12:35 process identity.
A fresh persisted-draft baseline recorded six drafts and zero attachment drafts,
using hashes and lengths without storing draft contents.

This launch records backend-worker-ready at 25.9 ms, preferences-worker-ready
at 87.1 ms, native-state-worker-ready at 127.6 ms, application-ready at 168.5 ms,
window-open-returned at 474.1 ms, backend-attached at 502.5 ms, session-lock
authorization at 519.7 ms, draft hydration at 552.4 ms, and workspace actions
ready at 553.1 ms. Cached/live frame markers occurred at 701.7 ms, while the
visible, active window action-paint marker occurred at 1,542.5 ms. All are
relative to main entry. The launch was performed by the peer, so this capture
has no external launch receipt and does not prove a sub-500-ms opening.

The peer observed a separate Git readiness error in idle panes, prepared a
focused correction, and owns its running release and GUI activation. This task
is holding GUI mutations for that handoff and reviewing the action-paint gap
read-only. It will not overwrite the final Git package with an older diagnostic
copy. Switching and typing acceptance remain unmeasured.

### Final Git release baseline and queued bootstrap drain

The peer installed final Git-readiness artifact `build-1792969972169`, hash
`e4fa46dc9cb93d4e4a448716e6789fce7895d3444a440e0253439ced4a36d417`,
as GUI PID 14175. Backend PID 22235 retained its original start identity. The
original PID-matched startup trace is retained in
`/tmp/monitter-ui-speed-20261025/runtime-20261026/observed-final-14175/`.
It records drafts hydrated at 898.2 ms, actions ready at 899.0 ms, and the visible
action-paint marker at 1,544.8 ms from main entry. There is no external launch
receipt for this peer launch; the sub-500-ms target remains unmet.

The user approved the measurement window. The two subsequent GUI quit actions
were rejected by the UI tool's user-change guard, including a retry after a
fresh accessibility read. No diagnostic installation or restart occurred. The
latest pre-measurement baseline contains seven drafts and one attachment-draft
entry; it supersedes the earlier six-draft/zero-attachment snapshot for any later
preservation check. The third pane's accessibility Git control showed main with
72 changed files; first-pane and narrow-layout visual coverage remains incomplete.

A reviewed patch now consumes at most three already-queued bootstrap receipts
after GPUI inserts the Desktop entity and before the window builder returns. It
never waits for the worker, retains a single receiver, and passes receipt flags
into the existing asynchronous completion path. Authorization, draft hydration,
and command/paint gates are unchanged. Real activation and visibility observers
record their events; a newly visible active window requests the pending action
paint. The prior hidden first draw and pre-draw callback were insufficient to
attribute the late visible-paint marker to actual user-visible delay.

The drain can synchronously perform the existing saved-window-state read when a
Preferences receipt is queued. The next release must compare root-builder and
window-open timings and the actual usable paint before accepting the change.
Source review and whitespace checks passed; compilation and runtime measurement
are still pending under request `9db6188c-eb45-4081-940e-d1ab461ac937`.

The first drain compilation was cancelled after shared `chat_watcher.rs` and
`chat_attachments.rs` changed during the build. The owned runner returned exit 1
after Ctrl-C, and no compiler/runner remained. Run
`bc869860-4bc2-404a-a5fa-c84b835a37ed` is recorded cancelled, with no accepted package.
The compatible retry request `b009e0f9-9c5d-416c-b36c-0b68a79bed77` was claimed
with the attachment task's exact matching request by that peer under
`55f9a819-9871-47aa-bfe3-cb6bebe156bb`. Only the peer runs that build. This task's
1,532-file observer snapshot was captured shortly after compilation began; it is
not represented as a pre-compilation snapshot.

Disk pressure cleanup removed six unused, superseded diagnostic app copies from
this task's temporary runtime-profile1 and runtime-profile2 directories. Source
release hashes were checked first; no running/installed copy matched the removed
paths or hashes. About 544 MiB was reclaimed as observed by free-space change.
All original releases, rollback copies, logs/provenance and warm caches remain.

The source evidence matrix is retained at
`/tmp/monitter-ui-speed-20261025/eight-source-evidence-20261026.md`. It confirms the
eight scoped implementation paths without implying runtime acceptance. Routine
saves capture dirty panes only; switching still captures all outgoing layout
panes. The borrowed-header change does not remove the separate task snapshot
clone used by other chat-render callbacks. Deferred large transcript cohorts are
explicitly distinct from fully prepared retained content.

### Combined release and measurement harness review — 2026-10-26

The successful peer claimant completed combined run
`55f9a819-9871-47aa-bfe3-cb6bebe156bb` with reported exit 0 and packaged
`build-1792971452080`. This task independently verified both bundles' strict
deep signatures and GUI SHA256
`67620528f66592e410d3daab39fa425852debd60e7737dd4099f583ab715cff9`
against the artifact metadata. The owner reports `main.rs` and
`chat_attachments.rs` unchanged before/after compilation. All 1,532 files in
this task's later observer snapshot remain unchanged; that snapshot does not
establish the full source state at compilation start. The owner completion
receipt and independent verification are retained under
`/tmp/monitter-ui-speed-20261025/profile4-bootstrap-drain-stable/`.

A measurement copy is prepared under `runtime-profile4-20261026/launch-1`
with the normal application identity and opt-in performance/workspace logs.
It has not been installed or launched. Its ad-hoc signature changes the executable
hash, recorded separately in its provenance. The installed GUI and backend
remain PID 14175 and PID 22235. Availability for the previously authorized
measurement remains pending after the two UI-control rejections.

Luna's harness review found that the runtime summarizer did not validate
`performance.json`. An aggregate profiler histogram also cannot separate typing
from workspace clicks inside the same interval. Correcting the temporary
summarizer to report independent profiler validity is in progress. A valid
typing-only interval must finish before workspace switching begins, with an
explicit action ledger. The paint endpoint remains platform submission, not
physical display scanout or OS event-queue latency. Persisted draft digests prove
only equality of on-disk metadata, not unsaved editor buffers or attachment bytes.
No new startup, switching, typing, or preservation result is claimed here.

### First accepted combined-release runtime capture — 2026-10-26

A subsequent fresh CUA attempt succeeded. The old GUI exited cleanly; this task
installed the prepared diagnostic from `build-1792971452080` and launched PID
22495 at the normal application path. Its executable hash was
`0c343692b969deffed4b303c2ec15b0c2e64066e8c45fc80fa08e1d851f207fd`.
The exact original backend PID/start identity remained unchanged. Actual CUA
wall/monotonic clocks, kernel process-creation time, PID-matched startup trace,
workspace log, action ledger and profiler output are retained under
`/tmp/monitter-ui-speed-20261025/runtime-profile4-20261026/launch-1/`.

- One launch reached action-ready paint **3673.700 ms** after the external
  launch request, including **1450 ms** before main entry; main-to-action-paint
  was **2223.700 ms**. Kernel process creation to main was 1397.334 ms. This is
  one instrumented launch, not a distribution; no cause is assigned to the
  pre-main delay. The sub-500-ms target is not met.
- Six workspace selections completed with matching request counts, PID/session
  header, monotone phases and no dropped records. The two prepared selections
  reached content paint in **46.343 and 47.056 ms**. Four unprepared selections
  took **3037.568, 298.798, 176.000 and 171.827 ms**. Two prepared samples are
  encouraging but insufficient to establish a reliable tail-latency guarantee.
- During a 120.036-second observation, 26 input-to-platform-submit samples had
  p50 **61.145 ms**, p95 **163.578 ms**, and p99/max **261.489 ms**. The controlled
  actions were one composer focus click, 12 individual insertion keys and 12
  deletion keys; the exact unsent probe appeared and was removed. This is mixed
  typing/focus input, not an exclusive typing distribution. The window was
  visible throughout and inactive for about 76 ms early in the observation.
  No mid-draw input events were excluded. Workspace selections started after
  the profiler interval ended. These input results do not establish responsive
  typing within two display frames.
- Mean draw time was **29.187 ms**, p95 **41.878 ms** across 3236 draws.
  Four chat chrome renders per draw were recorded (12944 total); there were
  586 transcript preparations, with snapshot reasons on 571. These counts guide
  the next source investigation but do not prove a particular cause.
- Fresh pre-run and post-quit persisted snapshots both contained six drafts and
  no attachment-draft records, with equal text/metadata hashes. The unsent
  probe was visibly restored to an empty composer, and the original workspace
  was selected. This proves the captured persisted state, not every unsaved
  buffer or attachment-byte scenario. Earlier seven-draft/one-attachment
  snapshots predated this run and are not used as its baseline.

During ordinary-release restoration, a peer concurrently replaced the installed
diagnostic. The hash guard refused this task's planned restore. The installed
bundle independently matched standard release SHA `67620528…cff9`, with no
measurement environment and a valid strict signature. This task did not launch
another GUI. Subsequent process observations found the peer's new standard GUI
PID 23771 and an unintended rollback-path GUI PID 23832; the standard PID later
exited while the rollback PID remained. An urgent coordination message requested
lifecycle reconciliation; further outgoing messages hit the turn limit and the
project board is disabled. Normal-release activation is therefore not yet
claimed. The accepted capture ended before this lifecycle overlap; current
post-cleanup runtime must be checked separately.

The optional nested startup timing patch remains isolated and uncompiled under
`/tmp/monitter-ui-speed-20261025/startup-nested-timing/`. It has not altered
production draw/font behavior. The overall performance goal remains active.


## Attribution diagnostics and resource checkpoint — 2026-10-26

The last goal turn made concrete progress by capturing a real release and exposing unmet startup/input targets. The subsequent bounded worker restart check returned non-PTY exit 0. This continuation does not claim either check proves the full performance goal.

### Integrated source, awaiting compilation

- `performance_chat.rs` observes one selected window during the existing finite profiling interval. It records anonymous per-pane chat render counts and child-logo render/evaluation counts, with a maximum of 64 combined entries and explicit dropped-event counters. No task text, task IDs, or window/entity IDs are exported.
- `MONITTER_GPUI_PERF_STATIC_WORKING_LOGO=1` is effective only in a profiling build and freezes only provider-logo motion. It is a diagnostic comparison control, not the proposed production solution. It affects every window in that process; a valid comparison must account for other windows, motion policy, task activity and publication differences.
- Logo counts distinguish total renders, image-present renders, policy-requested animation renders, effectively animated renders, and transformation evaluations. These are not notification/cache-miss counts and cannot alone establish invalidation causality.
- `MONITTER_STARTUP_NESTED_TIMING` records the first component theme initialization and font-catalog enumeration. The initial implicit window draw uses the existing profiler histogram only if exactly one sample is present. Unavailable spans are explicitly unobserved. Existing startup timing output carries the spans; no additional writer threads or files are introduced.
- Sol reviewed API/cfg/security ordering; Luna implemented the bounded counters; root corrected dropped owner-event attribution and integrated the small startup patch while preserving concurrent hooks. Source whitespace checks passed. No new compilation or automated tests have run. Review hashes are in `/tmp/monitter-ui-speed-20261025/profile5-attributed-startup-logo/source-review.json`.

### Capture interpretation correction

The runtime summarizer was rerun after separating profiler validity from typing-only claims. Capture `runtime-profile4-20261026/launch-1` remains valid for startup, workspace and mixed-input profiling. Its typing-only status is `not-claimed`, with no false claim that the later workspace switches overlapped typing. Raw receipts/logs were not rewritten.

### Build and installation state

Build request `0becf985-fef5-4b0c-8097-f5983a9f7b3f` retains the established profiling release command, canonical cwd and shared GPUI target key. It is planned, with source ready for compiler validation; it has not been claimed or executed. A different task owns the live MCP compilation.

Disk pressure was observed before compilation. Root removed 717.8 MiB allocated across four owned, superseded 2026-10-23 release app pairs and two unused diagnostic copies. Their build/provenance metadata remains; current release artifacts, install backups, rollback apps, source/user data and warm caches were retained. Receipt: `/tmp/monitter-ui-speed-20261025/owned-superseded-cleanup-20261026.json`. Concurrent compilation continued consuming space: free space fell from 9.66 GiB after cleanup to 5.6 GiB at the latest check, below the release build's 10 GiB reserve. The active build owner was notified; root did not touch that target or signal its processes.

Attachment-task activation reported ordinary GUI PID 26783 running verified SHA 67620528 with restored panes. Root independently observed both PID 26783 and old rollback GUI PID 23832 still alive, with backend PID 22235 unchanged. Root requested the activation owner clean up the unintended rollback instance and hand back GUI ownership; root made no UI changes during this continuation. Neither the two-instance state nor another task's launch is accepted as a new performance sample.

All latency targets and invariant checks remain subject to real-target acceptance; this checkpoint adds diagnostics, not a claim of achieved responsiveness.


## Measurement preparation and duplicate-GUI check — 2026-10-26, 01:10 local

This goal turn made concrete progress: completed the Luna/Sol acceptance preparation, reclaimed owned superseded artifacts, and obtained fresh evidence of the duplicate GUI refusing normal Quit. The bounded worker check immediately before this continuation printed `worker-restart-ok` and returned exit code 0; it was not a full lifecycle soak.

- Ready build request `0becf985-fef5-4b0c-8097-f5983a9f7b3f` still uses the recorded release/profiling command, cwd and shared target. It has no build run and has not executed. Peer run `53aaa799-c62b-452b-8b1b-0b0764e15474` is reserved and independently confirmed live as cargo PID 29097 with active rustc children. The eight reviewed diagnostic source hashes remain unchanged.
- Luna prepared `profile5-attributed-startup-logo/measurement-protocol.md` and `comparison-summary.py` under the existing temp evidence root. The read-only comparison validates all four drop counters, pane/logo count relationships, window conditions and override states. It rejects the older pre-instrumentation capture as an A/B sample. Syntax parsing passed; it has not processed new captures and no tests were run.
- Sol prepared `profile5-attributed-startup-logo/acceptance-matrix.md`, mapping all eight requirements and dirty capture, viewport preparation, drafts/attachments, lock gates and backend continuity. Missing observations remain explicit. The latest accepted launch-to-action-paint is 3673.7 ms; mixed-input p95 is 163.58 ms; only two prepared switches have 46.343/47.056 ms evidence. No typing-only claim or overall completion is justified.
- A second scoped cleanup removed 656.1 MiB allocated across five additional root-owned superseded release GUI/server pairs. Metadata, current releases, all install/Applications rollback copies, source/user data and warm caches remain. Receipt: `/tmp/monitter-ui-speed-20261025/owned-superseded-cleanup-second-20261026.json`. Actual free space rose 9.45 to 9.81 GiB during the peer compile and later measured about 9.7 GiB, still below the 10 GiB release guard. GPUI debug incremental was already empty; nothing was removed from it or the peer's active target.
- CUA independently distinguished the two existing GUI copies by their exact app paths. Rollback PID 23832 showed a blank workspace with an invalid-layout banner; installed PID 26783 showed the restored Monitter workspace. Both the rollback Quit shortcut and its application-menu Quit left it open. No reload/retry-save, process signal, new launch or installation occurred. All native-ui preference-field hashes and six persisted draft hashes were unchanged across the attempts; attachment draft count was zero. Backend PID 22235 and start identity remained unchanged. Evidence: `/tmp/monitter-ui-speed-20261025/duplicate-gui-cleanup-20261026/normal-quit-result.json`.
- The activation owner received the exact cleanup result and retains the requested lifecycle reconciliation. The distinct safe-exit follow-up is parked in Idea List entry `ca90d035-d51d-48fc-921b-205b18943464`. Two-GUI state is excluded from performance acceptance.

The goal remains active. Next release compilation requires the live reservation to finish and adequate free space; runtime comparison requires one verified GUI instance. This checkpoint does not change the original eight-item scope or latency targets.


## Compiler-cache cleanup receipt — 2026-10-26, 01:19 local

The previous goal turn made concrete progress; this continuation observed its outstanding cleanup PTY 74567 finish with exit 0. The cleanup removed exactly 2,303 regular owner-owned `.rlib`/`.rmeta` files older than 36 hours from the obsolete canonical `monitter/.cargo-target/gpui/debug/deps` cache after checking for open handles. Allocated space recovered was 3.163 GiB; actual free space rose from 8.946 to 12.110 GiB, then measured 11.912 GiB during ongoing peer compilation. Receipt: `/tmp/monitter-ui-speed-20261025/stale-canonical-gpui-compile-cache-cleanup-20261026.json`.

The current warm target at `monitter-gpui/.cargo-target/gpui`, canonical Tauri cache, test/runtime executables, metadata, user data, installed apps and rollback copies were retained. An earlier attempt to remove one historical fixed-PIN bundle stopped on an executable/metadata hash mismatch before deleting anything; that bundle remains untouched.

Root profiling request `0becf985-fef5-4b0c-8097-f5983a9f7b3f` remains ready and unclaimed. Its eight reviewed source hashes are unchanged. Peer cargo PID 29097 and rustc PID 30786 were independently observed live; reservation `53aaa799-c62b-452b-8b1b-0b0764e15474` remains owned by that task. Backend PID 22235 and current GUI PID 26783 remain alive; duplicate rollback GUI PID 23832 still prevents a valid new performance capture. No new build, test, app launch or process signal was performed by this checkpoint.


## Retained chat invalidation finding — 2026-10-26, 01:31 local

This continuation made implementation progress and obtained new actual-target diagnostic evidence. A five-second `sample` of installed GUI PID 26783 completed with exit 0; its executable still matched release SHA `67620528…cff9`. Of 258 main-thread samples, 174 were inside drawing and 125 in its nested prepaint subtree. These inclusive counts are not additive or per-function self time. The duplicate rollback GUI and peer compiler were present, so this is not a new latency benchmark. Raw sample, provenance and Luna's focused interpretation are under `profile5-attributed-startup-logo/current-gui-sample-*`.

Sol traced a specific limitation in the attempted animation isolation. WorkingLogo's 30-fps callback notifies its own view; GPUI marks that view and its ancestor path dirty. The owning ChatPane is wrapped in `.cached(...)`, whose prepaint branch rejects a dirty entity and invokes its full render. This does not show that one logo invalidates sibling panes. The existing uncached retained-view path can instead attempt to splice around changed children, subject to dependency, layout, hover and context checks. The rendering implementation work item was reopened rather than marked complete.

Luna added a disabled-by-default, profiling-only `MONITTER_GPUI_PERF_SPLICE_CHAT_PANES=1` comparison. Only ChatPane views take the existing uncached path, under a full-size wrapper, using the same retained entity; ordinary and non-profiling behavior stays cached. The JSON records `retainedChatSpliceOverrideEnabled` plus `viewRetentionEnvironmentEnabled`. Root integrated the early-bootstrap environment exception and preparation/comparison guards. Preparation rejects mixed logo/splice overrides and startup-only comparisons, and explicitly sets `GPUI_VIEW_RETENTION=1` in both cohorts. The comparator validates both flags and this environment value. Neither these fields nor lower render counts will prove splice success or layout/focus correctness by themselves.

Sol's source review found no additional blocker to compiler validation. `git diff --check` and Python syntax parsing passed; no automated tests or compilation ran. Nine current source fingerprints are recorded in `profile5-attributed-startup-logo/source-review.json`; the prior review was retained separately. Request `0becf985-fef5-4b0c-8097-f5983a9f7b3f` returned to ready with its original command/cwd/target. Its claim was refused because peer run `53aaa799-c62b-452b-8b1b-0b0764e15474` still holds the resource; no root build process was started.

The rollback GUI still displays only the invalid-layout window and refuses normal Quit. A new owner-only saved-preference backup contains six saved drafts and no attachment-draft records (`duplicate-gui-cleanup-20261026/pre-close-1792974074410`). Because the process has not acknowledged saving any possible in-memory state, a visible user confirmation request for force-quitting only PID 23832 remains pending. Current GUI 26783 and backend 22235 were not stopped. New performance acceptance requires resolving this duplicate, completing the queued build, and checking the full eight-item matrix; startup, typing and switching targets remain unmet or insufficiently evidenced.


## Source attribution and disk checkpoint — 2026-10-26, 01:38 local

The preceding performance turn made concrete progress through the retained-chat comparison candidate and an actual GUI sample. The bounded worker check separately returned non-PTY exit 0. This continuation preserved the full eight-item goal, revalidated all nine reviewed source hashes, and prepared the unchanged profiling build command in `profile5-attributed-startup-logo/build-config.json`; it has not started a build.

Disk fell to 9.75 GiB while the peer compiler was live. Five root-owned, superseded 2026-10-22 browser/responsiveness GUI/server release pairs were removed after metadata hash, owner, exact-path, inode and open-handle checks. Their allocated size totaled 640.15 MiB; actual available space measured 10.741 GiB immediately before removal and 10.908 GiB after, with concurrent compiler activity and filesystem sharing affecting the difference. All build metadata, current releases, installed apps, rollback/install copies, user data and warm caches were retained. Receipt: `/tmp/monitter-ui-speed-20261025/owned-superseded-oct22-cleanup-20261026.json`. The previously excluded mismatched historical bundle remains untouched.

A separate Sol source trace explains a remaining broad transcript invalidation path: `native_publication.rs` treats any projected global metadata inequality as changed context for every pane; `main.rs` then publishes a new shared snapshot, and visible panes lose the transcript preparation key's weak-pointer match. Existing unchanged-publication, revision-only and streaming-row fast paths are present. Profile4's 571 snapshot reason flags are nonexclusive and do not prove that those changes were irrelevant or expensive. Approvals, labels and nested collaboration content can depend on metadata, so no blanket row-equality shortcut was applied. The finding is retained in `profile5-attributed-startup-logo/transcript-snapshot-preparation-finding.md`; a conservative dependency review is ongoing.

At the latest check, peer cargo PID 29097 and rustc PID 35445 were actually running; root request `0becf985-fef5-4b0c-8097-f5983a9f7b3f` remained ready and unclaimed. Current GUI 26783, old duplicate 23832 and backend 22235 were alive with unchanged start identities. The duplicate's force-close confirmation remains pending; no new app lifecycle action, compilation, automated test or accepted timing run occurred here. Opening and typing targets remain unmet, and prepared-switch evidence remains limited to two samples.


### Transcript metadata scope correction — 2026-10-26, 01:42 local

The transitive source review identified two top-level metadata collections that the current transcript builder and generated row closures do not read: `channels` and `projects`. Luna added a borrowed, order-independent metadata comparison in `native_publication.rs` that excludes exactly those two keys for the transcript-context proof. All other and unknown keys remain conservative; non-object inputs fall back to full equality. Root short-circuited the extra traversal whenever full metadata was already equal. Whole-pane synchronization continues using full metadata equality, while reset, event, child/descendant, history and streaming guards remain intact.

Sol reviewed the implementation against settings/agent/task/approval/sharing/collaboration dependencies, attachment/OpenUI data from message rows, initial publication and unknown-key additions/removals. No source blocker was found for compilation. Scoped `git diff --check` passed. This is a production-source improvement to the scoped-publication requirement, but its benefit is unmeasured: the earlier 571 snapshot reason flags do not identify which roots changed. No tests or compilation ran.

The review now records ten source fingerprints (`native_publication.rs` SHA `27c4d68517f04acda32ccb86f8a8ba65c19eb21ace1be7ab08f03f438bdffeff`); the prior nine-file review is retained separately. The existing build request returned to ready after review. Its claim was refused by the live peer reservation. At 01:42, cargo 29097 and rustc 35445 were still running; free space had fallen again to 9.863 GiB, below the release guard. No root build started, and the duplicate-close confirmation remains unanswered.


## Idle cache cleanup and real-sidebar observation — 2026-10-26, 02:00 local

This goal continuation made concrete progress by recovering release disk headroom and obtaining a bounded installed-sidebar observation. The previous goal checkpoint supplied new GUI evidence and exposed the pending duplicate-window decision; the separate worker probe returned non-PTY exit 0 and was not a performance acceptance test.

- Luna inventoried Cargo-generated outputs in the old canonical `monitter/.cargo-target/gpui/debug` target; Sol verified that current macOS launch paths use separately packaged apps. After current queue/process/open-handle checks and peer coordination, root removed exactly 392 idle generated files (388 compiled dependencies/build scripts plus four raw Oct23 GUI/server executable copies), totaling 1.5266 GiB allocated. PTY exit 0 was observed. The old target remains a default when `CARGO_TARGET_DIR` is absent, so future unoverridden debug builds may regenerate it. Sources, lockfiles, dep-info/fingerprints, packaged apps, rollback copies and the separate warm GPUI/active Codex caches were retained. Receipt: `profile5-attributed-startup-logo/idle-canonical-debug-output-cleanup-20261026.json`.
- A second pass removed eight redundant app bundles from four old release directories after exact relative-file/mode/content tree-hash comparisons against retained counterparts and fresh open-handle/process checks. Every unique app tree, all build metadata and current/installed rollback paths remain. PTY exit 0 was observed. This pass removed 517.73 MiB allocated; actual free space rose from 9.620 to 10.034 GiB, then measured 10.009 GiB during the continuing peer compile. Receipt: `profile5-attributed-startup-logo/identical-release-dedup-cleanup-20261026.json`. Allocated bytes are not a promise of equal physical-space recovery.
- CUA opened the installed release's sidebar, selected its project view (Monitter displayed 57 chats), and scrolled to older rows. At the observed positions, labels and provider icons remained visible without blank row gaps. Root restored the activity view and collapsed sidebar; the selected workspace remained Monitter. This did not inspect all 57 rows, select a row, establish scrolling smoothness, or measure latency. Six persisted draft hashes still match the earlier backed-up state, and the attachment-draft set is empty. Observation: `profile5-attributed-startup-logo/sidebar-observation-20261026/observation.json`.
- Installed GUI PID26783 still matches SHA67620528f66592e410d3daab39fa425852debd60e7737dd4099f583ab715cff9. Backend PID22235/start identity is unchanged. Old duplicate GUI23832 remains; its force-close form was visibly present and unanswered, so no signal or lifecycle action was taken.
- Root request0becf985 remains ready with the same command/cwd/target and ten unchanged source review fingerprints. Peer cargo29097/rustc35445 were independently observed live at02:00 (compiler elapsed26m, about165%CPU), so this is a verified live reservation wait, not a stale queue inference. No root compilation, installation or new automated tests ran.

All eight requirements remain in scope. These observations strengthen basic sidebar/preservation evidence only; the <500-ms opening target, repeated prepared switching, focused typing latency, dynamic control correctness, locked branch, and new transcript/splice behavior still require release acceptance.


## Activation redraw and clean typing capture preparation — 2026-10-26, 02:13 local

This continuation makes source progress, following the prior sidebar/disk evidence and a successful bounded worker exit check. Luna corrected the startup activation observer in main.rs: when the window is active and visible with the first usable frame still pending, it now requests a redraw. This covers the ordering where visibility arrives before activation. Sol verified that notify cannot itself trigger an activation loop, and that the existing paint gate still requires hydrated drafts, no pending draft restore, a ready command facade, and an initialized/unlocked session lock. The main.rs SHA is d1a3a44359232cec1ea0f3e634b5ac9732d399791ed2983395bb7f99228f5bab. The other nine reviewed source hashes are unchanged. The earlier 136 ms activation-to-paint interval is an upper interval to investigate, not an attributed saving.

The diagnostic preparation helper now permits a 1–300 second warmup, matching the runtime clamp. Its default remains 15 seconds and capture duration remains 120 seconds. A dedicated typing capture can use 120 seconds to let editor focus setup finish first. Effective warmup is recorded in provenance, and the paired comparator rejects differing or invalid warmups. Action-time bounds must still be compared against the observed interval; an overlapping focus click or unknown input keeps the sample classified as mixed input. Sol reviewed these changes, and both Python files parsed successfully. No helper was launched against an app and no runtime behavior or speed improvement is claimed.

Root request 0becf985 was returned to planned during editing and is ready again after source review; command, cwd, and shared target are unchanged. At 02:13, peer cargo29097 and rustc35445 were observed live, with no exit receipt and 10.779 GiB free. No root compile, app lifecycle action, automated test, or additional cleanup occurred. Backend22235, current GUI26783 and old duplicate23832 remain; the specific force-close request is still unanswered. Opening, focused typing, repeated prepared switching and the remaining correctness checks are still required.


### Original-signature measurement route prepared

Local macOS open help/manual confirms per-launch --env support. The preparation helper can now copy the verified release without changing Info.plist, identifier or signature, recording a nonexecuted open -a /Applications/Monitter.app environment recipe. It omits -n and requires installed-path mode. This avoids treating the prior re-signed diagnostic as an unchanged signed release. No global launchctl environment or profile override is used. MONITTER_STARTUP_NESTED_TIMING does not match the early-bootstrap rejection prefixes, so it does not require a source allowlist change.

Sol source review caught and root corrected three evidence issues: the new launch method must read workspace-timing.log and validate its PID/session header; open exit0 must be a JSON integer rather than false; and original-signature acceptance must include the installed Info.plist hash and strict deep signature result as well as the executable hash. The revised after-capture and summary enforce these checks plus zero-before/single-after GUI snapshots, fresh process identity, exact environment recipe, clocks, readiness and backend continuity. All changed Python helpers parsed; none was executed against an app or staged a bundle. Runtime PID-matched output remains necessary to establish effective instrumentation. The prior 1,397.334 ms process-to-main interval is still unattributed and the 3,673.7 ms external opening result remains the only accepted timed baseline.

## Attribution release built and unchanged-signature copy staged — 2026-10-26, 02:39 local

The preceding full goal turn made concrete progress: release run `46d58586` exposed four unsupported `Window::window_id()` calls, and Luna corrected them to the vendored `window.window_handle().window_id()` API. Sol reviewed closure ownership and window identity. Retry request `00eedc06-3abb-49fa-9f02-d0c55919ee46`, run `802b3aad-75fc-47ac-b498-87efff0e297c`, then completed with PTY and durable runner exit 0. The optimized release took 5m20s and packaged `build-1792978251582`. Its GUI SHA256 is `0545b6ab81e1d6f6d67d30bcd3dd8988f13644176a86cbf6b08a1e72fe46a026`; metadata agrees and strict deep signature verification passed. All 1,533 captured source/dependency hashes stayed unchanged. The build reservation is released and the owned build processes stopped.

This release includes the activation redraw correction, transcript metadata scope correction, nested startup spans, per-window pane/logo counters and the disabled-by-default retained-chat comparison. Compilation proves these paths build; it does not establish a speed improvement or the outstanding correctness requirements. Disk measured 10.671 GiB before and 10.532 GiB after; the 12.4 GiB warm cache and useful artifacts were retained, with no cleanup during the build. Build evidence is in `/tmp/monitter-ui-speed-20261025/profile5-window-handle-retry-20261026/`.

The preparation helper executed successfully in preserve-signature, prepare-only mode. `/tmp/monitter-ui-speed-20261025/runtime-profile5-original-signature-20261026/launch-1/Monitter Speed Diagnostic.app` has the same executable and Info.plist hashes as the source, no bundle modifications, and a valid original signature. Its recipe uses the original `/Applications/Monitter.app` path and per-launch environment values, including 120-second profiler warmup and capture. Staging did not install or launch it. This continuation rechecked the packaged executable hash and strict signature and recovered the successful queue receipt.

Current installed GUI26783 still runs the earlier release, old duplicate rollback GUI23832 remains alive, and backend22235 retains its original start identity. Normal Quit previously failed for the duplicate and its exact force-close confirmation remains pending. No app lifecycle action was performed in this continuation. A bounded worker restart probe separately returned non-PTY exit 0; that is command-runner evidence, not a UI performance result. The full eight-item goal remains active with the earlier 3,673.7 ms opening result, limited prepared-switch samples, mixed-input result and correctness gaps unchanged.

The new `launch-original-signature.py` helper separates a read-only preflight, one guarded LaunchServices request and later receipt finalization. Sol reviewed the identity, timing and signature schema. Review exposed a missing evidence boundary: the profiler/log could arrive or change after a receipt. Finalization now requires expected outputs and PID-matched action paint, hashes raw files and the optional workload ledger, and the summarizer rejects subsequent changes. Root also retained the first observed GUI identity across polling so a timeout cannot hide replacement of an already-observed process. Python syntax parsing passed. The actual read-only preflight returned exit 2 because two GUI processes remain; no launch, installation, process signal, automated test or new performance sample occurred. See `profile5-attributed-startup-logo/guarded-launch-preflight-20261026.json` and the updated measurement protocol. This is concrete capture implementation progress; acceptance still depends on the real runtime matrix.


## 2026-10-26 10:42 — original-signature installation and live capture

User resumed and approved duplicate closure. Old duplicate PID23832 was already absent; no force-quit performed. Normal application-menu Quit succeeded for the current GUI. Installed build-1792978251582, executable SHA256 `0545b6ab81e1d6f6d67d30bcd3dd8988f13644176a86cbf6b08a1e72fe46a026`, at `/Applications/Monitter.app` with unchanged ad-hoc signature and strict deep codesign verification. Prior app retained at the installation receipt rollback path. Backend PID22235 and original start identity preserved. All six saved drafts and attachment dictionaries byte-equivalent as parsed before/after installation. No source edits, compilation or tests in this checkpoint.

Evidence: `/private/tmp/monitter-ui-speed-20261025/runtime-profile5-original-signature-20261026/launch-1`; summary in its parent. GUI PID93987. External startup sample is **invalid**: the helper subtracts monotonic values across Python processes, which this environment reports with incompatible origins (reported clock drift254064ms). Preserve original receipts; do not repair the result retrospectively. First action-paint observation timed out at20s; the marker later appeared at69809.309ms after main. Earlier live frame1302.181ms is not equivalent to active-visible actionable paint. The request-to-main wall interval1238ms is unqualified by the failed cross-process clock check.

The independent profiler observation is valid as **mixed live interaction**, not typing-only or idle-only:120.073s,202 input samples, median33.686ms,p95126.878ms,p99209.846ms,max473.432ms; active and visible throughout that profiler interval. User activity produced8 workspace requests,7superseded and1completed; zero controlled switches were planned, so workspace acceptance is invalid. CUA rejected two attempted actions after detecting concurrent user interaction; no test draft was entered. A two-minute quiet-window question is pending. No sub500ms opening, instant-switch or responsive-typing acceptance claim.

Disk12.636GiB after install; rollback and warm cache retained, no cleanup. Next necessary work is valid same-clock launch timing and controlled interaction capture; goal remains incomplete.

Future-capture clock fix completed in measurement scripts only: launch-original-signature.py SHA721899c1f24c0146c25df3cee953bc10789887e8a914632a9cc2a0f8021b4c21 uses a bounded120s same-process startup clock witness; summarize-runtime.py SHA954ed9c6230171d2e25dc2adb34a92d78d8b855ac730871dda310df492e1357e validates that witness separately from later workload finalization. AST parsing passed; no test or launch run. Legacy receipts and the invalid current startup result remain unchanged. Controlled rerun awaits quiet-window response.

## 2026-10-26 10:54 — bounded quiet-window interaction

Alex authorized two minutes of uninterrupted GUI interaction. CUA identified the installed `/Applications/Monitter.app`, and the live process retained PID 93987 and the packaged executable SHA256 `0545b6ab81e1d6f6d67d30bcd3dd8988f13644176a86cbf6b08a1e72fe46a026`. A short probe was typed into this task's composer without sending, then selected and erased; the accessibility tree confirmed the text area was empty. The `fbf` workspace was selected through its tab and the original `monitter` workspace was reselected; the accessibility tree confirmed `monitter` selected and `fbf` off at the end. No new draft or message was retained. Peer GUI inspection was released after restoration.

The PID-matched `workspace-timing.log` had new complete records through requests 26 and 27. Their content-paint observations were 136.830 ms and 317.201 ms, respectively; both records explicitly say `warmed=false`. Concurrent app and peer activity means these final records are only temporally associated with the controlled selections, not a clean per-action attribution. The coordinate click before the AX tab click did not switch workspace and is excluded. These observations do not establish immediate **prepared** workspace switching. CUA's ~17-second type/clear tool calls include accessibility and automation overhead and are not a typing input-to-frame sample. The prior 120-second profiler capture was mixed activity and ended before this probe, so responsive typing remains unverified. The current external startup number remains invalid owing to the separate-process monotonic-clock mismatch; the same-clock helper has not been run on a new launch. No new compilation, installation, backend restart or automated test occurred in this quiet window.

## 2026-10-26 11:05 — retained viewport readiness after hidden metadata

Source inspection found that every deferred hidden-chat publication made `transcript_viewport_ready` false, even when its transcript context, history, body, reset state and page boundary were unchanged. This can turn a retained workspace selection into a preparation path while unrelated metadata changes arrive. The new guard in `chat.rs` permits the retained ready path only when those transcript inputs remain unchanged and the pending snapshot matches the pane; actual history, body, reset or changed-boundary updates still require preparation. The deferred publication is still applied when the chat becomes visible, so pending controls are not discarded.

Focused regression `chat::hidden_publication_tests::hidden_metadata_keeps_retained_viewport_prepared` passed 1/1 in build run `42375db8-8198-4971-846a-4ae5e7685c57` (949 other tests filtered). It checks the metadata-only prepared state, pending history invalidation and reveal drain. Cargo and runner exited 0; checked source/Cargo hashes stayed unchanged through compilation, and no build process remained. Disk remained about 11 GiB free; the warm shared cache was retained without cleanup. This is source/test evidence only: a new release is planned but has not been packaged, installed or timed. It does not change the earlier unmet startup, prepared-switch and typing acceptance results.

## 2026-10-26 12:34 — signed release and valid live timing

The release build completed in run `ac739497-980b-45c0-8209-b819914aaff2`: PTY exit 0, optimized Cargo profile 19m31s, package `build-1793010345345`. The executable SHA256 `e90932d4e5c304d57330097ed4f2d65ce498ad0d780d59c31a8cd0831e32ff94` matches package metadata; strict deep codesign passed and five source/config fingerprints were unchanged. Owned build processes stopped. Disk was 10.9 GiB free before and 10.1 GiB after, with the 12.4 GiB warm cache retained.

An old rollback-path GUI and the installed GUI were both present before activation. The rollback GUI exited through normal Quit. The installed GUI did not respond to normal Quit or window close, so a guarded SIGTERM targeted only its verified PID 93987. Backend PID 22235 and start identity were unchanged. All six saved draft digests and the empty attachment-draft dictionary matched before shutdown, after shutdown, and after launch. The verified previous installed bundle remains at the recorded rollback path.

The unchanged signed release was installed at `/Applications/Monitter.app` and launched with its recorded per-launch profiling environment. GUI PID 7193 matched the installed executable. Same-process launch clock drift was 0.305 ms, so this startup sample is valid: 2,574.695 ms from the launch request to observed actionable paint, 2,161.188 ms from process creation, and 1,182.602 ms from Rust main entry. The opening target of 500 ms was missed. The actionable paint is a GPUI observation, not physical display scanout.

CUA selected `fbf`, restored `monitter`, and repeated that pair once, leaving the original workspace selected. The PID-matched timing log has exactly four complete records and zero dropped records. All four were marked `warmed=false`; content paint took 700.373, 215.823, 123.392 and 204.509 ms (unprepared cohort p95 700.373 ms). This does not demonstrate immediate prepared switching. The valid 120-second profiler capture included these switch actions and active task updates: four input-to-platform-submit samples, p95 66.585 ms. It is not a typing-only sample, so the typing target remains unverified. The profiler also counted 438 transcript preparations and 15,723 chat-chrome renders; 423 preparation invalidations involved snapshot identity, with overlapping row changes in 104. Those counters are diagnostic, not proof that one cause explains all switching delay.

Evidence: `/tmp/monitter-ui-speed-20261025/runtime-profile6-retained-metadata-20261026/launch-1` and its parent summary. The live app remains GUI PID 7193 on backend PID 22235. The release fixes a narrow hidden metadata invalidation but does not meet the opening goal or establish the prepared-switch and typing goals.

## 2026-10-26 12:06 — content-free readiness diagnostics packaged

The valid live capture above left four switches marked `warmed=false`. Read-only source tracing found several distinct gates: a retained view may be absent or restoring preferences; a chat may lack a prepared viewport or complete latest history; a deferred publication may alter transcript inputs; or the prepared rendering key may differ. Background preparation also deliberately defers timelines above 256 rows. The saved panes have much larger total histories, but that alone does not prove which gate rejected the four live requests because the initial warmed page and compact activity snapshot are bounded. A profiling-only log now records the failing reason and row count on a switch, without workspace, pane, task, or content identifiers. It does not change readiness or preparation behavior.

The first release attempt, run `8ffc7ede`, exited 1 at the disk preflight with 9.8 GiB free; no compilation or package started. I removed four superseded owned release bundles and three regenerable diagnostic app copies, reclaiming 614,336 KiB while retaining installed and rollback apps and the 12.4 GiB warm GPUI cache. Retry run `92404119-b731-4a68-85c1-8fc44d16d352` passed preflight at 10.1 GiB and exited 0 after 19m34s. It packaged `build-1793012705801`; GUI SHA256 `545c4dfbab8a1af1e57b643531d92ee8c9dbf68cd7dae977992ee8a23d6ead90` matches metadata, and both app bundles pass strict deep signature checks. The four diagnostic source hashes stayed unchanged and owned build processes stopped. Free disk was 15.5 GiB after the build; additional external free-space changes occurred during compilation and are not attributed to this cleanup.

The unchanged signed diagnostic copy and launch recipe are staged under `/tmp/monitter-ui-speed-20261025/runtime-profile7-readiness-reasons-20261026/launch-1`. It has **not** been installed, launched, or measured. A peer is inspecting the current GUI and preparing a compatible release for the clipped project status bar; the next GUI-only capture will wait for that handoff and can use the combined signed artifact. Backend and saved drafts have not been touched in this checkpoint. Opening remains above 500 ms, no prepared-switch result exists, and typing-only responsiveness remains unverified.

## 2026-10-26 12:17 — live readiness gate captured

After the GUI handoff, the signed diagnostic release was installed at `/Applications/Monitter.app` with its executable hash verified as `545c4dfbab8a1af1e57b643531d92ee8c9dbf68cd7dae977992ee8a23d6ead90`. The previous installed app remains in the receipt's rollback path. Normal Quit left the old GUI running, so an exact PID/start/path guard sent SIGTERM only to GUI 7193. The new GUI is PID 16558. Backend PID 22235 retained its original start identity, and all six persisted draft digests and the empty attachment-draft set matched before quit, after quit, and after launch. Strict deep signature verification passed at installation. The launch and same-process clock receipt is under `/tmp/monitter-ui-speed-20261025/runtime-profile7-readiness-reasons-20261026/launch-1`.

The initial attempted tab clicks were superseded; their timing requests 1–3 all reported `view_absent`. The next `fbf` request also reported `view_absent` and reached an **unprepared** paint after 2,253.754 ms, not a prepared-switch result. On the return toward `monitter`, request 5 found a retained view, but all four visible ChatPane instances reported `deferred_publication` with 104, 125, 125 and 128 resident rows. No paint marker for request 5 had appeared when the receipt was finalized, and the UI still showed `fbf` selected with `monitter` marked switching. This is a live stall to investigate, not a completed return switch. The row counts are below the 256-row background preparation guard; the trace does not by itself identify which deferred payload fields invalidated transcript content or why the preparatory tick did not settle.

The 120-second profiler output is complete, but the window was fully active and visible for only about 0.49 seconds of its observation interval and recorded zero input-to-platform-submit samples. It cannot support an input or typing latency claim. The occlusion also limits what this capture can establish about ordinary foreground preload scheduling. Same-process launch observation was 4,481.690 ms from the launch request to the actionable-paint observation; the startup file records the action-paint stage at 1,885.3 ms after Rust main entry. This run also misses the 500 ms opening target. The current GUI remains on `fbf` with a pending return selection, so the original workspace was **not** restored by this capture. Further GUI interaction is handed back to the status-bar work; source investigation continues without touching peer-owned `chat.rs` or `workspace_tabs.rs`.

The timestamp decomposition matters for the next capture: the external request
preceded process creation by 35.166 ms, process creation preceded Rust main entry
by 1,767.834 ms, and the action-paint marker followed main entry by 1,885.317
ms. The observing process detected the marker about 793.683 ms later.
The last interval is observation delay, not a measured application phase.
Even using the marker rather than the later observation gives about 3,688 ms
from request to action paint, still far above 500 ms. The 217.032 ms font catalog
span is nested within theme initialization, so eliminating it alone cannot
explain the full opening time. A fresh single-GUI run of the direct-font release
must separately measure request-to-main, main-to-marker and marker-to-observation
before attributing any improvement.

### Retained preload scheduler candidate

Startup now gives the active scene its first frame before beginning ancillary
history and workspace preparation, with a one-shot 100 ms timer if an occluded
window does not deliver that callback. Retained workspace preparation similarly
uses a one-shot 32 ms timer for each bounded tick when its next frame is delayed.
The timer and frame callbacks share a completion flag, so each scheduled step
runs at most once. This targets the observed `view_absent` and incomplete return
switch without broadening the amount of work in a single tick. The fallback
compiled as part of a peer-owned GPUI test build; that test failed later in its
synthetic geometry fixture. A compatible shared release build then completed
under reservation `a350dc4c-d70c-46d2-821f-e9a1a7520434` with observed exit
0. It packaged `build-1793015442208` with GUI SHA-256
`c49f5ff40498b5cf6ad45ed22d024b1b2ff5f2a241c0d1414b7663dafd939652`;
the hash matches `build.json`, and strict deep signing passed for both bundles.
The build owner verified unchanged hashes for `main.rs`, `workspace_windows.rs`
and its `chat.rs` fix during compilation. `workspace_tabs.rs` changed during
the build and was not part of that three-file verification, so this package
is not evidence for the final concurrent tab edit. Free disk went from about
14 GiB to 13.6 GiB; the warm shared cache was retained without cleanup. The release was
not installed or launched by this build. A live foreground/occluded comparison
is still required before attributing or accepting a speed change.

The subsequent status-bar geometry regression passed 1/1, but its first two
combined release attempts exited with Cargo source errors in concurrently edited
composer picker/footer files; neither staged a new app. The picker owner is
correcting those errors. The signed `build-1793015442208` remains a package
candidate only: the installed executable still hashes to
`545c4dfbab8a1af1e57b643531d92ee8c9dbf68cd7dae977992ee8a23d6ead90`,
and backend PID 22235 was unchanged at the last read-only check. No timer
fallback runtime measurement or typing-only acceptance has been claimed.

### Direct macOS font lookup candidate

The last PID-matched startup trace spent 217.0315 ms enumerating installed
fonts during the 229.2173 ms component theme initialization. The GPUI text
system now exposes a direct named-family load check; the macOS theme checks
its preferred monospace family and alternates through that path and resolves
the virtual system UI family without enumerating the catalog. Other platforms
retain the existing catalog behavior. Release run
`bf51c810-20aa-4526-85b9-8630ba2d102a` exited 0 after 21m01s and packaged
`build-1793019270552`; GUI SHA-256
`0ff14b3698ea3df83007490c1a0e45f9cc2d3a448f72b5273850f1448bd30142`
matches metadata. Both app bundles passed strict deep signature verification,
and all 1,382 captured desktop Rust/build inputs stayed unchanged. Disk was
46.6 GiB free before and 47.5 GiB after; the 12.5 GiB warm cache was retained
without cleanup. No automated test, installation or live timing has established
the actual startup saving or the sub-500-ms target. The installed app and
backend were not changed by this build.

The signed font candidate was rechecked against its release metadata and a
strict deep signature. An unchanged-signature measurement copy and per-launch
environment recipe are prepared under
`/tmp/monitter-ui-speed-20261025/font-probe-20261026/launch-1/`; no GUI was
installed or launched during preparation. The recipe records a 120-second
profiler warmup so workspace setup can finish before a separate focused typing
interval. At preparation time, the status-bar owner was still activating the
main app, and two GUI processes remained alongside the unchanged backend. This
state does not satisfy the single-GUI launch preflight. The measurement copy
does not establish runtime behavior until the GUI handoff and a fresh launch
receipt are complete.

### Direct-font release installed and measured

After the status-bar handoff, both earlier GUIs quit normally; all six saved
draft digests and the empty attachment-draft set stayed unchanged, and backend
PID 22235 kept its original start identity. The first prepared font-probe
launch rejected its preflight because it had recorded two GUIs. A fresh
single-GUI capture under
`/tmp/monitter-ui-speed-20261025/font-probe-20261026/launch-2/`
passed the preflight. The signed `build-1793019270552` was installed at
`/Applications/Monitter.app` with GUI hash `0ff14b3698ea3df83007490c1a0e45f9cc2d3a448f72b5273850f1448bd30142`;
the previous c892 build remains in the installation receipt's rollback path.
The one requested launch produced GUI PID 37988 and preserved backend PID
22235. The six draft digests matched again after launch.

The launch observer measured **2,976.694 ms** from request to action-paint
observation. The action-paint marker was **1,663.906 ms after Rust main**;
request to process creation was 32.342 ms and process creation to main was
990.658 ms. The observer detected the marker about 289.094 ms later. Component
theme initialization took 10.362 ms, versus 229.217 ms in the earlier single
capture. The font catalog span was absent on the direct path. These are
single-run observations with different pre-main and observer delays, so they
show the theme improvement but do not establish a total startup saving. The
500 ms opening target still fails.

The first selection of `fbf` reported `view_absent`, shell-ready at 348.266 ms,
and content-paint at **37,437.575 ms**. The attempted return to `monitter`
reported `deferred_publication` on four retained panes (105–126 rows) and
shell-ready at 34.990 ms, but still had **no shell or content paint** after
several minutes. The UI showed `fbf` selected and `monitter` switching. This
is a stalled selection, not a fast retained switch. The 120-second profiler
reported 0 active and 0 visible seconds, zero input samples, and four initial
transcript preparations; it cannot support a typing-latency claim. This
profile is not a valid foreground-input benchmark.

Read-only source tracing found that an explicitly selected retained workspace
could wait for every visible pane's background history warmup before mounting
its already restored native shell. A narrow scheduler change now marks that
selection's transcript preload deferred and leaves content readiness to the
visible panes, while retaining the existing workspace-selection and restore
gates. Compilation and a fresh signed runtime check are still required before
claiming this resolves the live stall. The installed GUI remains the direct-font
release; the scheduler change has not been installed.

The scheduler change compiled with `visual-export,performance-profiling` in
43.50 seconds (build queue run `97045220`; exit 0). Disk stayed at 49 GiB
free and the warm cache was retained. The first signed release attempt,
run `1b5a6165`, was cancelled when `composer_popup.rs` changed during
compilation in a concurrent peer task. The build runner recorded child exit
-2 after Ctrl-C, its PTY exited 254, and no matching build processes remained.
No package from that mixed source window is accepted or installed. Disk
remained at 49 GiB free with the warm cache and rollback artifacts retained.

A later second selection of the same retained `monitter` workspace superseded
the stalled request after 396,986.169 ms. That new request painted its shell
and content at **149.089 ms** (`warmed=false`). The restored UI is visible, but
the original selection still failed to complete on its own, so this retry
does not satisfy immediate first-attempt switching. It is consistent with
background history having settled by the retry; the trace does not prove the
exact history-completion time or whether another state change also mattered.

The next release retry, run `f2fc4bed-c9cf-465c-a030-83b0eca0fc4b`,
completed with exit 0 after 7m28s and packaged
`build-1793021854479`. GUI SHA-256
`605cdc1d2f8fee469573968ee052f59826626a591d05fcbddd712176b49ca1d4`
matches release metadata, both bundles pass strict deep codesign, and all
1,533 captured source/build-input hashes remained unchanged. Owned build
processes stopped. Free disk was about 49 GiB before and 49.3 GiB after;
the 12.7 GiB warm cache and rollback artifacts were retained. This proves
packaging, not runtime improvement. A signed measurement copy with a 30-second
profiler warmup is staged under
`/tmp/monitter-ui-speed-20261025/retained-history-shell-release-retry-20261026/launch-1/`.
The app bundle at `/Applications/Monitter.app` acquired this hash during
concurrent GUI work. This speed task did not perform the installation, so the
installer's identity and rollback receipt are not attributed here. The older
GUI PID 37988 subsequently exited and GUI PID 42680 started from the installed
path; the precise lifecycle trigger is unconfirmed. The backend remains PID
22235 with its original October 25 start identity. A read-only preference
capture before and after the restart found the same six draft keys and SHA-256
digests, and the empty attachment-draft set remained empty.

The restarted GUI's own `startup-timing.json` records action-paint observation
at **1,821.351 ms after Rust main**. Its `spans` array is empty, and this launch
did not use the staged diagnostic environment, so it supplies no end-to-end
request timing, foreground input samples, or workspace-selection trace. The
sub-500-ms opening target is still unmet. The new scheduler's first-selection
behavior remains unverified on the installed binary.

The new launch's milestone gaps narrow the next startup investigation. GPUI
application setup consumed 413.675 ms from Rust main; hidden window opening
returned at 938.543 ms, 469.727 ms after window options were ready. The
session lock then reported authorization at 1,049.441 ms; cached workspace
preparation finished at 1,092.519 ms, and the cached frame at 1,323.319 ms.
Input readiness was marked at 1,367.811 ms, but window visibility was not
observed until 1,744.714 ms, followed by action paint at 1,821.351 ms.
These are intervals on one launch, not exclusive CPU costs. The current source
opens a hidden window, waits for lock authority before preparing cached panes,
draws its initial scene, then activates the window. `session_lock.rs` sets a
fail-closed global state before asynchronously reading the PIN database. The
authorization gate must remain intact. A nested-timing launch is needed to
attribute the GPUI window-open interval and the visibility delay before making
another startup change; the current launch has no nested spans.

Source inspection rules out simply deleting the first hidden draw as a safe
speed change. `gpui_kit::open_window` calls GPUI's `App::open_window`, whose
vendored implementation deliberately draws once before returning because
otherwise a window can be returned with an empty dispatch tree and crash.
The prior direct-font detailed capture measured that implicit draw at
129.106 ms; it is only part of the larger window-open interval. In that
capture application readiness was 189.557 ms, window-open return 407.229 ms,
and window visibility 649.050 ms after main, markedly earlier than the new
single launch. The gap varies across launches, so the new launch's intervals
cannot yet be attributed to an intrinsic fixed cost in one source call.

### Controlled installed-release diagnostic, 2026-10-26

One GUI-only restart of `/Applications/Monitter.app` launched the signed
executable with SHA-256 `605cdc1d2f8fee469573968ee052f59826626a591d05fcbddd712176b49ca1d4`.
The existing backend retained PID 22235 and its October 25 start identity;
the new GUI was PID 45008. The independent launch-clock observation found
action paint **1,089.675 ms** after the request. The process startup trace
recorded the same action-paint stage **774.595 ms after Rust main**. The latter
is an internal stage offset, not the launch-to-paint result. Window-open return
was 381.458 ms and observed visibility was 703.624 ms after main. This single
launch misses the sub-500-ms opening target.

Four workspace selections reached first-request content-paint records at
193.861, 129.840, 87.396 and 137.554 ms. The second request returned to
`monitter` after a first visit elsewhere and painted without the earlier
second-attempt recovery. All four requests were logged `warmed=false`, and
the log also recorded `view_absent` or `deferred_publication` readiness.
`summarize-runtime.py` therefore marks the workspace sample **invalid** with
reason `workspace diagnostic error present`. These raw timings show that this
one run did not reproduce the stall; they do not establish a prepared-switch
latency distribution or satisfy that acceptance target.

The 120.031-second active, visible profiler capture included navigation and
temporary typing. It recorded 33 input frames with p95 **78.709 ms** and max
98.107 ms from input to platform submission; zero mid-draw input events were
excluded. The temporary composer text was cleared without sending. This is a
valid mixed-workload profiler sample, not a typing-only benchmark or a
streaming-workload result. Six saved draft digests and the empty attachment set
matched before and after the restart and temporary typing. The GUI and backend
remained running after capture.

The immutable evidence and receipt are in
`/tmp/monitter-ui-speed-20261025/retained-history-shell-release-retry-20261026/launch-1/`.
An initial manual after-capture was archived as `*-initial.json` before the
receipt finalizer made its guarded fresh capture; `evidence-reconciliation.json`
records the original file hashes. The formal startup and profiler samples are
valid, while the workspace sample is invalid for the readiness reason above.

### Single-transaction startup lock candidate

Source now reads the lock snapshot and verifier configuration in one SQLite
transaction during `SessionLock::load_storage`. The initial state remains locked
and uninitialized; a failed read still cannot reveal cached panes. This removes
one database setup and commit from the authorization path, which measured about
77 ms between window-open return and authorization in the controlled launch.
That interval is only an upper bound on any saving from this change.

Build queue run `470a10e7-56e1-4ee6-9630-110ba3774aa9` finished exit 0 in
6m31s and packaged `build-1793023743796`. The GUI executable SHA-256 is
`0758c803d93cb2b59647218176fdc88f4f81c470030b6cf01e49921d76778ccf`,
matching `build.json`; strict deep codesign passed. Disk space was 49.2 GiB
before and 49.9 GiB after; the 12.7 GiB warm build cache was retained. This
candidate has not been installed or measured. The current installed GUI and
backend were left running. The previous 1,089.675 ms startup result must not
be attributed to this candidate.

### Signed release and live measurement, 2026-10-26

The hidden-publication retained-viewport requeue and the single-transaction
session-lock change were packaged together in build queue run
`5b94a47c-3693-47ac-a9d2-ea9db0c451dc` (exit 0, 8m56s). The signed release is
`build-1793024782680`, GUI SHA-256
`7ca50bc60be5eb0e05cff51077250018fd67df534772b55bb870506ddc3babde`.
The GUI and server passed strict deep codesign verification; all 1,640 captured
source and build inputs stayed unchanged during compilation. Disk space was
49.8 GiB before and 49.6 GiB after the build. The 12.7 GiB warm cache was
retained without cleanup. A subsequent comparison found no changed or new
source inputs, so this remains the build of the current source.

The installed `/Applications/Monitter.app` executable matches that SHA-256 and
launched as GUI PID 50579. The existing backend kept PID 22235 and its October
25 start identity. Six saved draft digests and the empty attachment set matched
before and after the GUI restart and temporary typing; the temporary text was
cleared without sending. The signed app displayed the existing workspaces and
chats. A separate old rollback copy was briefly opened by ambiguous app-name
selection and showed an invalid-layout warning; that copy was closed. The
warning was not from this installed release.

The independent launch-clock receipt measured **2,221.562 ms** from launch
request to action paint, with 0.438 ms clock drift. The startup trace measured
1,007.144 ms from Rust main to action paint; the roughly 1,003 ms preceding
main is included in the launch-clock figure. This single release measurement
is slower than the preceding installed result of 1,089.675 ms and misses the
sub-500-ms target. It does not support a claim that the lock change improved
launch time.

Six workspace-switch requests logged content paints at 2,214.322, 2,677.277,
103.188, 166.263, 65.760, and 41.113 ms. The first two had absent views;
the fourth had deferred publication. An extra request and those readiness
diagnostics make the formal workspace sample invalid. The later fast paints
are observations, not a verified prepared-switch distribution. The 120.035 s
profiler window recorded five input-to-platform-submit samples with p95
65.765 ms, but no fully active visible interval; temporary typing occurred
after that window. Typing-only responsiveness remains unverified.

The immutable build and launch evidence is under
`/tmp/monitter-ui-speed-20261025/hidden-viewport-requeue-release-20261026/`.
The launch `receipt.json` and runtime summary distinguish valid startup timing
from the invalid workspace sample. No further build was started because the
source still matches this signed release.
