//! Explicit blank-profile creation. This intentionally never inspects or
//! migrates an existing profile directory.

use crate::{model::default_snapshot, store::Store};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

/// Creates one new, empty Monitter profile at an exact absolute path.
///
/// The caller must choose a nonexistent directory below an existing parent.
/// Existing profiles, including empty ones, are refused without being opened.
/// On an initialization error the new directory is deliberately retained for
/// inspection; this function never removes or modifies pre-existing data.
pub fn initialize_blank_profile(directory: PathBuf) -> Result<(), String> {
    if !directory.is_absolute() {
        return Err("Blank profile path must be absolute.".into());
    }
    let parent = directory
        .parent()
        .ok_or_else(|| "Blank profile path has no parent directory.".to_string())?;
    let file_name = directory
        .file_name()
        .ok_or_else(|| "Blank profile path has no directory name.".to_string())?;
    if file_name.is_empty() {
        return Err("Blank profile path has no directory name.".into());
    }
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|e| format!("Blank profile parent is unavailable: {e}"))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err("Blank profile parent must be a real existing directory.".into());
    }
    match fs::symlink_metadata(&directory) {
        Ok(_) => {
            return Err(format!(
                "Refusing to initialize existing profile directory {}.",
                directory.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot inspect blank profile path: {error}")),
    }
    create_private_directory(&directory)?;

    let (store, _, _, _) = Store::open(directory.clone())?;
    let mut snapshot = default_snapshot();
    snapshot.agents.clear();
    store.save(&snapshot, &HashMap::new(), &HashMap::new())?;
    drop(store);

    let (store, snapshot, task_hosts, attachments) = Store::open(directory)?;
    validate_blank_profile(&snapshot, &task_hosts, &attachments)?;
    drop(store);
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(path)
            .map_err(|e| format!("Cannot create blank profile directory: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path).map_err(|e| format!("Cannot create blank profile directory: {e}"))?;
    }
    Ok(())
}

fn validate_blank_profile(
    snapshot: &crate::model::Snapshot,
    task_hosts: &HashMap<String, crate::model::Host>,
    attachments: &HashMap<String, crate::attachments::StoredAttachment>,
) -> Result<(), String> {
    let no_history = snapshot.agents.is_empty()
        && snapshot.tasks.is_empty()
        && snapshot.messages.is_empty()
        && snapshot.events.is_empty()
        && snapshot.channels.is_empty()
        && snapshot.projects.is_empty()
        && snapshot.collaborations.is_empty()
        && snapshot.subagent_sessions.is_empty()
        && snapshot.subagent_transcripts.is_empty()
        && snapshot.queued_messages.is_empty()
        && snapshot.approval_requests.is_empty()
        && snapshot.approval_rules.is_empty()
        && task_hosts.is_empty()
        && attachments.is_empty();
    if !no_history {
        return Err("Blank profile verification found persisted workspace data.".into());
    }
    if snapshot.hosts.len() != 1
        || snapshot.hosts[0].kind != "local"
        || snapshot.hosts[0].address != "localhost"
        || snapshot.settings != default_snapshot().settings
    {
        return Err("Blank profile verification found unexpected defaults.".into());
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

    fn temporary_parent(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = env::temp_dir().join(format!("monitter-profile-init-{name}-{unique}"));
        fs::create_dir(&parent).unwrap();
        parent
    }

    #[test]
    fn initializes_no_agents_history_or_preferences_from_another_profile() {
        let parent = temporary_parent("blank");
        let directory = parent.join("fresh-profile");
        initialize_blank_profile(directory.clone()).unwrap();
        let (store, snapshot, task_hosts, attachments) = Store::open(directory).unwrap();
        assert!(snapshot.agents.is_empty());
        assert!(snapshot.tasks.is_empty());
        assert!(snapshot.messages.is_empty());
        assert!(snapshot.events.is_empty());
        assert!(snapshot.channels.is_empty());
        assert!(snapshot.projects.is_empty());
        assert!(snapshot.collaborations.is_empty());
        assert!(snapshot.subagent_sessions.is_empty());
        assert!(snapshot.subagent_transcripts.is_empty());
        assert!(snapshot.queued_messages.is_empty());
        assert!(snapshot.approval_requests.is_empty());
        assert!(snapshot.approval_rules.is_empty());
        assert!(task_hosts.is_empty());
        assert!(attachments.is_empty());
        assert_eq!(snapshot.settings, default_snapshot().settings);
        drop(store);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn refuses_existing_directory_without_changing_its_bytes() {
        let parent = temporary_parent("existing");
        let directory = parent.join("existing-profile");
        fs::create_dir(&directory).unwrap();
        let marker = directory.join("state.sqlite3");
        let original = b"existing profile must not be opened".to_vec();
        fs::write(&marker, &original).unwrap();
        assert!(initialize_blank_profile(directory).is_err());
        assert_eq!(fs::read(marker).unwrap(), original);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn refuses_relative_directory() {
        assert!(initialize_blank_profile(PathBuf::from("new-profile")).is_err());
    }
}
