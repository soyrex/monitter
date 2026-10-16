//! Single-writer drain loop. Owns no threads; callers drive ticks. Commits
//! the destination transaction before acknowledging the primary outbox.

use crate::work_log::store::{StoreError, WorkLogDb};
use crate::work_log::types::{CommittedRecord, OutboxSource};

/// Default batch limit. Held below the primary outbox cap
/// (`OUTBOX_MAX_ROWS = 10_000`) so the writer never drains more than the
/// primary store can hold and the ack value never references records the
/// primary store does not yet consider pending.
pub const DEFAULT_BATCH_LIMIT: usize = 100;
pub const MAX_BATCH_LIMIT: usize = 100;

/// Reason codes emitted to `WorkLogDb::health()` when a recoverable drain
/// failure occurs. Limited to a fixed set so the public health surface never
/// sees raw SQLite messages or filesystem paths.
#[derive(Debug, Clone, Copy)]
pub enum DrainErrorCode {
    OutboxRead,
    ApplyBatch,
    Ack,
}

impl DrainErrorCode {
    pub fn as_code(self) -> &'static str {
        match self {
            DrainErrorCode::OutboxRead => "drain:outbox_read",
            DrainErrorCode::ApplyBatch => "drain:apply_batch",
            DrainErrorCode::Ack => "drain:ack",
        }
    }
}

#[derive(Debug)]
pub enum DrainError {
    OutboxRead(String),
    ApplyBatch(StoreError),
    Ack(String),
}

impl std::fmt::Display for DrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrainError::OutboxRead(m) => write!(f, "outbox read failed: {}", m),
            DrainError::ApplyBatch(e) => write!(f, "apply batch failed: {}", e),
            DrainError::Ack(m) => write!(f, "ack failed: {}", m),
        }
    }
}
impl std::error::Error for DrainError {}

/// Outcome of one drain tick. Returned by `Writer::drain_once` so callers can
/// decide whether to keep going, back off, or surface a degraded state.
#[derive(Debug, Default, Clone, Copy)]
pub struct DrainSummary {
    pub applied: usize,
    pub highest_sequence: i64,
    pub acknowledged_through: i64,
}

/// Writer owns no threads. `drain_once` performs one full pass: read pending
/// records, apply them in a single FULL transaction, acknowledge through the
/// highest durable sequence. The caller is responsible for pacing; there is no
/// built-in sleep or backoff.
pub struct Writer<'a> {
    db: &'a WorkLogDb,
    batch_limit: usize,
    last_error: Option<String>,
}

impl<'a> Writer<'a> {
    pub fn new(db: &'a WorkLogDb) -> Self {
        Self {
            db,
            batch_limit: DEFAULT_BATCH_LIMIT,
            last_error: None,
        }
    }

    pub fn with_batch_limit(mut self, limit: usize) -> Self {
        self.batch_limit = limit.clamp(1, MAX_BATCH_LIMIT);
        self
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// One drain pass. Reads up to `batch_limit` pending records, applies them
    /// atomically, then acknowledges through the highest sequence applied.
    pub fn drain_once<S: OutboxSource>(&mut self, source: &S) -> Result<DrainSummary, DrainError> {
        let pending = source
            .pending(self.batch_limit)
            .map_err(|e| DrainError::OutboxRead(e))?;
        if pending.is_empty() {
            self.last_error = None;
            return Ok(DrainSummary::default());
        }
        let applied = pending.len();
        let highest = match self.db.apply_batch(&pending) {
            Ok(seq) => seq,
            Err(err) => {
                let code = DrainErrorCode::ApplyBatch.as_code();
                self.last_error = Some(code.into());
                return Err(DrainError::ApplyBatch(err));
            }
        };
        source.acknowledge(highest).map_err(|e| {
            let code = DrainErrorCode::Ack.as_code();
            self.last_error = Some(code.into());
            DrainError::Ack(format!("{}:{}", code, e))
        })?;
        self.last_error = None;
        Ok(DrainSummary {
            applied,
            highest_sequence: highest,
            acknowledged_through: highest,
        })
    }

    /// Flush helper for shutdown paths. Drains until the source returns an
    /// empty pending list or a recoverable error fires. Bounded by a caller
    /// supplied max-ticks guard so shutdown cannot loop forever if the source
    /// keeps returning records (e.g. live outbox writes during shutdown).
    pub fn flush<S: OutboxSource>(
        &mut self,
        source: &S,
        max_ticks: usize,
    ) -> Result<DrainSummary, DrainError> {
        let mut total = DrainSummary::default();
        for _ in 0..max_ticks {
            match self.drain_once(source)? {
                s if s.is_empty() => return Ok(total),
                s => total.merge(s),
            }
        }
        Ok(total)
    }
}

impl DrainSummary {
    pub fn is_empty(&self) -> bool {
        self.applied == 0
    }

