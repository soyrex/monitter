//! Work Log store: the single writer connection plus crate-private reader seam.
//!
//! Storage contract (frozen C0 v1):
//!
//! * `WorkLogDb::open(dir)` opens `work-log.sqlite3` in the canonical
//!   per-installation work-log directory. The directory is created with
//!   `0700` permissions and validated against symlinks; the database file
//!   is opened with `SQLITE_OPEN_NOFOLLOW` semantics and `0600` permissions
//!   on creation; sidecars inherit the directory protection. WAL +
//!   `synchronous=FULL` + `foreign_keys=ON` + `busy_timeout=100ms` are
//!   enforced.
//! * `apply_batch(records)` validates positive, increasing primary sequences
//!   (including AUTOINCREMENT gaps), schema version and source-key identity. Each
//!   record's durable source receipt is recorded in `seen_sources`. Terminal
//!   runs are monotone: a re-insert with the same source_key no-ops; a row
//!   transitioning from terminal to running is rejected. The transaction
//!   commits FULL, advances the watermark to the highest sequence applied
//!   in this batch, then returns that value. The primary outbox is
//!   acknowledged only with that returned value — never with the destination
//!   watermark read from `health()`, which may reference unrelated future
//!   records.
//! * `append_resources(samples)` writes resource rows independently of the
//!   outbox using a SAVEPOINT so partial inserts roll back on failure.
//!   Overflow produces a `CollectorGap` and increments the dropped counter;
//!   the writer never claims a durable success for dropped rows.
//! * `set_context_ref(snapshot)` upserts the latest snapshot. Re-snapshotting
//!   the same `context_ref` with a different availability writes a new
//!   version row, preserving the historical snapshot. Deletion is
//!   represented by `availability = Deleted`; the row stays for audit.
//! * `context_ref_lookup(id)` returns the latest snapshot for an opaque ID
//!   or `None` when no snapshot has been recorded.
//! * `health()` returns a sanitized snapshot. Reason strings are limited to
//!   a fixed set of error codes; the writer never passes raw SQLite errors
//!   or filesystem paths to the public health surface.
//! * `connection()` exposes the read-only seam (crate-private). It returns
//!   a fresh `Connection` opened in read-only mode against the same file
//!   with `SQLITE_OPEN_NOFOLLOW` and `busy_timeout=100ms`. The writer
//!   connection is not exposed; readers cannot accidentally mutate durable
//!   state.

use crate::model::RunUsageSample;
use crate::work_log::schema;
use crate::work_log::types::{
    CollectorGap, CollectorHealth, CommittedRecord, ContextRef, ResourceSample, WorkEvent,
    WorkRecord, WorkRun,
};
use rusqlite::{params, Connection, Error as SqliteError, OptionalExtension, Transaction};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const SETTINGS_DENY: &str = "work_log_store: settings lookup is a coordinator handoff";

/// Owning handle for the work-log database. Constructed via `WorkLogDb::open`.
pub struct WorkLogDb {
    path: PathBuf,
    writer: Arc<Mutex<Connection>>,
    budget: std::sync::atomic::AtomicU64,
}

/// Crate-private read access. Returned by `connection()` only when called
/// from within this crate. Other modules must request a reader through the
/// coordinator-owned seam; raw SQL is never exposed to the LAN/visitor surface.
pub struct WorkLogReader {
    conn: Connection,
}

