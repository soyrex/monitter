//! One-time, recoverable migration from deployed JSON/JSONL to SQLite.
//! Normal writes do not use this protocol: SQLite owns their atomicity.
use super::{legacy_store, sqlite_backend::Database};
use crate::model::id;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const DATABASE_NAME: &str = "state.sqlite3";
const GUARD_MARKER: &str = "monitter-sqlite-v1";
const INTENT_NAME: &str = "sqlite-migration.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Source {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    version: u32,
    generation: String,
    sources: Vec<Source>,
    state_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Guard {
    events: String,
    database: String,
    schema_version: u32,
}

/// One new-format owner at a time. The lease stays alive for Store's lifetime,
/// preventing two in-memory Service views from writing stale state to one DB.
pub(super) struct Lease(File);

impl Lease {
    pub(super) fn acquire(dir: &Path) -> Result<Self, String> {
        let path = dir.join("state.lock");
        if path.exists() {
            regular_file(&path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("Cannot open Monitter state lease: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("Monitter data is already open in another process. Close that version before opening this data directory.".into());
            }
        }
        Ok(Self(file))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

pub(super) fn private_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        let metadata = fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Monitter data must be a real directory, not a symlink.".into());
        }
    } else {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Cannot create Monitter data directory: {e}"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect Monitter state file: {e}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Monitter state files must be regular files, not symlinks.".into());
    }
    Ok(())
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Cannot make Monitter directory changes durable: {e}"))
}

fn private_create(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|e| format!("Cannot create private Monitter migration file: {e}"))
}

