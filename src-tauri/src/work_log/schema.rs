//! Private Work Log database schema. Additive, versioned, no destructive migrations.
//!
//! Storage contract (frozen C0 v1, 15 October 2026):
//!
//! * WAL with `synchronous=FULL` and `journal_size_limit` defaulted. The destination
//!   commit must be durable before any primary outbox acknowledgement; this file
//!   encodes that requirement and nothing here may lower it.
//! * Every durable row is keyed by the primary outbox `source_key`. Replay is
//!   idempotent through a UNIQUE(source_key) constraint combined with a
//!   separate `outbox_watermark` row that advances monotonically in the same
//!   transaction as the row writes.
//! * `runs` is monotone terminal: once a row has `ended_at IS NOT NULL` it
//!   must never be updated to a new state. Enforced in `store.rs`; this file
//!   only declares the columns.
//! * `context_refs` retains a snapshot per `(context_ref)`; later availability
//!   updates write a new row version (UNIQUE on context_ref, version) so a
//!   deleted or unsupported status becomes a tombstone without losing the
//!   historical snapshot.
//! * `resource_samples` is independent of the outbox. It has its own bounded
//!   queue and is dropped on overflow with a health gap; no durable-success
//!   claim is made for dropped rows.
//! * `usage_observations` stores the raw `RunUsageSample` payload; W2 owns
//!   projection and reads only what it needs.
//! * No command lines, environment, paths, transcript text or native session
//!   paths are stored in any resource row. `process_count` is the only
//!   process-derived attribute retained, alongside sanitised counters.
//! * `foreign_keys = ON` per the frozen C0 contract; no `REFERENCES` clauses
//!   are declared because events may legitimately arrive before their context
//!   snapshot. Referential integrity is enforced at query time.

use rusqlite::{params, Connection, Error as SqliteError, OpenFlags};
use std::fmt::{self, Display};
use std::path::PathBuf;

pub const SCHEMA_VERSION: u32 = 2;

/// Wrapper for schema-level errors. Allows non-SQLite errors to flow back to
/// callers without inventing synthetic SQL strings.
#[derive(Debug)]
pub struct SchemaError(pub String);
impl Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SchemaError {}
impl From<SchemaError> for SqliteError {
    fn from(err: SchemaError) -> Self {
        SqliteError::ToSqlConversionFailure(Box::new(err))
    }
}

/// "MWLG"; separate from the primary Monitter state database.
pub const APPLICATION_ID: i64 = 0x4d574c47;

pub const PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode = WAL",
    "PRAGMA synchronous = FULL",
    "PRAGMA foreign_keys = ON",
    "PRAGMA busy_timeout = 100",
    "PRAGMA temp_store = MEMORY",
    "PRAGMA wal_autocheckpoint = 256",
    "PRAGMA journal_size_limit = 4194304",
];

/// Default busy timeout in milliseconds used by the writer and reader.
pub const BUSY_TIMEOUT_MS: u32 = 100;

/// Apply the full schema if it has not yet been recorded. Safe to call on
/// every open; returns the post-migration `user_version`. Callers must have
/// already invoked `apply_pragmas` on the same connection; pragmas cannot be
/// executed inside a transaction.
pub fn apply(conn: &Connection) -> Result<u32, SqliteError> {
    validate_identity(conn)?;
    let tx = conn.unchecked_transaction()?;
    migrate(&tx)?;
    tx.commit()?;
    Ok(user_version(conn)?)
}

