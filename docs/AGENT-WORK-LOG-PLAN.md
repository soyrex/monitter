# Agent work log feature plan

Status: proposed design. Scope: collection and reporting foundation first; richer visualisations later.

Execution companion: [Work Logs swarm implementation plan](WORK-LOG-SWARM-IMPLEMENTATION.md).
It defines Codex orchestration, delegate ownership and acceptance gates, and
tightens destination durability before primary-outbox acknowledgement.

Monitter should keep a private, local SQLite work log that answers what we worked on, when, for how long, with which agents and models, and at what resource and token cost. Collect measured facts in the Rust backend and add short agent descriptions at meaningful checkpoints. Collection continues when the GUI is closed and adds no periodic model calls.

Use a dedicated `work-log.sqlite3` beside the active backend profile's existing store, with typed reporting tables, one background writer, bounded queues, and a modest process sampling rate. Reuse existing usage ingestion, process inspection and work plans. Keep telemetry out of the frontend Snapshot and its normal refresh path.

## Existing foundation and implementation location

The planning worktree is `/Users/alex/code/monitter-work-log-plan`, branch `plan/agent-work-log-20261015`, based on legacy/mobile commit `231a752`. Desktop implementation belongs in `/Users/alex/code/monitter-gpui` and a new implementation worktree from its current branch. Its live working tree contains substantial concurrent changes; this document does not copy or integrate those changes.

These source anchors describe the inspected GPUI checkout on 15 October 2026; line numbers may move:

| Existing code | What to reuse or address |
| --- | --- |
| `src-tauri/src/model.rs`, `RunUsageSample` and `UsageTokens` | Run/task identity, nullable token categories, cost and provider timing. |
| `src-tauri/src/runner.rs`, `normalize_usage` | Existing provider usage normalization. |
| `src-tauri/src/lib.rs`, `capture_usage`, `mark_usage_final`, `finish` | Bind collection to actual backend runs and lifecycle transitions. A final marker need not contain a final provider total. |
| `src-tauri/src/store.rs`, `stage_usage`, `usage_overview` | Existing sample deduplication and cumulative/delta aggregation. Staging alone is not a durable commit. |
| `src-tauri/src/sqlite_backend.rs`, `usage_samples` | Existing SQLite dependency and private-file/migration patterns. Usage rows currently contain JSON payloads rather than analytical columns. |
| `src-tauri/src/process_metrics.rs`, `sample`; `lib.rs`, `process_metrics` | macOS CPU time, RSS and process identities. Current samples cover the local Monitter process tree and are for live display; they are not a per-run resource history. |
| `src-tauri/src/collaboration.rs` and `collaboration_mcp.rs`, `work_plan_*` | Existing task-scoped agent checklists and updates. Reuse their descriptions and transitions. |
| `desktop-gpui/src/usage.rs` and `sidebar_usage.rs` | Existing live usage display. Resource logging must work independently of this UI polling. |

Before implementation, read the destination worktree's `AGENTS.md`, `docs/CONTRACT.md` and `src/lib/types.ts`. Preserve Rust authority, camelCase API fields, UUID identities and Unix millisecond timestamps.

## What the first version records

| Category | Fields and meaning |
| --- | --- |
| Work identity | Project, task/chat, work plan and item, run, turn when known, parent run/delegation, agent profile and host IDs. Snapshot labels so renamed or deleted chats remain understandable. |
| Source context | Stable context reference to the originating Monitter chat/session segment, run and message/event anchors. Future authorized agents can resolve it to canonical paginated chat history. Renames and context resets must not silently retarget it; deleted/unrecorded history stays explicitly unavailable. |
| Execution | Harness/adapter and version when available; configured and observed model separately; reasoning setting; local/SSH execution; backend session; worktree/repository identity and optional commit SHA captured once per run. Missing metadata remains null. |
| Work descriptions | Bounded summary, phase (`investigate`, `plan`, `implement`, `verify`, `review`, `other`), optional labels and evidence references. Keep measured facts separate from agent assertions. |
| Lifecycle | Start, state changes, approval/input wait, resume, success, failure, cancellation and interruption. Record trigger (`user`, `schedule`, `delegation`, `retry`, `unknown`) when known. |
| Timing | UTC start/end, monotonic elapsed intervals, runnable/working and known blocked intervals, tool intervals and first-response latency where adapters expose them. |
| Resources | Observed process CPU milliseconds, current/peak sampled RSS, RAM byte-milliseconds, observed coverage, process count and sampling gaps. Host context can include CPU count and memory pressure at a slower cadence. |
| Usage | Input, output, cache-read/write, reasoning and total tokens as nullable categories; provider cost, currency and source; provider/turn scope and cumulative/delta mode. |
| Outcomes | Backend exit/outcome, separate agent result summary, build/test references and observed exit codes where structured data already exists. A claimed fix is not proof of a successful release. |
| Collection quality | Source, schema/adapter version, observed timestamp, counter resets, partial attribution, dropped/coalesced samples, interruption and unavailable fields. |

