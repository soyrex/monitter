//! Work Log resource retention: bounded, restart-safe compaction from raw
//! samples to minute and hourly aggregate buckets.
//!
//! Frozen C0 v1 (W5 retention slice) — owned boundary. The parent owns
//! `schema.rs`, `mod.rs`, `types.rs`, `store.rs` and the runtime that hands
//! this module a `&mut Connection` to the single-writer work-log database.
//! This file does not touch those files.
//!
//! ## DDL
//!
//! `DDL` declares the two new tables — `resource_buckets` (cumulative
//! aggregates) and `resource_retention` (per-run compacted_through
//! watermark) — applied idempotently by `ensure_schema`. The DDL is
//! additive; it does not modify the v1 schema and does not depend on a
//! future user_version.
//!
//! ## Compaction model
//!
//! Completed raw samples whose `sampled_at` is at or before the raw cutoff
//! (`now - raw_retention_days`) are aggregated into minute buckets keyed
//! by `(run_id, width_ms=60_000, bucket_start=floor(interval_start/60_000))`.
//! Minute buckets older than the minute cutoff
//! (`now - minute_retention_days`) are rolled up into hourly buckets keyed
//! by `(run_id, width_ms=3_600_000, bucket_start=floor(bucket_start/3_600_000))`.
//!
//! `cpu_ms` and `rss_byte_ms` are summed; `peak_rss_bytes` is the maximum
//! observed; `covered_ms` is the sum of raw covered durations; `sample_count`
//! is the number of contributing samples; `missing_cpu` and `missing_rss`
//! count how many samples contributed `None` for those metrics. Buckets
//! preserve the actual `from_at`/`through_at` extent (min
//! `interval_start` and max `sampled_at` of contributing
//! samples) and disclose boundary coverage rather than prorating.
//!
//! ## Late-raw rejection
//!
//! The `resource_retention.compacted_through` watermark per run_id is the
//! highest raw `sampled_at` already folded into a bucket. A raw sample
//! whose `sampled_at <= compacted_through` and whose `interval_start` is
//! already within an aggregated window is rejected as a double-count
//! attempt. The parent wires that rejection path on raw insertion.
//! `compact` only advances the watermark; it does not refuse re-runs
//! because the source row is already gone after compaction.
//!
//! ## Restart safety
//!
//! `compact` is idempotent: re-running it on the same database after a
//! crash, replay, or interrupted tick produces no double totals. The
//! minute-bucket merge consumes selected raw rows and deletes them in the
//! same transaction. Hourly merge consumes selected minute buckets and
//! deletes them in that transaction. Reopen/retry sees only remaining source.
//!
//! ## Pressure
//!
//! When `pressure = true`, the tick compacts all completed raw observations
//! through `now`, even inside the normal raw retention window. The configured
//! minute-to-hour horizon is unchanged. The shared per-tick source-row budget
//! remains at most 1,000.
//!
//! ## Transaction shape
//!
//! One transaction per `compact` call. SAVEPOINTs are not used; the entire
//! tick is atomic. Rollback on any error leaves the database in its
//! pre-tick state. Existing and new buckets merge with checked integer
//! arithmetic in Rust before an `INSERT … ON CONFLICT` writes the result.

use crate::work_log::types::CollectionSettings;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::{BTreeSet, HashMap, HashSet};

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 3_600_000;

/// Hard cap on the per-tick aggregation batch. The brief limits it to 1000;
/// the parent passes `max_rows` in via `compact`, but the API also defends
/// against a runaway caller by clamping internally.
pub const MAX_BATCH_CAP: usize = 1_000;

/// Additive DDL for retention-owned tables. Idempotent. The DDL does not
/// touch any v1 table; it only creates `resource_buckets` and
/// `resource_retention`. Indexes serve keyset cursor reads (run_id +
/// bucket_start) and time-range queries.
pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS resource_buckets (
    run_id              TEXT NOT NULL,
    bucket_start        INTEGER NOT NULL,
    width_ms            INTEGER NOT NULL,
    from_at             INTEGER NOT NULL,
    through_at          INTEGER NOT NULL,
    covered_ms          INTEGER NOT NULL,
    cpu_ms              INTEGER,
    rss_byte_ms         INTEGER,
    peak_rss_bytes      INTEGER,
    sample_count        INTEGER NOT NULL,
    missing_cpu         INTEGER NOT NULL,
    missing_rss         INTEGER NOT NULL,
    quality_json        TEXT NOT NULL,
    PRIMARY KEY (run_id, width_ms, bucket_start)
);
CREATE INDEX IF NOT EXISTS resource_buckets_run_idx
    ON resource_buckets(run_id);
CREATE INDEX IF NOT EXISTS resource_buckets_from_idx
    ON resource_buckets(from_at);
CREATE INDEX IF NOT EXISTS resource_buckets_through_idx
    ON resource_buckets(through_at);
CREATE INDEX IF NOT EXISTS resource_buckets_width_idx
    ON resource_buckets(width_ms, bucket_start);
CREATE INDEX IF NOT EXISTS resource_samples_compact_idx
    ON resource_samples(sampled_at, sample_id);

CREATE TABLE IF NOT EXISTS resource_retention (
    run_id              TEXT PRIMARY KEY,
    compacted_through   INTEGER NOT NULL
);
"#;

/// Outcome of one compaction tick. Counts are rows affected in this call;
/// `remaining` is true when more rows are eligible for compaction on a
/// subsequent tick (so the caller can schedule another tick promptly).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub raw_to_minute_rows: u64,
    pub minute_to_hour_rows: u64,
    pub raw_rows_deleted: u64,
    pub minute_buckets_written: u64,
    pub hour_buckets_written: u64,
    pub runs_processed: u64,
    pub remaining: bool,
}

impl CompactionResult {
    pub fn is_empty(&self) -> bool {
        self.raw_to_minute_rows == 0
            && self.minute_to_hour_rows == 0
            && self.raw_rows_deleted == 0
            && self.minute_buckets_written == 0
            && self.hour_buckets_written == 0
    }

    pub fn merge(&mut self, other: &CompactionResult) -> Result<(), String> {
        self.raw_to_minute_rows = self
            .raw_to_minute_rows
            .checked_add(other.raw_to_minute_rows)
            .ok_or("compact: result overflow")?;
        self.minute_to_hour_rows = self
            .minute_to_hour_rows
            .checked_add(other.minute_to_hour_rows)
            .ok_or("compact: result overflow")?;
        self.raw_rows_deleted = self
            .raw_rows_deleted
            .checked_add(other.raw_rows_deleted)
            .ok_or("compact: result overflow")?;
        self.minute_buckets_written = self
            .minute_buckets_written
            .checked_add(other.minute_buckets_written)
            .ok_or("compact: result overflow")?;
        self.hour_buckets_written = self
            .hour_buckets_written
            .checked_add(other.hour_buckets_written)
            .ok_or("compact: result overflow")?;
        self.runs_processed = self
            .runs_processed
            .checked_add(other.runs_processed)
            .ok_or("compact: result overflow")?;
        self.remaining = self.remaining || other.remaining;
        Ok(())
    }
}

/// Idempotently apply the retention DDL. Safe to call on every tick; the
/// statements are `CREATE … IF NOT EXISTS`. Returns the number of
/// statements executed for diagnostics.
pub fn ensure_schema(conn: &mut Connection) -> Result<usize, String> {
    let count = DDL
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .count();
    conn.execute_batch(DDL)
        .map_err(|e| format!("retention ddl: {}", e))?;
    Ok(count)
}

