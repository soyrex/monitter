use crate::Service;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    let href = local_href_path(href)?;
    let base_dir = match base_path {
        Some(base_path) => canonical_markdown_path(&cwd, Path::new(base_path))?
            .parent()
            .map(Path::to_path_buf)
            .ok_or("Markdown base path has no parent directory.")?,
        None => cwd.clone(),
    };
    let candidate = if href.is_absolute() {
        href
    } else {
        base_dir.join(href)
    };
    let path = canonical_markdown_path(&cwd, &candidate)?;
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

fn local_href_path(href: &str) -> Result<PathBuf, String> {
    let href = href.trim();
    if href.is_empty() {
        return Err("Markdown link is empty.".into());
    }
    // A fragment/query selects renderer state rather than a local filename.
    let href = href.split(['#', '?']).next().unwrap_or_default();
    if href.is_empty() {
        return Err("Markdown link does not name a file.".into());
    }
    let lowered = href.to_ascii_lowercase();
    if let Some(path) = href.strip_prefix("file:///") {
        return Ok(PathBuf::from(format!("/{}", percent_decode(path)?)));
    }
    if href.starts_with("//")
        || lowered.starts_with("http:")
        || lowered.starts_with("https:")
        || lowered.starts_with("file:")
        || lowered.starts_with("mailto:")
        || lowered.contains("://")
    {
        return Err("Markdown links must point to a local file.".into());
    }
    Ok(PathBuf::from(href))
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("Markdown file URL has invalid percent encoding.".into());
        }
        let high = hex_value(bytes[index + 1])?;
        let low = hex_value(bytes[index + 2])?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| "Markdown file URL must use UTF-8 path encoding.".into())
}

fn hex_value(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("Markdown file URL has invalid percent encoding.".into()),
    }
}

fn canonical_markdown_path(cwd: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(candidate).map_err(|_| "Markdown file was not found.")?;
    if !path.starts_with(cwd) {
        return Err("Markdown file must be inside the task folder.".into());
    }
    let metadata = fs::metadata(&path).map_err(|_| "Markdown file was not found.")?;
    if !metadata.is_file() {
        return Err("Markdown link must point to a regular file.".into());
    }
    let allowed = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        });
    if !allowed {
        return Err("Only .md and .markdown files can be opened.".into());
    }
    Ok(path)
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
    use std::time::{SystemTime, UNIX_EPOCH};
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
