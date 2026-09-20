//! Transactional SQLite persistence primitives.
//!
//! Store owns migration, legacy import, recovery, and publication policy. This
//! module only stores a fully materialized state as individually queryable rows.

use crate::{attachments::StoredAttachment, model::*};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

const APPLICATION_ID: i64 = 0x4d4f4e54; // "MONT"
const SCHEMA_VERSION: i64 = 1;

pub(crate) type StateRef<'a> = (
    &'a Snapshot,
    &'a HashMap<String, Host>,
    &'a HashMap<String, StoredAttachment>,
);

pub(crate) struct Database {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl Database {
    /// Creates a fresh migration target. It refuses to touch an existing file.
    pub(crate) fn create(path: &Path) -> Result<Self, String> {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(format!(
                    "Refusing to replace existing SQLite store {}.",
                    path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect SQLite store {}: {error}",
                    path.display()
                ));
            }
        }
        validate_sidecars(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| "SQLite store path has no parent directory.".to_string())?;
        if !parent.is_dir() {
            return Err(format!(
                "SQLite parent directory does not exist: {}.",
                parent.display()
            ));
        }
        private_dir(parent)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        options
            .open(path)
            .map_err(|e| format!("Cannot create SQLite store {}: {e}", path.display()))?;
        if let Err(error) = private_file(path) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        let sqlite_path = match canonical_sqlite_path(path) {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_file(path);
                return Err(error);
            }
        };
        if let Err(error) = validate_sidecars(&sqlite_path) {
            let _ = fs::remove_file(path);
            return Err(error);
        }

        let result = (|| {
            let connection = Connection::open_with_flags(&sqlite_path, read_write_nofollow_flags())
                .map_err(|e| format!("Cannot open new SQLite store: {e}"))?;
            configure_connection(&connection)?;
            initialize_schema(&connection)?;
            private_sidecars(&sqlite_path)?;
            Ok(Self {
                connection: Mutex::new(connection),
                path: sqlite_path.clone(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    /// Opens only a complete, known schema. No DDL or header writes occur
    /// until the existing database has passed validation.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| format!("Cannot inspect SQLite store {}: {e}", path.display()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "SQLite store is not a regular file: {}.",
                path.display()
            ));
        }
        validate_sidecars(path)?;
        let sqlite_path = canonical_sqlite_path(path)?;
        validate_sidecars(&sqlite_path)?;
        let connection = Connection::open_with_flags(&sqlite_path, read_write_nofollow_flags())
            .map_err(|e| format!("Cannot open Monitter SQLite store: {e}"))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        validate_schema(&connection)?;
        configure_connection(&connection)?;
        private_file(&sqlite_path)?;
        private_sidecars(&sqlite_path)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: sqlite_path,
        })
    }

    pub(crate) fn replace_all(
        &self,
        state: StateRef<'_>,
        usage: &[RunUsageSample],
        captured_since: Option<i64>,
    ) -> Result<(), String> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM entities", [])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM usage_samples", [])
            .map_err(|e| e.to_string())?;
        write_state(&transaction, state)?;
        write_all_usage(&transaction, usage)?;
        set_capture_since(&transaction, captured_since)?;
        transaction
            .commit()
            .map_err(|e| format!("Cannot commit SQLite replacement: {e}"))?;
        private_sidecars(&self.path)
    }

    /// Commits the delta between two complete state views. It never writes an
    /// unchanged, retained row, and rolls back the entire delta on any error.
    pub(crate) fn apply_update(
        &self,
        before: StateRef<'_>,
        after: StateRef<'_>,
        usage_before: &[RunUsageSample],
        usage_after: &[RunUsageSample],
        captured_since: Option<i64>,
    ) -> Result<(), String> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        diff_state(&transaction, before, after)?;
        diff_usage(&transaction, usage_before, usage_after)?;
        set_capture_since_if_changed(&transaction, captured_since)?;
        transaction
            .commit()
            .map_err(|e| format!("Cannot commit SQLite update: {e}"))?;
        private_sidecars(&self.path)
    }

    pub(crate) fn read_snapshot(
        &self,
    ) -> Result<
        (
            Snapshot,
            HashMap<String, Host>,
            HashMap<String, StoredAttachment>,
            Vec<RunUsageSample>,
            Option<i64>,
        ),
        String,
    > {
        let connection = self.lock()?;
        let settings = get_meta::<Settings>(&connection, "settings")?
            .ok_or_else(|| "SQLite state is missing settings.".to_string())?;
        let snapshot = Snapshot {
            hosts: read_vec(&connection, "hosts")?,
            agents: read_vec(&connection, "agents")?,
            tasks: read_vec(&connection, "tasks")?,
            messages: read_vec(&connection, "messages")?,
            mail_batches: read_vec(&connection, "mail_batches")?,
            events: read_vec::<RunEvent>(&connection, "events")?
                .into_iter()
                .map(Arc::new)
                .collect(),
            channels: read_vec(&connection, "channels")?,
            projects: read_vec(&connection, "projects")?,
            settings,
            collaborations: read_vec(&connection, "collaborations")?,
            subagent_sessions: read_vec(&connection, "subagent_sessions")?,
            subagent_transcripts: read_map(&connection, "subagent_transcripts")?,
            queued_messages: read_vec(&connection, "queued_messages")?,
            approval_requests: read_vec(&connection, "approval_requests")?,
            approval_rules: read_vec(&connection, "approval_rules")?,
            schedules: read_vec(&connection, "schedules")?,
            schedule_runs: read_vec(&connection, "schedule_runs")?,
            pending_throwaway_task_ids: get_meta(&connection, "pending_throwaway_task_ids")?
                .unwrap_or_default(),
        };
        let captured_since = get_meta::<Option<i64>>(&connection, "capture_since")?.flatten();
        Ok((
            snapshot,
            read_map(&connection, "task_hosts")?,
            read_map(&connection, "attachments")?,
            read_usage(&connection)?,
            captured_since,
        ))
    }

    pub(crate) fn integrity_check(&self) -> Result<(), String> {
        let connection = self.lock()?;
        let result: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if result == "ok" {
            Ok(())
        } else {
            Err(format!("SQLite integrity check failed: {result}"))
        }
    }

    /// Durably folds the WAL into the database before a migration temp file is
    /// closed and atomically renamed by Store.
    pub(crate) fn checkpoint(&self) -> Result<(), String> {
        let connection = self.lock()?;
        let result: (i64, i64, i64) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("SQLite WAL checkpoint failed: {e}"))?;
        if result.0 != 0 {
            return Err(format!(
                "SQLite WAL checkpoint remained busy ({}).",
                result.0
            ));
        }
        private_sidecars(&self.path)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "SQLite store lock failed.".to_string())
    }
}

