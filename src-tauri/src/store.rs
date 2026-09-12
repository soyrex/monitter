use crate::{
    attachments::StoredAttachment,
    model::{default_snapshot, now, Host, Snapshot},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

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
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventJournal {
    generation: String,
    committed_count: usize,
}

/// Serialization view for state.json.  Events deliberately remain an empty
/// compatibility field: the durable event payload lives in the private JSONL
/// journal named by `_eventJournal`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreState<'a> {
    hosts: &'a [crate::model::Host],
    agents: &'a [crate::model::Agent],
    tasks: &'a [crate::model::Task],
    messages: &'a [crate::model::Message],
    events: &'a [crate::model::RunEvent],
    channels: &'a [crate::model::Channel],
    projects: &'a [crate::model::Project],
    settings: &'a crate::model::Settings,
    collaborations: &'a [crate::model::Collaboration],
    queued_messages: &'a [crate::model::QueuedMessage],
    approval_requests: &'a [crate::model::ApprovalRequest],
    #[serde(rename = "_taskHosts")]
    task_hosts: &'a HashMap<String, Host>,
    #[serde(rename = "_attachments")]
    attachments: &'a HashMap<String, StoredAttachment>,
    #[serde(rename = "_eventJournal", skip_serializing_if = "Option::is_none")]
    event_journal: Option<&'a EventJournal>,
}

impl<'a> CoreState<'a> {
    fn new(
        snapshot: &'a Snapshot,
        task_hosts: &'a HashMap<String, Host>,
        attachments: &'a HashMap<String, StoredAttachment>,
        event_journal: Option<&'a EventJournal>,
    ) -> Self {
        Self {
            hosts: &snapshot.hosts,
            agents: &snapshot.agents,
            tasks: &snapshot.tasks,
            messages: &snapshot.messages,
            events: &[],
            channels: &snapshot.channels,
            projects: &snapshot.projects,
            settings: &snapshot.settings,
            collaborations: &snapshot.collaborations,
            queued_messages: &snapshot.queued_messages,
            approval_requests: &snapshot.approval_requests,
            task_hosts,
            attachments,
            event_journal,
        }
    }
}

#[derive(Clone)]
struct JournalState {
    reference: EventJournal,
    bytes: u64,
}

pub struct Store {
    path: PathBuf,
    dir: PathBuf,
    // Keep compact fingerprints rather than a second multi-megabyte transcript.
    event_fingerprints: Mutex<Vec<u64>>,
    journal: Mutex<Option<JournalState>>,
}

impl Store {
    pub fn open(
        dir: PathBuf,
    ) -> Result<
        (
            Self,
            Snapshot,
            HashMap<String, Host>,
            HashMap<String, StoredAttachment>,
        ),
        String,
    > {
        fs::create_dir_all(&dir).map_err(|e| format!("Cannot create Monitter data folder: {e}"))?;
        private_dir(&dir)?;
        let path = dir.join("state.json");
        let existed = path.exists();
        let (mut snapshot, mut task_hosts, attachments, journal) = if existed {
            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("Cannot read Monitter state: {e}"))?;
            let data: DiskState = serde_json::from_str(&raw)
                .map_err(|e| format!("Monitter state is corrupt; it was not overwritten: {e}"))?;
            let mut snapshot = data.snapshot;
            let journal = match data.event_journal {
                Some(reference) => {
                    // A pointer is authoritative: never fall back to the empty
                    // compatibility field or silently discard a bad history.
                    let (events, bytes) = read_journal(&dir, &reference)?;
                    snapshot.events = events;
                    Some(JournalState { reference, bytes })
                }
                None => None,
            };
            (snapshot, data.task_hosts, data.attachments, journal)
        } else {
            (default_snapshot(), HashMap::new(), HashMap::new(), None)
        };

        // Old state files did not contain immutable task host snapshots. Migrate
        // them from the referenced host once, then keep them independent of edits.
        for task in &snapshot.tasks {
            if !task_hosts.contains_key(&task.id) {
                if let Some(host) = snapshot.hosts.iter().find(|host| host.id == task.host_id) {
                    task_hosts.insert(task.id.clone(), host.clone());
                }
            }
        }

