//! Resolve chat links against the task workspace before choosing a native pane.
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Browser(String),
    Markdown(PathBuf),
    Image(PathBuf),
    Document(PathBuf),
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
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods"
        | "ppt" | "pptx" | "odp" | "pages" | "numbers" | "key" => Target::Document(path),
        _ => Target::Editor(path),
    })
}

/// Presentation-only conversion; stored transcripts and literal code stay unchanged.
pub(crate) fn normalize_document_citations(source: &str) -> String {
    if !source.contains(":codex-file-citation{") { return source.to_owned(); }
    normalize_citations(source, false)
}

/// Keep streaming prose literal while making complete document citations usable.
pub(crate) fn normalize_streaming_document_citations(source: &str) -> String {
    normalize_citations(source, true)
}

fn push_citation_prose(output: &mut String, text: &str, streaming: bool) {
    for character in text.chars() {
        if streaming && (character == '\n' || character.is_ascii_punctuation()) {
            output.push('\\');
        }
        output.push(character);
    }
}

// Recognize fences inside Markdown quote/list containers as well as plain blocks.
fn citation_fence_line(line: &str) -> Option<&str> {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start_matches(' ');
        if rest.len() - trimmed.len() > 3 { return None; }
        rest = trimmed;
        if let Some(quoted) = rest.strip_prefix('>') {
            rest = quoted.strip_prefix(' ').unwrap_or(quoted);
            continue;
        }
        for bullet in ["- ", "* ", "+ "] {
            if let Some(listed) = rest.strip_prefix(bullet) {
                let content = listed.trim_start_matches(' ');
                return (listed.len() - content.len() <= 3 && !content.starts_with('\t')).then_some(content);
            }
        }
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && digits <= 9 {
            for delimiter in [". ", ") "] {
                if let Some(listed) = rest[digits..].strip_prefix(delimiter) {
                    let content = listed.trim_start_matches(' ');
                return (listed.len() - content.len() <= 3 && !content.starts_with('\t')).then_some(content);
                }
            }
        }
        return Some(rest);
    }
}

fn normalize_citations(source: &str, streaming: bool) -> String {
    let mut output = String::with_capacity(source.len());
    let mut fence: Option<(u8, usize)> = None;
    let mut inline_ticks = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let container_line = citation_fence_line(line);
        let fence_line = container_line.unwrap_or("");
        let marker = fence_line.as_bytes().first().copied().unwrap_or_default();
        let run = fence_line.bytes().take_while(|b| *b == marker).count();
        if let Some((kind, length)) = fence {
            push_citation_prose(&mut output, line, streaming);
            if marker == kind && run >= length
                && fence_line[run..].trim().is_empty() { fence = None; }
            continue;
        }
        if matches!(marker, b'`' | b'~') && run >= 3 {
            fence = Some((marker, run));
            inline_ticks = 0;
            push_citation_prose(&mut output, line, streaming);
            continue;
        }
        if indent >= 4 || container_line.is_none() || fence_line.starts_with('\t') {
            push_citation_prose(&mut output, line, streaming);
            continue;
        }
        let mut index = 0;
        while index < line.len() {
            let rest = &line[index..];
            if rest.starts_with('\\') {
                let length = rest.chars().take(2).map(char::len_utf8).sum::<usize>();
                push_citation_prose(&mut output, &rest[..length], streaming);
                index += length;
                continue;
            }
            if rest.starts_with('`') {
                let length = rest.bytes().take_while(|b| *b == b'`').count();
                if inline_ticks == 0 { inline_ticks = length; }
                else if inline_ticks == length { inline_ticks = 0; }
                push_citation_prose(&mut output, &rest[..length], streaming);
                index += length;
                continue;
            }
            if inline_ticks == 0 && rest.starts_with(":codex-file-citation{") {
                if let Some(end) = rest.bytes().take(8192).position(|byte| byte == b'}') {
                    if let Some(link) = document_citation_link(&rest[21..end]) {
                        output.push_str(&link);
                        index += end + 1;
                        continue;
                    }
                }
            }
            let character = rest.chars().next().unwrap();
            push_citation_prose(&mut output, &rest[..character.len_utf8()], streaming);
            index += character.len_utf8();
        }
    }
    output
}

