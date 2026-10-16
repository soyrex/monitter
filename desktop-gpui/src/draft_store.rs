//! Private GUI-only storage for unsent chat-creation forms.
//!
//! This file is intentionally separate from the backend-owned native UI
//! preferences and task database. It stores only bounded form state keyed by
//! the stable UUID of the draft pane.

use crate::draft_creation::{DraftFormState, DraftModelSelection};
use monitter_lib::experimental_desktop::{profile_dir, APP_ID};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const FILE_NAME: &str = "draft-forms.json";
const LOCK_NAME: &str = ".draft-forms.lock";
const SCHEMA_VERSION: u64 = 1;
const MAX_DRAFTS: usize = 64;
const MAX_DRAFT_BYTES: usize = 128 * 1024;
const MAX_STORE_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_BYTES: usize = 32 * 1024;
const MAX_TEXT_FIELD_BYTES: usize = 8 * 1024;
const MAX_ID_BYTES: usize = 256;
const MAX_MODEL_CACHES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DraftStoreSnapshot {
    pub revision: String,
    pub drafts: BTreeMap<String, DraftFormState>,
}

#[derive(Clone, Debug)]
pub(super) struct DraftStore {
    directory: std::path::PathBuf,
}

impl DraftStore {
    /// Open the already-created experimental profile. This never creates the
    /// profile directory or touches backend-owned preference/database files.
    pub(super) fn open() -> Result<Self, String> {
        let directory = profile_dir()?;
        validate_directory(&directory)?;
        Ok(Self { directory })
    }

    pub(super) fn load(&self) -> Result<DraftStoreSnapshot, String> {
        let lock = ProfileLock::acquire(&self.directory, false)?;
        read_snapshot(&lock.directory)
    }

    /// Replace the form map only if its revision still matches the caller's
    /// snapshot. All validation/encoding is complete before the atomic rename.
    pub(super) fn save(
        &self,
        expected_revision: &str,
        drafts: &BTreeMap<String, DraftFormState>,
    ) -> Result<DraftStoreSnapshot, String> {
        validate_drafts(drafts)?;
        let lock = ProfileLock::acquire(&self.directory, true)?;
        let current = read_snapshot(&lock.directory)?;
        if current.revision != expected_revision {
            return Err("Draft forms changed; reload before saving.".into());
        }
        let revision = uuid::Uuid::new_v4().to_string();
        let value = snapshot_value(&revision, drafts);
        let bytes = serde_json::to_vec(&value).map_err(|_| "Cannot encode draft forms.")?;
        if bytes.len() > MAX_STORE_BYTES {
            return Err("Draft forms exceed the 512 KiB store limit.".into());
        }
        write_atomic(&lock.directory, &bytes)?;
        Ok(DraftStoreSnapshot {
            revision,
            drafts: drafts.clone(),
        })
    }
}

fn validate_drafts(drafts: &BTreeMap<String, DraftFormState>) -> Result<(), String> {
    if drafts.len() > MAX_DRAFTS {
        return Err("Draft forms exceed the 64-form limit.".into());
    }
    for (id, state) in drafts {
        if !uuid::Uuid::parse_str(id)
            .is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == *id)
        {
            return Err("Draft form has an invalid pane UUID.".into());
        }
        validate_draft(state)?;
    }
    Ok(())
}

