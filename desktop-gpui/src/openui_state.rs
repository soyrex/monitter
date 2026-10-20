//! Bounded, profile-local presentation state. No generated UI chooses a path.
//! File work runs on one worker; rendering reads only the process cache.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
};

const MAX_ENTRIES: usize = 256;
const MAX_STATE_BYTES: usize = 64 * 1024;
const MAX_FILE_BYTES: u64 = (MAX_STATE_BYTES + 4096) as u64;
const DIRECTORY: &str = "openui-state-v1";

#[derive(Clone, Debug)]
pub enum Read {
    Loading,
    Ready(Option<Value>),
    Failed(String),
}
struct Entry {
    result: Read,
    generation: u64,
    touched: u64,
    read: bool,
    write: bool,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    clock: u64,
}
struct Store {
    cache: Arc<Mutex<Cache>>,
    wake: mpsc::SyncSender<()>,
}
static STORE: OnceLock<Store> = OnceLock::new();

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    key: String,
    state: Value,
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control)
}
fn validate(value: &Value) -> Result<(), String> {
    fn walk(value: &Value, depth: usize) -> bool {
        if depth > 12 {
            return false;
        }
        match value {
            Value::String(s) => s.len() <= 16 * 1024,
            Value::Array(a) => a.len() <= 256 && a.iter().all(|v| walk(v, depth + 1)),
            Value::Object(o) => {
                o.len() <= 256 && o.iter().all(|(k, v)| k.len() <= 256 && walk(v, depth + 1))
            }
            _ => true,
        }
    }
    if !walk(value, 0)
        || serde_json::to_vec(value)
            .map_err(|_| "Invalid form state".to_string())?
            .len()
            > MAX_STATE_BYTES
    {
        return Err("Form state exceeds the local storage limit".into());
    }
    Ok(())
}

fn store() -> &'static Store {
    STORE.get_or_init(|| {
        let cache = Arc::new(Mutex::new(Cache::default()));
        let (wake, receiver) = mpsc::sync_channel(1);
        let worker_cache = cache.clone();
        std::thread::Builder::new()
            .name("openui-state".into())
            .spawn(move || {
                let directory =
                    monitter_lib::experimental_desktop::profile_dir().map(|p| p.join(DIRECTORY));
                while receiver.recv().is_ok() {
                    let jobs = {
                        let mut cache = worker_cache.lock().unwrap_or_else(|p| p.into_inner());
                        cache
                            .entries
                            .iter_mut()
                            .filter_map(|(key, entry)| {
                                let job = if entry.write {
                                    match &entry.result {
                                        Read::Ready(Some(value)) => Some((
                                            key.clone(),
                                            entry.generation,
                                            Some(value.clone()),
                                        )),
                                        _ => None,
                                    }
                                } else if entry.read {
                                    Some((key.clone(), entry.generation, None))
                                } else {
                                    None
                                };
                                entry.read = false;
                                entry.write = false;
                                job
                            })
                            .collect::<Vec<_>>()
                    };
                    for (key, generation, value) in jobs {
                        let result = match &directory {
                            Err(_) => {
                                Err("Could not locate the local form-state profile".to_string())
                            }
                            Ok(directory) => match value {
                                Some(ref value) => {
                                    write_file(directory, &key, value).map(|_| Some(value.clone()))
                                }
                                None => read_file(directory, &key),
                            },
                        };
                        let mut cache = worker_cache.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(entry) = cache
                            .entries
                            .get_mut(&key)
                            .filter(|e| e.generation == generation)
                        {
                            entry.result = match result {
                                Ok(value) => Read::Ready(value),
                                Err(error) => Read::Failed(error),
                            };
                        }
                    }
                }
            })
            .expect("start local OpenUI presentation-state worker");
        Store { cache, wake }
    })
}

fn room(cache: &mut Cache, key: &str) -> bool {
    if cache.entries.contains_key(key) || cache.entries.len() < MAX_ENTRIES {
        return true;
    }
    let oldest = cache
        .entries
        .iter()
        .filter(|(_, e)| !e.write && !e.read && !matches!(e.result, Read::Loading))
        .min_by_key(|(_, e)| e.touched)
        .map(|(k, _)| k.clone());
    if let Some(key) = oldest {
        cache.entries.remove(&key);
        true
    } else {
        false
    }
}

pub fn read(key: &str) -> Read {
    if !valid_key(key) {
        return Read::Failed("Invalid form-state identity".into());
    }
    let store = store();
    let mut cache = store.cache.lock().unwrap_or_else(|p| p.into_inner());
    if !room(&mut cache, key) {
        return Read::Failed("Too many pending form-state operations".into());
    }
    cache.clock = cache.clock.wrapping_add(1);
    let clock = cache.clock;
    let entry = cache.entries.entry(key.to_owned()).or_insert(Entry {
        result: Read::Loading,
        generation: clock,
        touched: clock,
        read: true,
        write: false,
    });
    entry.touched = clock;
    let result = entry.result.clone();
    if entry.read {
        let _ = store.wake.try_send(());
    }
    result
}