fn atomic_bytes(dir: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = dir.join(format!(".sqlite-migration-write-{}", id()));
    let mut file = private_create(&temporary)?;
    let result = (|| {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        if fs::symlink_metadata(destination).is_ok() {
            regular_file(destination)?;
        }
        fs::rename(&temporary, destination)
            .map_err(|e| format!("Cannot publish Monitter migration metadata: {e}"))?;
        sync_dir(dir)
    })();
    // This is an exact, newly-created temporary file; never remove a target.
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = open_regular(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let length = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if length == 0 {
            break;
        }
        hasher.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn open_regular(path: &Path) -> Result<File, String> {
    regular_file(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Monitter state must be a regular file.".into());
    }
    Ok(file)
}

fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    open_regular(path)?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn state_hash(state: &legacy_store::ImportedState) -> Result<String, String> {
    // HashMap iteration order is not persistent. Canonicalize only the two
    // small maps, then stream serialization without copying historical text.
    let hosts: BTreeMap<_, _> = state.1.iter().collect();
    let attachments: BTreeMap<_, _> = state.2.iter().collect();
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(
        &mut writer,
        &(&state.0, hosts, attachments, &state.3, state.4),
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", writer.0.finalize()))
}

fn valid_source_name(name: &str) -> bool {
    if matches!(name, "state.json" | "usage-v1.jsonl") {
        return true;
    }
    name.strip_prefix("events-")
        .and_then(|s| s.strip_suffix(".jsonl"))
        .and_then(|s| {
            uuid::Uuid::parse_str(s)
                .ok()
                .map(|uuid| uuid.to_string() == s)
        })
        == Some(true)
}

fn validate_intent(intent: &Intent) -> Result<(), String> {
    let valid_generation =
        uuid::Uuid::parse_str(&intent.generation).is_ok_and(|v| v.to_string() == intent.generation);
    let valid_hash = |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    let mut names = std::collections::HashSet::new();
    if intent.version != 1
        || !valid_generation
        || !valid_hash(&intent.state_sha256)
        || intent.sources.iter().any(|s| {
            !valid_source_name(&s.name) || !valid_hash(&s.sha256) || !names.insert(&s.name)
        })
        || (!intent.sources.is_empty() && !names.contains(&"state.json".to_owned()))
    {
        return Err(
            "Monitter SQLite migration metadata is invalid; no state was overwritten.".into(),
        );
    }
    Ok(())
}

fn verify_sources(dir: &Path, intent: &Intent) -> Result<(), String> {
    if intent.sources.is_empty() && dir.join("state.json").exists() {
        return Err(
            "Legacy state appeared during SQLite migration; it was not overwritten.".into(),
        );
    }
    for source in &intent.sources {
        if file_hash(&dir.join(&source.name))? != source.sha256 {
            return Err("Legacy Monitter state changed during SQLite migration. Close other versions and review the preserved files; no state was overwritten.".into());
        }
    }
    Ok(())
}

fn backup_sources(dir: &Path, intent: &Intent) -> Result<(), String> {
    if intent.sources.is_empty() {
        return Ok(());
    }
    let backup = dir.join(format!("legacy-before-sqlite-{}", intent.generation));
    private_dir(&backup)?;
    for source in &intent.sources {
        let target = backup.join(&source.name);
        if target.exists() {
            if file_hash(&target)? != source.sha256 {
                return Err("Monitter migration backup does not match its manifest; it was not overwritten.".into());
            }
            continue;
        }
        let temporary = backup.join(format!(".copy-{}", id()));
        let mut output = private_create(&temporary)?;
        let mut input = open_regular(&dir.join(&source.name))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|e| format!("Cannot back up legacy Monitter state: {e}"))?;
        output.sync_all().map_err(|e| e.to_string())?;
        if file_hash(&temporary)? != source.sha256 {
            return Err(
                "Legacy state changed while copying its backup; migration was stopped.".into(),
            );
        }
        fs::rename(&temporary, &target).map_err(|e| e.to_string())?;
    }
    sync_dir(&backup)?;
    sync_dir(dir)
}

fn guard_present(dir: &Path) -> Result<bool, String> {
    let path = dir.join("state.json");
    if matches!(fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(false);
    }
    let raw = read_regular(&path)?;
    let value: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| format!("Monitter state is corrupt; it was not overwritten: {e}"))?;
    if value.get("events").and_then(serde_json::Value::as_str) != Some(GUARD_MARKER) {
        return Ok(false);
    }
    let guard: Guard = serde_json::from_value(value)
        .map_err(|e| format!("Invalid SQLite downgrade guard: {e}"))?;
    if guard.database != DATABASE_NAME || guard.schema_version != 1 {
        return Err("Unsupported Monitter SQLite guard; no state was changed.".into());
    }
    Ok(true)
}

fn write_guard(dir: &Path) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Guard {
        events: GUARD_MARKER.into(),
        database: DATABASE_NAME.into(),
        schema_version: 1,
    })
    .map_err(|e| e.to_string())?;
    atomic_bytes(dir, &dir.join("state.json"), &bytes)
}

fn finish_guard(dir: &Path, intent: &Intent, database: &Database) -> Result<(), String> {
    verify_sources(dir, intent)?;
    if state_hash(&database.read_snapshot()?)? != intent.state_sha256 {
        return Err(
            "SQLite import differs from its verified source; no downgrade guard was written."
                .into(),
        );
    }
    backup_sources(dir, intent)?;
    verify_sources(dir, intent)?;
    write_guard(dir)?;
    // Keeping a completed manifest is harmless, but remove it durably when
    // possible. A guard + a valid DB is authoritative on the next open.
    fs::remove_file(dir.join(INTENT_NAME)).map_err(|e| e.to_string())?;
    sync_dir(dir)
}

