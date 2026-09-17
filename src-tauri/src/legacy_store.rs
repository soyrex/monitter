//! Read-only importer for the deployed JSON and JSONL persistence format.
//! No legacy file is modified or removed by this module.
use crate::{
    attachments::StoredAttachment,
    model::{default_snapshot, Host, RunEvent, RunUsageSample, Snapshot},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

fn open_regular(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open legacy Monitter file {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(
            "Legacy Monitter state must contain regular files, not symlinks or devices.".into(),
        );
    }
    Ok(file)
}

pub(crate) type ImportedState = (
    Snapshot,
    HashMap<String, Host>,
    HashMap<String, StoredAttachment>,
    Vec<RunUsageSample>,
    Option<i64>,
);

#[derive(Serialize, Deserialize)]
struct DiskState {
    #[serde(flatten)]
    snapshot: Snapshot,
    #[serde(rename = "_taskHosts", default)]
    task_hosts: HashMap<String, Host>,
    #[serde(rename = "_attachments", default)]
    attachments: HashMap<String, StoredAttachment>,
    #[serde(rename = "_eventJournal", default)]
    event_journal: Option<EventJournal>,
    #[serde(rename = "_usageLedger", default)]
    usage_ledger: Option<UsageLedger>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventJournal {
    generation: String,
    committed_count: usize,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageLedger {
    committed_count: usize,
    captured_since: i64,
}

pub(crate) fn read(dir: &Path) -> Result<(ImportedState, Vec<PathBuf>), String> {
    let path = dir.join("state.json");
    if matches!(fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok((
            (
                default_snapshot(),
                HashMap::new(),
                HashMap::new(),
                vec![],
                None,
            ),
            vec![],
        ));
    }
    let mut raw = String::new();
    open_regular(&path)?
        .read_to_string(&mut raw)
        .map_err(|error| format!("Cannot read legacy Monitter state: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        format!("Monitter legacy state is corrupt; it was not overwritten: {error}")
    })?;
    if value
        .get("_updates")
        .is_some_and(|updates| !updates.is_null())
    {
        return Err("An experimental JSON-update journal cannot be silently imported; export or restore its complete state first.".into());
    }
    let data: DiskState = serde_json::from_value(value).map_err(|error| {
        format!("Monitter legacy state is corrupt or unsupported; it was not overwritten: {error}")
    })?;
    let mut snapshot = data.snapshot;
    let mut sources = vec![path];
    if let Some(reference) = data.event_journal {
        snapshot.events = read_journal(dir, &reference)?.0;
        sources.push(journal_path(dir, &reference));
    }
    let (usage, captured_since) = if let Some(reference) = data.usage_ledger {
        let samples = read_usage_ledger(dir, &reference)?;
        sources.push(usage_path(dir));
        (samples, Some(reference.captured_since))
    } else {
        (vec![], None)
    };
    Ok((
        (
            snapshot,
            data.task_hosts,
            data.attachments,
            usage,
            captured_since,
        ),
        sources,
    ))
}

fn usage_path(dir: &Path) -> PathBuf {
    dir.join("usage-v1.jsonl")
}
fn journal_path(dir: &Path, reference: &EventJournal) -> PathBuf {
    dir.join(format!("events-{}.jsonl", reference.generation))
}

fn read_usage_ledger(dir: &Path, pointer: &UsageLedger) -> Result<Vec<RunUsageSample>, String> {
    let path = usage_path(dir);
    let file = open_regular(&path).map_err(|e| {
        format!("Monitter usage ledger is missing or unreadable; state was not changed: {e}")
    })?;
    let mut samples = vec![];
    for line in BufReader::new(file)
        .split(b'\n')
        .take(pointer.committed_count)
    {
        let line = line.map_err(|e| format!("Cannot read Monitter usage ledger: {e}"))?;
        if line.is_empty() {
            return Err("Monitter usage ledger is corrupt; state was not changed.".into());
        }
        samples.push(serde_json::from_slice(&line).map_err(|e| {
            format!("Monitter usage ledger is corrupt; state was not changed: {e}")
        })?);
    }
    if samples.len() != pointer.committed_count {
        return Err("Monitter usage ledger is incomplete; state was not changed.".into());
    }
    Ok(samples)
}

fn read_journal(dir: &Path, reference: &EventJournal) -> Result<(Vec<Arc<RunEvent>>, u64), String> {
    let valid_generation = uuid::Uuid::parse_str(&reference.generation)
        .map(|id| id.to_string() == reference.generation)
        .unwrap_or(false);
    if !valid_generation {
        return Err("Monitter event journal reference is invalid; state was not changed.".into());
    }
    let path = journal_path(dir, reference);
    let file = open_regular(&path)?;
    let length = file
        .metadata()
        .map_err(|e| {
            format!("Monitter event journal is missing or unreadable; state was not changed: {e}")
        })?
        .len();
    if reference.committed_count > length as usize {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    let mut events = Vec::with_capacity(reference.committed_count);
    let mut bytes = 0;
    for line in BufReader::new(file)
        .split(b'\n')
        .take(reference.committed_count)
    {
        let line = line.map_err(|e| format!("Cannot read Monitter event journal: {e}"))?;
        if line.is_empty() {
            return Err("Monitter event journal is corrupt; state was not changed.".into());
        }
        bytes += (line.len() + 1) as u64;
        events.push(Arc::new(serde_json::from_slice(&line).map_err(|e| {
            format!("Monitter event journal is corrupt; state was not changed: {e}")
        })?));
    }
    if events.len() != reference.committed_count {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    if length < bytes {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    Ok((events, bytes))
}