fn read_write_nofollow_flags() -> OpenFlags {
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW
}

/// SQLite's no-follow flag also rejects a symlinked ancestor. macOS exposes
/// its standard temporary directory through `/var`, so canonicalize only the
/// existing parent and retain the un-resolved database basename for no-follow.
fn canonical_sqlite_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "SQLite store path has no parent directory.".to_string())?;
    let filename = path
        .file_name()
        .ok_or_else(|| "SQLite store path has no filename.".to_string())?;
    fs::canonicalize(parent)
        .map_err(|e| {
            format!(
                "Cannot resolve SQLite parent directory {}: {e}",
                parent.display()
            )
        })
        .map(|parent| parent.join(filename))
}

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Cannot configure SQLite store: {e}"))
}

fn initialize_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID};
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE entities (
                 collection TEXT NOT NULL,
                 id TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 task_id TEXT,
                 created_at INTEGER,
                 payload TEXT NOT NULL,
                 PRIMARY KEY(collection, id)
             );
             CREATE INDEX entities_collection_position ON entities(collection, position);
             CREATE INDEX entities_task_created ON entities(collection, task_id, created_at DESC);
             CREATE TABLE usage_samples (
                 sample_id TEXT PRIMARY KEY,
                 position INTEGER NOT NULL,
                 payload TEXT NOT NULL
             );
             CREATE INDEX usage_samples_position ON usage_samples(position);
             INSERT INTO meta(key, value) VALUES('schema_version', '{SCHEMA_VERSION}');"
        ))
        .map_err(|e| format!("Cannot initialise SQLite schema: {e}"))
}

fn validate_schema(connection: &Connection) -> Result<(), String> {
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if application_id != APPLICATION_ID {
        return Err("SQLite store has an unknown application identifier.".to_string());
    }
    for (table, columns) in [
        ("meta", &["key", "value"][..]),
        (
            "entities",
            &[
                "collection",
                "id",
                "position",
                "task_id",
                "created_at",
                "payload",
            ][..],
        ),
        ("usage_samples", &["sample_id", "position", "payload"][..]),
    ] {
        validate_columns(connection, table, columns)?;
    }
    for index in [
        "entities_collection_position",
        "entities_task_created",
        "usage_samples_position",
    ] {
        let exists: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("SQLite store is missing required index {index}."));
        }
    }
    let version: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "SQLite store is missing schema version.".to_string())?;
    let version: i64 = version
        .parse()
        .map_err(|_| "SQLite store has an invalid schema version.".to_string())?;
    if version != SCHEMA_VERSION {
        return Err(format!(
            "Unsupported Monitter SQLite schema version {version}."
        ));
    }
    Ok(())
}