fn migrate(conn: &Connection) -> Result<(), SchemaError> {
    let current = user_version(conn).map_err(|e| SchemaError(e.to_string()))?;
    if current == 0 {
        v1(conn).map_err(|e| SchemaError(e.to_string()))?;
        conn.execute_batch(&format!("PRAGMA application_id = {APPLICATION_ID}"))
            .map_err(|e| SchemaError(e.to_string()))?;
    } else if current > SCHEMA_VERSION {
        return Err(SchemaError(format!(
            "work-log schema newer than supported: {}",
            current
        )));
    }
    if current < 2 {
        conn.execute_batch("CREATE TABLE work_intervals(interval_id TEXT PRIMARY KEY,run_id TEXT NOT NULL,kind TEXT NOT NULL,started_at INTEGER NOT NULL,ended_at INTEGER NOT NULL,duration_ms INTEGER NOT NULL,coverage TEXT NOT NULL,raw_json TEXT NOT NULL); CREATE INDEX intervals_run_time ON work_intervals(run_id,started_at); CREATE INDEX events_run_sequence ON events(run_id,sequence); CREATE INDEX runs_project_time ON runs(project_id,started_at); CREATE INDEX gaps_run_time ON collector_gaps(run_id,started_at); CREATE INDEX resource_run_start ON resource_samples(run_id,interval_start); CREATE INDEX events_sequence_id ON events(sequence,event_id);
            CREATE TABLE deleted_runs(run_id TEXT PRIMARY KEY,deleted_at INTEGER NOT NULL);")
            .map_err(|e| SchemaError(e.to_string()))?;
        conn.execute_batch(super::retention::DDL)
            .map_err(|e| SchemaError(e.to_string()))?;
        conn.execute("PRAGMA user_version = 2", [])
            .map_err(|e| SchemaError(e.to_string()))?;
    }
    Ok(())
}

/// Read-only preflight, called before WAL pragmas or schema creation. An empty
/// SQLite file is the only accepted uninitialised database. An existing work
/// log must carry our application ID and the core v1 layout.
pub fn validate_identity(conn: &Connection) -> Result<(), SqliteError> {
    let app_id: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let version = user_version(conn)?;
    let has_objects: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND type IN ('table','view','trigger','index'))",
        [], |row| row.get(0),
    )?;
    if version == 0 && app_id == 0 && !has_objects {
        return Ok(());
    }
    if !(1..=SCHEMA_VERSION).contains(&version) || app_id != APPLICATION_ID {
        return Err(SchemaError("foreign or unsupported work-log database".into()).into());
    }
    for (table, required) in [
        ("runs", &["run_id", "source_key", "raw_json"][..]),
        ("events", &["event_id", "source_key", "sequence"][..]),
        (
            "context_refs",
            &["context_ref", "version", "source_key"][..],
        ),
        (
            "usage_observations",
            &["sample_id", "source_key", "sequence"][..],
        ),
        ("resource_samples", &["sample_id", "raw_json"][..]),
        ("collector_gaps", &["gap_id", "raw_json"][..]),
        ("outbox_watermark", &["id", "watermark"][..]),
        (
            "seen_sources",
            &["source_key", "sequence", "schema_version", "content_sha256"][..],
        ),
        ("health_state", &["id", "state"][..]),
        ("settings", &["id", "enabled"][..]),
    ] {
        let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        if required.iter().any(|column| !actual.contains(*column)) {
            return Err(SchemaError(format!("invalid work-log table: {table}")).into());
        }
    }
    Ok(())
}

fn user_version(conn: &Connection) -> Result<u32, SqliteError> {
    conn.query_row("PRAGMA user_version", [], |row| {
        let v: i64 = row.get(0)?;
        Ok(v as u32)
    })
}

