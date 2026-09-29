use crate::{local_file_link::resolve_local_link, Service};
use serde::Serialize;
use std::{fs, path::Path};

const MAX_MARKDOWN_BYTES: u64 = 2 * 1024 * 1024;

/// A deliberately small native-only document shape. Paths are canonical local
/// paths, so a renderer can use them as the validated base for a later link.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownDocument {
    pub path: String,
    pub title: String,
    pub content: String,
}

impl Service {
    pub(crate) fn read_markdown_file(
        &self,
        task_id: &str,
        href: &str,
        base_path: Option<&str>,
    ) -> Result<MarkdownDocument, String> {
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
            return Err("Markdown links can only be opened for a local task.".into());
        }
        read_task_markdown(&cwd, href, base_path)
    }
}

const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

fn read_task_markdown(
    cwd: &str,
    href: &str,
    base_path: Option<&str>,
) -> Result<MarkdownDocument, String> {
    let cwd = fs::canonicalize(cwd).map_err(|_| "Task folder is not available locally.")?;
    if !fs::metadata(&cwd)
        .map_err(|_| "Task folder is not available locally.")?
        .is_dir()
    {
        return Err("Task folder is not a directory.".into());
    }
    let path = resolve_local_link(&cwd, href, base_path, MARKDOWN_EXTENSIONS)?;
    let metadata = fs::metadata(&path).map_err(|_| "Markdown file is not available.")?;
    if metadata.len() > MAX_MARKDOWN_BYTES {
        return Err("Markdown file is larger than 2 MiB.".into());
    }
    let content =
        fs::read_to_string(&path).map_err(|_| "Markdown file must be valid UTF-8 text.")?;
    Ok(MarkdownDocument {
        title: markdown_title(&content, &path),
        path: path.to_string_lossy().into_owned(),
        content,
    })
}

fn markdown_title(content: &str, path: &Path) -> String {
    content
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| !stem.is_empty())
                .unwrap_or("Markdown document")
                .to_owned()
        })
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
        let dir = std::env::temp_dir().join(format!("monitter-markdown-{stamp}"));
        fs::create_dir_all(dir.join("docs")).unwrap();
        dir
    }
    #[test]
    fn reads_task_relative_markdown_and_uses_heading_as_title() {
        let dir = fixture_dir();
        let document = dir.join("docs/readme.md");
        fs::write(&document, "# Read me\n\nHello").unwrap();
        let result = read_task_markdown(dir.to_str().unwrap(), "docs/readme.md", None).unwrap();
        assert_eq!(result.title, "Read me");
        assert_eq!(result.content, "# Read me\n\nHello");
        assert_eq!(
            PathBuf::from(result.path),
            fs::canonicalize(&document).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn resolves_relative_links_from_a_validated_document_base() {
        let dir = fixture_dir();
        let index = dir.join("docs/index.md");
        let child = dir.join("docs/child.markdown");
        fs::write(&index, "# Index").unwrap();
        fs::write(&child, "Child").unwrap();
        let result = read_task_markdown(
            dir.to_str().unwrap(),
            "child.markdown#section",
            Some(index.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            PathBuf::from(result.path),
            fs::canonicalize(&child).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_remote_non_markdown_and_outside_paths() {
        let dir = fixture_dir();
        fs::write(dir.join("notes.txt"), "no").unwrap();
        let outside = std::env::temp_dir().join(format!(
            "{}-outside.md",
            dir.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        fs::write(&outside, "no").unwrap();
        assert!(read_task_markdown(
            dir.to_str().unwrap(),
            "https://example.test/readme.md",
            None
        )
        .is_err());
        assert!(read_task_markdown(dir.to_str().unwrap(), "notes.txt", None).is_err());
        assert!(
            read_task_markdown(dir.to_str().unwrap(), outside.to_str().unwrap(), None).is_err()
        );
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn accepts_file_url_inside_the_task_folder() {
        let dir = fixture_dir();
        let document = dir.join("docs/read me.md");
        fs::write(&document, "Hello").unwrap();
        let href = format!("file://{}#intro", document.display()).replace(' ', "%20");

        let result = read_task_markdown(dir.to_str().unwrap(), &href, None).unwrap();
        assert_eq!(
            PathBuf::from(result.path),
            fs::canonicalize(&document).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_invalid_file_url_encoding() {
        let dir = fixture_dir();
        assert!(read_task_markdown(dir.to_str().unwrap(), "file:///tmp/%zz.md", None).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