fn validate_columns(connection: &Connection, table: &str, required: &[&str]) -> Result<(), String> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut statement = connection.prepare(&pragma).map_err(|e| e.to_string())?;
    let columns: HashSet<String> = statement
        .query_map([], |row| row.get(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if required.iter().all(|column| columns.contains(*column)) {
        Ok(())
    } else {
        Err(format!("SQLite store has an incomplete {table} table."))
    }
}

fn write_state(transaction: &Transaction<'_>, state: StateRef<'_>) -> Result<(), String> {
    let (snapshot, task_hosts, attachments) = state;
    write_vec(transaction, "hosts", &snapshot.hosts, plain_id)?;
    write_vec(transaction, "agents", &snapshot.agents, plain_id)?;
    write_vec(transaction, "tasks", &snapshot.tasks, plain_id)?;
    write_vec(transaction, "messages", &snapshot.messages, message_key)?;
    write_vec(
        transaction,
        "mail_batches",
        &snapshot.mail_batches,
        mail_batch_key,
    )?;
    write_vec(transaction, "events", &snapshot.events, event_key)?;
    write_vec(transaction, "channels", &snapshot.channels, plain_id)?;
    write_vec(transaction, "projects", &snapshot.projects, plain_id)?;
    write_vec(
        transaction,
        "collaborations",
        &snapshot.collaborations,
        plain_id,
    )?;
    write_vec(
        transaction,
        "subagent_sessions",
        &snapshot.subagent_sessions,
        plain_id,
    )?;
    write_map(
        transaction,
        "subagent_transcripts",
        &snapshot.subagent_transcripts,
    )?;
    write_vec(
        transaction,
        "queued_messages",
        &snapshot.queued_messages,
        queued_message_key,
    )?;
    write_vec(
        transaction,
        "approval_requests",
        &snapshot.approval_requests,
        approval_request_key,
    )?;
    write_vec(
        transaction,
        "approval_rules",
        &snapshot.approval_rules,
        plain_id,
    )?;
    write_vec(
        transaction,
        "schedules",
        &snapshot.schedules,
        plain_id,
    )?;
    write_vec(
        transaction,
        "schedule_runs",
        &snapshot.schedule_runs,
        plain_id,
    )?;
    write_map(transaction, "task_hosts", task_hosts)?;
    write_map(transaction, "attachments", attachments)?;
    set_meta(transaction, "settings", &snapshot.settings)?;
    set_meta(
        transaction,
        "pending_throwaway_task_ids",
        &snapshot.pending_throwaway_task_ids,
    )
}

fn diff_state(
    transaction: &Transaction<'_>,
    before: StateRef<'_>,
    after: StateRef<'_>,
) -> Result<(), String> {
    let (before_snapshot, before_hosts, before_attachments) = before;
    let (after_snapshot, after_hosts, after_attachments) = after;
    diff_vec(
        transaction,
        "hosts",
        &before_snapshot.hosts,
        &after_snapshot.hosts,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "agents",
        &before_snapshot.agents,
        &after_snapshot.agents,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "tasks",
        &before_snapshot.tasks,
        &after_snapshot.tasks,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "messages",
        &before_snapshot.messages,
        &after_snapshot.messages,
        message_key,
    )?;
    diff_vec(
        transaction,
        "mail_batches",
        &before_snapshot.mail_batches,
        &after_snapshot.mail_batches,
        mail_batch_key,
    )?;
    diff_events(transaction, &before_snapshot.events, &after_snapshot.events)?;
    diff_vec(
        transaction,
        "channels",
        &before_snapshot.channels,
        &after_snapshot.channels,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "projects",
        &before_snapshot.projects,
        &after_snapshot.projects,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "collaborations",
        &before_snapshot.collaborations,
        &after_snapshot.collaborations,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "subagent_sessions",
        &before_snapshot.subagent_sessions,
        &after_snapshot.subagent_sessions,
        plain_id,
    )?;
    diff_map(
        transaction,
        "subagent_transcripts",
        &before_snapshot.subagent_transcripts,
        &after_snapshot.subagent_transcripts,
    )?;
    diff_vec(
        transaction,
        "queued_messages",
        &before_snapshot.queued_messages,
        &after_snapshot.queued_messages,
        queued_message_key,
    )?;
    diff_vec(
        transaction,
        "approval_requests",
        &before_snapshot.approval_requests,
        &after_snapshot.approval_requests,
        approval_request_key,
    )?;
    diff_vec(
        transaction,
        "approval_rules",
        &before_snapshot.approval_rules,
        &after_snapshot.approval_rules,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "schedules",
        &before_snapshot.schedules,
        &after_snapshot.schedules,
        plain_id,
    )?;
    diff_vec(
        transaction,
        "schedule_runs",
        &before_snapshot.schedule_runs,
        &after_snapshot.schedule_runs,
        plain_id,
    )?;
    diff_map(transaction, "task_hosts", before_hosts, after_hosts)?;
    diff_map(
        transaction,
        "attachments",
        before_attachments,
        after_attachments,
    )?;
    if before_snapshot.settings != after_snapshot.settings {
        set_meta(transaction, "settings", &after_snapshot.settings)?;
    }
    if before_snapshot.pending_throwaway_task_ids
        != after_snapshot.pending_throwaway_task_ids
    {
        set_meta(
            transaction,
            "pending_throwaway_task_ids",
            &after_snapshot.pending_throwaway_task_ids,
        )?;
    }
    Ok(())
}

fn plain_id<T>(value: &T) -> (&str, Option<&str>, Option<i64>)
where
    T: HasId,
{
    (value.id(), None, None)
}

trait HasId {
    fn id(&self) -> &str;
}

macro_rules! has_id {
    ($($type:ty),+ $(,)?) => {
        $(
            impl HasId for $type {
                fn id(&self) -> &str {
                    &self.id
                }
            }
        )+
    };
}
has_id!(
    Host,
    Agent,
    Task,
    Message,
    MailBatch,
    RunEvent,
    Channel,
    Project,
    Collaboration,
    SubagentSession,
    QueuedMessage,
    ApprovalRequest,
    ApprovalRule,
    Schedule,
    ScheduleRun
);

fn mail_batch_key(value: &MailBatch) -> (&str, Option<&str>, Option<i64>) {
    (
        &value.id,
        Some(value.task_id.as_str()),
        Some(value.created_at),
    )
}

fn message_key(value: &Message) -> (&str, Option<&str>, Option<i64>) {
    (
        &value.id,
        Some(value.task_id.as_str()),
        Some(value.created_at),
    )
}
fn event_key(value: &Arc<RunEvent>) -> (&str, Option<&str>, Option<i64>) {
    (
        &value.id,
        Some(value.task_id.as_str()),
        Some(value.created_at),
    )
}
fn queued_message_key(value: &QueuedMessage) -> (&str, Option<&str>, Option<i64>) {
    (
        &value.id,
        Some(value.task_id.as_str()),
        Some(value.created_at),
    )
}
fn approval_request_key(value: &ApprovalRequest) -> (&str, Option<&str>, Option<i64>) {
    (
        &value.id,
        Some(value.task_id.as_str()),
        Some(value.created_at),
    )
}

fn write_vec<T: Serialize>(
    transaction: &Transaction<'_>,
    collection: &str,
    values: &[T],
    key: impl Fn(&T) -> (&str, Option<&str>, Option<i64>),
) -> Result<(), String> {
    let mut ids = HashSet::with_capacity(values.len());
    for (position, value) in values.iter().enumerate() {
        let (id, task_id, created_at) = key(value);
        if !ids.insert(id) {
            return Err(format!("Duplicate {collection} id {id}"));
        }
        put(
            transaction,
            collection,
            id,
            position,
            task_id,
            created_at,
            value,
        )?;
    }
    Ok(())
}

fn diff_vec<T: Serialize + PartialEq>(
    transaction: &Transaction<'_>,
    collection: &str,
    before: &[T],
    after: &[T],
    key: impl Fn(&T) -> (&str, Option<&str>, Option<i64>),
) -> Result<(), String> {
    // The committed before view has unique IDs. Stable-order edits need no
    // temporary history index; SQLite validates appended IDs with strict INSERT.
    if after.len() >= before.len()
        && before
            .iter()
            .zip(after)
            .all(|(old, new)| key(old).0 == key(new).0)
    {
        for (position, (old, new)) in before.iter().zip(after).enumerate() {
            if old != new {
                let (id, task_id, created_at) = key(new);
                put(
                    transaction,
                    collection,
                    id,
                    position,
                    task_id,
                    created_at,
                    new,
                )?;
            }
        }
        for (offset, value) in after[before.len()..].iter().enumerate() {
            let (id, task_id, created_at) = key(value);
            insert_new(
                transaction,
                collection,
                id,
                before.len() + offset,
                task_id,
                created_at,
                value,
            )?;
        }
        return Ok(());
    }
    let mut old = HashMap::with_capacity(before.len());
    for (position, value) in before.iter().enumerate() {
        let (id, ..) = key(value);
        if old.insert(id, (position, value)).is_some() {
            return Err(format!("Duplicate {collection} id {id}"));
        }
    }
    let mut seen = HashSet::with_capacity(after.len());
    for (position, value) in after.iter().enumerate() {
        let (id, task_id, created_at) = key(value);
        if !seen.insert(id) {
            return Err(format!("Duplicate {collection} id {id}"));
        }
        match old.get(id) {
            Some((old_position, old_value)) if *old_value == value && *old_position == position => {
            }
            Some((old_position, old_value)) if *old_value == value => {
                update_position(transaction, collection, id, position)?
            }
            _ => put(
                transaction,
                collection,
                id,
                position,
                task_id,
                created_at,
                value,
            )?,
        }
    }
    for id in old.keys() {
        if !seen.contains(*id) {
            delete_entity(transaction, collection, id)?;
        }
    }
    Ok(())
}

fn diff_events(
    transaction: &Transaction<'_>,
    before: &[Arc<RunEvent>],
    after: &[Arc<RunEvent>],
) -> Result<(), String> {
    // Streaming normally preserves the committed order and appends
    // a few events. Scanning shared Arc pointers is cheap; rebuilding two
    // 50,000-entry hash indexes on every token is not. The before state comes
    // from the committed Service view (or read_snapshot), whose IDs are unique.
    // Strict INSERT below lets SQLite's existing primary-key index reject an
    // appended duplicate, including duplicates within the new tail, atomically.
    if after.len() >= before.len()
        && before
            .iter()
            .zip(after)
            .all(|(old, new)| Arc::ptr_eq(old, new) || old.id == new.id)
    {
        for (position, (old, new)) in before.iter().zip(after).enumerate() {
            if !same_event(old, new) {
                put(
                    transaction,
                    "events",
                    &new.id,
                    position,
                    Some(&new.task_id),
                    Some(new.created_at),
                    new,
                )?;
            }
        }
        for (offset, event) in after[before.len()..].iter().enumerate() {
            insert_new(
                transaction,
                "events",
                &event.id,
                before.len() + offset,
                Some(&event.task_id),
                Some(event.created_at),
                event,
            )?;
        }
        return Ok(());
    }
    let mut old = HashMap::with_capacity(before.len());
    for (position, event) in before.iter().enumerate() {
        if old.insert(event.id.as_str(), (position, event)).is_some() {
            return Err(format!("Duplicate events id {}", event.id));
        }
    }
    let mut seen = HashSet::with_capacity(after.len());
    for (position, event) in after.iter().enumerate() {
        let id = event.id.as_str();
        if !seen.insert(id) {
            return Err(format!("Duplicate events id {id}"));
        }
        match old.get(id) {
            Some((old_position, old_event))
                if same_event(*old_event, event) && *old_position == position => {}
            Some((_, old_event)) if same_event(*old_event, event) => {
                update_position(transaction, "events", id, position)?
            }
            _ => put(
                transaction,
                "events",
                id,
                position,
                Some(event.task_id.as_str()),
                Some(event.created_at),
                event,
            )?,
        }
    }
    for id in old.keys() {
        if !seen.contains(*id) {
            delete_entity(transaction, "events", id)?;
        }
    }
    Ok(())
}

fn same_event(before: &Arc<RunEvent>, after: &Arc<RunEvent>) -> bool {
    Arc::ptr_eq(before, after) || before.as_ref() == after.as_ref()
}

fn put<T: Serialize>(
    transaction: &Transaction<'_>,
    collection: &str,
    id: &str,
    position: usize,
    task_id: Option<&str>,
    created_at: Option<i64>,
    value: &T,
) -> Result<(), String> {
    let payload = serde_json::to_string(value).map_err(|e| e.to_string())?;
    transaction.execute("INSERT INTO entities(collection,id,position,task_id,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(collection,id) DO UPDATE SET position=excluded.position,task_id=excluded.task_id,created_at=excluded.created_at,payload=excluded.payload", params![collection, id, position as i64, task_id, created_at, payload]).map_err(|e| e.to_string())?;
    Ok(())
}

fn insert_new<T: Serialize>(
    transaction: &Transaction<'_>,
    collection: &str,
    id: &str,
    position: usize,
    task_id: Option<&str>,
    created_at: Option<i64>,
    value: &T,
) -> Result<(), String> {
    let payload = serde_json::to_string(value).map_err(|e| e.to_string())?;
    transaction.execute("INSERT INTO entities(collection,id,position,task_id,created_at,payload) VALUES(?1,?2,?3,?4,?5,?6)", params![collection, id, position as i64, task_id, created_at, payload]).map_err(|e| format!("Cannot append {collection} record to SQLite state: {e}"))?;
    Ok(())
}

fn update_position(
    transaction: &Transaction<'_>,
    collection: &str,
    id: &str,
    position: usize,
) -> Result<(), String> {
    transaction
        .execute(
            "UPDATE entities SET position=?1 WHERE collection=?2 AND id=?3",
            params![position as i64, collection, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn delete_entity(transaction: &Transaction<'_>, collection: &str, id: &str) -> Result<(), String> {
    transaction
        .execute(
            "DELETE FROM entities WHERE collection=?1 AND id=?2",
            params![collection, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn write_map<T: Serialize>(
    transaction: &Transaction<'_>,
    collection: &str,
    values: &HashMap<String, T>,
) -> Result<(), String> {
    for (id, value) in values {
        put(transaction, collection, id, 0, None, None, value)?;
    }
    Ok(())
}
fn diff_map<T: Serialize + PartialEq>(
    transaction: &Transaction<'_>,
    collection: &str,
    before: &HashMap<String, T>,
    after: &HashMap<String, T>,
) -> Result<(), String> {
    for (id, value) in after {
        if before.get(id) != Some(value) {
            put(transaction, collection, id, 0, None, None, value)?;
        }
    }
    for id in before.keys() {
        if !after.contains_key(id) {
            delete_entity(transaction, collection, id)?;
        }
    }
    Ok(())
}

fn read_vec<T: DeserializeOwned + HasId>(
    connection: &Connection,
    collection: &str,
) -> Result<Vec<T>, String> {
    let mut statement = connection
        .prepare("SELECT id,position,payload FROM entities WHERE collection=?1 ORDER BY position")
        .map_err(|e| e.to_string())?;
    let mut values = Vec::new();
    for row in statement
        .query_map([collection], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
    {
        let (id, position, payload) = row.map_err(|e| e.to_string())?;
        if position != values.len() as i64 {
            return Err(format!("SQLite {collection} positions are malformed."));
        }
        let value: T = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        if value.id() != id {
            return Err(format!(
                "SQLite {collection} row ID does not match its payload."
            ));
        }
        values.push(value);
    }
    Ok(values)
}
fn read_map<T: DeserializeOwned>(
    connection: &Connection,
    collection: &str,
) -> Result<HashMap<String, T>, String> {
    let mut statement = connection
        .prepare("SELECT id,payload FROM entities WHERE collection=?1")
        .map_err(|e| e.to_string())?;
    let mut values = HashMap::new();
    for row in statement
        .query_map([collection], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
    {
        let (id, payload) = row.map_err(|e| e.to_string())?;
        if values
            .insert(
                id,
                serde_json::from_str(&payload).map_err(|e| e.to_string())?,
            )
            .is_some()
        {
            return Err(format!("Duplicate {collection} id"));
        }
    }
    Ok(values)
}

fn set_meta<T: Serialize>(
    transaction: &Transaction<'_>,
    key: &str,
    value: &T,
) -> Result<(), String> {
    put(transaction, "meta", key, 0, None, None, value)
}

fn set_capture_since(
    transaction: &Transaction<'_>,
    captured_since: Option<i64>,
) -> Result<(), String> {
    set_meta(transaction, "capture_since", &captured_since)
}

fn set_capture_since_if_changed(
    transaction: &Transaction<'_>,
    captured_since: Option<i64>,
) -> Result<(), String> {
    let stored = transaction
        .query_row(
            "SELECT payload FROM entities WHERE collection='meta' AND id='capture_since'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .map(|payload| serde_json::from_str::<Option<i64>>(&payload).map_err(|e| e.to_string()))
        .transpose()?;
    if stored != Some(captured_since) {
        set_capture_since(transaction, captured_since)?;
    }
    Ok(())
}
fn get_meta<T: DeserializeOwned>(connection: &Connection, key: &str) -> Result<Option<T>, String> {
    connection
        .query_row(
            "SELECT payload FROM entities WHERE collection='meta' AND id=?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .map(|payload| serde_json::from_str(&payload).map_err(|e| e.to_string()))
        .transpose()
}

fn write_all_usage(transaction: &Transaction<'_>, usage: &[RunUsageSample]) -> Result<(), String> {
    validate_usage_ids(usage)?;
    for (position, sample) in usage.iter().enumerate() {
        put_usage(transaction, sample, position)?;
    }
    Ok(())
}
fn diff_usage(
    transaction: &Transaction<'_>,
    before: &[RunUsageSample],
    after: &[RunUsageSample],
) -> Result<(), String> {
    let old = usage_index(before)?;
    let new = usage_index(after)?;
    for (position, sample) in after.iter().enumerate() {
        match old.get(sample.sample_id.as_str()) {
            Some((old_position, old_sample))
                if *old_sample == sample && *old_position == position => {}
            Some((_, old_sample)) if *old_sample == sample => {
                transaction
                    .execute(
                        "UPDATE usage_samples SET position=?1 WHERE sample_id=?2",
                        params![position as i64, sample.sample_id],
                    )
                    .map_err(|e| e.to_string())?;
            }
            _ => put_usage(transaction, sample, position)?,
        };
    }
    for id in old.keys() {
        if !new.contains_key(*id) {
            transaction
                .execute("DELETE FROM usage_samples WHERE sample_id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn usage_index<'a>(
    usage: &'a [RunUsageSample],
) -> Result<HashMap<&'a str, (usize, &'a RunUsageSample)>, String> {
    let mut values = HashMap::with_capacity(usage.len());
    for (position, sample) in usage.iter().enumerate() {
        if values
            .insert(sample.sample_id.as_str(), (position, sample))
            .is_some()
        {
            return Err(format!("Duplicate usage sample id {}", sample.sample_id));
        }
    }
    Ok(values)
}
fn validate_usage_ids(usage: &[RunUsageSample]) -> Result<(), String> {
    usage_index(usage).map(|_| ())
}
fn put_usage(
    transaction: &Transaction<'_>,
    sample: &RunUsageSample,
    position: usize,
) -> Result<(), String> {
    transaction.execute("INSERT INTO usage_samples(sample_id,position,payload) VALUES(?1,?2,?3) ON CONFLICT(sample_id) DO UPDATE SET position=excluded.position,payload=excluded.payload", params![sample.sample_id, position as i64, serde_json::to_string(sample).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    Ok(())
}
fn read_usage(connection: &Connection) -> Result<Vec<RunUsageSample>, String> {
    let mut statement = connection
        .prepare("SELECT sample_id,payload FROM usage_samples ORDER BY position")
        .map_err(|e| e.to_string())?;
    let mut samples = Vec::new();
    let mut ids = HashSet::new();
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
    {
        let (id, payload) = row.map_err(|e| e.to_string())?;
        let sample: RunUsageSample = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        if sample.sample_id != id {
            return Err("SQLite usage sample ID does not match its payload.".to_string());
        }
        if !ids.insert(id) {
            return Err("SQLite contains duplicate usage sample IDs.".to_string());
        }
        samples.push(sample);
    }
    Ok(samples)
}

fn private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn private_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect private SQLite file {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "SQLite file is not a regular file: {}.",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn private_sidecars(path: &Path) -> Result<(), String> {
    private_file(path)?;
    validate_sidecars(path)?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar_path(path, suffix);
        if sidecar.exists() {
            private_file(&sidecar)?;
        }
    }
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

/// SQLite may open WAL and shared-memory paths before this module can chmod
/// them. Reject pre-existing links before opening the database at all.
fn validate_sidecars(path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar_path(path, suffix);
        match fs::symlink_metadata(&sidecar) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "SQLite sidecar is not a regular file: {}.",
                    sidecar.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect SQLite sidecar {}: {error}",
                    sidecar.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_path(name: &str) -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!("monitter-sqlite-{name}-{unique}"));
        fs::create_dir(&directory).unwrap();
        let database = directory.join("state.sqlite3");
        (directory, database)
    }

    fn event(id: &str) -> Arc<RunEvent> {
        Arc::new(RunEvent {
            id: id.into(),
            task_id: "task".into(),
            kind: "log".into(),
            title: id.into(),
            detail: Arc::from("detail"),
            created_at: 1,
        })
    }

    fn usage(id: &str) -> RunUsageSample {
        RunUsageSample {
            sample_id: id.into(),
            run_id: "run".into(),
            task_id: "task".into(),
            provider: "codex".into(),
            configured_model: None,
            started_at: 1,
            observed_at: 2,
            final_sample: false,
            classification: "delta".into(),
            provider_turn_id: None,
            tokens: UsageTokens::default(),
            cost_usd: None,
            duration_ms: None,
            api_duration_ms: None,
            provider_turns: None,
            context: None,
        }
    }

    fn state(snapshot: &Snapshot) -> StateRef<'_> {
        static EMPTY_HOSTS: std::sync::OnceLock<HashMap<String, Host>> = std::sync::OnceLock::new();
        static EMPTY_ATTACHMENTS: std::sync::OnceLock<HashMap<String, StoredAttachment>> =
            std::sync::OnceLock::new();
        (
            snapshot,
            EMPTY_HOSTS.get_or_init(HashMap::new),
            EMPTY_ATTACHMENTS.get_or_init(HashMap::new),
        )
    }

    #[test]
    fn round_trip_and_noop_write_leave_rows_unchanged() {
        let (directory, path) = temp_path("roundtrip");
        let database = Database::create(&path).unwrap();
        let mut snapshot = default_snapshot();
        snapshot.events = vec![event("first")];
        let samples = vec![usage("one")];
        database
            .replace_all(state(&snapshot), &samples, Some(7))
            .unwrap();
        let changes_before = database.lock().unwrap().total_changes();
        database
            .apply_update(
                state(&snapshot),
                state(&snapshot),
                &samples,
                &samples,
                Some(7),
            )
            .unwrap();
        assert_eq!(database.lock().unwrap().total_changes(), changes_before);
        let (loaded, _, _, loaded_usage, captured) = database.read_snapshot().unwrap();
        assert_eq!(loaded, snapshot);
        assert_eq!(loaded_usage, samples);
        assert_eq!(captured, Some(7));
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_temp_directory_round_trip_preserves_nofollow_primary() {
        let (directory, path) = temp_path("macos-system-temp");
        let database = Database::create(&path).unwrap();
        let snapshot = default_snapshot();
        database.replace_all(state(&snapshot), &[], None).unwrap();
        drop(database);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.read_snapshot().unwrap().0, snapshot);
        drop(reopened);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn small_update_only_changes_affected_rows_and_preserves_order() {
        let (directory, path) = temp_path("delta");
        let database = Database::create(&path).unwrap();
        let mut before = default_snapshot();
        before.events = vec![event("one"), event("two")];
        database.replace_all(state(&before), &[], None).unwrap();
        let mut after = before.clone();
        after.events = vec![event("two"), event("three")];
        let changes_before = database.lock().unwrap().total_changes();
        database
            .apply_update(state(&before), state(&after), &[], &[], None)
            .unwrap();
        let changed = database.lock().unwrap().total_changes() - changes_before;
        assert!(
            changed <= 3,
            "changed {changed} rows for one delete/reorder/append"
        );
        let (loaded, _, _, _, _) = database.read_snapshot().unwrap();
        assert_eq!(loaded.events, after.events);
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn duplicate_input_rolls_back_the_whole_transaction() {
        let (directory, path) = temp_path("duplicate");
        let database = Database::create(&path).unwrap();
        let mut before = default_snapshot();
        before.events = vec![event("one")];
        database.replace_all(state(&before), &[], None).unwrap();
        let mut invalid = before.clone();
        invalid.events.push(event("one"));
        assert!(database
            .apply_update(state(&before), state(&invalid), &[], &[], None)
            .is_err());
        assert_eq!(database.read_snapshot().unwrap().0, before);
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn usage_removal_and_reorder_are_durable() {
        let (directory, path) = temp_path("usage");
        let database = Database::create(&path).unwrap();
        let snapshot = default_snapshot();
        let before = vec![usage("one"), usage("two")];
        let after = vec![usage("two")];
        database
            .replace_all(state(&snapshot), &before, None)
            .unwrap();
        database
            .apply_update(state(&snapshot), state(&snapshot), &before, &after, None)
            .unwrap();
        assert_eq!(database.read_snapshot().unwrap().3, after);
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn bad_schema_and_payload_are_rejected_without_reinitializing() {
        let (directory, path) = temp_path("strict");
        let database = Database::create(&path).unwrap();
        let snapshot = default_snapshot();
        database.replace_all(state(&snapshot), &[], None).unwrap();
        {
            let connection = database.lock().unwrap();
            connection
                .execute("UPDATE meta SET value='2' WHERE key='schema_version'", [])
                .unwrap();
        }
        drop(database);
        assert!(Database::open(&path).is_err());
        let raw = Connection::open(&path).unwrap();
        let version: String = raw
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2");
        drop(raw);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn wrong_application_id_and_malformed_payload_fail_closed() {
        let (directory, path) = temp_path("corrupt");
        let database = Database::create(&path).unwrap();
        let snapshot = default_snapshot();
        database.replace_all(state(&snapshot), &[], None).unwrap();
        {
            let connection = database.lock().unwrap();
            connection
                .execute(
                    "UPDATE entities SET payload='not-json' WHERE collection='hosts'",
                    [],
                )
                .unwrap();
        }
        assert!(database.read_snapshot().is_err());
        drop(database);

        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("PRAGMA application_id=1234;").unwrap();
        drop(raw);
        assert!(Database::open(&path).is_err());
        let raw = Connection::open(&path).unwrap();
        let application_id: i64 = raw
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        assert_eq!(application_id, 1234);
        drop(raw);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (directory, path) = temp_path("permissions");
        let database = Database::create(&path).unwrap();
        let snapshot = default_snapshot();
        database.replace_all(state(&snapshot), &[], None).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for suffix in ["-wal", "-shm"] {
            let sidecar = sidecar_path(&path, suffix);
            if sidecar.exists() {
                assert_eq!(
                    fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn create_and_open_reject_symlinked_sidecars() {
        use std::os::unix::fs::symlink;

        let (directory, path) = temp_path("sidecar-link");
        let target = directory.join("target");
        fs::write(&target, "not a database").unwrap();
        let primary_link = directory.join("primary-link.sqlite3");
        symlink(&target, &primary_link).unwrap();
        assert!(Database::create(&primary_link).is_err());
        assert!(Database::open(&primary_link).is_err());

        let sidecar = sidecar_path(&path, "-wal");
        symlink(&target, &sidecar).unwrap();
        assert!(Database::create(&path).is_err());
        assert!(!path.exists());
        fs::remove_file(&sidecar).unwrap();

        let database = Database::create(&path).unwrap();
        drop(database);
        let sidecar = sidecar_path(&path, "-shm");
        if sidecar.exists() {
            fs::remove_file(&sidecar).unwrap();
        }
        symlink(&target, &sidecar).unwrap();
        assert!(Database::open(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