#[derive(Debug)]
pub enum StoreError {
    Sqlite(SqliteError),
    Schema(String),
    Invalid(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "work-log sqlite error: {}", sanitized_code(e)),
            StoreError::Schema(m) => write!(f, "work-log schema error: {}", m),
            StoreError::Invalid(m) => write!(f, "work-log invalid: {}", m),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<SqliteError> for StoreError {
    fn from(e: SqliteError) -> Self {
        StoreError::Sqlite(e)
    }
}

impl WorkLogDb {
    pub fn set_budget(&self, limit: u64) {
        self.budget
            .store(limit, std::sync::atomic::Ordering::Release);
    }
    fn reserve_budget<T: serde::Serialize>(&self, records: &[T]) -> Result<(), StoreError> {
        let limit = self.budget.load(std::sync::atomic::Ordering::Acquire);
        if limit == u64::MAX {
            return Ok(());
        }
        let (main, wal) = schema::database_bytes(&self.path);
        // Reserve conservatively for indexes, page splits, WAL frames and one
        // fixed transaction overhead before entering a destination transaction.
        let raw = serde_json::to_vec(records)
            .map_err(|_| StoreError::Invalid("invalidBudgetRecord".into()))?
            .len() as u64;
        let reserve = raw
            .saturating_mul(16)
            .saturating_add((records.len() as u64).saturating_mul(4096))
            .saturating_add(65536);
        if main.saturating_add(wal).saturating_add(reserve) > limit {
            return Err(StoreError::Invalid("historyBudget".into()));
        }
        Ok(())
    }
    /// Only the backend collector invokes maintenance and history deletion.
    /// Read APIs never acquire a second writer connection.
    pub fn compact(
        &self,
        settings: &crate::work_log::CollectionSettings,
        at: i64,
        pressure: bool,
    ) -> Result<bool, StoreError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("writerUnavailable".into()))?;
        let summary =
            crate::work_log::retention::compact(&mut writer, settings, at, pressure, 1000)
                .map_err(StoreError::Invalid)?;
        writer.execute_batch("PRAGMA incremental_vacuum(256); PRAGMA wal_checkpoint(PASSIVE)")?;
        Ok(summary.remaining)
    }
    pub fn delete_runs(&self, run_ids: &[String]) -> Result<usize, StoreError> {
        if run_ids.is_empty() || run_ids.len() > 100 {
            return Err(StoreError::Invalid("invalidHistorySelection".into()));
        }
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("writerUnavailable".into()))?;
        let tx = writer.transaction()?;
        let mut count = 0;
        for id in run_ids {
            tx.execute(
                "INSERT INTO deleted_runs(run_id,deleted_at)VALUES(?1,?2)ON CONFLICT DO NOTHING",
                params![id, schema::now_ms()],
            )?;
            for table in [
                "events",
                "context_refs",
                "usage_observations",
                "usage_projection",
                "resource_samples",
                "resource_buckets",
                "resource_retention",
                "work_intervals",
                "collector_gaps",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE run_id=?1"), [id])?;
            }
            count += tx.execute("DELETE FROM runs WHERE run_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(count)
    }
    /// Open or create the work-log database at `<dir>/work-log.sqlite3`.
    /// The directory is created with `0700` permissions and validated against
    /// symlinks; the database file is opened with `SQLITE_OPEN_NOFOLLOW`
    /// semantics and `0600` permissions on creation. WAL with
    /// `synchronous=FULL` is enforced. Returns an error if the database
    /// cannot be opened, hardened, or migrated.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        schema::ensure_dir(dir).map_err(|e| {
            StoreError::Invalid(format!(
                "work-log dir ensure failed: {}",
                sanitized_code_io(&e)
            ))
        })?;
        let canonical_dir = dir
            .canonicalize()
            .map_err(|e| StoreError::Invalid(sanitized_code_io(&e)))?;
        let path = schema::default_path(&canonical_dir);
        let conn = schema::open_writer_connection(&path)?;
        schema::validate_identity(&conn)?;
        schema::apply_pragmas(&conn)?;
        let version = schema::apply(&conn)?;
        schema::harden_sidecars(&path).map_err(|e| StoreError::Invalid(sanitized_code_io(&e)))?;
        if version != schema::SCHEMA_VERSION {
            return Err(StoreError::Schema(format!(
                "schema version mismatch: expected {}, got {}",
                schema::SCHEMA_VERSION,
                version
            )));
        }
        Ok(Self {
            path,
            writer: Arc::new(Mutex::new(conn)),
            budget: std::sync::atomic::AtomicU64::new(u64::MAX),
        })
    }

    /// Open an additional read-only connection for query use. Crate-private.
    /// `SQLITE_OPEN_NOFOLLOW` is set on the database path; the reader does not
    /// mutate sidecars. A 100 ms busy timeout is applied.
    pub fn connection(&self) -> Result<WorkLogReader, StoreError> {
        let conn = schema::open_reader_connection(&self.path)?;
        schema::validate_identity(&conn)?;
        conn.execute_batch(&format!(
            "PRAGMA busy_timeout = {}",
            schema::BUSY_TIMEOUT_MS
        ))?;
        Ok(WorkLogReader { conn })
    }

    /// Apply a batch of primary outbox records in a single FULL transaction.
    /// Validation performed before any row is written:
    ///
    /// * Each record's `schema_version` must equal `crate::work_log::types::VERSION`.
    /// * Source keys must be unique and positive sequences strictly increasing
    ///   within the batch. AUTOINCREMENT
    ///   gaps are valid when the primary has deduplicated source keys.
    /// * Each `source_key` is recorded in `seen_sources`. Replays of the same
    ///   `source_key` are no-ops at the row layer (unique constraints) and
    ///   skipped here once seen.
    /// * Terminal runs are monotone: a re-insert of the same `run_id` whose
    ///   stored row is already terminal is a no-op; a row transitioning from
    ///   terminal to non-terminal is rejected.
    ///
    /// Returns the highest sequence offered in this batch. The
    /// caller (writer) acknowledges the primary store with this value only
    /// after `Ok` is returned; `acknowledge(through_sequence)` MUST NOT be
    /// called with a value pulled from `health().watermark`, which may have
    /// advanced past this batch due to unrelated future records.
    pub fn apply_batch(&self, records: &[CommittedRecord]) -> Result<i64, StoreError> {
        if records.is_empty() {
            return Ok(self.current_watermark());
        }
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("work-log writer mutex poisoned".into()))?;
        let tx = guard.transaction()?;
        let current = self.read_watermark(&tx)?;
        let offered_highest = records
            .last()
            .map(|record| record.sequence)
            .unwrap_or(current);
        let now = schema::now_ms();
        // Validate every row, including replay-only batches, before any write.
        validate_batch(records)?;
        let mut unseen = Vec::new();
        for record in records {
            let digest = content_digest(record)?;
            if let Some((sequence, schema_version, stored_digest)) =
                seen_source(&tx, &record.source_key)?
            {
                if sequence != record.sequence
                    || schema_version != record.schema_version
                    || stored_digest != digest
                {
                    return Err(StoreError::Invalid("changed work-log source replay".into()));
                }
                continue;
            }
            if record.sequence <= current {
                return Err(StoreError::Invalid(
                    "unseen work-log source below watermark".into(),
                ));
            }
            unseen.push((record, digest));
        }
        // A fully validated replay is safe to acknowledge after a prior
        // destination commit, even if the destination has since reached its
        // byte budget. It must not mutate health or the watermark.
        if unseen.is_empty() {
            return Ok(offered_highest);
        }
        // Reserve only rows that will actually be inserted. This remains in
        // the same transaction as the write so a mixed replay/new batch is
        // all-or-nothing.
        self.reserve_budget(&unseen)?;
        for (record, digest) in unseen {
            apply_record(&tx, record, now)?;
            record_seen_source(
                &tx,
                &record.source_key,
                record.sequence,
                record.schema_version,
                &digest,
                now,
            )?;
        }
        advance_watermark(&tx, current.max(offered_highest), now)?;
        tx.commit()?;
        Ok(offered_highest)
    }

    /// Append resource samples independently of the outbox. Returns the number
    /// of rows actually written; overflow is recorded as a `CollectorGap` and
    /// increments the health dropped counter. The queue is bounded by
    /// `RESOURCE_MAX_ROWS`; samples beyond the bound are dropped before commit
    /// so the writer never claims a durable success for dropped rows.
    pub fn append_resources(&self, samples: &[ResourceSample]) -> Result<usize, StoreError> {
        if samples.is_empty() {
            return Ok(0);
        }
        let max_rows = crate::work_log::types::RESOURCE_MAX_ROWS;
        let max_bytes = crate::work_log::types::RESOURCE_MAX_BYTES;
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("work-log writer mutex poisoned".into()))?;
        let mut tx = guard.transaction()?;
        // Retries and repeated sample IDs are already durable facts, not
        // dropped measurements. Filter them before applying queue bounds.
        let mut unseen = Vec::new();
        let mut in_batch = std::collections::HashMap::<&str, String>::new();
        let mut late_or_replayed = std::collections::HashSet::<&str>::new();
        for sample in samples {
            let deleted: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM deleted_runs WHERE run_id=?1)",
                [&sample.run_id],
                |r| r.get(0),
            )?;
            if deleted {
                continue;
            }
            let compacted: Option<i64> = tx
                .query_row(
                    "SELECT compacted_through FROM resource_retention WHERE run_id=?1",
                    [&sample.run_id],
                    |r| r.get(0),
                )
                .optional()?;
            if compacted.is_some_and(|at| sample.sampled_at <= at) {
                // A compacted observation cannot be replayed as fresh raw detail.
                // No per-sample receipt survives compaction, so this could be
                // either a duplicate or a late sample. Preserve that uncertainty
                // as bounded coverage metadata without adding resource totals.
                late_or_replayed.insert(&sample.sample_id);
                continue;
            }
            let raw =
                serde_json::to_string(sample).map_err(|e| StoreError::Schema(e.to_string()))?;
            if let Some(previous) = in_batch.get(sample.sample_id.as_str()) {
                if previous != &raw {
                    return Err(StoreError::Invalid("changed resource sample replay".into()));
                }
                continue;
            }
            in_batch.insert(&sample.sample_id, raw.clone());
            let stored: Option<String> = tx
                .query_row(
                    "SELECT raw_json FROM resource_samples WHERE sample_id=?1",
                    [&sample.sample_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = stored {
                if previous != raw {
                    return Err(StoreError::Invalid("changed resource sample replay".into()));
                }
                continue;
            }
            unseen.push(sample.clone());
        }
        if unseen.is_empty() && late_or_replayed.is_empty() {
            return Ok(0);
        }
        let late_gap = if late_or_replayed.is_empty() {
            None
        } else {
            Some(CollectorGap {
                gap_id: crate::model::id(),
                run_id: None,
                started_at: schema::now_ms(),
                ended_at: Some(schema::now_ms()),
                reason: "lateOrReplayedResource".into(),
                dropped: u64::try_from(late_or_replayed.len())
                    .map_err(|_| StoreError::Invalid("droppedCountOverflow".into()))?,
            })
        };
        // Include every newly persisted fact in one conservative reservation.
        let mut budget_facts = Vec::<JsonValue>::new();
        for sample in &unseen {
            budget_facts
                .push(serde_json::to_value(sample).map_err(|e| StoreError::Schema(e.to_string()))?);
        }
        if let Some(gap) = &late_gap {
            budget_facts
                .push(serde_json::to_value(gap).map_err(|e| StoreError::Schema(e.to_string()))?);
        }
        self.reserve_budget(&budget_facts)?;
        if let Some(gap) = &late_gap {
            insert_gap(&tx, gap)?;
        }
        let count = unseen.len();
        let (accepted, dropped_pre, _bytes) = bound_resources(&unseen, max_rows, max_bytes);
        let written = match insert_resources(&mut tx, &accepted) {
            Ok(n) => n,
            Err(e) => {
                let gap = CollectorGap {
                    gap_id: crate::model::id(),
                    run_id: None,
                    started_at: schema::now_ms(),
                    ended_at: Some(schema::now_ms()),
                    reason: match &e {
                        StoreError::Sqlite(e) => sanitized_code(e),
                        _ => "resourceWriteRejected".into(),
                    },
                    dropped: count as u64,
                };
                insert_gap(&tx, &gap)?;
                bump_dropped(&tx, count as u64)?;
                tx.commit()?;
                return Ok(0);
            }
        };
        if dropped_pre > 0 {
            let total_dropped = dropped_pre;
            let gap = CollectorGap {
                gap_id: crate::model::id(),
                run_id: None,
                started_at: schema::now_ms(),
                ended_at: Some(schema::now_ms()),
                reason: "resource_overflow".into(),
                dropped: total_dropped,
            };
            insert_gap(&tx, &gap)?;
            bump_dropped(&tx, total_dropped)?;
        }
        touch_health_write(&tx)?;
        tx.commit()?;
        Ok(written)
    }

    /// Persist a context reference snapshot. A new availability value is
    /// recorded as a fresh version row; the historical snapshot remains
    /// intact for audit. Tombstones are explicit `Deleted` rows.
    pub fn set_context_ref(&self, snapshot: &ContextRef) -> Result<(), StoreError> {
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("work-log writer mutex poisoned".into()))?;
        let tx = guard.transaction()?;
        let version = next_context_version(&tx, &snapshot.context_ref)?;
        insert_context_ref(&tx, snapshot, version)?;
        touch_health_write(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Look up the latest version of a context reference by its opaque ID.
    /// Returns `None` when no snapshot has been recorded for the ID.
    pub fn context_ref_lookup(&self, id: &str) -> Result<Option<ContextRef>, StoreError> {
        let guard = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("work-log writer mutex poisoned".into()))?;
        let conn = &*guard;
        let raw: Option<String> = conn
            .query_row(
                "SELECT raw_json FROM context_refs WHERE context_ref = ?1
                    ORDER BY version DESC LIMIT 1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            None => Ok(None),
            Some(s) => serde_json::from_str::<ContextRef>(&s)
                .map(Some)
                .map_err(|e| StoreError::Schema(e.to_string())),
        }
    }

    /// Sanitized health snapshot. Reason strings are restricted to error
    /// codes; raw SQLite messages or filesystem paths never appear here.
    pub fn health(&self) -> Result<CollectorHealth, StoreError> {
        let guard = self
            .writer
            .lock()
            .map_err(|_| StoreError::Invalid("work-log writer mutex poisoned".into()))?;
        let conn = &*guard;
        let (db, wal) = schema::database_bytes(&self.path);
        let row = conn.query_row(
            "SELECT state, reason, last_write_at, pending_rows, pending_bytes,
                    dropped_resources,
                    (SELECT watermark FROM outbox_watermark WHERE id = 1)
             FROM health_state WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
        Ok(CollectorHealth {
            state: row.0,
            reason: row.1,
            last_write_at: row.2,
            pending_rows: row.3 as u64,
            pending_bytes: row.4 as u64,
            dropped_resources: row.5 as u64,
            database_bytes: db,
            wal_bytes: wal,
            watermark: row.6,
        })
    }

    fn current_watermark(&self) -> i64 {
        if let Ok(guard) = self.writer.lock() {
            if let Ok(value) = guard.query_row(
                "SELECT watermark FROM outbox_watermark WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            ) {
                return value;
            }
        }
        0
    }

    fn read_watermark(&self, conn: &Connection) -> Result<i64, StoreError> {
        Ok(conn.query_row(
            "SELECT watermark FROM outbox_watermark WHERE id = 1",
            [],
            |row| row.get(0),
        )?)
    }
}