Do not collect prompts, transcripts, environment variables, full command lines, raw tool arguments, credentials or file contents. Process inspection already exposes command lines; the new persistence path must explicitly exclude them. Repository paths, labels and summaries remain private user data. Store bounded structured error codes rather than raw errors that might contain secrets.

## Automatic collection and agent reports

The backend automatically records run lifecycle, structured tool events, usage, process samples, work-plan transitions, collaboration lineage and build outcomes. These facts remain useful without agent participation. Attach IDs at the source; do not infer run identity from a chat title or the currently visible pane.

Reuse `work_plan_start/update/close` as semantic work-log events. Add one optional MCP tool, provisionally `work_log_report`, for work that has no checklist or needs a checkpoint summary:

```json
{
  "request_id": "stable-checkpoint-id",
  "phase": "implement",
  "summary": "Added retry handling to the adapter",
  "plan_item_id": null,
  "labels": ["reliability"]
}
```

Backend authentication supplies project/task/run/agent/host and receipt time. The caller cannot claim another agent, supply measured CPU/tokens, backdate measured intervals, or change run outcome. Limit summaries to 1,000 characters and labels to 8 bounded values. Deduplicate by caller identity and request ID; an exact retry returns its receipt, while changed content with the same ID rejects. Apply existing MCP quotas, plus a proposed ceiling of 20 semantic reports per run per minute. Report acceptance only after durable insertion into the primary-store outbox.

Guidance asks for a report when work starts, changes phase, becomes blocked, or ends; existing checklist updates satisfy this. Do not report on every tool call, add timer-driven agent messages, or call a model to classify each event. An optional retrospective can be added separately later.

## SQLite model

Keep analytical columns typed and indexed; use bounded JSON only for sparse extensions. Proposed logical tables:

| Table | Principal columns and invariants |
| --- | --- |
| `work_runs` | `run_id` primary key; project/task/agent/host IDs; parent run; backend session; label snapshots; harness/model metadata; start/end; outcome; last observation and coverage. One row per execution, not per chat. |
| `work_context_refs` | Opaque reference ID; originating installation/profile, project and stable chat/task ID; context boundary and message/event anchors; run/turn references; owner-private native-session locator when available; availability/tombstone metadata. Links context without copying transcript text. |
| `work_events` | UUID primary key, unique `source_key`, run/turn/plan/item references, backend sequence, occurred/observed times, event kind, phase, source and bounded summary/metadata. Append events; corrections reference earlier events. |
| `work_intervals` | Stable interval ID, run/turn, kind, UTC boundaries, monotonic duration, observation quality and closing event. Open intervals have null end; closing/correction is idempotent. Derived from ordered events. |
| `usage_observations` | Unique canonical sample ID, run/turn, source and counter epoch, delta/cumulative mode, final-marker flag, nullable token columns, reported monetary value/currency, observed time and source precision. |
| `usage_totals` | Rebuildable per-run/per-turn projection with effective counts, completeness and cost provenance. Delta observations add once; cumulative observations replace within their documented counter scope. |
| `resource_samples` | Stable run/sample ID, actual interval and timestamp, CPU delta, sampled RSS, byte-ms integral, sampled peak, process count, coverage and quality. Unique run/sample key. |
| `resource_buckets` | Run plus UTC bucket start/width, CPU-ms sum, RSS byte-ms, sampled maximum, covered-ms and sample count. Upsert transactionally with a compaction watermark. |
| `collector_gaps` | Session/run scope, gap boundaries, reason and dropped counts. Null end means ongoing. |
| `collector_meta` | Schema version, source cursors, compaction watermark, retained detail horizon and collector health. |

