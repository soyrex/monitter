//! Resolve chat links against the task workspace before choosing a native pane.
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Browser(String),
    Markdown(PathBuf),
    Image(PathBuf),
    Editor(PathBuf),
    External(String),
}

pub(crate) fn resolve(link: &str, cwd: &str, local: bool) -> Result<Target, String> {
    let link = link.trim();
    let parsed = url::Url::parse(link).ok();
    let source_reference = parsed.as_ref().is_some_and(|url| !matches!(url.scheme(),
        "http" | "https" | "file" | "mailto" | "tel" | "sms"))
        && !link.contains("://") && link.rsplit_once(':')
        .is_some_and(|(_, line)| !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()));
    if let Some(url) = parsed && !source_reference {
        match url.scheme() {
            "http" | "https" => return Ok(Target::Browser(url.into())),
            "file" => {
                if !local { return Err(remote_file_error()); }
                return classify(url.to_file_path().map_err(|_| "Invalid local file link.")?);
            }
            _ => return Ok(Target::External(link.to_owned())),
        }
    }
    if link.starts_with("www.") {
        return Ok(Target::Browser(format!("https://{link}")));
    }
    if !local { return Err(remote_file_error()); }
    // URL joining decodes escaped spaces and removes query/fragment syntax.
    // Strip source references such as app.rs:12:4 before resolving the path.
    let path = link.split(['?', '#']).next().unwrap_or(link);
    let mut path = path;
    for _ in 0..2 {
        if let Some((base, line)) = path.rsplit_once(':') {
            if !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) { path = base; }
        }
    }
    if path.is_empty() { return Err("This link has no file or website target.".into()); }
    let base = if Path::new(path).is_absolute() { Path::new("/") } else { Path::new(cwd) };
    let base = url::Url::from_directory_path(base)
        .map_err(|_| "The chat has no absolute workspace directory for this file link.")?;
    let path = base.join(path).map_err(|e| format!("Invalid file link: {e}"))?
        .to_file_path().map_err(|_| "Invalid local file link.")?;
    classify(path)
}

fn remote_file_error() -> String {
    "This file link belongs to a remote task. Ask the agent for an HTTP(S) preview URL.".into()
}

fn classify(path: PathBuf) -> Result<Target, String> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Ok(match extension.as_str() {
        "md" | "markdown" | "mdown" => Target::Markdown(path),
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico"
        | "pnm" | "ppm" | "pgm" | "pbm" | "pam" => Target::Image(path),
        "html" | "htm" => Target::Browser(url::Url::from_file_path(path)
            .map_err(|_| "Invalid HTML file path.")?.into()),
        _ => Target::Editor(path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_links_open_previews_instead_of_the_editor() {
        for extension in ["png", "PNG", "jpg", "JPEG", "gif", "webp", "bmp", "tif", "tiff", "ico", "pnm"] {
            let path = format!("/work/image one.{extension}");
            assert_eq!(resolve(&format!("image%20one.{extension}"), "/work", true).unwrap(), Target::Image(path.into()));
        }
        assert_eq!(resolve("file:///work/photo%20one.jpg", "/other", true).unwrap(), Target::Image("/work/photo one.jpg".into()));
        assert_eq!(resolve("https://example.com/photo.png", "/work", false).unwrap(), Target::Browser("https://example.com/photo.png".into()));
        assert!(resolve("photo.png", "/work", false).is_err());
        assert!(resolve("file:///work/photo.jpg", "/work", false).is_err());
    }
    #[test]
    fn chat_links_route_web_documents_and_source_files() {
        assert_eq!(resolve("https://example.com/a?q=1#part", "/work", false).unwrap(), Target::Browser("https://example.com/a?q=1#part".into()));
        assert_eq!(resolve("docs/guide%20one.MD#intro", "/work", true).unwrap(), Target::Markdown("/work/docs/guide one.MD".into()));
        assert_eq!(resolve("/work/src/app.rs:12:4", "/elsewhere", true).unwrap(), Target::Editor("/work/src/app.rs".into()));
        assert_eq!(resolve("app.rs:12", "/work", true).unwrap(), Target::Editor("/work/app.rs".into()));
        assert_eq!(resolve("tel:1234", "/work", true).unwrap(), Target::External("tel:1234".into()));
        assert_eq!(resolve("mailto:person@example.com", "/work", false).unwrap(), Target::External("mailto:person@example.com".into()));
        assert_eq!(resolve("file:///work/notes.txt#L7", "/elsewhere", true).unwrap(), Target::Editor("/work/notes.txt".into()));
        assert_eq!(resolve("/work/report.html", "/work", true).unwrap(), Target::Browser("file:///work/report.html".into()));
        assert!(resolve("docs/guide.md", "/work", false).is_err());
        assert!(resolve("file:///work/notes.txt", "/work", false).is_err());
    }
}