fn validate_draft(state: &DraftFormState) -> Result<(), String> {
    for (name, value, limit) in [
        ("agent ID", state.agent_id.as_str(), MAX_ID_BYTES),
        ("project ID", state.project_id.as_str(), MAX_ID_BYTES),
        ("title", state.title.as_str(), MAX_TEXT_FIELD_BYTES),
        ("working folder", state.cwd.as_str(), MAX_TEXT_FIELD_BYTES),
        ("message", state.message.as_str(), MAX_MESSAGE_BYTES),
        ("access mode", state.sandbox.as_str(), MAX_ID_BYTES),
        ("model ID", state.selected_model.as_str(), MAX_ID_BYTES),
    ] {
        if value.len() > limit {
            return Err(format!("Draft form {name} exceeds its {limit}-byte limit."));
        }
    }
    for (name, value) in [
        ("parent task ID", state.parent_task_id.as_deref()),
        ("reasoning effort", state.selected_effort.as_deref()),
        ("created task ID", state.created_task_id.as_deref()),
    ] {
        if value.is_some_and(|value| value.len() > MAX_ID_BYTES) {
            return Err(format!(
                "Draft form {name} exceeds its {MAX_ID_BYTES}-byte limit."
            ));
        }
    }
    if state.parent_task_id.as_deref() == Some("") || state.created_task_id.as_deref() == Some("") {
        return Err("Draft form contains an empty task ID.".into());
    }
    if state.model_drafts.len() > MAX_MODEL_CACHES {
        return Err("Draft form has too many cached model selections.".into());
    }
    let mut total = state.agent_id.len()
        + state.project_id.len()
        + state.parent_task_id.as_deref().unwrap_or_default().len()
        + state.title.len()
        + state.cwd.len()
        + state.message.len()
        + state.sandbox.len()
        + state.selected_model.len()
        + state.selected_effort.as_deref().unwrap_or_default().len()
        + state.created_task_id.as_deref().unwrap_or_default().len();
    for (key, selection) in &state.model_drafts {
        if key.len() > MAX_ID_BYTES * 2 + 1
            || selection.model.len() > MAX_ID_BYTES
            || selection
                .effort
                .as_deref()
                .is_some_and(|v| v.len() > MAX_ID_BYTES)
        {
            return Err("Draft form contains an oversized model selection.".into());
        }
        total = total
            .checked_add(key.len())
            .and_then(|value| value.checked_add(selection.model.len()))
            .and_then(|value| {
                value.checked_add(selection.effort.as_deref().unwrap_or_default().len())
            })
            .ok_or("Draft form size overflow.")?;
    }
    if total > MAX_DRAFT_BYTES {
        return Err("Draft form exceeds the 128 KiB per-form limit.".into());
    }
    let encoded =
        serde_json::to_vec(&draft_value(state)).map_err(|_| "Cannot encode draft form.")?;
    if encoded.len() > MAX_DRAFT_BYTES {
        return Err("Encoded draft form exceeds the 128 KiB per-form limit.".into());
    }
    Ok(())
}

fn snapshot_value(revision: &str, drafts: &BTreeMap<String, DraftFormState>) -> Value {
    let drafts = drafts
        .iter()
        .map(|(id, state)| (id.clone(), draft_value(state)))
        .collect::<Map<_, _>>();
    json!({"version": SCHEMA_VERSION, "revision": revision, "drafts": drafts})
}

fn draft_value(state: &DraftFormState) -> Value {
    let model_drafts = state
        .model_drafts
        .iter()
        .map(|(key, selection)| {
            (
                key.clone(),
                json!({
                    "model": selection.model,
                    "effort": selection.effort,
                    "fastMode": selection.fast_mode,
                }),
            )
        })
        .collect::<Map<_, _>>();
    json!({
        "agentId": state.agent_id,
        "projectId": state.project_id,
        "parentTaskId": state.parent_task_id,
        "title": state.title,
        "cwd": state.cwd,
        "message": state.message,
        "sandbox": state.sandbox,
        "selectedModel": state.selected_model,
        "selectedEffort": state.selected_effort,
        "selectedFastMode": state.selected_fast_mode,
        "modelDrafts": model_drafts,
        "createdTaskId": state.created_task_id,
    })
}

