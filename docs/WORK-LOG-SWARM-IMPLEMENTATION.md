# Work Logs: swarm implementation plan

Status: ready for implementation review, not implementation authorization.
Prepared 15 October 2026. Coordinator: Codex. Preferred delegates: MiniMax and
OpenCode; Gemini may join after its route is configured and verified.

This is the execution companion to [the Work Log design](AGENT-WORK-LOG-PLAN.md).
Work Logs are an independent underlying backend feature. They must work without
project sharing, a relay, GitHub issues, hooks, build queues, an open GUI, or an
agent remembering to write a summary. Those features can contribute references
later. They are not prerequisites for collection.

## Outcome and delivery boundary

Persist what Monitter's agents actually did, when, for how long, with which
harnesses/models, and at what observed token, cost, CPU and RAM usage. Add short
agent-declared descriptions while preserving the difference between assertions
and measurements. Use a lightweight private SQLite store with bounded collection.

The underlying first release includes:

- Durable run lifecycle and historical identity snapshots.
- Stable links back to the originating chat/context, with scoped lookup tools
  so future agents can recover the supporting conversation.
- Existing work-plan transitions and an optional task-scoped `work_log_report`.
- Projection of canonical provider usage with provenance and missing coverage.
- Sampled local macOS CPU/RAM history, with explicit gaps and attribution limits.
- Retention, compaction, health, indexed owner-only queries and local JSON/CSV export.

A rich dashboard, remote/GPU collectors, pricing downloads, provider comparisons,
relay replication, issue connectors and an arbitrary-SQL endpoint are outside
this implementation. A small GPUI viewer is an optional follow-on; its absence
must not stop logging. No periodic LLM or Jev calls are needed for collection.

Ship behind a collection flag for initial rollout. The coordinator must freeze
its exact default, pause/resume semantics and retention policy before workers
change persisted settings. Never infer old working time from chat age.

## Coordinator and delegate routes

Codex owns architecture, contracts, shared-file integration, build coordination,
cross-module tests, review and the final implementation report. Delegates return
bounded patches and evidence; they do not merge or activate the live backend.

Directory discovery during this planning session found:

| Route | Configured default | Use in this plan |
| --- | --- | --- |
| MiniMax, agent `ef0b8027-8ccd-4c12-9478-f56332d0bb12` | `m:minimax:MiniMax-M3:v:` through ACP | Storage/writer implementation; persistence review. |
| OpenCode, agent `b4963375-f33c-417a-98a4-b064f1819e6b` | `opencode-go/deepseek-v4-flash` through ACP | Bounded pure transforms, resource attribution and deterministic fixtures. |
| Gemini | No configured directory entry found | Optional independent reviewer or query/export worker after onboarding. |

The MiniMax read-only review completed successfully during planning. OpenCode's
review failed with an ACP option-advertisement error; its subsequent model lookup
timed out. Treat OpenCode assignments as blocked until a valid advertised
configuration and working turn are verified, without silently changing its saved
profile. Gemini remains unconfigured. If no additional route is ready, Codex can
sequence bounded packages through the verified MiniMax route rather than launch
a nominal multi-provider swarm that cannot execute.

Recheck availability and access to the assigned worktree before execution. Start
with a bounded read-only task; do not install harnesses, change credentials,
download models, buy credits or silently substitute a provider. Use a saved
default or an exact advertised model ID. OpenCode is the harness; its model and
provider are separate choices. These assignments do not claim a price ranking.

Default to two implementation delegates plus Codex. A third delegate is useful
only for an independent package or review, and only within the actual concurrency
limit. Reuse agent sessions for related corrections. If a route fails, stop that
assignment, record the failure and reassign deliberately; do not replay uncertain
write operations blindly.

## Delegate permissions: mandatory launch gate

Alex explicitly requires MiniMax to have full permissions for its assignment or
for Codex to handle its permission requests. Apply the same readiness gate to
other implementation delegates. A configured profile or successful text reply
does not establish filesystem, shell or test access.

Before assigning implementation, Codex records the task's actual harness mode,
worktree/cwd, reachable tools and approval policy. Use one supported approach:

- **Full task access:** select a harness-advertised mode that permits the owned
  worktree and required commands for the assignment. Verify the effective mode
  through the actual session; do not assume a flag or mode name works across ACP
  harnesses, or silently modify the operator's global configuration.