pub fn save(key: String, state: Value) -> Result<(), String> {
    if !valid_key(&key) {
        return Err("Invalid form-state identity".into());
    }
    validate(&state)?;
    let store = store();
    let mut cache = store.cache.lock().unwrap_or_else(|p| p.into_inner());
    if !room(&mut cache, &key) {
        return Err("Too many pending form-state operations".into());
    }
    // A failed disk write must be visible to the caller rather than silently
    // presenting the next change as durably saved.
    if let Some(Entry {
        result: Read::Failed(error),
        ..
    }) = cache.entries.get(&key)
    {
        return Err(error.clone());
    }
    cache.clock = cache.clock.wrapping_add(1);
    let clock = cache.clock;
    cache.entries.insert(
        key,
        Entry {
            result: Read::Ready(Some(state)),
            generation: clock,
            touched: clock,
            read: false,
            write: true,
        },
    );
    store
        .wake
        .try_send(())
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => String::new(),
            mpsc::TrySendError::Disconnected(_) => "Local form-state worker stopped".into(),
        })
        .or_else(|e| if e.is_empty() { Ok(()) } else { Err(e) })
}

fn path(directory: &Path, key: &str) -> PathBuf {
    // Stable filename, never an authorization token. Stored key verification
    // detects a hash collision before either reading or overwriting a value.
    let mut hash = 0xcbf29ce484222325u64;
    for b in key.bytes() {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    directory.join(format!("state-{hash:016x}.json"))
}

fn read_file(directory: &Path, key: &str) -> Result<Option<Value>, String> {
    if let Ok(metadata) = std::fs::symlink_metadata(directory) {
        if !metadata.is_dir() {
            return Err("Invalid form-state storage directory".into());
        }
    }
    let file = path(directory, key);
    let metadata = match std::fs::symlink_metadata(&file) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read saved form state".into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("Invalid saved form-state file".into());
    }
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| "Could not read saved form state".to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Saved form state exceeds the limit".into());
    }
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid saved form state".to_string())?;
    if envelope.version != 1 || envelope.key != key {
        return Err("Saved form-state identity does not match".into());
    }
    validate(&envelope.state)?;
    Ok(Some(envelope.state))
}

fn write_file(directory: &Path, key: &str, state: &Value) -> Result<(), String> {
    use std::io::Write;
    validate(state)?;
    std::fs::create_dir_all(directory)
        .map_err(|_| "Could not create local form-state storage".to_string())?;
    if !std::fs::symlink_metadata(directory)
        .map_err(|_| "Could not inspect form-state storage".to_string())?
        .is_dir()
    {
        return Err("Invalid form-state storage directory".into());
    }
    // Do not replace a colliding, malformed or externally substituted file.
    read_file(directory, key)?;
    prune(directory, &path(directory, key))?;
    let bytes = serde_json::to_vec(&Envelope {
        version: 1,
        key: key.to_owned(),
        state: state.clone(),
    })
    .map_err(|_| "Could not encode form state".to_string())?;
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = directory.join(format!(".write-{}-{serial}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "Could not create form-state snapshot".to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not save form state".to_string())?;
        std::fs::rename(&temporary, path(directory, key))
            .map_err(|_| "Could not commit form-state snapshot".to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn prune(directory: &Path, keep: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)
        .map_err(|_| "Could not inspect form-state cache".to_string())?
        .take(1025)
    {
        let entry = entry.map_err(|_| "Could not inspect form-state cache entry".to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.len() != 27
            || !name.starts_with("state-")
            || !name.ends_with(".json")
            || !name[6..22].bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|_| "Could not inspect form-state cache entry".to_string())?;
        if metadata.is_file() {
            files.push((
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                entry.path(),
            ));
        }
    }
    files.sort_by_key(|(time, _)| *time);
    let remove = files
        .len()
        .saturating_sub(MAX_ENTRIES - usize::from(!keep.exists()));
    for (_, path) in files.into_iter().filter(|(_, p)| p != keep).take(remove) {
        std::fs::remove_file(path)
            .map_err(|_| "Could not expire old form-state cache entry".to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod openui_state_tests {
    use super::*;
    fn temporary() -> PathBuf {
        std::env::temp_dir().join(format!("monitter-openui-state-{}", uuid::Uuid::new_v4()))
    }
    #[test]
    fn independent_keys_round_trip_across_cache_recreation() {
        let dir = temporary();
        let a = serde_json::json!({"name":{"value":"Alex","componentType":"Input"}});
        write_file(&dir, "task/message/0", &a).unwrap();
        write_file(
            &dir,
            "task/message/1",
            &serde_json::json!({"approved":false}),
        )
        .unwrap();
        assert_eq!(read_file(&dir, "task/message/0").unwrap(), Some(a));
        assert_eq!(read_file(&dir, "task/other/0").unwrap(), None);
        assert!(read_file(&dir, "task/message/1").unwrap().is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path(&dir, "task/message/0"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_oversize_deep_or_mismatched_state() {
        assert!(validate(&Value::String("a".repeat(16 * 1024 + 1))).is_err());
        let mut deep = Value::Null;
        for _ in 0..14 {
            deep = serde_json::json!([deep]);
        }
        assert!(validate(&deep).is_err());
        let dir = temporary();
        write_file(&dir, "one", &Value::Bool(true)).unwrap();
        std::fs::copy(path(&dir, "one"), path(&dir, "two")).unwrap();
        assert!(read_file(&dir, "two").is_err());
        assert!(write_file(&dir, "two", &Value::Bool(false)).is_err());
        assert!(!valid_key("../\n"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cache_evicts_only_settled_entries() {
        let mut cache = Cache::default();
        for n in 0..MAX_ENTRIES {
            cache.entries.insert(
                n.to_string(),
                Entry {
                    result: Read::Loading,
                    generation: 0,
                    touched: n as u64,
                    read: true,
                    write: false,
                },
            );
        }
        assert!(!room(&mut cache, "extra"));
        let entry = cache.entries.get_mut("0").unwrap();
        entry.result = Read::Ready(None);
        entry.read = false;
        assert!(room(&mut cache, "extra"));
        assert!(!cache.entries.contains_key("0"));
    }
}