In the primary store add a small `work_log_outbox` for low-frequency lifecycle/report events. Write it in the same transaction as the corresponding state mutation. The work-log writer inserts using `source_key`, commits, then acknowledges/removes the outbox row. Crash between commit and acknowledgement produces a harmless replay. No cross-database atomicity is assumed. Avoid replacing or weakening the existing state-store durability settings.

Usage observations are projected from committed canonical usage samples using an incremental cursor and overlap deduplication. Coordinate pruning so unprojected usage is not discarded. Task/agent deletion currently calls `remove_task_usage`, which can remove samples before projection: preserve those observations and historical identity snapshots in the same primary-store transaction through the outbox before deleting canonical rows. Include staged samples in that transaction; do not rely on a background drain racing deletion. If no stable persisted sequence exists, add one; scanning the entire usage history on every batch is unacceptable. At rollout, explicitly choose a bounded historical import with a coverage marker, or start prospectively. Old chat creation times do not establish historical working time.

Resource samples use an independent bounded memory queue and can be coalesced or dropped with gap counters; they do not create primary-store writes at every tick. Reserve queue space for collector health events. Bound the durable outbox by bytes and count; if unavailable or exhausted, expose degraded collection, reject an undurable explicit report, and allow agent execution to continue. Recovery marks the uncertain interval; it cannot recreate lost transitions as measured history.

Index `work_runs(project_id, started_at)`, `(task_id, started_at)`, `(agent_id, started_at)`; `work_events(run_id, backend_sequence)` and `(observed_at)`; usage by `(run_id, counter_epoch, observed_at)`; resources by `(run_id, sampled_at)`; buckets by `(run_id, bucket_start)`. Add model/label indexes only when query plans justify them. Preserve historical identities if live tasks/projects are removed; provide a separate explicit work-log deletion operation.

## Accounting rules that keep reports honest

**Time:** distinguish run elapsed time, observed working time, known waiting time, tool time and CPU time. An open chat is not continuous work. Silence alone does not prove waiting. Phase descriptions are agent-declared; measured interval boundaries come from lifecycle/events. Use monotonic elapsed time within a backend session and wall time for calendar grouping. Record sleep, clock changes, backend restart and observation loss as gaps; do not extend the last phase across them. Interrupted runs retain their last observed boundary and an unknown tail.

**Parallel work:** sum durations for agent-hours; union intervals for project wall-clock coverage. Five agents running together for one hour means five agent-hours and one hour of coverage. Avoid adding nested tool time to working time. Delegated children have their own runs; parent reports reference them without duplicating child usage. Human effort and productivity are not inferred from agent runtime.

**CPU and RAM:** track `(host_id, pid, process_started_at)` before calculating counter deltas. A disappeared/reused PID or reset aggregate counter must not yield negative CPU or a fake spike. Assign descendants to the nearest verified owned harness root; count each process once. Shared backend/provider helpers remain shared or unattributed. Sum per-process CPU deltas rather than subtracting whole-tree totals when membership changes. CPU percent is `100 × cpu_delta_ms / covered_elapsed_ms`; it can exceed 100% across cores. Sum RSS only over attributed processes; it is resident footprint, not exact unique physical memory. Show sampled peaks as sampled, and average RSS using covered time. Short-lived children may be missed.

**Remote work:** the local SSH client is transport overhead, not remote workload CPU/RAM. Record unavailable remote resources as null. A later authenticated remote collector must report host identity, sequence and clock uncertainty before remote samples can be compared. Local-model GPU/VRAM and shared model-server resource allocation are future capabilities; do not label them as per-agent measurements without attribution.