fn v1(conn: &Connection) -> Result<(), SqliteError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS runs (
            run_id              TEXT PRIMARY KEY,
            source_key          TEXT NOT NULL UNIQUE,
            task_id             TEXT NOT NULL,
            project_id          TEXT,
            agent_id            TEXT NOT NULL,
            host_id             TEXT NOT NULL,
            backend_session_id  TEXT NOT NULL,
            parent_run_id       TEXT,
            parent_task_id      TEXT,
            context_ref         TEXT NOT NULL,
            title               TEXT NOT NULL,
            project_name        TEXT,
            agent_name          TEXT NOT NULL,
            host_name           TEXT NOT NULL,
            harness             TEXT NOT NULL,
            configured_model    TEXT,
            observed_model      TEXT,
            remote              INTEGER NOT NULL,
            started_at          INTEGER NOT NULL,
            observed_at         INTEGER NOT NULL,
            ended_at            INTEGER,
            elapsed_ms          INTEGER,
            outcome             TEXT,
            coverage            TEXT NOT NULL,
            raw_json            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS runs_task_idx ON runs(task_id);
        CREATE INDEX IF NOT EXISTS runs_project_idx ON runs(project_id);
        CREATE INDEX IF NOT EXISTS runs_started_idx ON runs(started_at);

        CREATE TABLE IF NOT EXISTS events (
            event_id            TEXT PRIMARY KEY,
            run_id              TEXT NOT NULL,
            context_ref         TEXT NOT NULL,
            kind                TEXT NOT NULL,
            occurred_at         INTEGER NOT NULL,
            observed_at         INTEGER NOT NULL,
            phase               TEXT,
            summary             TEXT,
            plan_id             TEXT,
            plan_item_id        TEXT,
            labels_json         TEXT NOT NULL DEFAULT '[]',
            source              TEXT NOT NULL,
            coverage            TEXT NOT NULL,
            source_key          TEXT NOT NULL UNIQUE,
            sequence            INTEGER NOT NULL,
            raw_json            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS events_run_idx ON events(run_id);
        CREATE INDEX IF NOT EXISTS events_kind_idx ON events(kind);
        CREATE INDEX IF NOT EXISTS events_occurred_idx ON events(occurred_at);
        CREATE INDEX IF NOT EXISTS events_seq_idx ON events(sequence);

        CREATE TABLE IF NOT EXISTS context_refs (
            context_ref         TEXT NOT NULL,
            version             INTEGER NOT NULL,
            installation_id     TEXT NOT NULL,
            profile_id          TEXT NOT NULL,
            project_id          TEXT,
            task_id             TEXT NOT NULL,
            run_id              TEXT NOT NULL,
            provider_turn_id    TEXT,
            segment_start_id    TEXT,
            source_message_id   TEXT,
            through_message_id  TEXT,
            title               TEXT NOT NULL,
            agent_name          TEXT NOT NULL,
            harness             TEXT NOT NULL,
            native_session_id   TEXT,
            availability        TEXT NOT NULL,
            unavailable_reason  TEXT,
            observed_at         INTEGER NOT NULL,
            source_key          TEXT NOT NULL UNIQUE,
            raw_json            TEXT NOT NULL,
            PRIMARY KEY (context_ref, version)
        );
        CREATE INDEX IF NOT EXISTS context_refs_run_idx ON context_refs(run_id);
        CREATE INDEX IF NOT EXISTS context_refs_observed_idx ON context_refs(observed_at);

        CREATE TABLE IF NOT EXISTS usage_observations (
            sample_id           TEXT PRIMARY KEY,
            run_id              TEXT NOT NULL,
            task_id             TEXT NOT NULL,
            provider            TEXT NOT NULL,
            configured_model    TEXT,
            started_at          INTEGER NOT NULL,
            observed_at         INTEGER NOT NULL,
            final_sample        INTEGER NOT NULL,
            classification      TEXT NOT NULL,
            provider_turn_id    TEXT,
            input_tokens        INTEGER,
            output_tokens       INTEGER,
            cache_read_tokens   INTEGER,
            cache_write_tokens  INTEGER,
            reasoning_tokens    INTEGER,
            total_tokens        INTEGER,
            cost_usd            REAL,
            duration_ms         INTEGER,
            api_duration_ms     INTEGER,
            provider_turns      INTEGER,
            context_used        INTEGER,
            context_size        INTEGER,
            source_key          TEXT NOT NULL UNIQUE,
            sequence            INTEGER NOT NULL,
            raw_json            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_run_idx ON usage_observations(run_id);
        CREATE INDEX IF NOT EXISTS usage_started_idx ON usage_observations(started_at);

        CREATE TABLE IF NOT EXISTS usage_projection (
            scope_key TEXT PRIMARY KEY, run_id TEXT NOT NULL,
            state_json TEXT NOT NULL, totals_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_projection_run ON usage_projection(run_id);
        CREATE TABLE IF NOT EXISTS resource_samples (
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
        CREATE INDEX IF NOT EXISTS resource_sampled_idx ON resource_samples(sampled_at);

        CREATE TABLE IF NOT EXISTS collector_gaps (
            gap_id              TEXT PRIMARY KEY,
            run_id              TEXT,
            started_at          INTEGER NOT NULL,
            ended_at            INTEGER,
            reason              TEXT NOT NULL,
            dropped             INTEGER NOT NULL,
            raw_json            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS gaps_started_idx ON collector_gaps(started_at);

        CREATE TABLE IF NOT EXISTS outbox_watermark (
            id                  INTEGER PRIMARY KEY CHECK (id = 1),
            watermark           INTEGER NOT NULL DEFAULT 0,
            updated_at          INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO outbox_watermark (id, watermark, updated_at) VALUES (1, 0, 0);

        CREATE TABLE IF NOT EXISTS seen_sources (
            source_key          TEXT PRIMARY KEY,
            sequence            INTEGER NOT NULL,
            schema_version      INTEGER NOT NULL,
            content_sha256      TEXT NOT NULL,
            applied_at          INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS seen_sources_seq_idx ON seen_sources(sequence);

        CREATE TABLE IF NOT EXISTS health_state (
            id                  INTEGER PRIMARY KEY CHECK (id = 1),
            state               TEXT NOT NULL,
            reason              TEXT,
            last_write_at       INTEGER,
            pending_rows        INTEGER NOT NULL DEFAULT 0,
            pending_bytes       INTEGER NOT NULL DEFAULT 0,
            dropped_resources   INTEGER NOT NULL DEFAULT 0,
            database_bytes      INTEGER NOT NULL DEFAULT 0,
            wal_bytes           INTEGER NOT NULL DEFAULT 0,
            watermark           INTEGER NOT NULL DEFAULT 0,
            updated_at          INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO health_state (id, state, updated_at) VALUES (1, 'healthy', strftime('%s','now')*1000);

        CREATE TABLE IF NOT EXISTS settings (
            id                  INTEGER PRIMARY KEY CHECK (id = 1),
            enabled             INTEGER NOT NULL DEFAULT 0,
            raw_retention_days  INTEGER NOT NULL DEFAULT 30,
            minute_retention_days INTEGER NOT NULL DEFAULT 365,
            soft_limit_bytes    INTEGER NOT NULL DEFAULT 1073741824,
            hard_limit_bytes    INTEGER NOT NULL DEFAULT 2147483648,
            updated_at          INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO settings (id, updated_at) VALUES (1, strftime('%s','now')*1000);
        "#,
    )
}

/// Path used by `WorkLogDb::open`. The directory is the canonical per-installation
/// work-log root; the file name is fixed and includes no user-supplied data.
pub fn default_path(dir: &std::path::Path) -> PathBuf {
    dir.join("work-log.sqlite3")
}

/// Best-effort byte accounting for the on-disk database and its WAL. Used by
/// `WorkLogDb::health()` to drive soft/hard limit decisions.
pub fn database_bytes(path: &std::path::Path) -> (u64, u64) {
    let main = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let wal = std::fs::metadata(wal_path(path))
        .map(|m| m.len())
        .unwrap_or(0);
    (main, wal)
}

fn wal_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push("-wal");
    std::path::PathBuf::from(s)
}

/// Apply pragmas that cannot run inside a transaction. Must be called before
/// `apply`. Repeated calls are safe.
pub fn apply_pragmas(conn: &Connection) -> Result<(), SqliteError> {
    if user_version(conn)? == 0 {
        conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL")?;
    }
    for pragma in PRAGMAS {
        conn.execute_batch(pragma)?;
    }
    Ok(())
}

/// Create the per-installation work-log directory with `0700` permissions.
/// Refuses to traverse a symlink at the leaf: if the directory already exists
/// the caller is expected to have validated it via `ensure_dir_no_follow`.
pub fn ensure_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if std::fs::symlink_metadata(dir).is_ok() {
        return ensure_dir_no_follow(dir);
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(dir)?;
    ensure_dir_no_follow(dir)?;
    Ok(())
}

/// Refuse to follow symlinks: the directory at `dir` must be a real directory
/// owned by the current user. Used as a defence-in-depth check before opening
/// the database file. Sidecars inherit the same protection because the
/// directory itself is private.
pub fn ensure_dir_no_follow(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(dir)?;
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "work-log dir is a symlink",
        ));
    }
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "work-log path is not a directory",
        ));
    }
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "work-log dir owner mismatch",
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Open the database file with `O_NOFOLLOW` semantics and `0600` permissions
/// on creation. The file is opened read-write because SQLite's WAL writer
/// must mutate it. Symlinks at the path are rejected; the parent directory
/// is validated by `ensure_dir`. When the file is created here it is forced
/// to `0600` before SQLite opens it, so the WAL/SHM sidecars inherit a
/// private parent directory and never appear world-readable. Existing files
/// have their mode re-tightened to `0600` to defend against prior drift.
pub fn open_writer_connection(path: &std::path::Path) -> Result<Connection, SqliteError> {
    secure_regular_file(path, true)
        .map_err(|e| SqliteError::ToSqlConversionFailure(Box::new(e)))?;
    harden_sidecars(path).map_err(|e| SqliteError::ToSqlConversionFailure(Box::new(e)))?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(conn)
}

/// Inspect through an `O_NOFOLLOW` descriptor before changing permissions.
/// Existing hardlinks, devices and FIFOs are rejected without modification.
fn secure_regular_file(path: &std::path::Path, create: bool) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    if !create
        && std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .mode(0o600);
    if create {
        opts.create(true);
    }
    let file = opts.open(path)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() }
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "work-log file is not private regular file",
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn harden_sidecars(path: &std::path::Path) -> std::io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        secure_regular_file(std::path::Path::new(&sidecar), false)?;
    }
    Ok(())
}

/// Open a read-only connection with `O_NOFOLLOW` semantics. Used by
/// `WorkLogDb::connection()` and by any future W5 query helpers. WAL readers
/// do not write to sidecars; the parent directory protection still applies.
pub fn open_reader_connection(path: &std::path::Path) -> Result<Connection, SqliteError> {
    secure_regular_file(path, false)
        .map_err(|e| SqliteError::ToSqlConversionFailure(Box::new(e)))?;
    harden_sidecars(path).map_err(|e| SqliteError::ToSqlConversionFailure(Box::new(e)))?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(conn)
}

/// Read the post-open pragma state for diagnostics. Returns the synchronous
/// level as recorded by SQLite.
pub fn pragma_synchronous(conn: &Connection) -> Result<i64, SqliteError> {
    conn.query_row("PRAGMA synchronous", [], |row| row.get(0))
}

/// Convenience: append a sanitized error code to the `health_state` table.
/// Used by the writer when a recoverable failure occurs.
pub fn record_health_error(conn: &Connection, code: &str) -> Result<(), SqliteError> {
    conn.execute(
        "UPDATE health_state SET state='degraded', reason = ?1, updated_at = ?2 WHERE id = 1",
        params![code, now_ms()],
    )?;
    Ok(())
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