/// One bounded compaction tick. The caller owns the connection; this
/// function runs everything inside a single transaction and rolls back on
/// any error so partial progress is impossible. `max_rows` is the shared
/// per-tick cap across raw and minute source rows; it is clamped to
/// `MAX_BATCH_CAP`. Pressure compacts all completed raw observations through
/// `now` while retaining the configured minute-to-hour horizon.
pub fn compact(
    conn: &mut Connection,
    settings: &CollectionSettings,
    now: i64,
    pressure: bool,
    max_rows: usize,
) -> Result<CompactionResult, String> {
    let max_rows = max_rows.min(MAX_BATCH_CAP);
    if max_rows == 0 {
        return Err("compact: max_rows must be > 0".into());
    }
    // Under pressure, compact every completed observed raw interval. Minute
    // buckets still retain their configured horizon before hourly rollup.
    let raw_cutoff = if pressure {
        now
    } else {
        cutoff_raw(now, settings)
    };
    let minute_cutoff = cutoff_minute(now, settings);
    let tx = conn
        .transaction()
        .map_err(|e| format!("compact: open tx: {}", e))?;
    tx.execute_batch(DDL)
        .map_err(|_| "compact: retention schema unavailable")?;
    let mut result = CompactionResult::default();
    // Phase 1: raw -> minute buckets.
    let minute_summary = compact_raw_to_minute(&tx, raw_cutoff, max_rows, &mut result)?;
    // Phase 2: minute -> hour buckets.
    let remaining_budget = max_rows - minute_summary.rows_scanned as usize;
    let hour_summary = compact_minute_to_hour(&tx, minute_cutoff, remaining_budget, &mut result)?;
    // Roll forward the per-run compacted_through watermark so a future
    // raw insert with sampled_at <= the watermark is rejected by the
    // parent writer. We use the highest sampled_at observed in phase 1.
    advance_compacted_through(&tx, &minute_summary, &mut result)?;
    result.runs_processed = minute_summary
        .highest_sampled
        .keys()
        .chain(hour_summary.runs.iter())
        .collect::<HashSet<_>>()
        .len() as u64;
    result.remaining = has_remaining(&tx, raw_cutoff, minute_cutoff)?;
    tx.commit().map_err(|e| format!("compact: commit: {}", e))?;
    Ok(result)
}

fn has_remaining(
    tx: &rusqlite::Transaction<'_>,
    raw_cutoff: i64,
    minute_cutoff: i64,
) -> Result<bool, String> {
    let raw: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM resource_samples WHERE sampled_at<=?1 LIMIT 1)",
            [raw_cutoff],
            |r| r.get(0),
        )
        .map_err(|_| "compact: remaining raw unavailable")?;
    let minute:i64=tx.query_row("SELECT EXISTS(SELECT 1 FROM resource_buckets WHERE width_ms=?1 AND bucket_start<?2 AND through_at<=?2 LIMIT 1)",params![MINUTE_MS,minute_cutoff],|r|r.get(0)).map_err(|_|"compact: remaining minute unavailable")?;
    Ok(raw != 0 || minute != 0)
}

fn cutoff_raw(now: i64, settings: &CollectionSettings) -> i64 {
    // `raw_retention_days` is bounded by CollectionSettings::validate to
    // <= 365. We saturate rather than panic on absurd inputs.
    let days = settings.raw_retention_days.min(365) as i64;
    let ms = days.saturating_mul(86_400_000);
    now.saturating_sub(ms)
}

fn cutoff_minute(now: i64, settings: &CollectionSettings) -> i64 {
    let days = settings.minute_retention_days.min(3650) as i64;
    let ms = days.saturating_mul(86_400_000);
    now.saturating_sub(ms)
}

#[derive(Debug, Default)]
struct MinuteSummary {
    /// Highest raw `sampled_at` observed in this phase, per run_id.
    highest_sampled: HashMap<String, i64>,
    /// Number of raw rows scanned in this phase.
    rows_scanned: u64,
}

#[derive(Debug, Default)]
struct HourSummary {
    runs: HashSet<String>,
}