**Tokens and money:** use adapter-specific counter scopes/epochs, not a blanket assumption that all providers behave alike. Exact duplicate events add nothing. Retries that actually execute are separate chargeable work. Never add a final cumulative total to earlier cumulative totals. Reasoning/cache tokens can be subsets of other categories; preserve provider semantics and do not sum all columns to invent a total. Unknown counts/cost remain null, with known totals and missing-coverage counts in aggregates.

Keep provider-reported cost separate from an optional estimate using versioned rates and a captured model identity. Persist integer monetary units with explicit scale/currency and source precision; the existing floating USD source is not proof of billing precision. Subscription allowance is a separate concept, not a per-run invoice. Cross-currency totals require an explicit conversion policy. No provider pricing fetches or paid calls are needed for collection.

## Performance, durability and retention defaults

These are proposed budgets to measure, rather than performance claims:

| Control | Starting default |
| --- | --- |
| Process sampling | One shared process-tree walk every 10 seconds while runs execute; none when no runs execute. Deduplicate with live UI samples where practical. |
| Writer | Single background owner; commit every 2 seconds or 100 records, whichever comes first. Flush lifecycle batches promptly. Never hold the task-state mutex during work-log I/O. |
| Buffer | At most 4 MiB or 4,096 pending resource records; bound event payload size before enqueue. Coalesce resources under pressure. |
| SQLite | WAL, foreign keys, prepared statements, short transactions, 100 ms writer busy timeout with bounded retry. Read-only reporting connections with query deadlines and pagination. |
| Durability | Work-log WAL writer uses `synchronous=FULL` before acknowledging primary outbox rows. Otherwise a power failure could lose both an acknowledged destination commit and its deleted source row. Uncommitted resource samples remain loss-tolerant with explicit gaps. |
| Detail retention | Raw resources for 30 days; one-minute buckets for one year; hourly buckets thereafter. Keep semantic events, run summaries and usage totals until explicit deletion. |
| Disk budget | Proposed 1 GiB soft limit and 2 GiB hard limit including WAL. Compact resources first. At the hard limit pause resource detail; if retained history alone fills it, pause new logging visibly until space/settings change. Never silently delete semantic history. |
| Collector overhead | Target <1% of one CPU core averaged over 10 minutes and <32 MiB incremental RSS with 20 active runs. Report sampling and writer p95 separately. |
| Responsiveness | No filesystem/database work on UI or agent streaming threads; enqueue p95 target <1 ms. Typical indexed monthly summary target <250 ms at one million events. |