    pub fn merge(&mut self, other: DrainSummary) {
        self.applied += other.applied;
        self.highest_sequence = self.highest_sequence.max(other.highest_sequence);
        self.acknowledged_through = self.acknowledged_through.max(other.acknowledged_through);
    }
}

/// Trait extension: idempotent helper that the coordinator can call to push
/// a single committed record through the same path without going through the
/// batch flow. The caller still owns outbox acknowledgement semantics.
pub trait WriteOne {
    fn write_one(&mut self, record: CommittedRecord) -> Result<i64, StoreError>;
}

impl<'a> WriteOne for Writer<'a> {
    fn write_one(&mut self, record: CommittedRecord) -> Result<i64, StoreError> {
        self.db.apply_batch(std::slice::from_ref(&record))
    }
}

#[cfg(test)]
mod tests {
    //! G1 fixtures: real temporary SQLite databases, fault injection via
    //! in-memory fakes, never a live backend. The fake `OutboxSource`
    //! honours ack semantics: ack failure on commit succeeds leaves the
    //! batch replayable; ack succeeds without prior commit never advances
    //! the watermark (coordinator test, not storage).

    use super::*;
    use crate::work_log::schema;
    use crate::work_log::store::{envelope_json, StoreError, WorkLogDb};
    use crate::work_log::types::{
        CommittedRecord, OutboxSource, PendingRecord, ResourceSample, WorkEvent, WorkRecord,
        WorkRun,
    };
    use rusqlite::Connection;
    use std::sync::Mutex;