        let mut recovered = false;
        for message in &mut snapshot.messages {
            if message.stream_status.as_deref() == Some("streaming") {
                message.stream_status = Some("interrupted".into());
                recovered = true;
            }
        }
        let mut interrupted_task_ids = std::collections::HashSet::new();
        for task in &mut snapshot.tasks {
            if task.status == "running" {
                task.status = "interrupted".into();
                task.updated_at = now();
                interrupted_task_ids.insert(task.id.clone());
                recovered = true;
            }
        }
        if !interrupted_task_ids.is_empty() {
            let resolved_at = now();
            for request in &mut snapshot.approval_requests {
                if request.status == "pending" && interrupted_task_ids.contains(&request.task_id) {
                    request.status = "expired".into();
                    request.resolved_at = Some(resolved_at);
                    recovered = true;
                }
            }
        }
        for queued in &mut snapshot.queued_messages {
            if queued.status == "sending" {
                queued.status = "error".into();
                queued.error = Some("Monitter restarted before this queued message could be confirmed. Review and retry it manually.".into());
                recovered = true;
            }
        }
        let had_running_deliveries = snapshot
            .collaborations
            .iter()
            .any(|delivery| delivery.status == "running");
        crate::Service::recover_collaborations(&mut snapshot);
        recovered |= had_running_deliveries;
        // Pay the one-time legacy conversion at startup, never in the first
        // user's send acknowledgement. The original file is backed up by save.
        let needs_migration = journal.is_none() && !snapshot.events.is_empty();
        let store = Self {
            path,
            dir,
            event_fingerprints: Mutex::new(event_fingerprints(&snapshot.events)),
            journal: Mutex::new(journal),
        };
        if !existed || recovered || needs_migration {
            store.save(&snapshot, &task_hosts, &attachments)?;
        }
        Ok((store, snapshot, task_hosts, attachments))
    }

    pub fn save(
        &self,
        snapshot: &Snapshot,
        task_hosts: &HashMap<String, Host>,
        attachments: &HashMap<String, StoredAttachment>,
    ) -> Result<(), String> {
        let mut cached_fingerprints = self
            .event_fingerprints
            .lock()
            .map_err(|_| "Monitter event journal lock failed.".to_string())?;
        let mut cached_journal = self
            .journal
            .lock()
            .map_err(|_| "Monitter event journal lock failed.".to_string())?;
        let fingerprints = event_fingerprints(&snapshot.events);
        let is_append = fingerprints.starts_with(&cached_fingerprints[..]);
        let events_changed = fingerprints != *cached_fingerprints;
        // A legacy embedded history is migrated at the next successful save,
        // even when that save changes only unrelated core state.
        let needs_journal = cached_journal.is_none() && !snapshot.events.is_empty();
        let mut next_journal = cached_journal.clone();

        if events_changed || needs_journal {
            if is_append && cached_journal.is_some() {
                let journal = cached_journal.as_ref().unwrap();
                append_events(
                    &self.dir,
                    journal,
                    &snapshot.events[cached_fingerprints.len()..],
                )?;
                let bytes = fs::metadata(journal_path(&self.dir, &journal.reference))
                    .map_err(|e| format!("Cannot inspect Monitter event journal: {e}"))?
                    .len();
                next_journal = Some(JournalState {
                    reference: EventJournal {
                        generation: journal.reference.generation.clone(),
                        committed_count: snapshot.events.len(),
                    },
                    bytes,
                });
            } else {
                // A legacy state is only considered migrated after both its full
                // immutable journal and the atomic pointer have been committed.
                if cached_journal.is_none() && self.path.exists() {
                    let backup = self
                        .path
                        .with_file_name(format!("state.legacy-{}.json", crate::model::id()));
                    fs::copy(&self.path, &backup).map_err(|e| {
                        format!("Cannot preserve legacy Monitter state before migration: {e}")
                    })?;
                    private_file(&backup)?;
                }
                let reference = EventJournal {
                    generation: crate::model::id(),
                    committed_count: snapshot.events.len(),
                };
                let bytes = write_generation(&self.dir, &reference, &snapshot.events)?;
                next_journal = Some(JournalState { reference, bytes });
            }
        }

        self.write_core(
            snapshot,
            task_hosts,
            attachments,
            next_journal.as_ref().map(|j| &j.reference),
        )?;
        if events_changed || needs_journal {
            *cached_fingerprints = fingerprints;
            *cached_journal = next_journal;
        }
        Ok(())
    }

    fn write_core(
        &self,
        snapshot: &Snapshot,
        task_hosts: &HashMap<String, Host>,
        attachments: &HashMap<String, StoredAttachment>,
        event_journal: Option<&EventJournal>,
    ) -> Result<(), String> {
        let temp = self
            .path
            .with_extension(format!("tmp-{}", crate::model::id()));
        let json = serde_json::to_vec_pretty(&CoreState::new(
            snapshot,
            task_hosts,
            attachments,
            event_journal,
        ))
        .map_err(|e| format!("Cannot encode Monitter state: {e}"))?;
        let mut file = private_create(&temp)?;
        if let Err(error) = file.write_all(&json).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temp);
            return Err(format!("Cannot write Monitter state: {error}"));
        }
        drop(file);
        if let Err(error) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(format!("Cannot atomically save Monitter state: {error}"));
        }
        private_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| format!("Cannot make Monitter state durable: {e}"))?;
        }
        Ok(())
    }
}

