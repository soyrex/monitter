//! Shared policy for native browser presentation targets.
use std::path::{Path, PathBuf};
use tauri::Url;
const MAX_URL: usize = 8192;
fn parsed(value: &str) -> Result<Url, String> {
    if value.is_empty() || value.len() > MAX_URL || value.chars().any(char::is_control) { return Err("Browser URL is empty, too long, or contains control characters.".into()); }
    Url::parse(value).map_err(|_| "Browser target must be an absolute HTML path or an HTTP(S) URL.".into())
}
fn html_path(path: &Path) -> bool { path.is_absolute() && path.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("html") || v.eq_ignore_ascii_case("htm")) }
fn file_path(url: &Url) -> Result<PathBuf, String> {
    if url.host_str().is_some() || !url.username().is_empty() || url.password().is_some() || url.port().is_some() || url.query().is_some() { return Err("Local HTML URLs cannot contain a host, credentials, port, or query.".into()); }
    let path = url.to_file_path().map_err(|_| "Invalid local HTML file URL.".to_string())?;
    if path.to_string_lossy().chars().any(char::is_control) || !html_path(&path) { return Err("Local browser targets must be absolute .html or .htm file paths without control characters.".into()); }
    Ok(path)
}
fn validate(url: &Url, allow_blank: bool) -> Result<(), String> { match url.scheme() {
    "about" if allow_blank && url.as_str() == "about:blank" => Ok(()),
    "http" | "https" if url.host_str().is_some() && url.username().is_empty() && url.password().is_none() => Ok(()),
    "file" => file_path(url).map(|_| ()),
    _ => Err("Browser target must be a local HTML file or credential-free HTTP(S) URL.".into()),
} }
pub fn validate_saved_url(value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) { return Err("Browser URL contains control characters.".into()); }
    validate(&parsed(value.trim())?, true)
}
pub fn normalize_target(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) { return Err("Browser target contains control characters.".into()); }
    let value = value.trim();
    let mut url = if value.starts_with('/') {
        if value.starts_with("//") { return Err("Browser target cannot use a remote file authority.".into()); }
        if value.len() > MAX_URL { return Err("Browser path is too long.".into()); }
        Url::from_file_path(value).map_err(|_| "Invalid absolute browser path.".to_string())?
    } else { parsed(value)? };
    validate(&url, false)?;
    if url.scheme() == "file" {
        let fragment = url.fragment().map(str::to_owned);
        let path = std::fs::canonicalize(file_path(&url)?).map_err(|_| "Local HTML browser target does not exist or cannot be resolved.".to_string())?;
        if !html_path(&path) || !path.is_file() { return Err("Local browser target must resolve to an existing regular HTML file.".into()); }
        url = Url::from_file_path(path).map_err(|_| "Cannot encode local HTML browser target.".to_string())?;
        url.set_fragment(fragment.as_deref());
    }
    if url.as_str().len() > MAX_URL { return Err("Canonical browser URL is too long.".into()); }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_html_canonicalizes_spaces_and_preserves_fragment() {
        let dir = std::env::temp_dir().join(format!("monitter-html-{}", crate::model::id()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("report space.html");
        std::fs::write(&file, "<h1>Report</h1>").unwrap();
        let target = normalize_target(file.to_str().unwrap()).unwrap();
        assert!(target.contains("report%20space.html"));
        assert_eq!(normalize_target(&format!("{target}#result")).unwrap(), format!("{target}#result"));
        let disguised = dir.join("directory.html"); std::fs::create_dir(&disguised).unwrap();
        assert!(normalize_target(disguised.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
        assert!(validate_saved_url(&target).is_ok());
        assert!(normalize_target(&target).is_err());
    }
    #[test]
    fn rejects_privileged_or_remote_file_targets() {
        for target in ["file://server/tmp/report.html", "file:///etc/passwd", "file:///tmp/report.html?q=1", "file:///tmp/report%0A.html", "javascript:alert(1)", "data:text/html,hi", "https://user:pass@example.com", "//server/report.html", "relative.html", "https://example.com/\n"] {
            assert!(normalize_target(target).is_err(), "{target}");
        }
        assert_eq!(normalize_target("https://example.com/report").unwrap(), "https://example.com/report");
        assert!(validate_saved_url("about:blank").is_ok());
    }
}