fn read_snapshot(directory: &std::fs::File) -> Result<DraftStoreSnapshot, String> {
    let bytes = match read_file(directory, FILE_NAME, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error == "Draft form store does not exist." => {
            return Ok(DraftStoreSnapshot {
                revision: "0".into(),
                drafts: BTreeMap::new(),
            });
        }
        Err(error) => return Err(error),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid draft form store.")?;
    parse_snapshot(value)
}

fn parse_snapshot(value: Value) -> Result<DraftStoreSnapshot, String> {
    let object = exact_object(&value, &["version", "revision", "drafts"])?;
    if object.get("version").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err("Unsupported draft form store version.".into());
    }
    let revision = object
        .get("revision")
        .and_then(Value::as_str)
        .ok_or("Invalid draft form store revision.")?
        .to_owned();
    if revision != "0"
        && !uuid::Uuid::parse_str(&revision)
            .is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == revision)
    {
        return Err("Invalid draft form store revision.".into());
    }
    let draft_values = object
        .get("drafts")
        .and_then(Value::as_object)
        .ok_or("Invalid draft form map.")?;
    if draft_values.len() > MAX_DRAFTS {
        return Err("Draft forms exceed the 64-form limit.".into());
    }
    let mut drafts = BTreeMap::new();
    for (id, value) in draft_values {
        let state = parse_draft(value)?;
        drafts.insert(id.clone(), state);
    }
    validate_drafts(&drafts)?;
    Ok(DraftStoreSnapshot { revision, drafts })
}

fn parse_draft(value: &Value) -> Result<DraftFormState, String> {
    let object = exact_object(
        value,
        &[
            "agentId",
            "projectId",
            "parentTaskId",
            "title",
            "cwd",
            "message",
            "sandbox",
            "selectedModel",
            "selectedEffort",
            "selectedFastMode",
            "modelDrafts",
            "createdTaskId",
        ],
    )?;
    let model_values = object
        .get("modelDrafts")
        .and_then(Value::as_object)
        .ok_or("Invalid cached model selection map.")?;
    if model_values.len() > MAX_MODEL_CACHES {
        return Err("Draft form has too many cached model selections.".into());
    }
    let mut model_drafts = std::collections::HashMap::new();
    for (key, value) in model_values {
        let selection = exact_object(value, &["model", "effort", "fastMode"])?;
        model_drafts.insert(
            key.clone(),
            DraftModelSelection {
                model: required_string(selection, "model")?,
                effort: optional_string(selection, "effort")?,
                fast_mode: optional_bool(selection, "fastMode")?,
            },
        );
    }
    Ok(DraftFormState {
        agent_id: required_string(object, "agentId")?,
        project_id: required_string(object, "projectId")?,
        parent_task_id: optional_string(object, "parentTaskId")?,
        title: required_string(object, "title")?,
        cwd: required_string(object, "cwd")?,
        message: required_string(object, "message")?,
        sandbox: required_string(object, "sandbox")?,
        selected_model: required_string(object, "selectedModel")?,
        selected_effort: optional_string(object, "selectedEffort")?,
        selected_fast_mode: optional_bool(object, "selectedFastMode")?,
        model_drafts,
        created_task_id: optional_string(object, "createdTaskId")?,
    })
}

fn exact_object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let object = value.as_object().ok_or("Invalid draft form object.")?;
    if object.len() != keys.len() || object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("Unexpected or missing field in draft form store.".into());
    }
    Ok(object)
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or("Invalid draft form string field.".into())
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err("Invalid optional draft form string field.".into()),
    }
}

fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err("Invalid optional draft form boolean field.".into()),
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::CString,
        fs::File,
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd, RawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };

    fn open_directory(path: &std::path::Path) -> Result<File, String> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "Invalid draft form profile path.")?;
        // SAFETY: path is NUL-terminated and the returned descriptor is owned below.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err("Cannot safely open experimental profile for draft forms.".into());
        }
        // SAFETY: open returned a fresh descriptor owned by this function.
        let directory = unsafe { File::from_raw_fd(fd) };
        let metadata = directory
            .metadata()
            .map_err(|_| "Cannot inspect experimental profile.")?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err("Experimental profile is not a private owned directory.".into());
        }
        Ok(directory)
    }

    pub(super) fn validate_directory(path: &std::path::Path) -> Result<(), String> {
        if path.file_name().and_then(|name| name.to_str()) != Some(APP_ID) {
            return Err("Only the isolated experimental profile is allowed.".into());
        }
        open_directory(path).map(drop)
    }

    fn openat_file(
        dirfd: RawFd,
        name: &str,
        flags: i32,
        mode: libc::mode_t,
    ) -> std::io::Result<File> {
        let name = CString::new(name).expect("static or generated file name has no NUL");
        // SAFETY: dirfd is a retained directory descriptor and name is NUL-terminated.
        let fd = unsafe {
            libc::openat(
                dirfd,
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: openat returned a new owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    fn checked_file(file: &File) -> Result<(), String> {
        let metadata = file
            .metadata()
            .map_err(|_| "Cannot inspect draft form file.")?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err("Draft form file is not private and safely owned.".into());
        }
        Ok(())
    }

    pub(super) struct ProfileLock {
        pub(super) directory: File,
        _lock_file: File,
    }
    impl ProfileLock {
        pub(super) fn acquire(
            directory: &std::path::Path,
            exclusive: bool,
        ) -> Result<Self, String> {
            let directory = open_directory(directory)?;
            let file = openat_file(
                directory.as_raw_fd(),
                LOCK_NAME,
                libc::O_RDWR | libc::O_CREAT,
                0o600,
            )
            .map_err(|_| "Cannot open private draft form lock.")?;
            checked_file(&file)?;
            // SAFETY: flock operates on this open lock descriptor and is released on drop.
            let operation = if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            };
            if unsafe { libc::flock(file.as_raw_fd(), operation) } != 0 {
                return Err("Cannot lock draft form store.".into());
            }
            Ok(Self {
                directory,
                _lock_file: file,
            })
        }
    }

    fn read_file(directory: &File, name: &str, limit: usize) -> Result<Vec<u8>, String> {
        let mut file = match openat_file(
            directory.as_raw_fd(),
            name,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err("Draft form store does not exist.".into());
            }
            Err(_) => return Err("Cannot safely open draft form store.".into()),
        };
        checked_file(&file)?;
        if file
            .metadata()
            .map_err(|_| "Cannot inspect draft form store.")?
            .len()
            > limit as u64
        {
            return Err("Draft form store exceeds the 512 KiB limit.".into());
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read draft form store.")?;
        if bytes.len() > limit {
            return Err("Draft form store exceeds the 512 KiB limit.".into());
        }
        Ok(bytes)
    }

    fn write_atomic(directory: &File, bytes: &[u8]) -> Result<(), String> {
        let temp_name = format!(".draft-forms-{}.tmp", uuid::Uuid::new_v4());
        let destination = CString::new(FILE_NAME).unwrap();
        let temp = CString::new(temp_name.clone()).unwrap();
        let result = (|| {
            let mut file = openat_file(
                directory.as_raw_fd(),
                &temp_name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )
            .map_err(|_| "Cannot create private draft form temporary file.")?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Cannot write draft form store.")?;
            // SAFETY: both names are NUL-terminated basenames in this directory.
            if unsafe {
                libc::renameat(
                    directory.as_raw_fd(),
                    temp.as_ptr(),
                    directory.as_raw_fd(),
                    destination.as_ptr(),
                )
            } != 0
            {
                return Err("Cannot atomically replace draft form store.".into());
            }
            directory
                .sync_all()
                .map_err(|_| "Draft form store replaced, but directory sync failed.")?;
            Ok(())
        })();
        if result.is_err() {
            // SAFETY: same opened directory and generated NUL-terminated basename.
            unsafe { libc::unlinkat(directory.as_raw_fd(), temp.as_ptr(), 0) };
        }
        result
    }
}

#[cfg(unix)]
use unix::{read_file, validate_directory, write_atomic, ProfileLock};

