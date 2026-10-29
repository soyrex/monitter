//! Bounded, read-only native Markdown documents for the workspace dock.
use gpui_kit::{component::{button::{Button, ButtonVariants as _}, text::TextView}, prelude::*, *};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_MARKDOWN_BYTES: u64 = 8 * 1024 * 1024;
const PANE_PREFIX: &str = "markdown:";

#[derive(Clone, PartialEq)]
pub(crate) struct Typography {
    requested_family: String,
    font_catalog_revision: u64,
    family: SharedString,
    size: f32,
    weight: f32,
    line_height: f32,
}
impl Global for Typography {}

pub(crate) fn sync_typography(settings: &serde_json::Value, cx: &mut App) {
    let mut typography = typography_from_settings(settings);
    typography.font_catalog_revision = monitter_gpui_desktop::fonts::catalog_revision(cx);
    if let Some(current) = cx.try_global::<Typography>() {
        typography.family = current.family.clone();
        if *current == typography { return; }
    }
    typography.family = monitter_gpui_desktop::fonts::resolve_family(
        &typography.requested_family, crate::design::FONT_SANS, cx);
    cx.set_global(typography);
}

fn typography_from_settings(settings: &serde_json::Value) -> Typography {
    let requested = settings["chatFont"].as_str().unwrap_or("");
    let family = if requested.is_empty() { crate::design::FONT_SANS } else { requested };
    let size = settings["chatFontSize"].as_f64()
        .unwrap_or(crate::design::CHAT_FONT_SIZE as f64).clamp(8., 32.) as f32;
    Typography {
        requested_family: requested.to_owned(),
        font_catalog_revision: 0,
        family: family.to_owned().into(),
        size,
        weight: settings["chatFontWeight"].as_f64().unwrap_or(400.).clamp(300., 700.) as f32,
        line_height: size * settings["chatLineHeight"].as_f64().unwrap_or(1.65).clamp(1., 2.5) as f32,
    }
}

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
        cx.observe_global::<Typography>(|_, cx| cx.notify()).detach();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let typography = cx.try_global::<Typography>().cloned().unwrap_or(Typography {
            requested_family: String::new(),
            font_catalog_revision: monitter_gpui_desktop::fonts::catalog_revision(cx),
            family: crate::design::FONT_SANS.into(),
            size: crate::design::CHAT_FONT_SIZE,
            weight: 400.,
            line_height: crate::design::CHAT_FONT_SIZE * 1.65,
        });
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
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x343A43))
                    .text_size(px(12.))
                    .text_color(rgb(0x9BA7B5))
                    .child(div().min_w_0().truncate().child(self.title.clone()))
                    .when(cfg!(target_os = "macos"), |header| {
                        header.child(
                            Button::new("reveal-markdown-in-finder")
                                .link()
                                .compact()
                                .text_size(px(12.))
                                .flex_shrink_0()
                                .label("Reveal in Finder")
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    cx.reveal_path(&pane.path);
                                })),
                        )
                    }),
            )
            .child(
                div().flex_1().min_h_0().min_w_0().flex().justify_center().px_5().py_4()
                    .child(div().w_full().min_w_0().h_full()
                        .max_w(crate::design::ui_scaled_px(crate::design::CHAT_MAX_WIDTH))
                        .font_family(typography.family)
                        .font_weight(FontWeight(typography.weight))
                        .text_size(crate::design::ui_scaled_px(typography.size))
                        .line_height(crate::design::ui_scaled_px(typography.line_height))
                        .child(body)),
            )
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
    fn markdown_pane_typography_matches_chat_preferences_and_limits() {
        let default = super::typography_from_settings(&serde_json::json!({}));
        assert_eq!(default.size, crate::design::CHAT_FONT_SIZE);
        assert_eq!(default.line_height, default.size * 1.65);
        let custom = super::typography_from_settings(&serde_json::json!({
            "chatFont": "IBM Plex Mono", "chatFontSize": 18,
            "chatFontWeight": 500, "chatLineHeight": 1.5
        }));
        assert_eq!(custom.family.as_ref(), "IBM Plex Mono");
        assert_eq!((custom.size, custom.weight, custom.line_height), (18., 500., 27.));
        let bounded = super::typography_from_settings(&serde_json::json!({
            "chatFontSize": 100, "chatFontWeight": 900, "chatLineHeight": 9
        }));
        assert_eq!((bounded.size, bounded.weight, bounded.line_height), (32., 700., 80.));
    }
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