fn journal_path(dir: &Path, reference: &EventJournal) -> PathBuf {
    dir.join(format!("events-{}.jsonl", reference.generation))
}

fn event_fingerprints(events: &[crate::model::RunEvent]) -> Vec<u64> {
    events
        .iter()
        .map(|event| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            event.id.hash(&mut hasher);
            event.task_id.hash(&mut hasher);
            event.kind.hash(&mut hasher);
            event.title.hash(&mut hasher);
            event.detail.hash(&mut hasher);
            event.created_at.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

fn read_journal(
    dir: &Path,
    reference: &EventJournal,
) -> Result<(Vec<crate::model::RunEvent>, u64), String> {
    let valid_generation = uuid::Uuid::parse_str(&reference.generation)
        .map(|id| id.to_string() == reference.generation)
        .unwrap_or(false);
    if !valid_generation {
        return Err("Monitter event journal reference is invalid; state was not changed.".into());
    }
    let path = journal_path(dir, reference);
    let length = fs::metadata(&path)
        .map_err(|e| {
            format!("Monitter event journal is missing or unreadable; state was not changed: {e}")
        })?
        .len();
    if reference.committed_count > length as usize {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    let file = File::open(&path).map_err(|e| {
        format!("Monitter event journal is missing or unreadable; state was not changed: {e}")
    })?;
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
        events.push(serde_json::from_slice(&line).map_err(|e| {
            format!("Monitter event journal is corrupt; state was not changed: {e}")
        })?);
    }
    if events.len() != reference.committed_count {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    if length < bytes {
        return Err("Monitter event journal is incomplete; state was not changed.".into());
    }
    Ok((events, bytes))
}

fn write_generation(
    dir: &Path,
    reference: &EventJournal,
    events: &[crate::model::RunEvent],
) -> Result<u64, String> {
    let path = journal_path(dir, reference);
    let mut file = private_create(&path)?;
    for event in events {
        serde_json::to_writer(&mut file, event)
            .map_err(|e| format!("Cannot encode Monitter event journal: {e}"))?;
        file.write_all(b"\n")
            .map_err(|e| format!("Cannot write Monitter event journal: {e}"))?;
    }
    file.sync_all()
        .map_err(|e| format!("Cannot make Monitter event journal durable: {e}"))?;
    Ok(file
        .metadata()
        .map_err(|e| format!("Cannot inspect Monitter event journal: {e}"))?
        .len())
}

fn append_events(
    dir: &Path,
    journal: &JournalState,
    events: &[crate::model::RunEvent],
) -> Result<(), String> {
    let path = journal_path(dir, &journal.reference);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Cannot open Monitter event journal: {e}"))?;
    // A prior failed pointer commit can leave a durable tail.  It was never
    // referenced, so remove it before retrying rather than treating it as data.
    file.set_len(journal.bytes)
        .map_err(|e| format!("Cannot discard uncommitted Monitter event journal tail: {e}"))?;
    file.seek(SeekFrom::End(0))
        .map_err(|e| format!("Cannot seek Monitter event journal: {e}"))?;
    for event in events {
        serde_json::to_writer(&mut file, event)
            .map_err(|e| format!("Cannot encode Monitter event journal: {e}"))?;
        file.write_all(b"\n")
            .map_err(|e| format!("Cannot append Monitter event journal: {e}"))?;
    }
    file.sync_all()
        .map_err(|e| format!("Cannot make Monitter event journal durable: {e}"))
}

#[cfg(unix)]
fn private_create(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Cannot create private Monitter state: {e}"))
}

#[cfg(not(unix))]
fn private_create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create Monitter state: {e}"))
}

#[cfg(unix)]
fn private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("Cannot secure Monitter data folder: {e}"))
}