impl WorkLogReader {
    /// Read the current consumer watermark.
    pub fn watermark(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT watermark FROM outbox_watermark WHERE id = 1",
            [],
            |row| row.get(0),
        )?)
    }

    /// Read all runs by id; helper for W5 timeline builds.
    pub fn runs(&self) -> Result<Vec<WorkRun>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw_json FROM runs ORDER BY started_at")?;
        let rows = stmt.query_map([], |row| {
            let raw: String = row.get(0)?;
            serde_json::from_str::<WorkRun>(&raw).map_err(|e| {
                SqliteError::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Read all events; helper for W5 timeline builds.
    pub fn events(&self) -> Result<Vec<WorkEvent>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw_json FROM events ORDER BY occurred_at")?;
        let rows = stmt.query_map([], |row| {
            let raw: String = row.get(0)?;
            serde_json::from_str::<WorkEvent>(&raw).map_err(|e| {
                SqliteError::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Read usage observations; W2 owns projection. Helper exposed for tests
    /// and W2 only.
    pub fn usage_observations(&self) -> Result<Vec<RunUsageSample>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw_json FROM usage_observations ORDER BY started_at")?;
        let rows = stmt.query_map([], |row| {
            let raw: String = row.get(0)?;
            serde_json::from_str::<RunUsageSample>(&raw).map_err(|e| {
                SqliteError::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn validate_batch(records: &[CommittedRecord]) -> Result<(), StoreError> {
    let mut last_seq: Option<i64> = None;
    let mut keys_seen = std::collections::HashSet::new();
    for record in records {
        if record.schema_version != crate::work_log::types::VERSION {
            return Err(StoreError::Invalid(format!(
                "schema version mismatch: record {} has {}, expected {}",
                record.source_key,
                record.schema_version,
                crate::work_log::types::VERSION
            )));
        }
        if record.sequence <= 0 || last_seq.is_some_and(|previous| record.sequence <= previous) {
            return Err(StoreError::Invalid(
                "work-log source sequences must increase".into(),
            ));
        }
        last_seq = Some(record.sequence);
        if !keys_seen.insert(record.source_key.as_str()) {
            return Err(StoreError::Invalid(format!(
                "duplicate source_key in batch: {}",
                record.source_key
            )));
        }
    }
    Ok(())
}

fn content_digest(record: &CommittedRecord) -> Result<String, StoreError> {
    let payload =
        serde_json::to_vec(&record.record).map_err(|e| StoreError::Schema(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

fn seen_source(
    conn: &Connection,
    source_key: &str,
) -> Result<Option<(i64, u32, String)>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT sequence,schema_version,content_sha256 FROM seen_sources WHERE source_key=?1",
            params![source_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

fn record_seen_source(
    conn: &Connection,
    source_key: &str,
    sequence: i64,
    schema_version: u32,
    content_sha256: &str,
    applied_at: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO seen_sources (source_key, sequence, schema_version, content_sha256, applied_at)
            VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source_key, sequence, schema_version as i64, content_sha256, applied_at],
    )?;
    Ok(())
}

fn apply_record(conn: &Connection, record: &CommittedRecord, now: i64) -> Result<(), StoreError> {
    let run_id = match &record.record {
        WorkRecord::Run(r) => Some(r.run_id.as_str()),
        WorkRecord::Context(r) => Some(r.run_id.as_str()),
        WorkRecord::Event(r) => Some(r.run_id.as_str()),
        WorkRecord::Usage(r) => Some(r.run_id.as_str()),
        WorkRecord::Interval(r) => Some(r.run_id.as_str()),
        WorkRecord::Gap(r) => r.run_id.as_deref(),
    };
    if let Some(run) = run_id {
        if conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM deleted_runs WHERE run_id=?1)",
            [run],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
    }
    match &record.record {
        WorkRecord::Run(run) => apply_run(conn, run, &record.source_key, record.sequence, now)?,
        WorkRecord::Event(event) => {
            insert_event(conn, event, &record.source_key, record.sequence, now)?
        }
        WorkRecord::Context(ctx) => {
            let version = next_context_version(conn, &ctx.context_ref)?;
            insert_context_ref(conn, ctx, version)?;
        }
        WorkRecord::Usage(sample) => {
            insert_sample(conn, sample, &record.source_key, record.sequence, now)?
        }
        WorkRecord::Gap(gap) => insert_gap(conn, gap)?,
        WorkRecord::Interval(interval) => {
            let duration = sqlite_metric(interval.duration_ms)?;
            let previous:Option<String>=conn.query_row("SELECT raw_json FROM work_intervals WHERE run_id=?1 ORDER BY started_at DESC LIMIT 1",[&interval.run_id],|r|r.get(0)).optional()?;
            let previous = previous
                .and_then(|s| serde_json::from_str::<crate::work_log::WorkInterval>(&s).ok());
            if let Some(mut prior) = previous.filter(|p| {
                p.ended_at == interval.started_at
                    && p.kind == interval.kind
                    && p.coverage == "observed"
                    && interval.coverage == "observed"
            }) {
                prior.ended_at = interval.ended_at;
                prior.duration_ms = prior
                    .duration_ms
                    .checked_add(interval.duration_ms)
                    .ok_or_else(|| StoreError::Invalid("intervalOverflow".into()))?;
                let duration = sqlite_metric(prior.duration_ms)?;
                conn.execute("UPDATE work_intervals SET ended_at=?2,duration_ms=?3,raw_json=?4 WHERE interval_id=?1",params![prior.interval_id,prior.ended_at,duration,serde_json::to_string(&prior).map_err(|_|StoreError::Invalid("invalidInterval".into()))?])?;
            } else {
                let raw = serde_json::to_string(interval)
                    .map_err(|_| StoreError::Invalid("invalidInterval".into()))?;
                conn.execute("INSERT INTO work_intervals(interval_id,run_id,kind,started_at,ended_at,duration_ms,coverage,raw_json)VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![interval.interval_id,interval.run_id,interval.kind,interval.started_at,interval.ended_at,duration,interval.coverage,raw])?;
            }
        }
    }
    Ok(())
}

fn apply_run(
    conn: &Connection,
    run: &WorkRun,
    source_key: &str,
    sequence: i64,
    now: i64,
) -> Result<(), StoreError> {
    let raw = serde_json::to_string(run).map_err(|e| StoreError::Schema(e.to_string()))?;
    let existing_ended: Option<Option<i64>> = conn
        .query_row(
            "SELECT ended_at FROM runs WHERE run_id = ?1",
            params![run.run_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok();
    if let Some(prev_ended) = existing_ended {
        // prev_ended is Some(Some(ts)) when the existing run is terminal.
        if prev_ended.is_some() && run.ended_at.is_none() {
            // Monotone terminal: a terminal run cannot revert to running.
            return Err(StoreError::Invalid(format!(
                "run terminal-to-running rejected: {}",
                run.run_id
            )));
        }
        // Same-row update from a fresh source_key; preserve prior identity
        // columns where the new record omits them.
        conn.execute(
            "UPDATE runs SET source_key = ?8,
                    ended_at = COALESCE(?2, ended_at),
                    elapsed_ms = COALESCE(?3, elapsed_ms),
                    outcome = COALESCE(?4, outcome),
                    observed_at = ?5,
                    coverage = ?6,
                    raw_json = ?7, observed_model = ?9,
                    context_ref = ?10
                WHERE run_id = ?1",
            params![
                run.run_id,
                run.ended_at,
                run.elapsed_ms.map(sqlite_metric).transpose()?,
                run.outcome,
                run.observed_at,
                run.coverage,
                raw,
                source_key,
                run.observed_model,
                run.context_ref,
            ],
        )?;
    } else {
        conn.execute(
            "INSERT OR IGNORE INTO runs
                (run_id, source_key, task_id, project_id, agent_id, host_id,
                 backend_session_id, parent_run_id, parent_task_id, context_ref,
                 title, project_name, agent_name, host_name, harness,
                 configured_model, observed_model, remote, started_at,
                 observed_at, ended_at, elapsed_ms, outcome, coverage, raw_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                        ?23, ?24, ?25)",
            params![
                run.run_id,
                source_key,
                run.task_id,
                run.project_id,
                run.agent_id,
                run.host_id,
                run.backend_session_id,
                run.parent_run_id,
                run.parent_task_id,
                run.context_ref,
                run.title,
                run.project_name,
                run.agent_name,
                run.host_name,
                run.harness,
                run.configured_model,
                run.observed_model,
                run.remote as i64,
                run.started_at,
                run.observed_at,
                run.ended_at,
                run.elapsed_ms.map(sqlite_metric).transpose()?,
                run.outcome,
                run.coverage,
                raw,
            ],
        )?;
    }
    let _ = sequence;
    let _ = now;
    Ok(())
}

fn insert_event(
    conn: &Connection,
    event: &WorkEvent,
    source_key: &str,
    sequence: i64,
    _now: i64,
) -> Result<(), StoreError> {
    let raw = serde_json::to_string(event).map_err(|e| StoreError::Schema(e.to_string()))?;
    let labels_json =
        serde_json::to_string(&event.labels).map_err(|e| StoreError::Schema(e.to_string()))?;
    conn.execute(
        "INSERT OR IGNORE INTO events
            (event_id, run_id, context_ref, kind, occurred_at, observed_at, phase,
             summary, plan_id, plan_item_id, labels_json, source, coverage,
             source_key, sequence, raw_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16)",
        params![
            event.event_id,
            event.run_id,
            event.context_ref,
            event.kind,
            event.occurred_at,
            event.observed_at,
            event.phase,
            event.summary,
            event.plan_id,
            event.plan_item_id,
            labels_json,
            event.source,
            event.coverage,
            source_key,
            sequence,
            raw,
        ],
    )?;
    Ok(())
}

fn insert_sample(
    conn: &Connection,
    sample: &RunUsageSample,
    source_key: &str,
    sequence: i64,
    _now: i64,
) -> Result<(), StoreError> {
    let raw = serde_json::to_string(sample).map_err(|e| StoreError::Schema(e.to_string()))?;
    let old: Option<String> = conn
        .query_row(
            "SELECT raw_json FROM usage_observations WHERE sample_id=?1",
            [&sample.sample_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        return if old == raw {
            Ok(())
        } else {
            Err(StoreError::Invalid(
                "usage sample identity changed content".into(),
            ))
        };
    }
    conn.execute(
        "INSERT OR IGNORE INTO usage_observations
            (sample_id, run_id, task_id, provider, configured_model, started_at,
             observed_at, final_sample, classification, provider_turn_id,
             input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
             reasoning_tokens, total_tokens, cost_usd, duration_ms,
             api_duration_ms, provider_turns, context_used, context_size,
             source_key, sequence, raw_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
        params![
            sample.sample_id,
            sample.run_id,
            sample.task_id,
            sample.provider,
            sample.configured_model,
            sample.started_at,
            sample.observed_at,
            sample.final_sample as i64,
            sample.classification,
            sample.provider_turn_id,
            sample.tokens.input,
            sample.tokens.output,
            sample.tokens.cache_read,
            sample.tokens.cache_write,
            sample.tokens.reasoning,
            sample.tokens.total,
            sample.cost_usd,
            sample.duration_ms,
            sample.api_duration_ms,
            sample.provider_turns,
            sample.context.as_ref().map(|c| c.used),
            sample.context.as_ref().map(|c| c.size),
            source_key,
            sequence,
            raw,
        ],
    )?;
    crate::work_log::usage::apply_committed(
        conn,
        &[crate::work_log::UsageObservation {
            sequence,
            sample: sample.clone(),
        }],
    )
    .map_err(StoreError::Invalid)?;
    Ok(())
}

fn insert_resources(
    conn: &mut Transaction<'_>,
    samples: &[ResourceSample],
) -> Result<usize, StoreError> {
    // Savepoint ensures partial inserts roll back on error; the outer
    // transaction still records the gap and dropped counter without claiming
    // durable success for the dropped rows.
    let mut sp = conn.savepoint()?;
    let mut written = 0usize;
    for sample in samples {
        let raw = serde_json::to_string(sample).map_err(|e| StoreError::Schema(e.to_string()))?;
        let changes = sp.execute(
            "INSERT INTO resource_samples
                (sample_id, run_id, sampled_at, interval_start, covered_ms,
                 cpu_delta_ms, rss_bytes, rss_byte_ms, sampled_peak_rss_bytes,
                 process_count, quality, raw_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                sample.sample_id,
                sample.run_id,
                sample.sampled_at,
                sample.interval_start,
                sqlite_metric(sample.covered_ms)?,
                sample.cpu_delta_ms.map(sqlite_metric).transpose()?,
                sample.rss_bytes.map(sqlite_metric).transpose()?,
                sample.rss_byte_ms.map(sqlite_metric).transpose()?,
                sample
                    .sampled_peak_rss_bytes
                    .map(sqlite_metric)
                    .transpose()?,
                sample.process_count as i64,
                sample.quality,
                raw,
            ],
        )?;
        written += changes;
    }
    sp.commit()?;
    Ok(written)
}

/// Trim `samples` to fit within the bounded resource queue. Returns the
/// accepted prefix, the dropped count, and the byte footprint of the accepted
/// prefix. Bytes are computed from the serialized JSON envelope.
fn sqlite_metric(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Invalid("resourceIntegerOverflow".into()))
}
fn bound_resources(
    samples: &[crate::work_log::types::ResourceSample],
    max_rows: usize,
    max_bytes: usize,
) -> (Vec<crate::work_log::types::ResourceSample>, u64, usize) {
    let mut accepted: Vec<crate::work_log::types::ResourceSample> =
        Vec::with_capacity(samples.len().min(max_rows));
    let mut bytes: usize = 0;
    let mut dropped: u64 = 0;
    for sample in samples {
        if accepted.len() >= max_rows {
            dropped += 1;
            continue;
        }
        let size = serde_json::to_string(sample).map(|s| s.len()).unwrap_or(0);
        if bytes + size > max_bytes {
            dropped += 1;
            continue;
        }
        bytes += size;
        accepted.push(sample.clone());
    }
    (accepted, dropped, bytes)
}

fn insert_gap(conn: &Connection, gap: &CollectorGap) -> Result<(), StoreError> {
    let dropped = i64::try_from(gap.dropped)
        .map_err(|_| StoreError::Invalid("droppedCountOverflow".into()))?;
    let raw = serde_json::to_string(gap).map_err(|e| StoreError::Schema(e.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO collector_gaps
            (gap_id, run_id, started_at, ended_at, reason, dropped, raw_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            gap.gap_id,
            gap.run_id,
            gap.started_at,
            gap.ended_at,
            gap.reason,
            dropped,
            raw,
        ],
    )?;
    Ok(())
}

fn insert_context_ref(conn: &Connection, ctx: &ContextRef, version: i64) -> Result<(), StoreError> {
    let raw = serde_json::to_string(ctx).map_err(|e| StoreError::Schema(e.to_string()))?;
    conn.execute(
        "INSERT INTO context_refs
            (context_ref, version, installation_id, profile_id, project_id,
             task_id, run_id, provider_turn_id, segment_start_id,
             source_message_id, through_message_id, title, agent_name, harness,
             native_session_id, availability, unavailable_reason, observed_at,
             source_key, raw_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            ctx.context_ref,
            version,
            ctx.installation_id,
            ctx.profile_id,
            ctx.project_id,
            ctx.task_id,
            ctx.run_id,
            ctx.provider_turn_id,
            ctx.segment_start_id,
            ctx.source_message_id,
            ctx.through_message_id,
            ctx.title,
            ctx.agent_name,
            ctx.harness,
            ctx.native_session_id,
            availability_label(&ctx.availability),
            ctx.unavailable_reason,
            schema::now_ms(),
            format!("ctx:{}-{}", ctx.context_ref, version),
            raw,
        ],
    )?;
    Ok(())
}

fn next_context_version(conn: &Connection, context_ref: &str) -> Result<i64, StoreError> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM context_refs WHERE context_ref = ?1",
        params![context_ref],
        |row| row.get(0),
    )?;
    Ok(next)
}

fn advance_watermark(conn: &Connection, value: i64, now: i64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE outbox_watermark SET watermark = ?1, updated_at = ?2 WHERE id = 1",
        params![value, now],
    )?;
    Ok(())
}

fn touch_health_write(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE health_state SET last_write_at = ?1, updated_at = ?1 WHERE id = 1",
        params![schema::now_ms()],
    )?;
    Ok(())
}

fn bump_dropped(conn: &Connection, count: u64) -> Result<(), StoreError> {
    let increment =
        i64::try_from(count).map_err(|_| StoreError::Invalid("droppedCountOverflow".into()))?;
    let current: i64 = conn.query_row(
        "SELECT dropped_resources FROM health_state WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let total = current
        .checked_add(increment)
        .ok_or_else(|| StoreError::Invalid("droppedCountOverflow".into()))?;
    conn.execute(
        "UPDATE health_state SET dropped_resources = ?1,
                updated_at = ?2 WHERE id = 1",
        params![total, schema::now_ms()],
    )?;
    Ok(())
}

fn availability_label(a: &crate::work_log::types::ContextAvailability) -> &'static str {
    use crate::work_log::types::ContextAvailability;
    match a {
        ContextAvailability::Available => "available",
        ContextAvailability::Deleted => "deleted",
        ContextAvailability::NotRecorded => "notRecorded",
        ContextAvailability::Unsupported => "unsupported",
        ContextAvailability::TemporarilyUnavailable => "temporarilyUnavailable",
    }
}

fn sanitized_code(err: &SqliteError) -> String {
    match err {
        SqliteError::SqliteFailure(code, _) => format!("sqlite:{:?}", code),
        SqliteError::ToSqlConversionFailure(_) => "sqlite:ToSqlConversionFailure".into(),
        _ => "sqlite:other".into(),
    }
}

fn sanitized_code_io(err: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => "io:NotFound".into(),
        ErrorKind::PermissionDenied => "io:PermissionDenied".into(),
        ErrorKind::AlreadyExists => "io:AlreadyExists".into(),
        ErrorKind::OutOfMemory => "io:OutOfMemory".into(),
        _ => "io:other".into(),
    }
}

// Internal guard: settings is a coordinator-owned handoff. This stand-in keeps
// the API surface honest; the coordinator wires the real source.
#[allow(dead_code)]
const _: &str = SETTINGS_DENY;

// Helper used by tests and writers to materialize the canonical raw JSON
// envelope shape. Public for fixture use; not part of the integration API.
#[doc(hidden)]
pub fn envelope_json(record: &CommittedRecord) -> Result<JsonValue, serde_json::Error> {
    let value = serde_json::to_value(&record.record)?;
    Ok(json!({
        "sequence": record.sequence,
        "source_key": record.source_key,
        "schema_version": record.schema_version,
        "record": value,
    }))
}
