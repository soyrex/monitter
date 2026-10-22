//! Bounded, read-only native Markdown documents for the workspace dock.
use gpui_kit::{component::text::TextView, prelude::*, *};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_MARKDOWN_BYTES: u64 = 8 * 1024 * 1024;
const PANE_PREFIX: &str = "markdown:";

pub(crate) fn pane_id(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("Markdown path is not valid UTF-8.")?;
    let mut id = String::from(PANE_PREFIX);
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-') {
            id.push(byte as char);
        } else {
            id.push_str(&format!("%{byte:02X}"));
        }
    }
    if id.len() > 128 {
        return Err("Markdown file path is too long to retain in a workspace layout.".into());
    }
    Ok(id)
}

pub(crate) fn path_from_pane_id(id: &str) -> Option<PathBuf> {
    let encoded = id.strip_prefix(PANE_PREFIX)?;
    if encoded.is_empty() {
        return None;
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(decoded).ok()?))
}

fn read_markdown(path: &Path) -> Result<String, String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown"
            )
        })
    {
        return Err("Only Markdown files can be opened in this pane.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Cannot open Markdown file: {error}"))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("Cannot inspect Markdown file: {error}"))?;
    if !metadata.is_file() {
        return Err("Markdown path is not a regular file.".into());
    }
    if metadata.len() > MAX_MARKDOWN_BYTES {
        return Err("Markdown file exceeds the 8 MiB preview limit.".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(canonical)
        .map_err(|error| format!("Cannot read Markdown file: {error}"))?
        .take(MAX_MARKDOWN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read Markdown file: {error}"))?;
    if bytes.len() as u64 > MAX_MARKDOWN_BYTES {
        return Err("Markdown file exceeds the 8 MiB preview limit.".into());
    }
    String::from_utf8(bytes).map_err(|_| "Markdown file is not valid UTF-8.".into())
}

pub(crate) struct MarkdownPane {
    path: PathBuf,
    title: String,
    markdown: Option<Result<SharedString, String>>,
    load_generation: u64,
    _loading: Task<()>,
    focus: FocusHandle,
}

impl MarkdownPane {
    pub(crate) fn new(path: PathBuf, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Markdown")
            .to_owned();
        let read_path = path.clone();
        let generation = 1;
        let worker = cx
            .background_executor()
            .spawn(async move { read_markdown(&read_path) });
        let loading = cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.load_generation != generation {
                    return;
                }
                pane.markdown = Some(result.map(SharedString::from));
                cx.notify();
            });
        });
        Self {
            path,
            title,
            markdown: None,
            load_generation: generation,
            _loading: loading,
            focus: cx.focus_handle(),
        }
    }
    pub(crate) fn title(&self) -> &str {
        &self.title
    }
}

impl Render for MarkdownPane {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.markdown {
            None => div()
                .text_color(rgb(0x9BA7B5))
                .child("Loading Markdown…")
                .into_any_element(),
            Some(Ok(source)) => TextView::markdown(
                SharedString::from(format!("markdown:{}", self.path.display())),
                source.clone(),
            )
            .selectable(true)
            .scrollable(true)
            .w_full()
            .min_w_0()
            .max_w_full()
            .into_any_element(),
            Some(Err(error)) => div()
                .text_color(rgb(0xD97878))
                .child(error.clone())
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x343A43))
                    .text_size(px(12.))
                    .text_color(rgb(0x9BA7B5))
                    .child(self.title.clone()),
            )
            .child(div().flex_1().min_h_0().px_5().py_4().child(body))
    }
}

impl Focusable for MarkdownPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{pane_id, path_from_pane_id};
    use std::path::Path;
    #[test]
    fn pane_id_round_trips_spaces_and_unicode() {
        let path = Path::new("/Users/alex/reports/triage notes-日本.md");
        assert_eq!(
            path_from_pane_id(&pane_id(path).unwrap()).as_deref(),
            Some(path)
        );
    }
    #[test]
    fn pane_id_rejects_oversized_and_malformed_ids() {
        assert!(pane_id(Path::new(&format!("/{}", "a".repeat(130)))).is_err());
        assert!(path_from_pane_id("markdown:/bad%Q0.md").is_none());
    }
}
