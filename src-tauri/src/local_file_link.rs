//! Path resolution shared by every reader that turns a relative link inside a
//! task's own files (a Markdown href, an image src) into a validated,
//! canonical path. Centralizing this keeps the path-traversal and
//! percent-decoding rules identical across readers instead of drifting apart.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Resolve `href` (optionally relative to `base_path`, otherwise to `cwd`)
/// into a canonical path that is confirmed to stay inside `cwd` and exist as
/// a regular file. An empty `allowed_extensions` means any extension is
/// accepted (used by readers that hand a file to the OS rather than parsing
/// its contents, so there is no type-specific reason to restrict it); a
/// non-empty slice requires the path's extension (already lowercase) to be
/// one of them.
pub(crate) fn resolve_local_link(
    cwd: &Path,
    href: &str,
    base_path: Option<&str>,
    allowed_extensions: &[&str],
) -> Result<PathBuf, String> {
    let href = local_href_path(href)?;
    let base_dir = match base_path {
        Some(base_path) => canonical_existing_file(cwd, Path::new(base_path))?
            .parent()
            .map(Path::to_path_buf)
            .ok_or("Link base path has no parent directory.")?,
        None => cwd.to_path_buf(),
    };
    let candidate = if href.is_absolute() {
        href
    } else {
        base_dir.join(href)
    };
    let path = canonical_existing_file(cwd, &candidate)?;
    if allowed_extensions.is_empty() {
        return Ok(path);
    }
    let allowed = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed_extensions.contains(&extension.to_ascii_lowercase().as_str())
        });
    if !allowed {
        return Err(format!(
            "Only {} files can be opened here.",
            allowed_extensions.join(", ")
        ));
    }
    Ok(path)
}

pub(crate) fn local_href_path(href: &str) -> Result<PathBuf, String> {
    let href = href.trim();
    if href.is_empty() {
        return Err("Link is empty.".into());
    }
    // A fragment/query selects renderer state rather than a local filename.
    let href = href.split(['#', '?']).next().unwrap_or_default();
    if href.is_empty() {
        return Err("Link does not name a file.".into());
    }
    let lowered = href.to_ascii_lowercase();
    if let Some(path) = href.strip_prefix("file:///") {
        return Ok(PathBuf::from(format!("/{}", percent_decode(path)?)));
    }
    // Tauri's asset protocol (and its legacy `tauri://` name) maps a URL path
    // straight onto the filesystem root, the same way `file:///` does.
    for prefix in ["tauri://localhost/", "asset://localhost/"] {
        if lowered.starts_with(prefix) {
            return Ok(PathBuf::from(format!(
                "/{}",
                percent_decode(&href[prefix.len()..])?
            )));
        }
    }
    if href.starts_with("//")
        || lowered.starts_with("http:")
        || lowered.starts_with("https:")
        || lowered.starts_with("file:")
        || lowered.starts_with("mailto:")
        || lowered.contains("://")
    {
        return Err("Links must point to a local file.".into());
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
            return Err("File URL has invalid percent encoding.".into());
        }
        let high = hex_value(bytes[index + 1])?;
        let low = hex_value(bytes[index + 2])?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| "File URL must use UTF-8 path encoding.".into())
}

fn hex_value(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("File URL has invalid percent encoding.".into()),
    }
}

fn canonical_existing_file(cwd: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(candidate).map_err(|_| "File was not found.")?;
    if !path.starts_with(cwd) {
        return Err("File must be inside the task folder.".into());
    }
    let metadata = fs::metadata(&path).map_err(|_| "File was not found.")?;
    if !metadata.is_file() {
        return Err("Link must point to a regular file.".into());
    }
    Ok(path)
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
        let dir = std::env::temp_dir().join(format!("monitter-local-link-{stamp}"));
        fs::create_dir_all(dir.join("docs")).unwrap();
        dir
    }
    #[test]
    fn resolves_task_relative_link_with_allowed_extension() {
        let dir = fixture_dir();
        let file = dir.join("docs/photo.png");
        fs::write(&file, "x").unwrap();
        let resolved = resolve_local_link(&fs::canonicalize(&dir).unwrap(), "docs/photo.png", None, &["png"]).unwrap();
        assert_eq!(resolved, fs::canonicalize(&file).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn resolves_relative_links_from_a_validated_base() {
        let dir = fixture_dir();
        let index = dir.join("docs/index.md");
        let child = dir.join("docs/child.png");
        fs::write(&index, "index").unwrap();
        fs::write(&child, "child").unwrap();
        let cwd = fs::canonicalize(&dir).unwrap();
        let resolved = resolve_local_link(&cwd, "child.png#frag", Some(index.to_str().unwrap()), &["png"]).unwrap();
        assert_eq!(resolved, fs::canonicalize(&child).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_remote_disallowed_extension_and_outside_paths() {
        let dir = fixture_dir();
        fs::write(dir.join("notes.txt"), "no").unwrap();
        let outside = std::env::temp_dir().join(format!(
            "{}-outside.png",
            dir.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        fs::write(&outside, "no").unwrap();
        let cwd = fs::canonicalize(&dir).unwrap();
        assert!(resolve_local_link(&cwd, "https://example.test/photo.png", None, &["png"]).is_err());
        assert!(resolve_local_link(&cwd, "notes.txt", None, &["png"]).is_err());
        assert!(resolve_local_link(&cwd, outside.to_str().unwrap(), None, &["png"]).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_a_bare_absolute_path_inside_cwd() {
        let dir = fixture_dir();
        let file = dir.join("docs/photo.png");
        fs::write(&file, "x").unwrap();
        let cwd = fs::canonicalize(&dir).unwrap();
        let absolute = fs::canonicalize(&file).unwrap();
        let resolved = resolve_local_link(&cwd, absolute.to_str().unwrap(), None, &["png"]).unwrap();
        assert_eq!(resolved, absolute);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_tauri_and_asset_protocol_links_inside_cwd() {
        let dir = fixture_dir();
        let file = dir.join("docs/a b.png");
        fs::write(&file, "x").unwrap();
        let cwd = fs::canonicalize(&dir).unwrap();
        let absolute = fs::canonicalize(&file).unwrap();
        let encoded = absolute.display().to_string().replace(' ', "%20");
        for scheme in ["tauri://localhost", "asset://localhost"] {
            let href = format!("{scheme}{encoded}");
            let resolved = resolve_local_link(&cwd, &href, None, &["png"]).unwrap();
            assert_eq!(resolved, absolute);
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn accepts_file_url_inside_cwd() {
        let dir = fixture_dir();
        let file = dir.join("docs/a b.png");
        fs::write(&file, "x").unwrap();
        let cwd = fs::canonicalize(&dir).unwrap();
        let href = format!("file://{}", file.display()).replace(' ', "%20");
        let resolved = resolve_local_link(&cwd, &href, None, &["png"]).unwrap();
        assert_eq!(resolved, fs::canonicalize(&file).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_invalid_file_url_encoding() {
        let dir = fixture_dir();
        let cwd = fs::canonicalize(&dir).unwrap();
        assert!(resolve_local_link(&cwd, "file:///tmp/%zz.png", None, &["png"]).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