/// Phase 1: aggregate completed raw rows through `raw_cutoff` into minute
/// buckets. Bounded by `max_rows` raw rows per tick. Buckets are upserted
/// so prior committed partial buckets are merged in the same transaction.
/// Returns the per-run
/// highest sampled_at observed, which becomes the
/// `resource_retention.compacted_through` watermark.
fn compact_raw_to_minute(
    tx: &rusqlite::Transaction<'_>,
    raw_cutoff: i64,
    max_rows: usize,
    result: &mut CompactionResult,
) -> Result<MinuteSummary, String> {
    let mut summary = MinuteSummary::default();
    // Bounded scan: read at most `max_rows + 1` rows to detect overflow.
    let mut stmt = tx
        .prepare(
            "SELECT sample_id, run_id, interval_start, covered_ms,
                    cpu_delta_ms, rss_bytes, rss_byte_ms,
                    sampled_peak_rss_bytes, process_count, quality, sampled_at
                FROM resource_samples
                WHERE sampled_at <= ?1
                ORDER BY sampled_at, sample_id
                LIMIT ?2",
        )
        .map_err(|e| format!("compact: prepare raw: {}", e))?;
    let rows: Vec<RawRow> = stmt
        .query_map(params![raw_cutoff, (max_rows as i64) + 1], row_to_raw)
        .map_err(|e| format!("compact: scan raw: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("compact: read raw row: {}", e))?;
    drop(stmt);
    if rows.len() > max_rows {
        result.remaining = true;
    }
    let bounded: &[RawRow] = if rows.len() > max_rows {
        &rows[..max_rows]
    } else {
        &rows[..]
    };
    // Group by (run_id, bucket_start). The bucket_start is the minute
    // floor of interval_start; minute width is fixed.
    let mut buckets: HashMap<(String, i64), BucketAccumulator> = HashMap::new();
    for row in bounded {
        let bucket_start = floor_to_checked(row.interval_start, MINUTE_MS)?;
        let key = (row.run_id.clone(), bucket_start);
        let acc = buckets.entry(key).or_default();
        acc.fold(row)?;
        summary
            .highest_sampled
            .entry(row.run_id.clone())
            .and_modify(|v| {
                if row.sampled_at > *v {
                    *v = row.sampled_at;
                }
            })
            .or_insert(row.sampled_at);
    }
    summary.rows_scanned = bounded.len() as u64;
    // Upsert each bucket.
    for ((run_id, bucket_start), acc) in buckets.iter() {
        upsert_bucket(tx, run_id, *bucket_start, MINUTE_MS, acc)?;
    }
    result.minute_buckets_written = buckets.len() as u64;
    // Delete the raw rows we just folded. We delete by sample_id list.
    if !bounded.is_empty() {
        let ids: Vec<&str> = bounded.iter().map(|r| r.sample_id.as_str()).collect();
        delete_raws(tx, &ids)?;
        result.raw_rows_deleted = bounded.len() as u64;
    }
    result.raw_to_minute_rows = bounded.len() as u64;
    Ok(summary)
}

/// Phase 2: roll up minute buckets older than `minute_cutoff` into hourly
/// buckets. We group existing minute buckets by
/// `(run_id, hour_bucket_start)` and aggregate.
fn compact_minute_to_hour(
    tx: &rusqlite::Transaction<'_>,
    minute_cutoff: i64,
    max_rows: usize,
    result: &mut CompactionResult,
) -> Result<HourSummary, String> {
    let mut summary = HourSummary::default();
    if max_rows == 0 {
        return Ok(summary);
    }
    let mut stmt = tx
        .prepare(
            "SELECT run_id, bucket_start, from_at, through_at, covered_ms,
                    cpu_ms, rss_byte_ms, peak_rss_bytes, sample_count,
                    missing_cpu, missing_rss, quality_json
                FROM resource_buckets
                WHERE width_ms = ?1 AND bucket_start < ?2 AND through_at<=?2
                ORDER BY bucket_start, run_id
                LIMIT ?3",
        )
        .map_err(|e| format!("compact: prepare minute: {}", e))?;
    let minute_rows: Vec<MinuteRow> = stmt
        .query_map(
            params![MINUTE_MS, minute_cutoff, (max_rows as i64) + 1],
            |row| {
                Ok(MinuteRow {
                    run_id: row.get(0)?,
                    bucket_start: row.get(1)?,
                    from_at: row.get(2)?,
                    through_at: row.get(3)?,
                    covered_ms: row.get(4)?,
                    cpu_ms: row.get::<_, Option<i64>>(5)?,
                    rss_byte_ms: row.get::<_, Option<i64>>(6)?,
                    peak_rss_bytes: row.get::<_, Option<i64>>(7)?,
                    sample_count: row.get(8)?,
                    missing_cpu: row.get(9)?,
                    missing_rss: row.get(10)?,
                    quality_json: row.get(11)?,
                })
            },
        )
        .map_err(|e| format!("compact: scan minute: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("compact: read minute row: {}", e))?;
    drop(stmt);
    if minute_rows.len() > max_rows {
        result.remaining = true;
    }
    let bounded: &[MinuteRow] = if minute_rows.len() > max_rows {
        &minute_rows[..max_rows]
    } else {
        &minute_rows[..]
    };
    // Group minute rows by (run_id, hour_bucket_start).
    let mut buckets: HashMap<(String, i64), BucketAccumulator> = HashMap::new();
    for m in bounded {
        summary.runs.insert(m.run_id.clone());
        let hour_start = floor_to_checked(m.bucket_start, HOUR_MS)?;
        let key = (m.run_id.clone(), hour_start);
        let acc = buckets.entry(key).or_default();
        // Translate minute bucket into the same accumulator.
        acc.merge_from_minute(m)?;
    }
    // Upsert hourly buckets.
    for ((run_id, hour_start), acc) in buckets.iter() {
        upsert_bucket(tx, run_id, *hour_start, HOUR_MS, acc)?;
    }
    result.hour_buckets_written = buckets.len() as u64;
    result.minute_to_hour_rows = bounded.len() as u64;
    // Delete the minute bucket rows we just folded.
    if !bounded.is_empty() {
        delete_minute_buckets(tx, bounded)?;
    }
    Ok(summary)
}

fn advance_compacted_through(
    tx: &rusqlite::Transaction<'_>,
    minute_summary: &MinuteSummary,
    _result: &mut CompactionResult,
) -> Result<(), String> {
    // Upsert one row per run_id with the highest sampled_at seen in this
    // tick. The watermark is the highest sampled_at we folded into a
    // bucket; a future raw insert whose sampled_at <= watermark and whose
    // interval_start is already in a folded window is rejected by the
    // parent writer.
    let mut stmt = tx
        .prepare(
            "INSERT INTO resource_retention (run_id, compacted_through)
                VALUES (?1, ?2)
                ON CONFLICT(run_id) DO UPDATE
                SET compacted_through = MAX(compacted_through, excluded.compacted_through)",
        )
        .map_err(|e| format!("compact: prepare retention: {}", e))?;
    for (run_id, sampled) in minute_summary.highest_sampled.iter() {
        stmt.execute(params![run_id, sampled])
            .map_err(|e| format!("compact: upsert retention: {}", e))?;
    }
    Ok(())
}

#[derive(Debug)]
struct RawRow {
    sample_id: String,
    run_id: String,
    interval_start: i64,
    covered_ms: i64,
    cpu_delta_ms: Option<i64>,
    rss_bytes: Option<i64>,
    rss_byte_ms: Option<i64>,
    sampled_peak_rss_bytes: Option<i64>,
    process_count: i64,
    quality: String,
    sampled_at: i64,
}

fn row_to_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        sample_id: row.get(0)?,
        run_id: row.get(1)?,
        interval_start: row.get(2)?,
        covered_ms: row.get(3)?,
        cpu_delta_ms: row.get::<_, Option<i64>>(4)?,
        rss_bytes: row.get::<_, Option<i64>>(5)?,
        rss_byte_ms: row.get::<_, Option<i64>>(6)?,
        sampled_peak_rss_bytes: row.get::<_, Option<i64>>(7)?,
        process_count: row.get(8)?,
        quality: row.get(9)?,
        sampled_at: row.get(10)?,
    })
}

#[derive(Debug, Clone)]
struct MinuteRow {
    run_id: String,
    bucket_start: i64,
    from_at: i64,
    through_at: i64,
    covered_ms: i64,
    cpu_ms: Option<i64>,
    rss_byte_ms: Option<i64>,
    peak_rss_bytes: Option<i64>,
    sample_count: i64,
    missing_cpu: i64,
    missing_rss: i64,
    quality_json: String,
}

#[derive(Debug, Default, Clone)]
struct BucketAccumulator {
    from_at: i64,
    through_at: i64,
    covered_ms: i64,
    /// Accumulator for `cpu_delta_ms`. `None` until at least one sample
    /// contributes a value; samples with `None` increment `missing_cpu`.
    cpu_ms: Option<i64>,
    rss_byte_ms: Option<i64>,
    missing_cpu: i64,
    missing_rss: i64,
    /// `process_count` is summed; we track peak-of-`sampled_peak_rss_bytes`
    /// as the bucket peak (only present in raw rows).
    peak_rss_bytes: Option<i64>,
    sample_count: i64,
    /// Bounded unique labels, never nested serialized JSON strings.
    qualities: BTreeSet<String>,
}

impl BucketAccumulator {
    fn fold(&mut self, row: &RawRow) -> Result<(), String> {
        if row.covered_ms < 0
            || row.cpu_delta_ms.is_some_and(|v| v < 0)
            || row.rss_byte_ms.is_some_and(|v| v < 0)
            || row.sampled_peak_rss_bytes.is_some_and(|v| v < 0)
        {
            return Err("compact: invalid raw metric".into());
        }
        if row.sampled_at < row.interval_start {
            return Err("compact: invalid raw interval".into());
        }
        // `covered_ms` is measured coverage, not elapsed wall duration.
        // Preserve the original observation endpoints independently.
        let row_through = row.sampled_at;
        if self.sample_count == 0 {
            self.from_at = row.interval_start;
            self.through_at = row_through;
        } else {
            if row.interval_start < self.from_at {
                self.from_at = row.interval_start;
            }
            if row_through > self.through_at {
                self.through_at = row_through;
            }
        }
        self.covered_ms = checked_add(self.covered_ms, row.covered_ms)?;
        match row.cpu_delta_ms {
            Some(v) => {
                self.cpu_ms = checked_optional(self.cpu_ms, Some(v))?;
            }
            None => {
                self.missing_cpu = checked_add(self.missing_cpu, 1)?;
            }
        }
        match row.rss_byte_ms {
            Some(v) => {
                self.rss_byte_ms = checked_optional(self.rss_byte_ms, Some(v))?;
            }
            None => {
                self.missing_rss = checked_add(self.missing_rss, 1)?;
            }
        }
        match row.sampled_peak_rss_bytes {
            Some(v) => {
                let cur = self.peak_rss_bytes.unwrap_or(0);
                self.peak_rss_bytes = Some(cur.max(v));
            }
            None => {}
        }
        self.sample_count = checked_add(self.sample_count, 1)?;
        // We deliberately do not include process_count in the bucket
        // because the brief restricts the bucket schema to the documented
        // fields. process_count stays in the deleted raw; aggregate query
        // tooling should read it before compaction.
        let _ = row.process_count;
        self.add_quality(&row.quality);
        Ok(())
    }

    fn merge_from_minute(&mut self, m: &MinuteRow) -> Result<(), String> {
        if m.through_at < m.from_at
            || m.covered_ms < 0
            || m.sample_count <= 0
            || m.missing_cpu < 0
            || m.missing_rss < 0
            || m.cpu_ms.is_some_and(|v| v < 0)
            || m.rss_byte_ms.is_some_and(|v| v < 0)
            || m.peak_rss_bytes.is_some_and(|v| v < 0)
        {
            return Err("compact: invalid bucket metric".into());
        }
        if self.sample_count == 0 {
            self.from_at = m.from_at;
            self.through_at = m.through_at;
        } else {
            if m.from_at < self.from_at {
                self.from_at = m.from_at;
            }
            if m.through_at > self.through_at {
                self.through_at = m.through_at;
            }
        }
        self.covered_ms = checked_add(self.covered_ms, m.covered_ms)?;
        self.cpu_ms = checked_optional(self.cpu_ms, m.cpu_ms)?;
        self.rss_byte_ms = checked_optional(self.rss_byte_ms, m.rss_byte_ms)?;
        match m.peak_rss_bytes {
            Some(v) => {
                let cur = self.peak_rss_bytes.unwrap_or(0);
                self.peak_rss_bytes = Some(cur.max(v));
            }
            None => {}
        }
        self.sample_count = checked_add(self.sample_count, m.sample_count)?;
        self.missing_cpu = checked_add(self.missing_cpu, m.missing_cpu)?;
        self.missing_rss = checked_add(self.missing_rss, m.missing_rss)?;
        if !m.quality_json.is_empty() {
            if m.quality_json.len() > 4096 {
                self.add_quality("qualityOmitted");
            } else {
                match serde_json::from_str::<Vec<String>>(&m.quality_json) {
                    Ok(labels) => {
                        for label in labels {
                            self.add_quality(&label);
                        }
                    }
                    Err(_) => self.add_quality("invalidQuality"),
                }
            }
        }
        Ok(())
    }
    fn add_quality(&mut self, quality: &str) {
        self.add_quality_inner(quality, 0);
    }
    fn add_quality_inner(&mut self, quality: &str, depth: u8) {
        if quality.is_empty() {
            return;
        }
        if quality.starts_with('[') && quality.len() <= 4096 && depth < 3 {
            if let Ok(labels) = serde_json::from_str::<Vec<String>>(quality) {
                for label in labels {
                    self.add_quality_inner(&label, depth + 1);
                }
                return;
            }
        }
        if quality.len() > 64
            || quality.chars().any(char::is_control)
            || (self.qualities.len() >= 16 && !self.qualities.contains(quality))
        {
            self.qualities.insert("qualityOmitted".into());
        } else {
            self.qualities.insert(quality.into());
        }
    }
}

fn checked_add(a: i64, b: i64) -> Result<i64, String> {
    a.checked_add(b)
        .ok_or_else(|| "compact: metric overflow".into())
}
fn checked_optional(a: Option<i64>, b: Option<i64>) -> Result<Option<i64>, String> {
    match (a, b) {
        (Some(a), Some(b)) => Ok(Some(checked_add(a, b)?)),
        (Some(a), None) => Ok(Some(a)),
        (None, Some(b)) => Ok(Some(b)),
        (None, None) => Ok(None),
    }
}

fn upsert_bucket(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    bucket_start: i64,
    width_ms: i64,
    acc: &BucketAccumulator,
) -> Result<(), String> {
    let existing:Option<MinuteRow>=tx.query_row(
        "SELECT run_id,bucket_start,from_at,through_at,covered_ms,cpu_ms,rss_byte_ms,peak_rss_bytes,sample_count,missing_cpu,missing_rss,quality_json FROM resource_buckets WHERE run_id=?1 AND width_ms=?2 AND bucket_start=?3",
        params![run_id,width_ms,bucket_start],|row|Ok(MinuteRow{run_id:row.get(0)?,bucket_start:row.get(1)?,from_at:row.get(2)?,through_at:row.get(3)?,covered_ms:row.get(4)?,cpu_ms:row.get(5)?,rss_byte_ms:row.get(6)?,peak_rss_bytes:row.get(7)?,sample_count:row.get(8)?,missing_cpu:row.get(9)?,missing_rss:row.get(10)?,quality_json:row.get(11)?})
    ).optional().map_err(|_|"compact: read bucket unavailable")?;
    let mut merged = acc.clone();
    if let Some(existing) = existing {
        merged.merge_from_minute(&existing)?;
    }
    let quality_json =
        serde_json::to_string(&merged.qualities).map_err(|_| "compact: quality unavailable")?;
    // All arithmetic is checked in Rust before this single transactional
    // replacement. SQLite must never coerce an overflowing integer into REAL.
    tx.execute(
        "INSERT INTO resource_buckets
            (run_id, bucket_start, width_ms, from_at, through_at,
             covered_ms, cpu_ms, rss_byte_ms, peak_rss_bytes,
             sample_count, missing_cpu, missing_rss, quality_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(run_id, width_ms, bucket_start) DO UPDATE SET
                from_at=excluded.from_at,through_at=excluded.through_at,
                covered_ms=excluded.covered_ms,cpu_ms=excluded.cpu_ms,
                rss_byte_ms=excluded.rss_byte_ms,peak_rss_bytes=excluded.peak_rss_bytes,
                sample_count=excluded.sample_count,missing_cpu=excluded.missing_cpu,
                missing_rss=excluded.missing_rss,quality_json=excluded.quality_json",
        params![
            run_id,
            bucket_start,
            width_ms,
            merged.from_at,
            merged.through_at,
            merged.covered_ms,
            merged.cpu_ms,
            merged.rss_byte_ms,
            merged.peak_rss_bytes,
            merged.sample_count,
            merged.missing_cpu,
            merged.missing_rss,
            quality_json,
        ],
    )
    .map_err(|e| format!("compact: upsert bucket: {}", e))?;
    Ok(())
}

fn delete_raws(tx: &rusqlite::Transaction<'_>, ids: &[&str]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("DELETE FROM resource_samples WHERE sample_id IN ({placeholders})");
    tx.execute(&sql, params_from_iter(ids.iter().copied()))
        .map_err(|e| format!("compact: delete raws: {}", e))?;
    Ok(())
}

fn delete_minute_buckets(tx: &rusqlite::Transaction<'_>, rows: &[MinuteRow]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    // Delete by (run_id, bucket_start) pair; minute buckets are unique on
    // (run_id, width_ms=MINUTE_MS, bucket_start).
    let mut deletes: Vec<(String, i64)> = Vec::with_capacity(rows.len());
    for r in rows {
        deletes.push((r.run_id.clone(), r.bucket_start));
    }
    let mut stmt = tx
        .prepare(
            "DELETE FROM resource_buckets
                WHERE width_ms = ?1 AND run_id = ?2 AND bucket_start = ?3",
        )
        .map_err(|e| format!("compact: prepare delete minute: {}", e))?;
    for (run_id, bucket_start) in &deletes {
        stmt.execute(params![MINUTE_MS, run_id, bucket_start])
            .map_err(|e| format!("compact: delete minute: {}", e))?;
    }
    Ok(())
}

/// Floor an interval_start to the minute boundary.
pub fn floor_minute(interval_start: i64) -> i64 {
    floor_to(interval_start, MINUTE_MS)
}

/// Floor a minute bucket_start to the hour boundary.
pub fn floor_hour(minute_bucket_start: i64) -> i64 {
    floor_to(minute_bucket_start, HOUR_MS)
}

fn floor_to(value: i64, unit: i64) -> i64 {
    let unit = unit.max(1);
    let rem = value.rem_euclid(unit);
    value.saturating_sub(rem)
}
fn floor_to_checked(value: i64, unit: i64) -> Result<i64, String> {
    value
        .checked_sub(value.rem_euclid(unit))
        .ok_or_else(|| "compact: timestamp out of range".into())
}

/// Diagnostics: read a `resource_retention.compacted_through` watermark.
/// Used by the parent writer to reject late-raw inserts.
pub fn compacted_through(conn: &Connection, run_id: &str) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT compacted_through FROM resource_retention WHERE run_id = ?1")
        .map_err(|e| format!("retention lookup: {}", e))?;
    let rows = stmt
        .query_map(params![run_id], |row| row.get::<_, i64>(0))
        .map_err(|e| format!("retention lookup query: {}", e))?;
    let mut iter = rows;
    match iter.next() {
        Some(v) => Ok(Some(v.map_err(|e| format!("retention lookup row: {}", e))?)),
        None => Ok(None),
    }
}

/// Diagnostics helper: total number of `resource_samples` rows left.
pub fn raw_row_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM resource_samples", [], |row| {
        row.get(0)
    })
    .map_err(|e| format!("raw count: {}", e))
}