- **Coordinator-managed approvals:** retain explicit approvals only when Codex
  can observe and resolve them through an available owner approval surface.
  Keep an approval watcher active alongside each worker. Resolve exact request
  and option IDs for already-authorized actions; preserve existing one-time
  decision, expiry, cancellation and scope checks. Do not leave a worker waiting
  unnoticed or expect it to approve its own requests.

Run a disposable preflight inside the assigned worktree before real edits:
read the frozen contract, create/read/edit/remove a uniquely owned scratch file,
and execute a harmless local shell command. Report observed success and any
approval IDs/decisions without exposing credentials. Compilation remains under
the build-claim protocol; permission to execute is not ownership of the cache.

If either access or the coordinator's approval channel is unavailable, the route
is blocked before implementation starts. Correct its supported task configuration
or use an explicitly selected verified delegate. Do not use "read-only" as an
implementation sandbox and discover halfway through that it cannot save work.

Full task access covers Alex's authorized source work. It does not permit
purchases, new paid commitments, live backend restarts, unrelated user-data edits
or permission changes on another operator's machine. Workers remain bound by
file ownership and the project's build reservation rules.

The MiniMax planning review completed as a read-only task. Write/shell access for
future implementation is not yet verified; this gate must produce its own
receipt before W1 starts.

## Worktrees and ownership

The existing planning worktree is based on an older mobile checkout. Keep this
document here, but start implementation from the current GPUI checkout at
`/Users/alex/code/monitter-gpui`, after inspecting its dirty state and active peers.
Never copy this planning checkout over the desktop source.

Create one integration worktree and separate worker worktrees from a named,
reviewed source snapshot. Preserve the canonical index, uncommitted peer files,
sessions, profiles and rollback copies. Where the source snapshot contains peer
work, record its baseline separately so worker patches contain only Work Logs.

Proposed implementation namespace: `src-tauri/src/work_log/`. The coordinator
freezes names and interfaces in package C0 before dispatch. Modules below are
proposed files, not claims that they already exist.

| Package | Owner | Owned new files | Shared files: coordinator only |
| --- | --- | --- | --- |
| C0 contract and integration | Codex | `work_log/mod.rs`, `types.rs`, integration fixtures | `lib.rs`, `model.rs`, `store.rs`, `sqlite_backend.rs`, lifecycle wiring. |
| W1 durable storage/writer | MiniMax | `schema.rs`, `store.rs`, `writer.rs`, storage fixtures | Provide migration/outbox requirements; Codex applies them. |
| W2 usage projection | OpenCode | `usage.rs`, usage fixtures | Provide changes needed in canonical usage ingestion/deletion. |
| W3 resource attribution | OpenCode after W2, or verified Gemini | `resources.rs`, `sampler.rs`, resource fixtures | Codex wires process roots and read-only inspection reuse. |
| W4 semantic events/MCP | MiniMax after W1 | `reports.rs`, report fixtures | Codex owns collaboration dispatch, grants, tool schemas and guidance. |
| W5 retention and reporting | MiniMax or verified Gemini | `retention.rs`, `query.rs`, `export.rs`, query fixtures | Codex owns command registration and public contract generation. |
| R1 independent acceptance review | Different available delegate from implementer | Review findings and standalone acceptance fixtures | No production edits without a new bounded assignment. |

Workers must not edit shared files "to make it compile." They return a precise
integration note or proposed diff for the coordinator. File ownership is
exclusive while a package is active, including tests. Disjoint files are not
permission to change a shared schema or API without updating the contract first.

## C0: freeze contracts before parallel implementation

Codex reads current `AGENTS.md`, `docs/CONTRACT.md`, `src/lib/types.ts`, usage
normalization, process inspection, state commit/recovery and run ownership.
Confirmed starting points include `capture_usage`, `mark_usage_final`, `finish`,
`Store::stage_usage`, `Store::remove_task_usage`, and `process_metrics::sample`.
Staging usage is currently in memory; it is not a durable outbox acknowledgement.
Current process samples include command lines: the new persisted DTO must not.

Publish a small contract commit containing typed envelopes, fake clock/process
sources, test builders and interfaces. Required decisions:

1. **Identity.** Every actual execution has its real `runId`; task/chat, agent,
   project, host, backend session and optional parent run are distinct. Snapshot
   display labels and configured/observed model identities. Define behavior for
   events arriving before run registration and after terminal outcomes.
   Chat-originated runs/events also carry the stable context reference below;
   a mutable title or provider session ID alone is insufficient.
2. **Events.** Assign a stable source key and committed sequence at the authority.
   Include occurred/observed UTC milliseconds, schema version, source, coverage,
   event kind and bounded optional phase/summary. Suggested event kinds include
   run start/end, wait/resume, plan transitions, explicit report and collection gap.
3. **Durable acceptance.** State changes and low-frequency Work Log outbox rows
   commit in one primary-store transaction. A report succeeds only after this
   commit. No main/work-log cross-database atomic transaction is assumed.
4. **Usage.** Specify per-adapter scope, counter epoch, delta/cumulative semantics,
   ordering and precision. Unknown values stay null. A final marker may contain
   no new totals. Add a stable committed usage cursor; mutable row positions are
   not a safe append sequence.
   Keep the primary committed source sequence distinct from the consumer's
   projection watermark. Commit work-log observations, totals and the consumer
   watermark in the same work-log transaction before acknowledging/pruning the
   corresponding source/outbox records. Never advance a primary-side cursor
   ahead of the destination commit or require an atomic cross-database commit.
5. **Resources.** Accept only sanitised numeric process observations with
   `(hostId,pid,processStartedAt)`, verified run roots and actual covered interval.
   Commands, environments and transcript text cannot enter resource rows.
6. **Queries.** Define bounded filters, stable keyset cursors, coverage fields and
   typed aggregate/export DTOs. Owner-local reporting and explicitly granted
   task-scoped log/context reads use separate authorization paths; task-scoped
   report writes are a further distinct grant. No general LAN/visitor exposure.
7. **Health and failure.** Define healthy/paused/degraded states, queue/outbox/WAL
   limits, last successful write, drop counters and recovery markers. Logging
   failure must not stop normal agent execution or be presented as success.
   This applies to collector-specific failures; real primary-store commit errors
   retain the existing poisoned-store/reopen behavior, not a new bypass.

The writer interface must distinguish durable lifecycle/report delivery from
loss-tolerant resource samples. Provide bounded enqueue, health, flush/shutdown
and stop methods. Do not silently turn durable events into best-effort samples.

Suggested test seams: `Clock`, `ProcessSource`, fake usage source and a faultable
SQLite writer. Use real temporary SQLite databases for transaction/reopen tests;
mocking a successful write is not durability proof.

### Required chat-session provenance and future-agent context access

Alex requires Work Logs to reference the chat session they came from so a future
agent can find the full supporting context. This is part of the underlying
feature, not a dashboard or sharing dependency.

Add `work_context_refs` in the Work Log database. Each chat-originated run/event
links to one opaque UUID `contextRef`. Its record snapshots:

- The originating local Monitter installation/profile identity and project ID.
- Stable Monitter chat/task ID, run ID and optional provider-turn ID.
- The context segment's start marker and source message/event anchor, with an
  end/through anchor when available. Keep immutable IDs, not just timestamps.
- The source title/agent/harness labels as historical display metadata.
- An owner-private native-session locator captured at the time, when available;
  never expose provider transcript paths, credentials or resume authority.
- Context availability and reason: available, deleted, not-recorded, unsupported
  or temporarily unavailable. Re-evaluate access and availability when resolving.

Reuse existing message history. Current `clear_task_context` retains the same
chat ID and prior messages, adds a context-cleared system message, and resets
the native session ID. Treat that durable marker as a segment boundary, and
snapshot anchors at event creation; do not resolve an old log against whichever
provider session happens to be current later. Verify `/new`, fork, resume and
handoff paths individually and persist their source/derived links rather than
assuming they all share `/clear` semantics. Backend restarts must not invalidate
these references. References resolve by authority, never by title searches.

Codex owns source-reference creation and history/clear/delete integration. W1
owns reference persistence, W4 the authenticated tool boundary, and W5 lookup
queries. Freeze these fields in C0 so workers cannot omit provenance as a later
enhancement. Logs without a chat source, or internal turns whose transcript was
intentionally not recorded, keep explicit unavailable provenance instead of
inventing context.

