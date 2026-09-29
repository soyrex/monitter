//! Native-only reader that turns a task-relative image link into an inline
//! data URL, mirroring `markdown::read_markdown_file` so images can be opened
//! as their own document tab the same way Markdown files are.
use crate::{local_file_link::resolve_local_link, Service};
use base64::Engine;
use serde::Serialize;
use std::{fs, path::Path};

const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "svg", "webp"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageDocument {
    pub path: String,
    pub title: String,
    pub mime_type: String,
    pub data_url: String,
}

impl Service {
    pub(crate) fn read_image_file(
        &self,
        task_id: &str,
        href: &str,
        base_path: Option<&str>,
    ) -> Result<ImageDocument, String> {
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
            return Err("Image links can only be opened for a local task.".into());
        }
        read_task_image(&cwd, href, base_path)
    }
}

fn read_task_image(cwd: &str, href: &str, base_path: Option<&str>) -> Result<ImageDocument, String> {
    let cwd = fs::canonicalize(cwd).map_err(|_| "Task folder is not available locally.")?;
    if !fs::metadata(&cwd)
        .map_err(|_| "Task folder is not available locally.")?
        .is_dir()
    {
        return Err("Task folder is not a directory.".into());
    }
    let path = resolve_local_link(&cwd, href, base_path, IMAGE_EXTENSIONS)?;
    let metadata = fs::metadata(&path).map_err(|_| "Image file is not available.")?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err("Image file is larger than 16 MiB.".into());
    }
    let bytes = fs::read(&path).map_err(|_| "Image file could not be read.")?;
    let mime_type = image_mime_type(&path).ok_or("Only PNG, JPG, GIF, SVG, and WebP images can be opened.")?;
    let data_url = format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    Ok(ImageDocument {
        title: image_title(&path),
        path: path.to_string_lossy().into_owned(),
        mime_type: mime_type.to_string(),
        data_url,
    })
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("svg") => Some("image/svg+xml"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn image_title(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Image")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
    fn fixture_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("monitter-image-{stamp}"));
        fs::create_dir_all(dir.join("assets")).unwrap();
        dir
    }
    #[test]
    fn reads_task_relative_image_and_encodes_data_url() {
        let dir = fixture_dir();
        let file = dir.join("assets/photo.png");
        fs::write(&file, [0x89, 0x50, 0x4e, 0x47]).unwrap();
        let result = read_task_image(dir.to_str().unwrap(), "assets/photo.png", None).unwrap();
        assert_eq!(result.title, "photo.png");
        assert_eq!(result.mime_type, "image/png");
        assert!(result.data_url.starts_with("data:image/png;base64,"));
        assert_eq!(
            PathBuf::from(result.path),
            fs::canonicalize(&file).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn recognizes_every_supported_extension() {
        let dir = fixture_dir();
        for (extension, mime) in [
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("gif", "image/gif"),
            ("svg", "image/svg+xml"),
            ("webp", "image/webp"),
        ] {
            let file = dir.join(format!("assets/pic.{extension}"));
            fs::write(&file, "x").unwrap();
            let result =
                read_task_image(dir.to_str().unwrap(), &format!("assets/pic.{extension}"), None)
                    .unwrap();
            assert_eq!(result.mime_type, mime);
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_unsupported_extension_and_outside_paths() {
        let dir = fixture_dir();
        fs::write(dir.join("notes.txt"), "no").unwrap();
        let outside = std::env::temp_dir().join(format!(
            "{}-outside.png",
            dir.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        fs::write(&outside, "no").unwrap();
        assert!(read_task_image(dir.to_str().unwrap(), "notes.txt", None).is_err());
        assert!(read_task_image(dir.to_str().unwrap(), outside.to_str().unwrap(), None).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
