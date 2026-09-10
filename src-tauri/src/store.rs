use crate::model::{default_snapshot, now, Host, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct DiskState {
    #[serde(flatten)]
    snapshot: Snapshot,
    #[serde(rename = "_taskHosts", default)]
    task_hosts: HashMap<String, Host>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<(Self, Snapshot, HashMap<String, Host>), String> {
        fs::create_dir_all(&dir).map_err(|e| format!("Cannot create Monitter data folder: {e}"))?;
        private_dir(&dir)?;
        let path = dir.join("state.json");
        let existed = path.exists();
        let (mut snapshot, mut task_hosts) = if existed {
            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("Cannot read Monitter state: {e}"))?;
            let data: DiskState = serde_json::from_str(&raw)
                .map_err(|e| format!("Monitter state is corrupt; it was not overwritten: {e}"))?;
            (data.snapshot, data.task_hosts)
        } else {
            (default_snapshot(), HashMap::new())
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
        for task in &mut snapshot.tasks {
            if task.status == "running" {
                task.status = "interrupted".into();
                task.updated_at = now();
                recovered = true;
            }
        }
        let had_running_deliveries = snapshot
            .collaborations
            .iter()
            .any(|delivery| delivery.status == "running");
        crate::Service::recover_collaborations(&mut snapshot);
        recovered |= had_running_deliveries;
        let store = Self { path };
        if !existed || recovered {
            store.save(&snapshot, &task_hosts)?;
        }
        Ok((store, snapshot, task_hosts))
    }

    pub fn save(
        &self,
        snapshot: &Snapshot,
        task_hosts: &HashMap<String, Host>,
    ) -> Result<(), String> {
        let temp = self
            .path
            .with_extension(format!("tmp-{}", crate::model::id()));
        let json = serde_json::to_vec_pretty(&DiskState {
            snapshot: snapshot.clone(),
            task_hosts: task_hosts.clone(),
        })
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
        let (store, snapshot, mut task_hosts) = Store::open(dir.clone()).unwrap();
        task_hosts.insert("task".into(), snapshot.hosts[0].clone());
        store.save(&snapshot, &task_hosts).unwrap();
        let (_, _, loaded) = Store::open(dir.clone()).unwrap();
        assert_eq!(loaded.get("task"), task_hosts.get("task"));
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
}