WAL permits concurrent readers and a writer, but SQLite still serializes writers and long-lived readers can delay checkpointing. Use bounded report queries and passive checkpoints; monitor actual WAL growth. Batch `FULL` commits and measure their cost rather than weakening durable delivery acknowledgements. Keep the database on the local filesystem. See [SQLite WAL documentation](https://sqlite.org/wal.html) and [synchronous settings](https://sqlite.org/pragma.html#pragma_synchronous).

At 20 continuously active runs and a 10-second cadence, sampling produces 172,800 rows per day. At an assumed 200–400 bytes per indexed row, that is roughly 35–69 MB/day before WAL and other tables. Measure actual file growth; the byte budget may trigger compaction earlier than the nominal retention horizon. Compaction must be restart-safe and preserve totals, observed duration and maxima without double counting raw/bucket overlap. Export reports disclose available resolution.

During disk pressure, stop optional detail first, expose health and preserve active runs and source data. Avoid foreground `VACUUM`; schedule bounded cleanup and checkpoints when idle. Use SQLite-consistent backup/export rather than copying the live main file without its WAL. Private database and sidecar permissions must match the existing store protections.

## Reports and later visualisations

First ship owner-only filtered queries and export: time range/timezone, project, agent, harness/model, host, phase and outcome. Return aggregate values, completeness/gaps and a keyset-paginated timeline. Store UTC; use the requested reporting timezone for days and daylight-saving boundaries. Expensive grouping/SQL stays in the backend. Exports are local SQLite-consistent snapshots or CSV/JSON; arbitrary SQL is not exposed through agent, LAN or visitor tools.

Also provide explicitly granted, task-scoped log search and context resolution for
future agents. A log's `contextRef` resolves through the canonical history reader
to the actual source chat segment, with bounded pagination and current access
checks. Preserve links across rename/archive/restart and identify `/clear` or
`/new` boundaries. A reference is not permission, and permanent history deletion
must return unavailable rather than secretly retaining a transcript copy. See
the execution companion for the provenance contract and acceptance fixtures.

Useful later views include:

- A project calendar heatmap and a timeline of agent work, waits, tools, builds and interruptions.
- Agent-hours beside project elapsed coverage, including peak concurrency and delegation trees.
- CPU core-hours, sampled memory peaks and RAM-hours overlaid on work phases.
- Token/cost breakdown by model, harness, project and phase, including cache ratios where categories support that calculation.
- Worktree/commit annotations, failed-build and retry costs, time waiting for approvals or the build queue.
- Quality overlays showing missing usage, interrupted collection and estimated costs.

A small GPUI Work Log entry can initially show recent entries, filters and collector health. Mobile/shared views follow only through their existing role permissions; collecting the foundation does not require adding mobile UI. Start with descriptive comparisons; model speed or token cost alone does not establish quality or productivity.

## Implementation sequence and acceptance

1. **Storage and lifecycle.** Add `work_log.rs`, `work_log_store.rs`, additive outbox migration and a backend-owned writer. Hook start/state/finish, record clock/session boundaries, expose health and run summaries. Validate migration/reopen, exact replay, crash between databases, disk-full handling, interrupted runs and task deletion. Rollout starts prospectively unless explicitly importing known usage.
2. **Canonical usage projection.** Reuse committed samples; add adapter-scoped counter normalization, idempotent cursoring and provenance. Validate cumulative and delta providers, duplicate/reordered events, empty final markers, model switches, context resets, resumed runs and real retries. Race task/agent deletion against staging, projection and restart to prove usage survives. Historical import must not claim unavailable phase/resource data.
3. **Resource history.** Extend existing inspection with verified run attribution and a backend sampler. Validate PID reuse, disappearing children, overlapping/shared roots, collector overload, sleep, unsupported platforms and SSH gaps. Measure overhead and disk growth at 1, 5 and 20 concurrent runs; use recorded or synthetic fixtures without paid model calls.
4. **Semantic reports.** Project existing work-plan transitions; add the bounded task-scoped report tool and guidance. Validate spoofed identities, changed retry payloads, limits and restart deduplication. Treat summaries as untrusted text when later rendered or passed to a model.
5. **Queries and initial desktop view.** Add owner-scoped command definitions and aligned Rust/TypeScript contracts, paginated timeline, aggregates and local exports. Validate parallel interval unions, null coverage, retention overlap, timezone/DST, permission boundaries and realistic query plans. Use GPUI retained state and virtualized lists.

Run focused tests for each slice, then the relevant contract and GPUI checks. Before any compilation, register planned work with `build_request`, inspect and coordinate the queue, mark ready, and successfully `build_claim`. Reuse the shared target resource key `/Users/alex/code/monitter-gpui/.cargo-target/gpui` across implementation worktrees; group only exact command/cwd/target matches. Check free disk, keep heartbeats, and complete with observed exit status and disk observations. Do not use the mobile checkout's macOS packaging scripts.

An end-to-end acceptance run must show one real local agent turn, checklist/report entry, tool event, measured resource interval and provider usage persisted across client close and backend reopen. Add controlled failure/cancellation and a replay without doubled usage. Verify collector overhead, unavailable-field rendering, export completeness and query latency. Source tests and a build are separate evidence from an installed-app test; installing or restarting the live backend is a separate rollout action.

## Proposed product defaults to review

Enable local measurement with a visible collection toggle and health indicator; keep collection paused when disabled. Default descriptions to existing checklist updates plus optional explicit checkpoints. Retain semantic history until deletion, compact resource detail automatically, and keep reported cost distinct from estimates. Start with macOS local resources and explicitly unavailable remote resources. These defaults allow useful history to accumulate before investing in richer dashboards.