pub(super) fn open_database(dir: &Path) -> Result<Database, String> {
    let path = dir.join(DATABASE_NAME);
    let database_exists = match fs::symlink_metadata(&path) {
        Ok(_) => {
            regular_file(&path)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("Cannot inspect Monitter SQLite database: {error}")),
    };
    let guarded = guard_present(dir)?;
    if guarded {
        if !database_exists {
            return Err("Monitter SQLite database is missing. Restore the complete backup; the legacy guard was not replaced.".into());
        }
        regular_file(&path)?;
        return Database::open(&path);
    }
    let intent_path = dir.join(INTENT_NAME);
    let mut intent = if !matches!(fs::symlink_metadata(&intent_path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        let value: Intent = serde_json::from_slice(&read_regular(&intent_path)?)
            .map_err(|e| format!("Invalid SQLite migration manifest: {e}"))?;
        validate_intent(&value)?;
        Some(value)
    } else {
        None
    };
    if database_exists {
        regular_file(&path)?;
        let saved = intent.as_ref().ok_or("An unguarded SQLite database already exists without migration metadata; no state was overwritten.")?;
        verify_sources(dir, saved)?;
        let database = Database::open(&path)?;
        database.integrity_check()?;
        finish_guard(dir, saved, &database)?;
        return Ok(database);
    }
    if let Some(saved) = &intent {
        verify_sources(dir, saved)?;
    }
    let (state, sources) = legacy_store::read(dir)?;
    let source_state_hash = state_hash(&state)?;
    if let Some(saved) = &mut intent {
        if source_state_hash != saved.state_sha256 {
            if saved.sources.is_empty() {
                // A fresh installation has no accepted state yet. Its default
                // host IDs are random; a pre-install retry may regenerate them.
                saved.state_sha256 = source_state_hash;
                atomic_bytes(
                    dir,
                    &intent_path,
                    &serde_json::to_vec(saved).map_err(|e| e.to_string())?,
                )?;
            } else {
                return Err("Legacy state no longer matches the interrupted SQLite import; preserved data needs review.".into());
            }
        }
    } else {
        let mut digests = Vec::new();
        for source in sources {
            let name = source
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Invalid legacy state filename")?
                .to_owned();
            if !valid_source_name(&name) {
                return Err("Unsafe legacy state filename.".into());
            }
            digests.push(Source {
                name,
                sha256: file_hash(&source)?,
            });
        }
        let created = Intent {
            version: 1,
            generation: id(),
            sources: digests,
            state_sha256: source_state_hash,
        };
        atomic_bytes(
            dir,
            &intent_path,
            &serde_json::to_vec(&created).map_err(|e| e.to_string())?,
        )?;
        intent = Some(created);
    }
    let saved = intent.as_ref().unwrap();
    backup_sources(dir, saved)?;
    let temporary = dir.join(format!("sqlite-import-{}.sqlite3", saved.generation));
    // An interrupted pre-install DB has never accepted application mutations.
    // Preserve its exact artifacts under unique names, then rebuild from the
    // unchanged source. Never fall back from an installed database.
    for suffix in ["", "-wal", "-shm"] {
        let artifact = PathBuf::from(format!("{}{suffix}", temporary.display()));
        if artifact.exists() {
            regular_file(&artifact)?;
            let preserved = dir.join(format!("sqlite-import-abandoned-{}{}", id(), suffix));
            fs::rename(&artifact, preserved).map_err(|e| e.to_string())?;
        }
    }
    sync_dir(dir)?;
    let database = Database::create(&temporary)?;
    database.replace_all((&state.0, &state.1, &state.2), &state.3, state.4)?;
    database.integrity_check()?;
    if state_hash(&database.read_snapshot()?)? != saved.state_sha256 {
        return Err("SQLite import validation failed; legacy files were preserved.".into());
    }
    database.checkpoint()?;
    drop(database);
    File::open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    verify_sources(dir, saved)?;
    // Re-read committed legacy state, not just its files' sizes, before the
    // atomic install. This catches any inconsistent source observed on import.
    if !saved.sources.is_empty() && state_hash(&legacy_store::read(dir)?.0)? != saved.state_sha256 {
        return Err(
            "Legacy state changed while SQLite was being verified; migration was stopped.".into(),
        );
    }
    fs::rename(&temporary, &path)
        .map_err(|e| format!("Cannot install verified Monitter SQLite database: {e}"))?;
    sync_dir(dir)?;
    let database = Database::open(&path)?;
    finish_guard(dir, saved, &database)?;
    Ok(database)
}

#[cfg(test)]
#[path = "store_migration_tests.rs"]
mod tests;