#[cfg(not(unix))]
fn private_dir(_: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Cannot secure Monitter state: {e}"))
}

#[cfg(not(unix))]
fn private_file(_: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("monitter-{name}-{}", crate::model::id()))
    }

    #[test]
    fn corrupt_state_is_not_overwritten() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, b"not json").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(Store::open(dir.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_is_private_and_preserves_task_hosts() {
        let dir = temp_dir("private");
        let (store, snapshot, mut task_hosts, mut attachments) = Store::open(dir.clone()).unwrap();
        task_hosts.insert("task".into(), snapshot.hosts[0].clone());
        attachments.insert(
            "attachment".into(),
            StoredAttachment {
                attachment: crate::attachments::Attachment {
                    id: "attachment".into(),
                    name: "x.txt".into(),
                    mime_type: "text/plain".into(),
                    size: 1,
                    path: "/tmp/x".into(),
                    preview_data_url: None,
                    source_id: None,
                },
                host_id: snapshot.hosts[0].id.clone(),
                cwd: "/tmp".into(),
            },
        );
        store.save(&snapshot, &task_hosts, &attachments).unwrap();
        let (_, _, loaded, loaded_attachments) = Store::open(dir.clone()).unwrap();
        assert_eq!(loaded.get("task"), task_hosts.get("task"));
        assert_eq!(loaded_attachments, attachments);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(dir.join("state.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    fn event(id: &str) -> crate::model::RunEvent {
        crate::model::RunEvent {
            id: id.into(),
            task_id: "task".into(),
            kind: "tool".into(),
            title: "event".into(),
            detail: "x".into(),
            created_at: 1,
        }
    }

    #[test]
    fn migrates_legacy_events_to_a_small_core_and_reopens() {
        let dir = temp_dir("journal-migration");
        fs::create_dir_all(&dir).unwrap();
        let mut snapshot = default_snapshot();
        snapshot.events = (0..2000).map(|n| event(&format!("event-{n}"))).collect();
        let legacy = serde_json::to_vec_pretty(&DiskState {
            snapshot: snapshot.clone(),
            task_hosts: HashMap::new(),
            attachments: HashMap::new(),
            event_journal: None,
        })
        .unwrap();
        fs::write(dir.join("state.json"), &legacy).unwrap();
        let (store, loaded, hosts, attachments) = Store::open(dir.clone()).unwrap();
        assert_eq!(loaded.events, snapshot.events);
        // Startup has migrated the legacy state; unrelated saves stay compact.
        store.save(&loaded, &hosts, &attachments).unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["events"].as_array().unwrap().len(), 0);
        let journal = state["_eventJournal"].as_object().unwrap();
        assert_eq!(journal["committedCount"], 2000);
        let backup = dir
            .read_dir()
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("state.legacy-")
            })
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), legacy);
        let (_, reopened, _, _) = Store::open(dir.clone()).unwrap();
        assert_eq!(reopened.events, snapshot.events);
        assert!(fs::metadata(dir.join("state.json")).unwrap().len() < 20_000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_core_commit_keeps_only_the_committed_journal_prefix() {
        let dir = temp_dir("journal-core-failure");
        let (mut store, mut snapshot, hosts, attachments) = Store::open(dir.clone()).unwrap();
        snapshot.events = vec![event("one")];
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let original_state = fs::read(dir.join("state.json")).unwrap();
        store.path = dir.join("state-write-blocked");
        fs::create_dir(&store.path).unwrap();
        snapshot.events.push(event("two"));
        assert!(store.save(&snapshot, &hosts, &attachments).is_err());
        store.path = dir.join("state.json");
        let (_, reopened, _, _) = Store::open(dir.clone()).unwrap();
        assert_eq!(reopened.events, vec![event("one")]);
        assert_eq!(fs::read(dir.join("state.json")).unwrap(), original_state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn eight_megabyte_history_keeps_core_save_small_and_bounded() {
        let dir = temp_dir("journal-large-history");
        let (store, mut snapshot, hosts, attachments) = Store::open(dir.clone()).unwrap();
        snapshot.events = (0..2048)
            .map(|n| crate::model::RunEvent {
                detail: "x".repeat(4096),
                ..event(&format!("large-{n}"))
            })
            .collect();
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let started = std::time::Instant::now();
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let elapsed = started.elapsed();
        let core_bytes = fs::metadata(dir.join("state.json")).unwrap().len();
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        let generation = state["_eventJournal"]["generation"].as_str().unwrap();
        let journal_bytes = fs::metadata(dir.join(format!("events-{generation}.jsonl")))
            .unwrap()
            .len();
        eprintln!(
            "8MiB fixture: core={core_bytes}B journal={journal_bytes}B subsequent_core_save={elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert!(core_bytes < 25_000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn appends_and_rebuilds_generation_for_event_edits_or_deletes() {
        let dir = temp_dir("journal-generations");
        let (store, mut snapshot, hosts, attachments) = Store::open(dir.clone()).unwrap();
        snapshot.events = vec![event("one")];
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        let generation = first["_eventJournal"]["generation"]
            .as_str()
            .unwrap()
            .to_owned();
        // Simulate a crash after an event append reached disk but before the
        // matching state pointer commit. A retry must remove that tail.
        let journal_path = dir.join(format!("events-{generation}.jsonl"));
        OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .unwrap()
            .write_all(b"uncommitted tail\n")
            .unwrap();
        snapshot.events.push(event("two"));
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let appended: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(appended["_eventJournal"]["generation"], generation);
        assert!(!String::from_utf8(fs::read(&journal_path).unwrap())
            .unwrap()
            .contains("uncommitted tail"));
        snapshot.events[0].detail = "changed".into();
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let edited: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        assert_ne!(edited["_eventJournal"]["generation"], generation);
        snapshot.events.pop();
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let (_, reopened, _, _) = Store::open(dir.clone()).unwrap();
        assert_eq!(reopened.events, snapshot.events);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_or_corrupt_referenced_journal_refuses_open() {
        let dir = temp_dir("journal-corrupt");
        let (store, mut snapshot, hosts, attachments) = Store::open(dir.clone()).unwrap();
        snapshot.events = vec![event("one")];
        store.save(&snapshot, &hosts, &attachments).unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("state.json")).unwrap()).unwrap();
        let generation = state["_eventJournal"]["generation"].as_str().unwrap();
        fs::write(
            dir.join(format!("events-{generation}.jsonl")),
            b"not json\n",
        )
        .unwrap();
        assert!(Store::open(dir.clone()).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