/// Diagnostics helper: total number of `resource_buckets` rows.
pub fn bucket_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM resource_buckets", [], |row| {
        row.get(0)
    })
    .map_err(|e| format!("bucket count: {}", e))
}

#[cfg(test)]
mod tests {
    //! Fixtures use real temporary SQLite databases (in-memory or tempdir).
    //! They never touch the live backend, the install, or any operator
    //! profile. `ensure_schema` is invoked directly; `schema::apply` is
    //! optional (the tests do not need the v1 application_id gate because
    //! they construct fresh in-memory databases).

    use super::*;
    use crate::work_log::schema;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tempdir() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "monitter-retention-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn open_memory_with_v1() -> Connection {
        // `:memory:` does not run the v1 application_id gate in production;
        // for fixtures we skip `schema::apply` (which checks the application
        // id) and only run ensure_schema so the retention tables exist.
        let conn = Connection::open_in_memory().unwrap();
        schema::apply_pragmas(&conn).unwrap();
        // Create the resource_samples table the production schema owns.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS resource_samples (
                sample_id           TEXT PRIMARY KEY,
                run_id              TEXT NOT NULL,
                sampled_at          INTEGER NOT NULL,
                interval_start      INTEGER NOT NULL,
                covered_ms          INTEGER NOT NULL,
                cpu_delta_ms        INTEGER,
                rss_bytes           INTEGER,
                rss_byte_ms         INTEGER,
                sampled_peak_rss_bytes INTEGER,
                process_count       INTEGER NOT NULL,
                quality             TEXT NOT NULL,
                raw_json            TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS resource_run_idx ON resource_samples(run_id);
             CREATE INDEX IF NOT EXISTS resource_sampled_idx ON resource_samples(sampled_at);",
        )
        .unwrap();
        conn
    }

    fn insert_raw(
        conn: &mut Connection,
        sample_id: &str,
        run_id: &str,
        interval_start: i64,
        covered_ms: i64,
        cpu_delta_ms: Option<i64>,
        rss_bytes: Option<i64>,
        rss_byte_ms: Option<i64>,
        sampled_peak_rss_bytes: Option<i64>,
        process_count: i64,
        quality: &str,
        sampled_at: i64,
    ) {
        conn.execute(
            "INSERT OR REPLACE INTO resource_samples
                (sample_id, run_id, sampled_at, interval_start, covered_ms,
                 cpu_delta_ms, rss_bytes, rss_byte_ms, sampled_peak_rss_bytes,
                 process_count, quality, raw_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, '')",
            params![
                sample_id,
                run_id,
                sampled_at,
                interval_start,
                covered_ms,
                cpu_delta_ms,
                rss_bytes,
                rss_byte_ms,
                sampled_peak_rss_bytes,
                process_count,
                quality,
            ],
        )
        .unwrap();
    }

    fn settings_with(raw_days: u32, minute_days: u32) -> CollectionSettings {
        CollectionSettings {
            enabled: true,
            raw_retention_days: raw_days,
            minute_retention_days: minute_days,
            soft_limit_bytes: 1024 * 1024 * 1024,
            hard_limit_bytes: 2 * 1024 * 1024 * 1024,
        }
    }

    /// Sanity: floor_minute and floor_hour are correct for negative and
    /// positive values.
    #[test]
    fn floor_boundaries() {
        assert_eq!(floor_minute(0), 0);
        assert_eq!(floor_minute(60_000), 60_000);
        assert_eq!(floor_minute(89_999), 60_000);
        assert_eq!(floor_minute(60_001), 60_000);
        assert_eq!(floor_hour(0), 0);
        assert_eq!(floor_hour(3_600_000), 3_600_000);
        assert_eq!(floor_hour(3_599_000), 0);
    }

    /// Phase 1: aggregate raw samples older than the raw cutoff into
    /// minute buckets, sum CPU, sum RSS byte-ms, max peak, count, and
    /// delete the raw rows.
    #[test]
    fn raw_to_minute_aggregates() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        // 30 days ago in ms.
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        // Three raw samples within a single minute, all older than the raw cutoff.
        for i in 0..3 {
            insert_raw(
                &mut conn,
                &format!("s-{i}"),
                "run-1",
                base + i * 10_000,
                5_000,
                Some(100),
                Some(1024),
                Some(1024 * 5_000),
                Some(2048 * (i as i64 + 1)),
                1,
                "ok",
                base + i * 10_000,
            );
        }
        let result = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert!(result.minute_buckets_written >= 1, "{:?}", result);
        assert_eq!(result.raw_rows_deleted, 3);
        assert_eq!(raw_row_count(&conn).unwrap(), 0);
        // Check bucket sums.
        let bucket: (i64, Option<i64>, Option<i64>, Option<i64>, i64) = conn
            .query_row(
                "SELECT covered_ms, cpu_ms, rss_byte_ms, peak_rss_bytes, sample_count
                    FROM resource_buckets
                    WHERE run_id = 'run-1' AND width_ms = 60000",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(bucket.0, 15_000, "covered_ms sum");
        assert_eq!(bucket.1, Some(300), "cpu sum");
        assert_eq!(bucket.2, Some(1024 * 5_000 * 3), "rss_byte_ms sum");
        assert_eq!(bucket.3, Some(2048 * 3), "peak max");
        assert_eq!(bucket.4, 3, "sample_count");
    }

    /// Phase 1: missing metrics. A sample with `cpu_delta_ms = None` and
    /// `rss_byte_ms = None` increments the missing counters without
    /// inventing values.
    #[test]
    fn raw_to_minute_missing_counted() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        insert_raw(
            &mut conn, "s-0", "run-x", base, 1000, None, None, None, None, 1, "ok", base,
        );
        insert_raw(
            &mut conn,
            "s-1",
            "run-x",
            base + 1_000,
            1000,
            Some(50),
            Some(64),
            Some(64 * 1_000),
            Some(128),
            1,
            "ok",
            base + 1_000,
        );
        let result = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert_eq!(result.raw_rows_deleted, 2);
        let row: (Option<i64>, Option<i64>, i64, i64) = conn
            .query_row(
                "SELECT cpu_ms, rss_byte_ms, missing_cpu, missing_rss
                    FROM resource_buckets WHERE run_id = 'run-x'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, Some(50), "cpu sum ignores missing");
        assert_eq!(row.1, Some(64 * 1_000), "rss sum ignores missing");
        assert_eq!(row.2, 1, "missing_cpu counted");
        assert_eq!(row.3, 1, "missing_rss counted");
    }

    /// Phase 2: minute -> hour aggregation. Three minute buckets within
    /// one hour roll up into one hour bucket with summed metrics.
    #[test]
    fn minute_to_hour_aggregates() {
        let mut conn = open_memory_with_v1();
        ensure_schema(&mut conn).unwrap();
        let now: i64 = 200_000_000_000;
        // Insert minute buckets directly so the test does not depend on
        // a 365-day-old timestamp setup.
        let hour_start: i64 = floor_hour(now - 366_i64 * 86_400_000);
        for i in 0..3 {
            let bucket_start = hour_start + i * 60_000;
            conn.execute(
                "INSERT INTO resource_buckets
                    (run_id, bucket_start, width_ms, from_at, through_at,
                     covered_ms, cpu_ms, rss_byte_ms, peak_rss_bytes,
                     sample_count, missing_cpu, missing_rss, quality_json)
                    VALUES (?1, ?2, 60000, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, '')",
                params![
                    "run-h",
                    bucket_start,
                    bucket_start,
                    bucket_start + 30_000,
                    10_000,
                    Some(50i64),
                    Some(1024 * 10_000),
                    Some(2048),
                    1i64,
                ],
            )
            .unwrap();
        }
        let result = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert_eq!(result.minute_to_hour_rows, 3);
        assert_eq!(result.hour_buckets_written, 1);
        // Minute buckets should be gone.
        let minutes_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_buckets WHERE width_ms = 60000",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(minutes_left, 0);
        let hour: (i64, Option<i64>, Option<i64>, i64) = conn
            .query_row(
                "SELECT covered_ms, cpu_ms, rss_byte_ms, sample_count
                    FROM resource_buckets
                    WHERE run_id = 'run-h' AND width_ms = 3600000",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(hour.0, 30_000);
        assert_eq!(hour.1, Some(150));
        assert_eq!(hour.2, Some(1024 * 10_000 * 3));
        assert_eq!(hour.3, 3);
    }

    /// Restart / replay safety: running compact twice on the same data
    /// does not double totals. After the first tick the raw rows are
    /// gone, so the second tick processes zero raw and produces no new
    /// bucket changes.
    #[test]
    fn restart_replay_no_double_count() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        for i in 0..5 {
            insert_raw(
                &mut conn,
                &format!("r-{i}"),
                "run-r",
                base + i * 10_000,
                2_000,
                Some(40),
                Some(512),
                Some(512 * 2_000),
                Some(256 * (i as i64 + 1)),
                1,
                "ok",
                base + i * 10_000,
            );
        }
        let first = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert_eq!(first.raw_rows_deleted, 5);
        let second = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert_eq!(second.raw_to_minute_rows, 0);
        assert_eq!(second.raw_rows_deleted, 0);
        let covered: i64 = conn
            .query_row(
                "SELECT covered_ms FROM resource_buckets
                    WHERE run_id = 'run-r' AND width_ms = 60000",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(covered, 10_000, "covered_ms must not double across replay");
    }

    /// Late-raw rejection: the parent contract is that a raw insert with
    /// `sampled_at <= compacted_through` for the run must be refused by
    /// the parent writer. Retention surfaces the watermark; it does not
    /// enforce the rejection. This test asserts the watermark is set to
    /// the highest observed sampled_at after compaction.
    #[test]
    fn compacted_through_advances() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        insert_raw(
            &mut conn,
            "s-0",
            "run-w",
            base,
            1000,
            Some(10),
            None,
            None,
            None,
            1,
            "ok",
            base,
        );
        insert_raw(
            &mut conn,
            "s-1",
            "run-w",
            base + 5_000,
            1_000,
            Some(20),
            None,
            None,
            None,
            1,
            "ok",
            base + 5_000,
        );
        compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        let wm = compacted_through(&conn, "run-w").unwrap();
        assert_eq!(wm, Some(base + 5_000));
    }

    /// Pressure: with `pressure = true`, the cutoff is widened so that
    /// raw rows inside the normal retention window can also be folded.
    #[test]
    fn pressure_widens_cutoff() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        // 25 days old: inside the 30-day raw retention window.
        let base: i64 = now - 25_i64 * 86_400_000;
        insert_raw(
            &mut conn,
            "p-0",
            "run-p",
            base,
            1000,
            Some(5),
            Some(64),
            Some(64 * 1000),
            Some(128),
            1,
            "ok",
            base,
        );
        // Without pressure: not compacted.
        let r1 = compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        assert_eq!(r1.raw_rows_deleted, 0);
        // Pressure includes completed observations through now.
        let r2 = compact(&mut conn, &settings_with(30, 365), now, true, 100).unwrap();
        assert_eq!(r2.raw_rows_deleted, 1);
    }

    /// Bounded batch: `max_rows` is enforced; over-limit rows stay for
    /// the next tick.
    #[test]
    fn bounded_batch_overflow() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        for i in 0..10 {
            insert_raw(
                &mut conn,
                &format!("b-{i}"),
                "run-b",
                base + i * 1_000,
                500,
                Some(5),
                Some(64),
                Some(64 * 500),
                Some(64),
                1,
                "ok",
                base + i * 1_000,
            );
        }
        let r1 = compact(&mut conn, &settings_with(30, 365), now, false, 3).unwrap();
        assert_eq!(r1.raw_rows_deleted, 3, "first tick bounded to 3 rows");
        assert!(r1.remaining, "remaining flag set when more rows eligible");
        let r2 = compact(&mut conn, &settings_with(30, 365), now, false, 3).unwrap();
        assert_eq!(r2.raw_rows_deleted, 3);
        let r3 = compact(&mut conn, &settings_with(30, 365), now, false, 3).unwrap();
        assert_eq!(r3.raw_rows_deleted, 3);
        let r4 = compact(&mut conn, &settings_with(30, 365), now, false, 3).unwrap();
        assert_eq!(r4.raw_rows_deleted, 1, "final tick takes the last row");
        assert!(!r4.remaining);
        assert_eq!(raw_row_count(&conn).unwrap(), 0);
    }

    /// Rollback: an injected SQL error mid-tick rolls back the whole
    /// transaction. We use a trigger to fire during the bucket upsert
    /// path; raw rows must still exist after the failed tick.
    #[test]
    fn rollback_injected_trigger() {
        let mut conn = open_memory_with_v1();
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        for i in 0..3 {
            insert_raw(
                &mut conn,
                &format!("x-{i}"),
                "run-x",
                base + i * 1_000,
                1_000,
                Some(10),
                Some(64),
                Some(64 * 1_000),
                Some(128),
                1,
                "ok",
                base + i * 1_000,
            );
        }
        // Inject a trigger that raises on the bucket upsert.
        ensure_schema(&mut conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER retention_rollback_trigger
                BEFORE INSERT ON resource_buckets
                BEGIN
                    SELECT RAISE(ABORT, 'injected');
                END;",
        )
        .unwrap();
        let r = compact(&mut conn, &settings_with(30, 365), now, false, 100);
        assert!(r.is_err(), "compact must propagate the error");
        // Raw rows still present: the entire tick was rolled back.
        assert_eq!(raw_row_count(&conn).unwrap(), 3);
        // No bucket was written.
        assert_eq!(bucket_count(&conn).unwrap(), 0);
    }

    /// Database reopen: compact, close, reopen, compact again. The
    /// schema must apply cleanly via `ensure_schema` on a fresh
    /// connection. Bucket and retention state survive.
    #[test]
    fn reopen_preserves_buckets() {
        let dir = tempdir();
        let path = dir.join("work-log.sqlite3");
        {
            let mut conn = Connection::open(&path).unwrap();
            schema::apply_pragmas(&conn).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS resource_samples (
                    sample_id TEXT PRIMARY KEY, run_id TEXT NOT NULL,
                    sampled_at INTEGER NOT NULL, interval_start INTEGER NOT NULL,
                    covered_ms INTEGER NOT NULL, cpu_delta_ms INTEGER,
                    rss_bytes INTEGER, rss_byte_ms INTEGER,
                    sampled_peak_rss_bytes INTEGER, process_count INTEGER NOT NULL,
                    quality TEXT NOT NULL, raw_json TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS resource_retention (
                    run_id TEXT PRIMARY KEY, compacted_through INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS resource_buckets (
                    run_id TEXT NOT NULL, bucket_start INTEGER NOT NULL,
                    width_ms INTEGER NOT NULL, from_at INTEGER NOT NULL,
                    through_at INTEGER NOT NULL, covered_ms INTEGER NOT NULL,
                    cpu_ms INTEGER, rss_byte_ms INTEGER, peak_rss_bytes INTEGER,
                    sample_count INTEGER NOT NULL, missing_cpu INTEGER NOT NULL,
                    missing_rss INTEGER NOT NULL, quality_json TEXT NOT NULL,
                    PRIMARY KEY (run_id, width_ms, bucket_start));",
            )
            .unwrap();
            let now: i64 = 100_000_000_000;
            let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
            insert_raw(
                &mut conn,
                "r-0",
                "run-p",
                base,
                1_000,
                Some(10),
                Some(64),
                Some(64_000),
                Some(128),
                1,
                "ok",
                base,
            );
            compact(&mut conn, &settings_with(30, 365), now, false, 100).unwrap();
        }
        // Reopen.
        let mut conn = Connection::open(&path).unwrap();
        schema::apply_pragmas(&conn).unwrap();
        ensure_schema(&mut conn).unwrap();
        // Bucket survives.
        let covered: i64 = conn
            .query_row(
                "SELECT covered_ms FROM resource_buckets
                    WHERE run_id = 'run-p' AND width_ms = 60000",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(covered, 1_000);
        // Watermark survives.
        let wm = compacted_through(&conn, "run-p").unwrap();
        assert_eq!(
            wm,
            Some(floor_minute(100_000_000_000 - 31_i64 * 86_400_000))
        );
    }

    /// `compact` with `max_rows = 0` is a contract violation.
    #[test]
    fn max_rows_zero_rejected() {
        let mut conn = open_memory_with_v1();
        let r = compact(&mut conn, &settings_with(30, 365), 100, false, 0);
        assert!(r.is_err());
    }

    /// `max_rows` greater than the cap is clamped to 1000.
    #[test]
    fn max_rows_clamped() {
        let mut conn = open_memory_with_v1();
        // Inserting 1001 raw rows; the tick must cap at 1000.
        let now: i64 = 100_000_000_000;
        let base: i64 = floor_minute(now - 31_i64 * 86_400_000);
        for i in 0..1001 {
            insert_raw(
                &mut conn,
                &format!("c-{i}"),
                "run-c",
                base + i,
                1,
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                1,
                "ok",
                base + i,
            );
        }
        let r = compact(&mut conn, &settings_with(30, 365), now, false, 1_000_000).unwrap();
        assert_eq!(r.raw_rows_deleted, 1_000);
        assert!(r.remaining);
    }

    /// `CompactionResult::merge` aggregates two partial results.
    #[test]
    fn compaction_result_merge() {
        let mut a = CompactionResult {
            raw_to_minute_rows: 3,
            raw_rows_deleted: 3,
            minute_buckets_written: 1,
            runs_processed: 1,
            remaining: true,
            ..Default::default()
        };
        let b = CompactionResult {
            minute_to_hour_rows: 5,
            hour_buckets_written: 1,
            remaining: false,
            ..Default::default()
        };
        a.merge(&b).unwrap();
        assert_eq!(a.raw_to_minute_rows, 3);
        assert_eq!(a.minute_to_hour_rows, 5);
        assert_eq!(a.hour_buckets_written, 1);
        assert!(a.remaining, "remaining ORs across merge");
    }

    #[test]
    fn same_minute_across_batches_keeps_known_and_missing() {
        let mut conn = open_memory_with_v1();
        let now = 100_000_000_000i64;
        let base = floor_minute(now - 31 * 86_400_000);
        insert_raw(
            &mut conn,
            "a",
            "run",
            base + 1000,
            1000,
            Some(10),
            None,
            Some(100),
            Some(20),
            1,
            "ok",
            base + 2000,
        );
        insert_raw(
            &mut conn,
            "b",
            "run",
            base + 3000,
            1000,
            None,
            None,
            None,
            None,
            1,
            "lost",
            base + 4000,
        );
        let a = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(a.raw_rows_deleted, 1);
        assert!(a.remaining);
        let b = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(b.raw_rows_deleted, 1);
        assert!(!b.remaining);
        let row:(i64,i64,i64,Option<i64>,Option<i64>,Option<i64>,i64,i64,i64,String)=conn.query_row("SELECT from_at,through_at,covered_ms,cpu_ms,rss_byte_ms,peak_rss_bytes,sample_count,missing_cpu,missing_rss,quality_json FROM resource_buckets WHERE run_id='run' AND width_ms=60000",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).unwrap();
        assert_eq!((row.0, row.1, row.2), (base + 1000, base + 4000, 2000));
        assert_eq!((row.3, row.4, row.5), (Some(10), Some(100), Some(20)));
        assert_eq!((row.6, row.7, row.8), (2, 1, 1));
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&row.9).unwrap(),
            vec!["lost", "ok"]
        );
    }

    #[test]
    fn same_hour_across_batches_preserves_partial_totals_and_flat_quality() {
        let mut conn = open_memory_with_v1();
        ensure_schema(&mut conn).unwrap();
        let now = 200_000_000_000i64;
        let hour = floor_hour(now - 366 * 86_400_000);
        for (i, cpu, missing, quality) in [
            (0, Some(10i64), 0, "[\"[\\\"ok\\\"]\"]"),
            (1, None, 1, "[\"lost\"]"),
        ] {
            let start = hour + i * MINUTE_MS;
            conn.execute(
                "INSERT INTO resource_buckets VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    "run",
                    start,
                    MINUTE_MS,
                    start,
                    start + 1000,
                    1000,
                    cpu,
                    Some(100i64),
                    Some(20i64),
                    1i64,
                    missing,
                    0i64,
                    quality
                ],
            )
            .unwrap();
        }
        let a = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(a.minute_to_hour_rows, 1);
        assert!(a.remaining);
        let b = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(b.minute_to_hour_rows, 1);
        assert!(!b.remaining);
        let row:(i64,Option<i64>,i64,String)=conn.query_row("SELECT covered_ms,cpu_ms,missing_cpu,quality_json FROM resource_buckets WHERE run_id='run' AND width_ms=3600000",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!((row.0, row.1, row.2), (2000, Some(10), 1));
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&row.3).unwrap(),
            vec!["lost", "ok"]
        );
    }

    #[test]
    fn checked_overflow_rolls_back_source_and_existing_bucket() {
        let mut conn = open_memory_with_v1();
        ensure_schema(&mut conn).unwrap();
        let now = 100_000_000_000i64;
        let base = floor_minute(now - 31 * 86_400_000);
        conn.execute("INSERT INTO resource_buckets VALUES (?1,?2,60000,?2,?3,1,?4,NULL,NULL,1,0,1,'[\"ok\"]')",params!["run",base,base+1,i64::MAX]).unwrap();
        insert_raw(
            &mut conn,
            "new",
            "run",
            base + 1000,
            1000,
            Some(1),
            None,
            None,
            None,
            1,
            "ok",
            base + 2000,
        );
        assert!(compact(&mut conn, &settings_with(30, 365), now, false, 10).is_err());
        assert_eq!(raw_row_count(&conn).unwrap(), 1);
        let cpu: i64 = conn
            .query_row(
                "SELECT cpu_ms FROM resource_buckets WHERE run_id='run' AND width_ms=60000",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cpu, i64::MAX);
    }

    #[test]
    fn negative_timestamp_floor_does_not_panic_or_mutate() {
        assert_eq!(floor_minute(i64::MIN), i64::MIN);
        let mut conn = open_memory_with_v1();
        insert_raw(
            &mut conn,
            "edge",
            "run",
            i64::MIN,
            0,
            None,
            None,
            None,
            None,
            1,
            "ok",
            i64::MIN,
        );
        assert!(compact(&mut conn, &settings_with(0, 365), 0, true, 10).is_err());
        assert_eq!(raw_row_count(&conn).unwrap(), 1);
    }

    #[test]
    fn pressure_only_compacts_completed_observations_and_global_budget() {
        let mut conn = open_memory_with_v1();
        ensure_schema(&mut conn).unwrap();
        let now = 100_000_000_000i64;
        insert_raw(
            &mut conn,
            "future",
            "run",
            now - 1000,
            1000,
            Some(1),
            None,
            None,
            None,
            1,
            "ok",
            now + 1000,
        );
        let old = floor_hour(now - 366 * 86_400_000);
        conn.execute(
            "INSERT INTO resource_buckets VALUES (?1,?2,60000,?2,?3,1,NULL,NULL,NULL,1,1,1,'[]')",
            params!["old", old, old + 1],
        )
        .unwrap();
        let a = compact(&mut conn, &settings_with(30, 365), now, true, 1).unwrap();
        assert_eq!(a.raw_to_minute_rows, 0);
        assert_eq!(a.minute_to_hour_rows, 1);
        assert_eq!(raw_row_count(&conn).unwrap(), 1);
        conn.execute(
            "UPDATE resource_samples SET sampled_at=?1 WHERE sample_id='future'",
            [now],
        )
        .unwrap();
        let b = compact(&mut conn, &settings_with(30, 365), now, true, 1).unwrap();
        assert_eq!(b.raw_to_minute_rows, 1);
        assert_eq!(b.minute_to_hour_rows, 0);
    }

    #[test]
    fn raw_minute_hour_pipeline_preserves_totals_across_reopen_tick() {
        let mut conn = open_memory_with_v1();
        let first_now = 100_000_000_000i64;
        let base = floor_minute(first_now - 31 * 86_400_000);
        insert_raw(
            &mut conn,
            "a",
            "run",
            base,
            1000,
            Some(7),
            None,
            Some(70),
            Some(9),
            1,
            "ok",
            base + 1000,
        );
        let first = compact(&mut conn, &settings_with(30, 365), first_now, false, 10).unwrap();
        assert_eq!(
            (first.raw_to_minute_rows, first.minute_to_hour_rows),
            (1, 0)
        );
        let second_now = first_now + 366 * 86_400_000;
        let second = compact(&mut conn, &settings_with(30, 365), second_now, false, 10).unwrap();
        assert_eq!(
            (second.raw_to_minute_rows, second.minute_to_hour_rows),
            (0, 1)
        );
        let row:(i64,Option<i64>,Option<i64>,i64)=conn.query_row("SELECT covered_ms,cpu_ms,rss_byte_ms,sample_count FROM resource_buckets WHERE run_id='run' AND width_ms=3600000",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row, (1000, Some(7), Some(70), 1));
        assert_eq!(raw_row_count(&conn).unwrap(), 0);
        assert_eq!(bucket_count(&conn).unwrap(), 1);
        let third = compact(&mut conn, &settings_with(30, 365), second_now, false, 10).unwrap();
        assert!(third.is_empty());
        assert!(!third.remaining);
    }

    #[test]
    fn one_global_row_budget_covers_raw_and_minute_sources() {
        let mut conn = open_memory_with_v1();
        ensure_schema(&mut conn).unwrap();
        let now = 200_000_000_000i64;
        let raw_at = floor_minute(now - 31 * 86_400_000);
        insert_raw(
            &mut conn,
            "raw",
            "recent",
            raw_at,
            1000,
            Some(1),
            None,
            None,
            None,
            1,
            "ok",
            raw_at + 1000,
        );
        let old = floor_hour(now - 366 * 86_400_000);
        conn.execute(
            "INSERT INTO resource_buckets VALUES (?1,?2,60000,?2,?3,1,NULL,NULL,NULL,1,1,1,'[]')",
            params!["old", old, old + 1],
        )
        .unwrap();
        let first = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(first.raw_to_minute_rows + first.minute_to_hour_rows, 1);
        assert!(first.remaining);
        let second = compact(&mut conn, &settings_with(30, 365), now, false, 1).unwrap();
        assert_eq!(second.raw_to_minute_rows + second.minute_to_hour_rows, 1);
        assert!(!second.remaining);
    }

    #[test]
    fn quality_labels_are_unique_and_bounded() {
        let mut acc = BucketAccumulator::default();
        for i in 0..100 {
            let row = RawRow {
                sample_id: i.to_string(),
                run_id: "run".into(),
                interval_start: i,
                covered_ms: 1,
                cpu_delta_ms: None,
                rss_bytes: None,
                rss_byte_ms: None,
                sampled_peak_rss_bytes: None,
                process_count: 1,
                quality: format!("q{i}"),
                sampled_at: i + 1,
            };
            acc.fold(&row).unwrap();
        }
        assert!(acc.qualities.len() <= 17);
        assert!(acc.qualities.contains("qualityOmitted"));
        let before = acc.qualities.len();
        acc.add_quality("q0");
        assert_eq!(acc.qualities.len(), before);
    }
}