Provide two read-only, bounded agent operations alongside owner-local queries:

1. `work_log_query`: searches authorized log history and returns event/run IDs,
   coverage and `contextRef` values.
2. `work_log_read_context`: accepts an authorized stored `contextRef` plus a
   pagination cursor, resolves it to the canonical chat-history reader, and
   returns the relevant source segment with source/through anchors. Permit
   further full-chat pages only under the same explicit history permissions.

These are proposed names, not existing commands. Backend identity supplies the
caller; owner-granted project/lineage access controls both steps at execution
time. Possessing a reference grants no access by itself. Recheck on every page,
including after asynchronous reads, and return a safe unavailable/denied result
if access changes. Do not expose saved system instructions, owner credentials,
native transcript files or unrelated chats through this resolver. Canonical
tool/detail readers retain their existing permissions and redaction rules.

Do not duplicate full chat transcripts into the Work Log database or pin all chat
history in memory. Use bounded on-disk history pages. If the user permanently
deletes a chat or provider history is missing, preserve the historical log and
mark its context unavailable; never silently restore or retain deleted content.
Archiving and renaming should preserve resolution. A future explicit history
retention policy may keep context longer, but reference storage alone does not
promise permanent transcript availability.

Add acceptance fixtures for rename/archive/restart stability, `/clear` and
`/new` segmentation, fork/handoff lineage, references created before a native
session ID exists, deletion tombstones, missing native files with retained
Monitter history, and permission revocation between pagination requests. An
end-to-end test must let a fresh authorized agent query an old log, resolve its
reference and retrieve the actual source conversation without title matching.

## Dependency waves and acceptance gates

### Wave 1 — a durable vertical slice

Dispatch W1 to MiniMax. While it works, Codex implements the primary-store outbox
transaction seam and lifecycle adapter. OpenCode can prepare pure usage transforms
and fixture cases against C0, without depending on an unfinished writer.

W1 builds the dedicated `work-log.sqlite3`, additive migrations, private file/WAL
permissions, single writer, idempotent outbox drain and health. The writer inserts
the source key, commits, then acknowledges the primary outbox. Never acknowledge
before commit. Keep hot state locks out of work-log I/O.