fn document_citation_link(attributes: &str) -> Option<String> {
    let mut rest = attributes.trim();
    let mut path = None;
    let mut purpose = false;
    while !rest.is_empty() {
        let equal = rest.find('=')?;
        let key = rest[..equal].trim();
        let value = rest[equal + 1..].strip_prefix('"')?;
        let end = value.find('"')?;
        match key {
            "path" if path.is_none() => path = Some(&value[..end]),
            "purpose" if !purpose && !value[..end].is_empty() => purpose = true,
            _ => return None,
        }
        rest = &value[end + 1..];
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) { return None; }
        rest = rest.trim_start();
    }
    let path = path?;
    if path.is_empty() || path.ends_with('/') || path.chars().any(char::is_control) || path.contains('\\')
        || path.contains("://") || path.contains(':') { return None; }
    let filename = Path::new(path).file_name()?.to_str()?;
    if filename.is_empty() || matches!(filename, "." | "..") { return None; }
    let mut label = String::new();
    for character in filename.chars() {
        if character.is_ascii_punctuation() { label.push('\\'); }
        label.push(character);
    }
    // Escape all URL delimiters in filenames; the existing resolver decodes them.
    let relative = !path.starts_with('/') && !path.starts_with("~/");
    let path = if relative { format!("./{path}") } else { path.to_owned() };
    let target = path.bytes().map(|byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            (byte as char).to_string()
        } else { format!("%{byte:02X}") }
    }).collect::<String>();
    Some(format!("[{label}](<{target}>)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ux_sweep_documents_use_system_handler_while_source_stays_in_editor() {
        for extension in ["pdf", "PDF", "docx", "xlsx", "pptx", "odt", "rtf"] {
            let path = format!("/work/report one.{extension}");
            assert_eq!(resolve(&path, "/work", true).unwrap(), Target::Document(path.into()));
            assert!(resolve(&format!("report.{extension}"), "/work", false).is_err());
        }
        assert_eq!(resolve("src/app.rs", "/work", true).unwrap(), Target::Editor("/work/src/app.rs".into()));
    }
    #[test]
    fn ux_sweep_citations_use_private_filename_labels_and_safe_targets() {
        let source = r#"Open :codex-file-citation{path="/Users/private/My [report] #1.pdf" purpose="output"}."#;
        let rendered = normalize_document_citations(source);
        assert_eq!(rendered, r#"Open [My \[report\] \#1\.pdf](</Users/private/My%20%5Breport%5D%20%231.pdf>)."#);
        assert_eq!(source, r#"Open :codex-file-citation{path="/Users/private/My [report] #1.pdf" purpose="output"}."#);
        assert_eq!(resolve("/Users/private/My%20%5Breport%5D%20%231.pdf", "/work", true).unwrap(), Target::Document("/Users/private/My [report] #1.pdf".into()));
        assert!(resolve("/Users/private/report.pdf", "/work", false).is_err());
        assert_eq!(normalize_document_citations(r#":codex-file-citation{path="www.report.pdf"}"#), r#"[www\.report\.pdf](<./www.report.pdf>)"#);
        assert_eq!(resolve("./www.report.pdf", "/work", true).unwrap(), Target::Document("/work/www.report.pdf".into()));
    }

    #[test]
    fn ux_sweep_citations_preserve_code_and_malformed_references() {
        for source in [
            "`:codex-file-citation{path=\"/work/a.pdf\"}`",
            "``literal ` :codex-file-citation{path=\"/work/a.pdf\"}``",
            "```text\n:codex-file-citation{path=\"/work/a.pdf\"}\n```\n",
            "~~~\n:codex-file-citation{path=\"/work/a.pdf\"}\n~~~\n",
            "> ```\n> :codex-file-citation{path=\"/work/a.pdf\"}\n> ```\n",
            "- ```\n  :codex-file-citation{path=\"/work/a.pdf\"}\n  ```\n",
            "    :codex-file-citation{path=\"/work/a.pdf\"}\n",
            ">     :codex-file-citation{path=\"/work/a.pdf\"}\n",
            "-     :codex-file-citation{path=\"/work/a.pdf\"}\n",
            "1.     :codex-file-citation{path=\"/work/a.pdf\"}\n",
            "> -     :codex-file-citation{path=\"/work/a.pdf\"}\n",
            "> \t:codex-file-citation{path=\"/work/a.pdf\"}\n",
            ":codex-file-citation{path=\"/work/a.pdf\"",
            ":codex-file-citation{path=\"https://example.com/a.pdf\"}",
            ":codex-file-citation{path=\"/work/a.pdf\" unknown=\"x\"}",
            ":codex-file-citation{path=\"/work/a.pdf\"path=\"/work/b.pdf\"}",
            ":codex-file-citation{path=\"C:\\private\\a.pdf\"}",
        ] { assert_eq!(normalize_document_citations(source), source); }
    }

    #[test]
    fn ux_sweep_citations_work_in_partial_frames_without_changing_prose() {
        let source = "**Partial** :codex-file-citation{path=\"docs/guide one.md\"} then :codex-file-citation{path=\"";
        assert_eq!(normalize_document_citations(source), "**Partial** [guide one\\.md](<./docs/guide%20one.md>) then :codex-file-citation{path=\"");
        assert_eq!(normalize_streaming_document_citations(source), "\\*\\*Partial\\*\\* [guide one\\.md](<./docs/guide%20one.md>) then \\:codex\\-file\\-citation\\{path\\=\\\"");
        assert_eq!(normalize_document_citations(":codex-file-citation{path=\"a.md\"} :codex-file-citation{path=\"b.png\"}"), "[a\\.md](<./a.md>) [b\\.png](<./b.png>)");
    }

    #[test]
    fn ux_sweep_citations_keep_streaming_literals_and_incomplete_tokens_safe() {
        fn literal_stream(source: &str) -> String {
            let mut output = String::new();
            for character in source.chars() {
                if character == '\n' || character.is_ascii_punctuation() { output.push('\\'); }
                output.push(character);
            }
            output
        }
        for source in [
            "Ordinary **streaming** [text](https://example.com)\nNext line!",
            ":codex-file-citation{path=\"docs/unfinished",
            "`:codex-file-citation{path=\"docs/literal.md\"}`",
            "```\n:codex-file-citation{path=\"docs/literal.md\"}\n```",
        ] {
            assert_eq!(normalize_streaming_document_citations(source), literal_stream(source));
        }
        assert_eq!(normalize_streaming_document_citations("Ready :codex-file-citation{path=\"docs/result.md\"}…"), "Ready [result\\.md](<./docs/result.md>)…");
    }

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
