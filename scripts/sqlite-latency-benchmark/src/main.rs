//! Isolated evidence for a possible future Monitter SQLite store.
//! This intentionally does not import user state or link to the app crate.

use rusqlite::{params, Connection};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EVENTS: usize = 53_000;
const MESSAGES: usize = 4_000;
const EVENT_BYTES: usize = 4_096;
const MESSAGE_BYTES: usize = 2_048;
const STREAMING_TRANSACTIONS: usize = 25;
const TASK_ID: &str = "synthetic-task";
const MIN_SQLITE: (u32, u32, u32) = (3, 51, 3);

struct BenchRoot(PathBuf);

impl Drop for BenchRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn percentile(values: &[Duration], percentile: f64) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[((sorted.len() - 1) as f64 * percentile).round() as usize]
}

fn version_at_least(version: &str, minimum: (u32, u32, u32)) -> bool {
    let mut values = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (values.next().unwrap_or(0), values.next().unwrap_or(0), values.next().unwrap_or(0)) >= minimum
}

fn file_bytes(path: &Path) -> u64 {
    [path.to_path_buf(), path.with_extension("db-wal"), path.with_extension("db-shm")]
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn payload(prefix: &str, index: usize, bytes: usize) -> String {
    let header = format!(r#"{{"id":"{prefix}-{index}","body":""#);
    let suffix = "\"}";
    let fill = bytes.saturating_sub(header.len() + suffix.len());
    format!("{header}{}{suffix}", "x".repeat(fill))
}

fn create_root() -> Result<BenchRoot, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Cannot create benchmark timestamp: {error}"))?
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "monitter-sqlite-latency-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root).map_err(|error| format!("Cannot create benchmark directory: {error}"))?;
    Ok(BenchRoot(root))
}

fn main() -> Result<(), String> {
    let verify_runtime_only = env::args().skip(1).any(|arg| arg == "--verify-runtime-only");
    let root = create_root()?;
    let database = root.0.join("state.db");
    let mut connection = Connection::open(&database)
        .map_err(|error| format!("Cannot open synthetic benchmark database: {error}"))?;
    let sqlite_version: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|error| format!("Cannot read bundled SQLite version: {error}"))?;
    if !version_at_least(&sqlite_version, MIN_SQLITE) {
        return Err(format!(
            "Bundled SQLite {sqlite_version} is older than required {}.{}.{}; benchmark aborted.",
            MIN_SQLITE.0, MIN_SQLITE.1, MIN_SQLITE.2
        ));
    }
    if verify_runtime_only {
        println!("sqlite-rusqlite=0.40.2 sqlite-runtime={sqlite_version} minimum=3.51.3 verified=true");
        return Ok(());
    }
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE tasks (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE messages (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id),
                created_at INTEGER NOT NULL, payload TEXT NOT NULL
             );
             CREATE INDEX messages_task_created ON messages(task_id, created_at DESC);
             CREATE TABLE events (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id),
                created_at INTEGER NOT NULL, payload TEXT NOT NULL
             );
             CREATE INDEX events_task_created ON events(task_id, created_at DESC);",
        )
        .map_err(|error| format!("Cannot create synthetic schema: {error}"))?;

    let import_started = Instant::now();
    {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Cannot begin synthetic import: {error}"))?;
        transaction
            .execute(
                "INSERT INTO tasks(id, payload) VALUES (?1, ?2)",
                params![TASK_ID, r#"{"title":"Synthetic benchmark task"}"#],
            )
            .map_err(|error| format!("Cannot insert synthetic task: {error}"))?;
        transaction
            .execute(
                "INSERT INTO meta(key, value) VALUES ('schema', 'entity-json-v1')",
                [],
            )
            .map_err(|error| format!("Cannot insert synthetic metadata: {error}"))?;
        for index in 0..MESSAGES {
            transaction
                .execute(
                    "INSERT INTO messages(id, task_id, created_at, payload) VALUES (?1, ?2, ?3, ?4)",
                    params![format!("message-{index}"), TASK_ID, index as i64, payload("message", index, MESSAGE_BYTES)],
                )
                .map_err(|error| format!("Cannot insert synthetic message: {error}"))?;
        }
        for index in 0..EVENTS {
            transaction
                .execute(
                    "INSERT INTO events(id, task_id, created_at, payload) VALUES (?1, ?2, ?3, ?4)",
                    params![format!("event-{index}"), TASK_ID, index as i64, payload("event", index, EVENT_BYTES)],
                )
                .map_err(|error| format!("Cannot insert synthetic event: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit synthetic import: {error}"))?;
    }
    let import_elapsed = import_started.elapsed();

    let mut streaming_times = Vec::with_capacity(STREAMING_TRANSACTIONS);
    for index in 0..STREAMING_TRANSACTIONS {
        let started = Instant::now();
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Cannot begin streaming transaction: {error}"))?;
        transaction
            .execute(
                "UPDATE messages SET payload = ?1 WHERE id = ?2",
                params![payload("stream-message", index, MESSAGE_BYTES), format!("message-{}", MESSAGES - 1)],
            )
            .map_err(|error| format!("Cannot update streaming message: {error}"))?;
        transaction
            .execute(
                "INSERT INTO events(id, task_id, created_at, payload) VALUES (?1, ?2, ?3, ?4)",
                params![format!("stream-event-{index}"), TASK_ID, (EVENTS + index) as i64, payload("stream-event", index, EVENT_BYTES)],
            )
            .map_err(|error| format!("Cannot insert streaming event: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit streaming transaction: {error}"))?;
        streaming_times.push(started.elapsed());
    }
    let database_bytes = file_bytes(&database);
    drop(connection);

    let reopened = Connection::open(&database)
        .map_err(|error| format!("Cannot reopen synthetic benchmark database: {error}"))?;
    let integrity: String = reopened
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("Cannot run integrity check: {error}"))?;
    if integrity != "ok" {
        return Err(format!("Synthetic benchmark integrity check failed: {integrity}"));
    }
    let event_count: i64 = reopened
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .map_err(|error| format!("Cannot count synthetic events: {error}"))?;
    let message_count: i64 = reopened
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .map_err(|error| format!("Cannot count synthetic messages: {error}"))?;
    if event_count != (EVENTS + STREAMING_TRANSACTIONS) as i64 || message_count != MESSAGES as i64 {
        return Err(format!("Synthetic row counts are wrong: events={event_count}, messages={message_count}"));
    }
    let latest_started = Instant::now();
    let latest = reopened
        .prepare("SELECT id FROM messages WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 20")
        .and_then(|mut statement| {
            statement
                .query_map([TASK_ID], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| format!("Cannot read indexed latest synthetic messages: {error}"))?;
    let latest_elapsed = latest_started.elapsed();
    if latest.len() != 20 {
        return Err(format!("Latest-message index returned {} rows, expected 20.", latest.len()));
    }
    println!(
        "sqlite-rusqlite=0.40.2 sqlite-runtime={sqlite_version} events={EVENTS} messages={MESSAGES} streaming_transactions={STREAMING_TRANSACTIONS} import={import_elapsed:?} streaming_p50={:?} streaming_p95={:?} latest20={latest_elapsed:?} bytes={database_bytes} integrity={integrity}",
        percentile(&streaming_times, 0.50),
        percentile(&streaming_times, 0.95),
    );
    Ok(())
}
