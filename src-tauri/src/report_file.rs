//! Native-only reader that turns a task-relative PDF or HTML link into an
//! inline data URL, mirroring `image_file::read_image_file` so reports can be
//! opened as their own document tab the same way images and Markdown are.
use crate::{local_file_link::resolve_local_link, Service};
use base64::Engine;
use serde::Serialize;
use std::{fs, path::Path};

const MAX_REPORT_BYTES: u64 = 24 * 1024 * 1024;
const REPORT_EXTENSIONS: &[&str] = &["pdf", "html", "htm"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportDocument {
    pub path: String,
    pub title: String,
    pub mime_type: String,
    pub data_url: String,
}

impl Service {
    pub(crate) fn read_report_file(
        &self,
        task_id: &str,
        href: &str,
        base_path: Option<&str>,
    ) -> Result<ReportDocument, String> {
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
            return Err("Report links can only be opened for a local task.".into());
        }
        read_task_report(&cwd, href, base_path)
    }
}

fn read_task_report(
    cwd: &str,
    href: &str,
    base_path: Option<&str>,
) -> Result<ReportDocument, String> {
    let cwd = fs::canonicalize(cwd).map_err(|_| "Task folder is not available locally.")?;
    if !fs::metadata(&cwd)
        .map_err(|_| "Task folder is not available locally.")?
        .is_dir()
    {
        return Err("Task folder is not a directory.".into());
    }
    let path = resolve_local_link(&cwd, href, base_path, REPORT_EXTENSIONS)?;
    let metadata = fs::metadata(&path).map_err(|_| "Report file is not available.")?;
    if metadata.len() > MAX_REPORT_BYTES {
        return Err("Report file is larger than 24 MiB.".into());
    }
    let bytes = fs::read(&path).map_err(|_| "Report file could not be read.")?;
    let mime_type =
        report_mime_type(&path).ok_or("Only PDF and HTML files can be opened here.")?;
    let data_url = format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    Ok(ReportDocument {
        title: report_title(&bytes, &path, mime_type),
        path: path.to_string_lossy().into_owned(),
        mime_type: mime_type.to_string(),
        data_url,
    })
}

fn report_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => Some("application/pdf"),
        Some("html") | Some("htm") => Some("text/html"),
        _ => None,
    }
}

/// HTML reports name themselves with `<title>`, the same way Markdown files
/// use a leading heading; PDFs fall straight back to the filename.
fn report_title(bytes: &[u8], path: &Path, mime_type: &str) -> String {
    if mime_type == "text/html" {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if let Some(title) = html_title(text) {
                return title;
            }
        }
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Report")
        .to_owned()
}

fn html_title(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let open_end = lower[open..].find('>')? + open + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let title = text[open_end..close].trim();
    (!title.is_empty()).then(|| title.to_owned())
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
        let dir = std::env::temp_dir().join(format!("monitter-report-{stamp}"));
        fs::create_dir_all(dir.join("reports")).unwrap();
        dir
    }
    #[test]
    fn reads_task_relative_pdf_and_encodes_data_url() {
        let dir = fixture_dir();
        let file = dir.join("reports/summary.pdf");
        fs::write(&file, b"%PDF-1.4").unwrap();
        let result = read_task_report(dir.to_str().unwrap(), "reports/summary.pdf", None).unwrap();
        assert_eq!(result.title, "summary.pdf");
        assert_eq!(result.mime_type, "application/pdf");
        assert!(result.data_url.starts_with("data:application/pdf;base64,"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn reads_html_and_uses_title_tag() {
        let dir = fixture_dir();
        let file = dir.join("reports/index.html");
        fs::write(&file, "<html><head><Title>  Q3 Report  </Title></head><body>hi</body></html>").unwrap();
        let result = read_task_report(dir.to_str().unwrap(), "reports/index.html", None).unwrap();
        assert_eq!(result.title, "Q3 Report");
        assert_eq!(result.mime_type, "text/html");
        assert!(result.data_url.starts_with("data:text/html;base64,"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn falls_back_to_filename_when_html_has_no_title() {
        let dir = fixture_dir();
        let file = dir.join("reports/untitled.htm");
        fs::write(&file, "<p>no title here</p>").unwrap();
        let result = read_task_report(dir.to_str().unwrap(), "reports/untitled.htm", None).unwrap();
        assert_eq!(result.title, "untitled.htm");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_unsupported_extension_and_outside_paths() {
        let dir = fixture_dir();
        fs::write(dir.join("notes.txt"), "no").unwrap();
        let outside = std::env::temp_dir().join(format!(
            "{}-outside.pdf",
            dir.file_name().and_then(|name| name.to_str()).unwrap()
        ));
        fs::write(&outside, "no").unwrap();
        assert!(read_task_report(dir.to_str().unwrap(), "notes.txt", None).is_err());
        assert!(read_task_report(dir.to_str().unwrap(), outside.to_str().unwrap(), None).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