    /// Self-contained temp directory. Avoids a dev-dep on `tempfile` since the
    /// ownership boundary says I cannot add new dev-deps to the shared
    /// `Cargo.toml`. The dir is removed on drop.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mut p = std::env::temp_dir();
            p.push(format!(
                "monitter-worklog-preflight-{}-{}",
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FakeOutbox {
        rows: Vec<PendingRecord>,
        last_ack: Mutex<i64>,
        fail_ack: bool,
    }
    impl FakeOutbox {
        fn new(rows: Vec<PendingRecord>) -> Self {
            Self {
                rows,
                last_ack: Mutex::new(0),
                fail_ack: false,
            }
        }
    }
    impl OutboxSource for FakeOutbox {
        fn pending(&self, limit: usize) -> Result<Vec<CommittedRecord>, String> {
            // Honour the last ack: drop rows whose primary sequence has been
            // acknowledged so the source behaves like a real primary outbox.
            let ack = *self.last_ack.lock().unwrap();
            let mut out = Vec::new();
            for (i, p) in self.rows.iter().take(limit).enumerate() {
                let seq = (i as i64) + 1;
                if seq <= ack {
                    continue;
                }
                out.push(CommittedRecord {
                    sequence: seq,
                    source_key: p.source_key.clone(),
                    schema_version: 1,
                    record: p.record.clone(),
                });
            }
            Ok(out)
        }
        fn acknowledge(&self, through_sequence: i64) -> Result<(), String> {
            if self.fail_ack {
                return Err("ack_failure".into());
            }
            *self.last_ack.lock().unwrap() = through_sequence;
            Ok(())
        }
    }

    fn fixture_run(id: &str) -> WorkRun {
        WorkRun {
            run_id: id.into(),
            task_id: format!("task-{id}"),
            project_id: Some("project".into()),
            agent_id: "agent".into(),
            host_id: "host".into(),
            backend_session_id: "session".into(),
            parent_run_id: None,
            parent_task_id: None,
            context_ref: format!("context-{id}"),
            title: "Fixture".into(),
            project_name: Some("Project".into()),
            agent_name: "Agent".into(),
            host_name: "Host".into(),
            harness: "codex".into(),
            configured_model: None,
            observed_model: None,
            cwd: None,
            repository_root: None,
            commit_sha: None,
            adapter_version: None,
            reasoning_setting: None,
            trigger: "unknown".into(),
            remote: false,
            started_at: 1000,
            observed_at: 1000,
            ended_at: None,
            elapsed_ms: None,
            outcome: None,
            coverage: "observed".into(),
        }
    }

    fn fixture_event(id: &str, run_id: &str) -> WorkEvent {
        WorkEvent {
            event_id: id.into(),
            run_id: run_id.into(),
            context_ref: format!("context-{run_id}"),
            kind: "phase".into(),
            occurred_at: 1100,
            observed_at: 1100,
            phase: Some("start".into()),
            summary: Some("start".into()),
            plan_id: None,
            plan_item_id: None,
            labels: vec!["seed".into()],
            source: "primary".into(),
            coverage: "observed".into(),
        }
    }

    fn fixture_resource(id: &str, run_id: &str) -> ResourceSample {
        ResourceSample {
            sample_id: id.into(),
            run_id: run_id.into(),
            sampled_at: 1200,
            interval_start: 1100,
            covered_ms: 100,
            cpu_delta_ms: Some(50),
            rss_bytes: Some(1024 * 1024),
            rss_byte_ms: Some(1024 * 1024 * 100),
            sampled_peak_rss_bytes: Some(2 * 1024 * 1024),
            process_count: 1,
            quality: "ok".into(),
        }
    }

    fn pending_for(record: WorkRecord, key: &str) -> PendingRecord {
        PendingRecord {
            source_key: key.into(),
            record,
        }
    }

    /// G1.a: open work log, drain a single run + event, reopen DB, recover
    /// exactly the same row.
    #[test]
    fn drain_then_reopen_roundtrip() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let outbox = FakeOutbox::new(vec![
            pending_for(WorkRecord::Run(fixture_run("run-1")), "primary:run-1"),
            pending_for(
                WorkRecord::Event(fixture_event("event-1", "run-1")),
                "primary:event-1",
            ),
        ]);
        let mut writer = Writer::new(&db);
        let summary = writer.drain_once(&outbox).unwrap();
        assert_eq!(summary.applied, 2);
        assert_eq!(summary.acknowledged_through, 2);
        assert_eq!(writer.last_error(), None);

        drop(db);
        let db2 = WorkLogDb::open(dir.path()).unwrap();
        let reader = db2.connection().unwrap();
        let runs = reader.runs().unwrap();
        let events = reader.events().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "event-1");
        assert_eq!(reader.watermark().unwrap(), 2);
    }

    /// G1.b: replay the same batch — writes must be idempotent and the
    /// watermark must remain at the same value, never regress. A source
    /// that always re-emits the same record simulates an ack-failed
    /// replay; the store must recognise every record as seen and ack
    /// through the highest offered sequence.
    #[test]
    fn duplicate_replay_idempotent() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let pending = vec![pending_for(
            WorkRecord::Run(fixture_run("run-1")),
            "primary:run-1",
        )];
        struct ReplaySource {
            rows: Vec<PendingRecord>,
        }
        impl OutboxSource for ReplaySource {
            fn pending(&self, limit: usize) -> Result<Vec<CommittedRecord>, String> {
                Ok(self
                    .rows
                    .iter()
                    .take(limit)
                    .enumerate()
                    .map(|(i, p)| CommittedRecord {
                        sequence: (i as i64) + 1,
                        source_key: p.source_key.clone(),
                        schema_version: 1,
                        record: p.record.clone(),
                    })
                    .collect())
            }
            fn acknowledge(&self, _: i64) -> Result<(), String> {
                Ok(())
            }
        }
        let source = ReplaySource { rows: pending };
        let mut writer = Writer::new(&db);
        let first = writer.drain_once(&source).unwrap();
        assert_eq!(first.highest_sequence, 1);
        let first_health = db.health().unwrap();
        assert_eq!(first_health.watermark, 1);
        let second = writer.drain_once(&source).unwrap();
        // Replay must no-op and ack through the highest offered sequence.
        assert_eq!(second.highest_sequence, 1);
        let reader = db.connection().unwrap();
        assert_eq!(reader.runs().unwrap().len(), 1);
        assert_eq!(reader.watermark().unwrap(), 1);
        let replay_health = db.health().unwrap();
        assert_eq!(replay_health.watermark, reader.watermark().unwrap());
        assert_eq!(replay_health.watermark, first_health.watermark);
    }

