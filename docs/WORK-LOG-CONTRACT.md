# Work Logs implementation contract v1

Frozen C0, 15 October 2026. Source baseline: `7d8890b` on branch
`feature/work-logs-20261015`. That baseline snapshots concurrent GPUI changes;
feature patches are measured from it, not from the older mobile planning branch.

Collection defaults **off**, prospective only. Owner-local configuration enables
it. Disabling ends observed intervals with a collection-paused gap, stops new
lifecycle/usage/resource capture, and retains accepted outbox entries for drain.
Resume starts new observed coverage at resume time; it never fills the pause with
inferred work. Retain semantic history/usage until explicit history deletion.
Raw resources: 30 days, minute buckets: 365 days, hourly thereafter. Disk soft /
hard limits: 1 / 2 GiB including WAL. Settings are private primary-store metadata,
not frontend Snapshot data. Unknown metrics are nullable.

Rust DTOs and writer/source seams are in `src-tauri/src/work_log/types.rs`.
Primary outbox source keys are unique; its AUTOINCREMENT sequence is the committed
source cursor. Destination observations, totals, and consumer watermark commit in
one FULL-synchronous transaction before source acknowledgement. Source commit
failures retain existing Store poisoning. Automatic collector-capacity failures
degrade logging without blocking normal work; explicit reports reject visibly.
Primary outbox: 10,000 rows / 16 MiB, record bound 32 KiB. Resources: independent
4,096 rows / 4 MiB queue; dropped records produce gaps, not durable-success claims.

W1 owns schema.rs/store.rs/writer.rs and storage fixtures. `WorkLogDb::open(dir)`
opens `work-log.sqlite3`; `apply_batch(&[CommittedRecord])`,
`append_resources(&[ResourceSample])`, `context_ref(&str)`, `health()` and
`connection()` (crate-private read access) are the integration seam. Writer owns
one DB connection; `drain(&mut WorkLogDb, &impl OutboxSource)` commits before ack.
Readers use separate read-only connections. W1 may propose signature corrections
but must not edit types.rs/shared files. Use only sanitized error codes in health.

W2 owns usage.rs: `project(&[UsageObservation]) -> Vec<UsageProjection>`. Group by
real run plus provider/model/turn scope; dedupe stable sample IDs; order by primary
sequence, never mutable row position. Empty final markers do not replace totals.
Preserve nullable token categories (no invented total); money uses integer units
at scale 1,000,000 USD with explicit floating-source precision. Counter decrease
without a documented new epoch marks incomplete coverage rather than inventing
new usage. Real retry run IDs remain independent.

W3 owns resources.rs/sampler.rs: stateful pure `ResourceTracker::observe(utc_ms,
monotonic_ms, roots, processes) -> Vec<ResourceSample>`. Identity is host/PID/start.
Nearest verified root owns descendants; ambiguous/shared roots stay unattributed.
Remote roots produce unavailable coverage, never remote CPU values. Detect clock
and monotonic discontinuities and >30 s gaps; baseline after gaps. No persisted
commands, names, environment, arguments, paths or transcript text.

W4 reports.rs validates bounded ReportInput (1,000 chars, 8 labels at 64 chars,
known phase), derives exact content fingerprint and caller-scoped source key.
Coordinator authenticates current running caller/run and handles durable primary
receipt, retry conflict and <=20/min/run before accepting. Plans are automatic.

W5 owns query.rs/retention.rs/export.rs after G1: bounded keyset timeline and
typed summary; raw/bucket exclusivity; transactionally compact then delete raw;
CSV formula protection and JSON export through read-only consistent transactions.
Owner APIs use the private IPC only. Agent query/context reads require separate
explicit owner grants and every page rechecks access. Context refs are opaque,
snapshotted message segment anchors; references never confer permissions. Do not
copy chat text to telemetry or expose native IDs/paths to agent tools.

Worker compilation is prohibited without coordinator handoff of a successful
exact build claim. No live backend/model calls, installs, profiles, purchases,
provider settings, or unrelated edits are authorized by these packages.