#[cfg(not(unix))]
fn validate_directory(_: &std::path::Path) -> Result<(), String> {
    Err("Private draft form storage is unsupported on this platform.".into())
}
#[cfg(not(unix))]
struct ProfileLock {
    directory: std::fs::File,
}
#[cfg(not(unix))]
impl ProfileLock {
    fn acquire(_: &std::path::Path, _: bool) -> Result<Self, String> {
        Err("Private draft form storage is unsupported on this platform.".into())
    }
}
#[cfg(not(unix))]
fn read_file(_: &std::fs::File, _: &str, _: usize) -> Result<Vec<u8>, String> {
    Err("Private draft form storage is unsupported on this platform.".into())
}
#[cfg(not(unix))]
fn write_atomic(_: &std::fs::File, _: &[u8]) -> Result<(), String> {
    Err("Private draft form storage is unsupported on this platform.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (std::path::PathBuf, DraftStore) {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("draft-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let directory = root.join(APP_ID);
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        validate_directory(&directory).unwrap();
        let store = DraftStore {
            directory: directory.clone(),
        };
        (root, store)
    }

    fn state(message: &str) -> DraftFormState {
        DraftFormState {
            agent_id: "agent".into(),
            project_id: "project".into(),
            parent_task_id: None,
            title: "Title".into(),
            cwd: "/tmp".into(),
            message: message.into(),
            sandbox: "read-only".into(),
            selected_model: "model".into(),
            selected_effort: Some("medium".into()),
            selected_fast_mode: Some(false),
            model_drafts: [(
                "agent:project".into(),
                DraftModelSelection {
                    model: "model".into(),
                    effort: Some("medium".into()),
                    fast_mode: Some(false),
                },
            )]
            .into_iter()
            .collect(),
            created_task_id: None,
        }
    }

    #[::core::prelude::v1::test]
    fn typed_forms_round_trip_and_revision_cas_rejects_stale_writer() {
        let (root, store) = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        let mut forms = BTreeMap::new();
        forms.insert(id.clone(), state("unsent content"));
        let initial = store.load().unwrap();
        let saved = store.save(&initial.revision, &forms).unwrap();
        assert_eq!(store.load().unwrap(), saved);
        assert!(store.save(&initial.revision, &forms).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[::core::prelude::v1::test]
    fn rejects_invalid_uuid_and_oversize_state_before_replacing_file() {
        let (root, store) = fixture();
        let initial = store.load().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let mut forms = BTreeMap::from([(id.clone(), state("kept"))]);
        let saved = store.save(&initial.revision, &forms).unwrap();
        let path = store.directory.join(FILE_NAME);
        let before = std::fs::read(&path).unwrap();
        forms.insert("not-a-uuid".into(), state("bad"));
        assert!(store.save(&saved.revision, &forms).is_err());
        forms.remove("not-a-uuid");
        forms.insert(id, state(&"x".repeat(MAX_MESSAGE_BYTES + 1)));
        assert!(store.save(&saved.revision, &forms).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[::core::prelude::v1::test]
    fn refuses_symlinked_store_without_touching_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (root, store) = fixture();
        let target = store.directory.join("target");
        std::fs::write(&target, b"untouched").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, store.directory.join(FILE_NAME)).unwrap();
        assert!(store.load().is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"untouched");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[::core::prelude::v1::test]
    fn locked_io_stays_on_opened_profile_inode_after_path_swap() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let (root, store) = fixture();
        let draft_id = uuid::Uuid::new_v4().to_string();
        let initial = store.load().unwrap();
        let saved = store
            .save(
                &initial.revision,
                &BTreeMap::from([(draft_id.clone(), state("anchored"))]),
            )
            .unwrap();
        let original_ino = std::fs::metadata(&store.directory).unwrap().ino();
        let lock = ProfileLock::acquire(&store.directory, true).unwrap();

        let moved = root.join("moved-profile");
        std::fs::rename(&store.directory, &moved).unwrap();
        std::fs::create_dir(&store.directory).unwrap();
        std::fs::set_permissions(&store.directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_ne!(
            std::fs::metadata(&store.directory).unwrap().ino(),
            original_ino
        );
        assert_eq!(lock.directory.metadata().unwrap().ino(), original_ino);

        assert_eq!(read_snapshot(&lock.directory).unwrap(), saved);
        let replacement = snapshot_value(&uuid::Uuid::new_v4().to_string(), &BTreeMap::new());
        let bytes = serde_json::to_vec(&replacement).unwrap();
        write_atomic(&lock.directory, &bytes).unwrap();
        assert!(moved.join(FILE_NAME).is_file());
        assert!(!store.directory.join(FILE_NAME).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