    /// G1.c: reader rejects deliberately corrupted raw JSON. This directly
    /// mutates a fixture row; it does not simulate a full disk or a crash.
    #[test]
    fn corrupt_envelope_rejected() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        // Seed the run legitimately.
        db.apply_batch(&[CommittedRecord {
            sequence: 1,
            source_key: "primary:run-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-1")),
        }])
        .unwrap();
        // Corrupt a fixture row through a separate SQLite connection; the
        // production API never writes malformed payloads.
        let bad = fixture_run("run-2");
        let fixture_connection = Connection::open(schema::default_path(dir.path())).unwrap();
        fixture_connection
            .execute(
                "INSERT OR REPLACE INTO runs
                (run_id, source_key, task_id, project_id, agent_id, host_id,
                 backend_session_id, parent_run_id, parent_task_id, context_ref,
                 title, project_name, agent_name, host_name, harness,
                 configured_model, observed_model, remote, started_at,
                 observed_at, ended_at, elapsed_ms, outcome, coverage, raw_json)
                VALUES (?1, 'corrupt-source-key', '', '', '', '', '', NULL, NULL,
                        '', '', '', '', '', '', NULL, NULL, 0, 0, 0, NULL,
                        NULL, NULL, '', 'not-json')",
                rusqlite::params![bad.run_id],
            )
            .unwrap();
        drop(fixture_connection);
        let reader = db.connection().unwrap();
        assert!(reader.runs().is_err());
    }

    /// G1.d: ack failure leaves the primary source pending. The destination
    /// watermark has advanced because its FULL commit succeeded; replay
    /// verifies source identity and returns the same receipt.
    #[test]
    fn ack_failure_replays() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let mut outbox = FakeOutbox::new(vec![pending_for(
            WorkRecord::Run(fixture_run("run-1")),
            "primary:run-1",
        )]);
        outbox.fail_ack = true;
        let mut writer = Writer::new(&db);
        let err = writer.drain_once(&outbox).unwrap_err();
        assert!(matches!(err, DrainError::Ack(_)));
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 1);
        assert_eq!(*outbox.last_ack.lock().unwrap(), 0);
        let health = db.health().unwrap();
        db.set_budget(health.database_bytes.saturating_add(health.wal_bytes));
        outbox.fail_ack = false;
        let second = writer.drain_once(&outbox).unwrap();
        assert_eq!(second.applied, 1);
        assert_eq!(second.acknowledged_through, 1);
    }

    /// G1.e: busy injection — hold a write transaction on a second connection,
    /// apply, expect serialised commit without corruption when the blocker
    /// releases. We read the current watermark and submit the next
    /// contiguous sequence once the lock is free.
    #[test]
    fn busy_injection_serialises() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let blocker = Connection::open_with_flags(
            schema::default_path(dir.path()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let result = db.apply_batch(&[CommittedRecord {
            sequence: 1,
            source_key: "primary:busy-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-1")),
        }]);
        match result {
            Ok(_) => {}
            Err(StoreError::Sqlite(_)) => {}
            Err(e) => panic!("unexpected error variant: {}", e),
        }
        blocker.execute_batch("ROLLBACK").unwrap();
        drop(blocker);
        // After the lock is free, the watermark is either 0 (BUSY rejected
        // the previous attempt) or 1 (the previous attempt succeeded).
        // Either way the next apply must succeed and remain consistent.
        let reader = db.connection().unwrap();
        let current = reader.watermark().unwrap();
        let next_seq = current + 1;
        let summary = db
            .apply_batch(&[CommittedRecord {
                sequence: next_seq,
                source_key: format!("primary:busy-{next_seq}"),
                schema_version: 1,
                record: WorkRecord::Run(fixture_run(&format!("run-{next_seq}"))),
            }])
            .unwrap();
        assert_eq!(summary, next_seq);
        assert_eq!(reader.watermark().unwrap(), next_seq);
    }

    /// G1.f: synthetic resource queue overflow, not a disk-full injection.
    #[test]
    fn resource_queue_overflow_records_gap() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        // Saturate with synthetic runs to trip the soft limit (test only —
        // production must use real byte measurements). We assert the writer
        // path stays alive and `health()` reports a sane state.
        let mut big: Vec<ResourceSample> = Vec::new();
        for i in 0..(super::super::types::RESOURCE_MAX_ROWS + 8) {
            big.push(fixture_resource(&format!("r-{i}"), "run-x"));
        }
        let written = db.append_resources(&big).unwrap();
        assert!(written <= super::super::types::RESOURCE_MAX_ROWS);
        let h = db.health().unwrap();
        assert!(h.state == "healthy" || h.state == "degraded");
        // Raw reason codes (if any) must come from the fixed sanitizer set.
        if let Some(reason) = h.reason.as_deref() {
            assert!(
                reason.starts_with("sqlite:")
                    || reason.starts_with("io:")
                    || reason.starts_with("drain:")
                    || reason == "resource_overflow"
            );
        }
    }

    /// G1.g: monotone terminal — once ended, a run cannot revert.
    #[test]
    fn terminal_run_is_monotone() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let mut ended = fixture_run("run-1");
        ended.ended_at = Some(2000);
        ended.elapsed_ms = Some(1000);
        db.apply_batch(&[CommittedRecord {
            sequence: 1,
            source_key: "primary:run-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(ended.clone()),
        }])
        .unwrap();
        let mut reverted = ended.clone();
        reverted.outcome = Some("pending".into());
        reverted.ended_at = None;
        let result = db.apply_batch(&[CommittedRecord {
            sequence: 2,
            source_key: "primary:run-1b".into(),
            schema_version: 1,
            record: WorkRecord::Run(reverted),
        }]);
        assert!(result.is_err(), "monotone terminal violation");
    }

    /// G1.g-replay: terminal replay with the same source_key no-ops.
    #[test]
    fn terminal_replay_noops() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let mut ended = fixture_run("run-1");
        ended.ended_at = Some(2000);
        db.apply_batch(&[CommittedRecord {
            sequence: 1,
            source_key: "primary:run-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(ended.clone()),
        }])
        .unwrap();
        let summary = db
            .apply_batch(&[CommittedRecord {
                sequence: 1,
                source_key: "primary:run-1".into(),
                schema_version: 1,
                record: WorkRecord::Run(ended),
            }])
            .unwrap();
        assert_eq!(summary, 1);
    }

    /// G1.h: the primary AUTOINCREMENT sequence may skip duplicate inserts.
    #[test]
    fn primary_sequence_gap_is_valid() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let result = db.apply_batch(&[CommittedRecord {
            sequence: 2,
            source_key: "primary:run-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-1")),
        }]);
        assert_eq!(result.unwrap(), 2);
        let reader = db.connection().unwrap();
        assert_eq!(reader.watermark().unwrap(), 2);
    }

    /// G1.i: schema version mismatch is rejected.
    #[test]
    fn schema_version_mismatch_rejected() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let result = db.apply_batch(&[CommittedRecord {
            sequence: 1,
            source_key: "primary:run-1".into(),
            schema_version: 99,
            record: WorkRecord::Run(fixture_run("run-1")),
        }]);
        assert!(result.is_err(), "schema version mismatch must be rejected");
    }

    /// G1.j: context_ref set/lookup round-trip.
    #[test]
    fn context_ref_lookup_roundtrip() {
        use crate::work_log::types::{ContextAvailability, ContextRef};
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let snap = ContextRef {
            context_ref: "ctx-1".into(),
            installation_id: "inst".into(),
            profile_id: "profile".into(),
            project_id: Some("project".into()),
            task_id: "task".into(),
            run_id: "run".into(),
            provider_turn_id: None,
            segment_start_id: None,
            source_message_id: None,
            through_message_id: None,
            through_message_text_bytes: None,
            title: "T".into(),
            agent_name: "A".into(),
            harness: "codex".into(),
            native_session_id: None,
            lineage_kind: None,
            lineage_root_task_id: None,
            source_task_id: None,
            source_through_message_id: None,
            availability: ContextAvailability::Available,
            unavailable_reason: None,
        };
        assert!(db.context_ref_lookup("ctx-1").unwrap().is_none());
        db.set_context_ref(&snap).unwrap();
        let found = db
            .context_ref_lookup("ctx-1")
            .unwrap()
            .expect("snapshot present");
        assert_eq!(found.context_ref, "ctx-1");
        assert_eq!(found.title, "T");
        // Re-snapshot with availability=NotRecorded becomes a tombstone row.
        let mut tomb = snap.clone();
        tomb.availability = ContextAvailability::Deleted;
        db.set_context_ref(&tomb).unwrap();
        let found2 = db
            .context_ref_lookup("ctx-1")
            .unwrap()
            .expect("tombstone present");
        assert_eq!(found2.availability, ContextAvailability::Deleted);
    }

    /// G1.k: directory hardening rejects symlinks.
    #[test]
    #[cfg(unix)]
    fn dir_symlink_rejected() {
        use std::os::unix::fs::symlink;
        let parent = TempDir::new();
        let real = parent.0.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = parent.0.join("link");
        symlink(&real, &link).unwrap();
        let result = WorkLogDb::open(&link);
        assert!(
            result.is_err(),
            "opening a symlinked work-log dir must fail"
        );
    }

    #[test]
    fn mixed_replay_and_autoincrement_gap_acknowledges_only_offered() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let record = |sequence, name: &str| CommittedRecord {
            sequence,
            source_key: format!("primary:{name}"),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run(name)),
        };
        db.apply_batch(&[record(1, "one"), record(3, "three")])
            .unwrap();
        // A replay-only page behind the destination watermark must not ack 3.
        assert_eq!(db.apply_batch(&[record(1, "one")]).unwrap(), 1);
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 3);
        assert_eq!(
            db.apply_batch(&[record(3, "three"), record(7, "seven")])
                .unwrap(),
            7
        );
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 7);
    }

    #[test]
    fn changed_or_invalid_replay_cannot_advance_watermark() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let original = CommittedRecord {
            sequence: 2,
            source_key: "source".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run")),
        };
        db.apply_batch(&[original.clone()]).unwrap();
        let mut changed = original.clone();
        if let WorkRecord::Run(run) = &mut changed.record {
            run.title = "changed".into();
        }
        assert!(db.apply_batch(&[changed]).is_err());
        let mut invalid_schema = original.clone();
        invalid_schema.schema_version = 99;
        assert!(db.apply_batch(&[invalid_schema]).is_err());
        let mut wrong_sequence = original.clone();
        wrong_sequence.sequence = 3;
        assert!(db.apply_batch(&[wrong_sequence]).is_err());
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 2);
        assert_eq!(db.connection().unwrap().runs().unwrap().len(), 1);
    }

    #[test]
    fn changed_replay_at_full_budget_is_rejected_without_mutation() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let original = CommittedRecord {
            sequence: 1,
            source_key: "source-full".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-full")),
        };
        db.apply_batch(&[original.clone()]).unwrap();
        let health = db.health().unwrap();
        db.set_budget(health.database_bytes.saturating_add(health.wal_bytes));
        let mut changed = original;
        if let WorkRecord::Run(run) = &mut changed.record {
            run.title = "different replay".into();
        }
        assert!(db.apply_batch(&[changed]).is_err());
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 1);
        let receipts: i64 = Connection::open(schema::default_path(dir.path()))
            .unwrap()
            .query_row("SELECT count(*) FROM seen_sources", [], |row| row.get(0))
            .unwrap();
        assert_eq!(receipts, 1);
    }

    #[test]
    fn mixed_replay_and_new_record_at_full_budget_is_atomic() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let first = CommittedRecord {
            sequence: 1,
            source_key: "source-one".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-one")),
        };
        db.apply_batch(&[first.clone()]).unwrap();
        let health = db.health().unwrap();
        db.set_budget(health.database_bytes.saturating_add(health.wal_bytes));
        let second = CommittedRecord {
            sequence: 2,
            source_key: "source-two".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-two")),
        };
        assert!(db.apply_batch(&[first, second]).is_err());
        let reader = db.connection().unwrap();
        assert_eq!(reader.watermark().unwrap(), 1);
        assert_eq!(reader.runs().unwrap().len(), 1);
    }

    #[test]
    fn resource_replay_is_not_counted_as_loss() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let sample = fixture_resource("same", "run");
        assert_eq!(db.append_resources(&[sample.clone()]).unwrap(), 1);
        let before = db.health().unwrap().dropped_resources;
        let health = db.health().unwrap();
        db.set_budget(health.database_bytes.saturating_add(health.wal_bytes));
        assert_eq!(
            db.append_resources(&[sample.clone(), sample.clone()])
                .unwrap(),
            0
        );
        assert_eq!(db.health().unwrap().dropped_resources, before);
        let mut changed = sample;
        changed.cpu_delta_ms = Some(99);
        assert!(db.append_resources(&[changed]).is_err());
    }

    #[test]
    fn compacted_old_resource_is_reported_as_ambiguous_without_readding_totals() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let path = schema::default_path(dir.path());
        Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO resource_retention(run_id, compacted_through) VALUES ('run', 1200)",
                [],
            )
            .unwrap();
        let sample = fixture_resource("compacted-late", "run");
        assert_eq!(db.append_resources(&[sample]).unwrap(), 0);
        let conn = Connection::open(&path).unwrap();
        let raw: i64 = conn
            .query_row("SELECT count(*) FROM resource_samples", [], |r| r.get(0))
            .unwrap();
        let reason: String = conn
            .query_row("SELECT reason FROM collector_gaps", [], |r| r.get(0))
            .unwrap();
        let dropped: i64 = conn
            .query_row(
                "SELECT dropped_resources FROM health_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            (raw, reason.as_str(), dropped),
            (0, "lateOrReplayedResource", 0)
        );
    }

    #[test]
    fn gap_u64_max_is_rejected_without_receipt_or_watermark() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let gap = crate::work_log::types::CollectorGap {
            gap_id: "gap-overflow".into(),
            run_id: None,
            started_at: 1,
            ended_at: Some(2),
            reason: "fixture".into(),
            dropped: u64::MAX,
        };
        let record = CommittedRecord {
            sequence: 1,
            source_key: "gap-overflow-source".into(),
            schema_version: 1,
            record: WorkRecord::Gap(gap.clone()),
        };
        assert!(db.apply_batch(&[record]).is_err());
        assert_eq!(db.connection().unwrap().watermark().unwrap(), 0);
        let conn = Connection::open(schema::default_path(dir.path())).unwrap();
        let receipts: i64 = conn
            .query_row("SELECT count(*) FROM seen_sources", [], |r| r.get(0))
            .unwrap();
        let gaps: i64 = conn
            .query_row("SELECT count(*) FROM collector_gaps", [], |r| r.get(0))
            .unwrap();
        assert_eq!((receipts, gaps), (0, 0));
    }

    #[test]
    fn dropped_counter_overflow_rolls_back_resource_rows_and_gap() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let path = schema::default_path(dir.path());
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE health_state SET dropped_resources=?1 WHERE id=1",
                [i64::MAX],
            )
            .unwrap();
        let rows: Vec<_> = (0..=super::super::types::RESOURCE_MAX_ROWS)
            .map(|i| fixture_resource(&format!("overflow-{i}"), "run"))
            .collect();
        assert!(db.append_resources(&rows).is_err());
        let conn = Connection::open(&path).unwrap();
        let persisted: i64 = conn
            .query_row("SELECT count(*) FROM resource_samples", [], |r| r.get(0))
            .unwrap();
        let gaps: i64 = conn
            .query_row("SELECT count(*) FROM collector_gaps", [], |r| r.get(0))
            .unwrap();
        let dropped: i64 = conn
            .query_row(
                "SELECT dropped_resources FROM health_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!((persisted, gaps, dropped), (0, 0, i64::MAX));
    }

    #[test]
    fn same_run_resumes_with_new_context_but_old_event_keeps_its_context() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let run = fixture_run("resumed-run");
        db.apply_batch(&[
            CommittedRecord {
                sequence: 1,
                source_key: "run-start".into(),
                schema_version: 1,
                record: WorkRecord::Run(run.clone()),
            },
            CommittedRecord {
                sequence: 2,
                source_key: "old-event".into(),
                schema_version: 1,
                record: WorkRecord::Event(fixture_event("event-old", "resumed-run")),
            },
        ])
        .unwrap();
        let mut resumed = run;
        resumed.context_ref = "context-resumed-segment".into();
        resumed.observed_at += 1000;
        db.apply_batch(&[CommittedRecord {
            sequence: 3,
            source_key: "run-resume".into(),
            schema_version: 1,
            record: WorkRecord::Run(resumed),
        }])
        .unwrap();
        let conn = Connection::open(schema::default_path(dir.path())).unwrap();
        let run_context: String = conn
            .query_row(
                "SELECT context_ref FROM runs WHERE run_id='resumed-run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event_context: String = conn
            .query_row(
                "SELECT context_ref FROM events WHERE event_id='event-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(run_context, "context-resumed-segment");
        assert_eq!(event_context, "context-resumed-run");
    }

    #[test]
    fn resource_partial_insert_fault_rolls_back_then_records_gap() {
        let dir = TempDir::new();
        let db = WorkLogDb::open(dir.path()).unwrap();
        let path = schema::default_path(dir.path());
        let fixture_connection = Connection::open(&path).unwrap();
        fixture_connection
            .execute_batch(
                "CREATE TRIGGER reject_bad_resource BEFORE INSERT ON resource_samples \
             WHEN NEW.sample_id='bad' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
            )
            .unwrap();
        drop(fixture_connection);
        let written = db
            .append_resources(&[
                fixture_resource("good", "run"),
                fixture_resource("bad", "run"),
            ])
            .unwrap();
        assert_eq!(written, 0);
        assert_eq!(db.health().unwrap().dropped_resources, 2);
        let check = Connection::open(&path).unwrap();
        let persisted: i64 = check
            .query_row("SELECT count(*) FROM resource_samples", [], |row| {
                row.get(0)
            })
            .unwrap();
        let gaps: i64 = check
            .query_row("SELECT count(*) FROM collector_gaps", [], |row| row.get(0))
            .unwrap();
        assert_eq!(persisted, 0);
        assert_eq!(gaps, 1);
    }

    #[test]
    fn foreign_database_rejected_before_wal_mutation() {
        let dir = TempDir::new();
        let path = schema::default_path(dir.path());
        let foreign = Connection::open(&path).unwrap();
        foreign
            .execute_batch("CREATE TABLE foreign_data(secret TEXT);")
            .unwrap();
        drop(foreign);
        assert!(WorkLogDb::open(dir.path()).is_err());
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        assert!(!std::path::Path::new(&wal).exists());
    }

    #[test]
    #[cfg(unix)]
    fn linked_sidecar_rejected_and_main_file_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = TempDir::new();
        let path = schema::default_path(dir.path());
        let target = dir.path().join("target");
        std::fs::write(&target, b"untouched").unwrap();
        let mut shm = path.as_os_str().to_owned();
        shm.push("-shm");
        symlink(&target, std::path::Path::new(&shm)).unwrap();
        assert!(WorkLogDb::open(dir.path()).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        std::fs::remove_file(std::path::Path::new(&shm)).unwrap();
        let db = WorkLogDb::open(dir.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(db);
    }

    #[test]
    #[cfg(unix)]
    fn hardlinked_sidecar_and_fifo_main_are_rejected_without_chmod() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let path = schema::default_path(dir.path());
        let target = dir.path().join("target");
        std::fs::write(&target, b"private fixture").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        std::fs::hard_link(&target, std::path::Path::new(&wal)).unwrap();
        assert!(WorkLogDb::open(dir.path()).is_err());
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        std::fs::remove_file(std::path::Path::new(&wal)).unwrap();
        std::fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o644) }, 0);
        assert!(WorkLogDb::open(dir.path()).is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    /// Sanity: the envelope JSON helper produces the documented shape and
    /// round-trips through the writer. Hidden doctest for the integration.
    #[test]
    fn envelope_shape() {
        let record = CommittedRecord {
            sequence: 7,
            source_key: "primary:run-1".into(),
            schema_version: 1,
            record: WorkRecord::Run(fixture_run("run-1")),
        };
        let v = envelope_json(&record).unwrap();
        assert_eq!(v["sequence"], 7);
        assert_eq!(v["source_key"], "primary:run-1");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["record"]["kind"], "run");
    }
}
