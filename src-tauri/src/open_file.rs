//! Native-only handler for a local link whose file type has no internal
//! viewer (Markdown, image, PDF, and HTML links already open as their own
//! document tab — see markdown.rs, image_file.rs, report_file.rs). Every
//! other file type hands off to the OS default application, the same outcome
//! as double-clicking it in Finder, instead of the link doing nothing.
use crate::{local_file_link::resolve_local_link, Service};
use std::fs;

impl Service {
    pub(crate) fn open_task_file(
        &self,
        task_id: &str,
        href: &str,
        base_path: Option<&str>,
    ) -> Result<(), String> {
        let (cwd, host_is_local) = {
            let data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.".to_string())?;
            let task = data
                .snapshot
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .ok_or("Task was not found.")?;
            let host_is_local = data
                .snapshot
                .hosts
                .iter()
                .find(|host| host.id == task.host_id)
                .is_some_and(|host| host.kind == "local");
            (task.cwd.clone(), host_is_local)
        };
        if !host_is_local {
            return Err("Files can only be opened for a local task.".into());
        }
        open_task_relative_file(&cwd, href, base_path)
    }
}

fn open_task_relative_file(cwd: &str, href: &str, base_path: Option<&str>) -> Result<(), String> {
    let cwd = fs::canonicalize(cwd).map_err(|_| "Task folder is not available locally.")?;
    if !fs::metadata(&cwd)
        .map_err(|_| "Task folder is not available locally.")?
        .is_dir()
    {
        return Err("Task folder is not a directory.".into());
    }
    // Empty slice: any extension is fine here, since the OS — not Monitter —
    // decides how to open it.
    let path = resolve_local_link(&cwd, href, base_path, &[])?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|error| format!("Could not open file: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn fixture_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("monitter-open-file-{stamp}"));
        fs::create_dir_all(dir.join("docs")).unwrap();
        dir
    }
    #[test]
    fn rejects_paths_outside_the_task_folder() {
        let dir = fixture_dir();
        let outside = std::env::temp_dir().join(format!(
            "{}-outside.bin",
            dir.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        fs::write(&outside, "no").unwrap();
        assert!(open_task_relative_file(dir.to_str().unwrap(), outside.to_str().unwrap(), None).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_a_missing_file() {
        let dir = fixture_dir();
        assert!(open_task_relative_file(dir.to_str().unwrap(), "docs/missing.docx", None).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