Use WAL with `synchronous=FULL` on the work-log writer before acknowledging
durable outbox delivery. This tightens the original design's proposed `NORMAL`:
acknowledging/removing an outbox record after a destination commit that can be
lost on power failure would lose the only durable copy. Batch writes to measure
and control sync cost; resource samples may still be dropped before commit with
coverage gaps. Preserve the primary store's existing durability and macOS sync
policy. See [SQLite's synchronous documentation](https://sqlite.org/pragma.html#pragma_synchronous).

Gate G1: start and finish a fixture run, reopen both databases, and recover the
same run/events exactly once. Crash before each commit, after work-log commit but
before outbox acknowledgement, and during recovery. A terminal run cannot
silently revert to running. Explicit reports under outbox exhaustion reject
visibly; ordinary agent work can continue with degraded collection.

Use deterministic fault injection or an owned child-process crash harness, never
kill a test thread or the live backend. Test configured byte thresholds with
small temporary databases; inject full-disk I/O errors instead of filling the
developer's disk. At the hard limit, bounded durable outbox retention and visible
degradation replace a promise to accept semantic history forever without space.

Do not dispatch dependent storage integrations until G1 passes. A first isolated
local runtime smoke may follow; it must use a temporary profile, not the live app.

### Wave 2 — independent accounting lanes

Run W2 and W3 in parallel only after their C0 contracts are stable; let W2 finish
first if only one OpenCode route is available. Codex owns wiring and deletion
transactions. No worker writes another lane's tables directly.

W2 projects committed usage with idempotent cursors and provider-specific
normalisation. Preserve usage and historical identities in the primary commit
before deleting tasks/agents or staged observations. Rates are not fetched;
reported monetary values retain provenance/precision, with explicit units.

W3 reuses one backend-owned process-tree inspection at a proposed 10-second
cadence while runs exist. Assign each child to the nearest verified owned root;
shared helpers remain shared/unattributed. Track individual process counters
across tree membership changes, PID reuse and disappearance. SSH client usage is
local transport overhead; remote workload CPU/RAM stays unavailable.

Gate G2: cumulative and delta usage fixtures, duplicate/reordered samples, empty
final markers, model/reset epochs, real retry identity and deletion/restart races
all preserve totals. Resource fixtures cover overlapping roots, PID reuse,
short-lived children, counter resets, sleep/restart gaps and unsupported hosts.
No negative CPU, duplicate attribution, invented remote samples or "unknown=0."

### Wave 3 — descriptions, retention and owner reporting

W4 can begin after G1. Reuse work-plan transitions automatically; add bounded
`work_log_report` only for additional semantic checkpoints. The backend supplies
identity/time, deduplicates caller plus request ID, rejects changed retries and
applies explicit tool/rate limits. Agent text cannot alter measured usage or
outcomes. Summaries are untrusted text at every rendering/model boundary.

W5 begins once W1/W2/W3 table contracts are frozen. Implement restart-safe
resource compaction, retention watermarks, owner-only read queries and local
JSON/CSV export. Query either raw detail or the authoritative bucket coverage,
never both for the same interval. Preserve CPU sums, covered duration, RAM
byte-milliseconds and sampled maxima across compaction.

Gate G3: task deletion does not erase history; explicit history deletion is
separate. Parallel agent-hours and project interval unions differ correctly.
Timezone/DST boundaries, missing costs, currency boundaries, interrupted runs,
raw/bucket overlap, cancelled exports and query deadlines have deterministic
fixtures. Shared visitors/LAN callers cannot read the private store. Export rows
disclose available resolution and missing coverage; CSV handles spreadsheet
formula prefixes safely. No arbitrary SQL is exposed.

### Wave 4 — integration, performance and independent review

Codex integrates reviewed patches in dependency order. Give R1 a fixed commit and
explicit invariants, preferably through a different provider than the relevant
implementation. Review storage crash recovery and usage accounting separately
from the resource sampler. Address findings through the original owner before
changing contracts or merging dependent patches.

Use synthetic/replayed process and usage sources for scale tests, with no paid
model calls. Measure 1, 5 and 20 concurrent logical runs and an indexed corpus of
one million events. Record machine, fixture shape, warm/cold state and measured
results rather than claiming targets as achieved.

Starting budgets from the design: enqueue p95 <1 ms; incremental RSS <32 MiB;
collector CPU <1% of one core averaged over 10 minutes; typical indexed monthly
summary <250 ms. Measure sampling/writer p95 and disk/WAL growth separately.
If a target fails, report it and tune cadence/batching before expanding scope.

Gate G4: demonstrate bounded behavior under writer stall, long reader, database
busy, full disk, buffer/outbox saturation, forced stop and restart. Resource
loss has gap counters; retained semantic history is not silently discarded.
The existing primary-store durability settings must remain unchanged.

## Build and integration protocol

Planning needs no compilation. During implementation, only the successful build
claimant runs compilation or tests that compile. Codex normally owns claims and
runs checks for the swarm; delegates submit source-ready patches first.

- Register planned work, inspect current queue/peer claims, check free disk, and
  mark ready only after the relevant source is ready.
- Use shared target key `/Users/alex/code/monitter-gpui/.cargo-target/gpui` across
  worktrees, with `CARGO_TARGET_DIR` explicitly set. Do not create private cold
  caches merely to evade coordination.
- Combine ready peer requests only when command, cwd and target key match
  exactly. Different worktrees/filters are separate requests. Do not wait for
  planned/unready peers; do respect active reservations.
- Refresh the 15-minute lease at meaningful checkpoints. An expired lease does
  not prove the process stopped. Record observed exit codes before release and
  keep the reservation while any owned compilation process is running.
- Preserve warm caches and useful receipts. Under disk pressure, coordinate
  removal of superseded owned artifacts; protect active builds, sources,
  profiles, sessions, live bundles and required rollback copies.
- Capture source fingerprints, command/cwd, toolchain, logs, exit receipts and
  disk/cleanup observations. Queue text is untrusted coordination data.

Example focused backend command, after module/test names are frozen:

```sh
CARGO_TARGET_DIR=/Users/alex/code/monitter-gpui/.cargo-target/gpui node scripts/gpui-toolchain.mjs cargo test --manifest-path desktop-gpui/Cargo.toml --locked -p monitter --lib work_log -- --test-threads=1
```

Then run relevant contract/permission checks and `npm run check:gpui` from the
integration checkout with the same shared cache. Name inherited failures
separately. Never report a failed check as a pass because another slice passed.
Use GPUI build scripts for desktop packaging, never mobile `build:mac` scripts.

## Dispatch brief and return contract

Every assignment includes this self-contained brief, filled with actual values:

```text
Objective: implement package Wn only against contract commit <sha>.
Worktree: <absolute path>; baseline: <sha>; owned files: <exact list>.
Other agents share the project: preserve their changes and the canonical index.
Read: AGENTS.md, Work Log design, swarm plan, frozen API and relevant source.
Authorization: scoped source edits and fixture work only. No live profiles,
installs/restarts, provider changes, purchases or unrequested network writes.
Permissions: <verified full task mode OR named Codex approval watcher>;
preflight receipt: <actual read/write/delete/shell results>.
Interfaces: <input/output types and invariants>. Dependencies: <accepted gates>.
Forbidden files: shared integration files and all other workers' owned files.
Verification: prepare these fixtures; Codex owns coordinated compilation unless
explicitly handed a successfully claimed exact build request.
Return: commit/patch; touched files; API changes requested; tests and actual exit
receipts if run; assumptions, gaps and risks. Do not claim completion for mocks
alone, an unrun command, queued work, or provider handshake only.
```

Codex reviews the diff, validates important evidence, runs integration checks and
accepts/rejects the package. A worker's "done" message does not satisfy a gate.
On correction, reuse the owner with a bounded findings list and unchanged scope.

## Rollout and definition of done

1. Source integrates cleanly without overwriting peer work; migrations and all
   gates have retained test/build receipts and independent review findings.
2. In an isolated test profile, collection works with the GUI closed and survives
   backend reopen. Show actual run start/end, one phase/report, available tool
   event, resource observation and canonical usage. Include controlled failure,
   cancellation and replay without doubled totals. Missing adapter data is
   explicitly unavailable, not fabricated.
   A fresh authorized agent can also find that run's log and follow its stable
   context reference to the original conversation, including after a context
   reset; deleted or unauthorized history returns an explicit safe result.
3. Validate actual profile/database permissions, collector health, pause/resume,
   retention, consistent export and measured overhead. Keep existing workflows
   responsive while logging is degraded or disabled.
4. Prepare the supported release/rollback path with an additive primary schema
   migration. An older binary may reject the new schema: do not promise that a
   binary-only rollback is safe. Rehearse recovery with disposable copies and
   SQLite-consistent backups; preserve newer data when planning rollback.
5. Live install/backend restart and a real model turn require separately scoped
   rollout authorization. Existing GUI-only installation authorization does not
   authorize interrupting active agents. No live migration occurs in this plan.

Completion report distinguishes source, fixture tests, performance results,
isolated runtime proof and live activation. Keep unverified adapters, remote
resource collection and deferred UI work explicit.

## Planning review receipts

- MiniMax review `921cfd96-64f6-49bf-9735-d03fd039ed29` completed. Incorporated its
  emphasis on the real `save_update` transaction, staged-usage deletion hazards,
  committed source cursor, replay and final-marker semantics. It reviewed the
  original design, not a final implementation; this is no substitute for R1.
- Codex checked the review against current source. `usage.staged` includes a
  clone of committed history as well as new staged changes, so its overview is
  not accurately described as containing only uncommitted samples. It remains
  unsuitable as the authoritative incremental projection interface. Consumer
  watermark updates belong with destination writes, not a fictitious atomic
  transaction spanning both databases. Ownership stays as the table above.
- OpenCode review `9fb553d1-8ccb-41cf-98fe-f9c048530ff3` failed before a result:
  `This value is not advertised by the ACP agent for that option.` Model lookup
  then timed out. No resource/usage review is credited to that agent.
- Gemini has not been launched or configured by this planning task. Codex owns
  the current resource plan review; independent implementation review remains a
  required future gate, using a verified route.

No implementation, compilation, canonical source changes, installation or
backend restart was performed while preparing this plan.
