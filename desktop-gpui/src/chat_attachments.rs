//! Retained native chat attachment drafts. File access follows explicit picker
//! selection through the owner service; removal drops references, never files.
use crate::design::ui_scaled_px as px;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::{
    DesktopService, NativeAttachmentDraft, NativeAttachmentRef, ReadAttachmentFile,
};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
#[path = "attachment_preview.rs"]
mod attachment_preview;

/// Transcript renderers can use this same on-demand registered-ID reader.
pub(super) fn open_image(
    service: DesktopService,
    attachment_id: String,
    cx: &mut App,
) -> Result<(), String> {
    attachment_preview::open(service, attachment_id, cx)
}
pub(super) fn open_image_modal(
    service: DesktopService,
    attachment_id: String,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    attachment_preview::open_modal(service, attachment_id, window, cx)
}
pub(super) fn image_thumbnail(
    service: DesktopService,
    attachment_id: &str,
) -> Result<(Arc<Image>, u32, u32), String> {
    attachment_preview::thumbnail(service, attachment_id)
}
const MAX_FILE: usize = 20 * 1024 * 1024;
const MAX_REPLY_TEXT: usize = 8 * 1024 * 1024;
const MAX_PREVIEW: usize = 256 * 1024;
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AttachmentScope {
    pub host_id: String,
    pub cwd: String,
}
pub(super) struct AttachmentDraft {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: usize,
    preview: Option<Arc<Image>>,
}
fn field(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn inline_preview(mime: &str, encoded: &str) -> Option<(String, Arc<Image>)> {
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/webp" => image::ImageFormat::WebP,
        "image/gif" => image::ImageFormat::Gif,
        _ => return None,
    };
    if encoded.len() > ((MAX_FILE + 2) / 3) * 4 + 4 {
        return None;
    }
    let bytes = STANDARD.decode(encoded).ok()?;
    let dimensions = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
        .into_dimensions()
        .ok()?;
    if dimensions.0 > 8192
        || dimensions.1 > 8192
        || u64::from(dimensions.0) * u64::from(dimensions.1) > 16 * 1024 * 1024
    {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let thumbnail = reader.decode().ok()?.thumbnail(dimensions.0.min(128), dimensions.1.min(96));
    let mut output = std::io::Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut output, image::ImageFormat::Png)
        .ok()?;
    let bytes = output.into_inner();
    let url = format!("data:image/png;base64,{}", STANDARD.encode(&bytes));
    if url.len() > MAX_PREVIEW {
        return None;
    }
    Some((url, Arc::new(Image::from_bytes(ImageFormat::Png, bytes))))
}
fn clipboard_image_file(image: &Image) -> Result<ReadAttachmentFile, String> {
    if image.bytes().len() > MAX_FILE {
        return Err("Attachments are limited to 20 MiB.".into());
    }
    let format = match image.format() {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Webp => image::ImageFormat::WebP,
        ImageFormat::Gif => image::ImageFormat::Gif,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Bmp => image::ImageFormat::Bmp,
        ImageFormat::Ico => image::ImageFormat::Ico,
        ImageFormat::Pnm => image::ImageFormat::Pnm,
        ImageFormat::Svg => {
            return Err("Paste a raster image, or attach the SVG using the file picker.".into());
        }
    };
    let dimensions = image::ImageReader::with_format(std::io::Cursor::new(image.bytes()), format)
        .into_dimensions()
        .map_err(|error| format!("Could not read pasted image: {error}"))?;
    if dimensions.0 > 8192
        || dimensions.1 > 8192
        || u64::from(dimensions.0) * u64::from(dimensions.1) > 16 * 1024 * 1024
    {
        return Err("Pasted images are limited to 8192 pixels per side and 16 megapixels.".into());
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(image.bytes()), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("Could not decode pasted image: {error}"))?;
    let mut output = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not encode pasted image: {error}"))?;
    let bytes = output.into_inner();
    if bytes.len() > MAX_FILE {
        return Err("Attachments are limited to 20 MiB.".into());
    }
    Ok(ReadAttachmentFile {
        filename: "Pasted image.png".into(),
        mime_type: "image/png".into(),
        data_base64: STANDARD.encode(bytes),
    })
}
fn stored_draft(value: Value, preview: Option<Arc<Image>>) -> Result<AttachmentDraft, String> {
    let id = field(&value, "id");
    let name = field(&value, "name");
    let size = value["size"]
        .as_u64()
        .ok_or("Attachment response has no valid size.")?;
    if id.is_empty() || name.is_empty() || size > MAX_FILE as u64 {
        return Err("Attachment response is invalid.".into());
    }
    Ok(AttachmentDraft {
        id,
        name,
        mime_type: field(&value, "mimeType"),
        size: size as usize,
        preview,
    })
}
fn message_text_filename(author: &str, text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return Err("This message has no visible text to attach.".into());
    }
    if bytes.len() > MAX_REPLY_TEXT {
        return Err("Message attachments are limited to 8 MiB of text.".into());
    }
    let safe_author = author
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '_' | '-')
        })
        .take(48)
        .collect::<String>()
        .trim()
        .to_owned();
    let safe_author = if safe_author.is_empty() {
        "message"
    } else {
        &safe_author
    };
    Ok(format!("Reply to {safe_author}.md"))
}
fn message_text_attachment(filename: String, text: &str) -> ReadAttachmentFile {
    ReadAttachmentFile {
        filename,
        mime_type: "text/markdown".into(),
        data_base64: STANDARD.encode(text.as_bytes()),
    }
}
fn persistence_scope_matches(
    current_task: &str,
    saved_task: &str,
    current_scope: &AttachmentScope,
    saved_scope: &NativeAttachmentDraft,
) -> bool {
    !current_task.is_empty()
        && current_task == saved_task
        && !current_scope.host_id.is_empty()
        && !current_scope.cwd.is_empty()
        && current_scope.host_id == saved_scope.host_id
        && current_scope.cwd == saved_scope.cwd
}
#[derive(Clone)]
pub(super) enum AttachmentEvent {
    Changed,
}
pub(super) struct ChatAttachments {
    service: DesktopService,
    task_id: String,
    scope: AttachmentScope,
    drafts: Vec<AttachmentDraft>,
    generation: u64,
    pending: bool,
    touched: bool,
    palette: Option<crate::design::Palette>,
    interface_font_ratio: f32,
    error: Option<String>,
    notice: Option<String>,
}
impl EventEmitter<AttachmentEvent> for ChatAttachments {}
impl ChatAttachments {
    pub fn new(service: DesktopService, task_id: String, scope: AttachmentScope) -> Self {
        Self {
            service,
            task_id,
            scope,
            drafts: vec![],
            generation: 0,
            pending: false,
            touched: false,
            palette: None,
            interface_font_ratio: 1.,
            error: None,
            notice: None,
        }
    }
    pub fn ids(&self) -> Vec<String> {
        self.drafts.iter().map(|draft| draft.id.clone()).collect()
    }
    /// Safe preference projection: registered IDs and display metadata only.
    /// The task UUID is the containing NativePreferenceData map key.
    pub(super) fn persistence_snapshot(&self) -> NativeAttachmentDraft {
        NativeAttachmentDraft {
            host_id: self.scope.host_id.clone(),
            cwd: self.scope.cwd.clone(),
            attachments: self
                .drafts
                .iter()
                .map(|draft| NativeAttachmentRef {
                    id: draft.id.clone(),
                    name: draft.name.clone(),
                    mime_type: draft.mime_type.clone(),
                    size: draft.size as u64,
                })
                .collect(),
        }
    }
    /// Restore only into an untouched, exact task/host/folder scope. This is
    /// deliberately silent so preference restoration cannot enqueue itself.
    pub(super) fn restore_persistence_snapshot(
        &mut self,
        current_task: &str,
        saved_task: &str,
        saved: &NativeAttachmentDraft,
        cx: &mut Context<Self>,
    ) -> bool {
        if !persistence_scope_matches(current_task, saved_task, &self.scope, saved)
            || self.pending
            || self.touched
            || !self.drafts.is_empty()
        {
            return false;
        }
        let mut seen = HashSet::new();
        let restored = saved
            .attachments
            .iter()
            .map(|attachment| {
                let valid_name = !attachment.name.is_empty()
                    && attachment.name.len() <= 120
                    && attachment.name != "."
                    && attachment.name != ".."
                    && attachment
                        .name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '));
                let size = usize::try_from(attachment.size).ok()?;
                if attachment.id.is_empty()
                    || !seen.insert(attachment.id.as_str())
                    || !valid_name
                    || attachment.mime_type.len() > 256
                    || attachment.mime_type.chars().any(char::is_control)
                    || size > MAX_FILE
                {
                    return None;
                }
                Some(AttachmentDraft {
                    id: attachment.id.clone(),
                    name: attachment.name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    size,
                    preview: None,
                })
            })
            .collect::<Option<Vec<_>>>();
        let Some(restored) = restored else {
            return false;
        };
        self.drafts = restored;
        self.restore_thumbnails(cx);
        cx.notify();
        true
    }
    fn restore_thumbnails(&self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .drafts
            .iter()
            .filter(|draft| {
                draft.preview.is_none()
                    && matches!(
                        draft.mime_type.as_str(),
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                    )
            })
            .map(|draft| draft.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        let service = self.service.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            ids.into_iter()
                .filter_map(|id| {
                    image_thumbnail(service.clone(), &id)
                        .ok()
                        .map(|(image, _, _)| (id, image))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |view, cx| {
            let thumbnails = task.await;
            let _ = view.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                for (id, image) in thumbnails {
                    if let Some(draft) = view.drafts.iter_mut().find(|draft| draft.id == id) {
                        draft.preview = Some(image);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn touched(&self) -> bool {
        self.touched
    }
    pub fn set_appearance(
        &mut self,
        palette: crate::design::Palette,
        interface_font_ratio: f32,
        cx: &mut Context<Self>,
    ) {
        let ratio = if interface_font_ratio.is_finite() {
            interface_font_ratio.clamp(8. / 14., 32. / 14.)
        } else {
            1.
        };
        if self.palette != Some(palette) || self.interface_font_ratio != ratio {
            self.palette = Some(palette);
            self.interface_font_ratio = ratio;
            cx.notify();
        }
    }
    fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        let previous_len = self.drafts.len();
        self.drafts.retain(|draft| draft.id != id);
        if self.drafts.len() != previous_len {
            self.touched = true;
            self.changed(cx);
        }
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn set_scope(&mut self, scope: AttachmentScope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.generation = self.generation.wrapping_add(1);
            self.drafts.clear();
            self.touched = true;
            self.pending = false;
            self.error = None;
            self.notice =
                Some("Working folder changed. Attach files again for this destination.".into());
            self.changed(cx);
        }
    }
    pub fn acknowledge(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let ids = ids.iter().collect::<HashSet<_>>();
        let previous_len = self.drafts.len();
        self.drafts.retain(|draft| !ids.contains(&draft.id));
        if self.drafts.len() != previous_len {
            self.touched = true;
            self.changed(cx);
        }
    }
    /// Register visible transcript text as a task-scoped Markdown attachment.
    /// The existing owner-only store path checks the captured host and folder;
    /// no temporary file or new backend command is needed.
    pub fn attach_message_text(&mut self, author: &str, text: &str, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        if self.scope.host_id.is_empty() || self.scope.cwd.is_empty() || self.task_id.is_empty() {
            self.error = Some("This chat has no available attachment destination.".into());
            self.changed(cx);
            return;
        }
        let filename = match message_text_filename(author, text) {
            Ok(filename) => filename,
            Err(error) => {
                self.error = Some(error);
                self.changed(cx);
                return;
            }
        };
        self.pending = true;
        self.error = None;
        self.notice = None;
        let generation = self.generation;
        let task_id = self.task_id.clone();
        let scope = self.scope.clone();
        let service = self.service.clone();
        let store_task_id = task_id.clone();
        let store_scope = scope.clone();
        let text = text.to_owned();
        let (sender, receiver) = futures::channel::oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("monitter-message-reply-attachment".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let file = message_text_attachment(filename, &text);
                let stored = service
                    .store_selected_attachment(
                        &store_task_id,
                        &store_scope.host_id,
                        &store_scope.cwd,
                        file,
                        None,
                    )
                    .and_then(|stored| {
                        serde_json::to_value(stored).map_err(|error| error.to_string())
                    })
                    .and_then(|stored| stored_draft(stored, None));
                let _ = sender.send(stored);
            });
        if let Err(error) = worker {
            self.pending = false;
            self.error = Some(format!(
                "Could not start the message attachment worker: {error}"
            ));
            self.changed(cx);
            return;
        }
        cx.spawn(async move |view, cx| {
            let result = receiver.await.unwrap_or_else(|_| {
                Err("The message attachment worker stopped before saving the attachment.".into())
            });
            let _ = view.update(cx, |view, cx| {
                if view.generation != generation || view.task_id != task_id || view.scope != scope {
                    return;
                }
                view.pending = false;
                match result {
                    Ok(draft) => {
                        view.drafts.push(draft);
                        view.touched = true;
                    }
                    Err(error) => view.error = Some(error),
                }
                view.changed(cx);
            });
        })
        .detach();
        self.changed(cx);
    }
    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(AttachmentEvent::Changed);
        cx.notify();
    }
    /// Consume native image paste payloads; ordinary text keeps the input's
    /// default paste behavior. Encoding and registration run away from the UI.
    pub(super) fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        if self.pending {
            self.error = Some(
                "Wait for the current attachment to finish before pasting another image.".into(),
            );
            self.changed(cx);
            return true;
        }
        if !self.can_choose() || self.task_id.is_empty() {
            self.error = Some("This chat has no available attachment destination.".into());
            self.changed(cx);
            return true;
        }
        self.pending = true;
        self.error = None;
        self.notice = None;
        let generation = self.generation;
        let task_id = self.task_id.clone();
        let scope = self.scope.clone();
        let service = self.service.clone();
        let worker_task_id = task_id.clone();
        let worker_scope = scope.clone();
        let task = cx.background_executor().spawn(async move {
            let file = clipboard_image_file(&image)?;
            let preview = inline_preview(&file.mime_type, &file.data_base64);
            let stored = service.store_selected_attachment(
                &worker_task_id,
                &worker_scope.host_id,
                &worker_scope.cwd,
                file,
                preview.as_ref().map(|(url, _)| url.clone()),
            )?;
            stored_draft(
                serde_json::to_value(stored).map_err(|error| error.to_string())?,
                preview.map(|(_, image)| image),
            )
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                if view.generation != generation || view.task_id != task_id || view.scope != scope {
                    return;
                }
                view.pending = false;
                match result {
                    Ok(draft) => {
                        view.drafts.push(draft);
                        view.touched = true;
                    }
                    Err(error) => view.error = Some(error),
                }
                view.changed(cx);
            });
        })
        .detach();
        self.changed(cx);
        true
    }
    pub(super) fn choose(&mut self, cx: &mut Context<Self>) {
        if self.pending || self.scope.host_id.is_empty() || self.scope.cwd.is_empty() {
            return;
        }
        self.pending = true;
        self.error = None;
        self.notice = None;
        let generation = self.generation;
        let task_id = self.task_id.clone();
        let scope = self.scope.clone();
        let service = self.service.clone();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files to this chat (up to 20 MiB each)".into()),
        });
        cx.spawn(async move |view, cx| {
            let paths = match prompt.await {
                Ok(Ok(paths)) => paths,
                Ok(Err(error)) => {
                    let _ = view.update(cx, |view, cx| {
                        if view.generation == generation {
                            view.pending = false;
                            view.error = Some(error.to_string());
                            view.changed(cx);
                        }
                    });
                    return;
                }
                Err(error) => {
                    let _ = view.update(cx, |view, cx| {
                        if view.generation == generation {
                            view.pending = false;
                            view.error = Some(error.to_string());
                            view.changed(cx);
                        }
                    });
                    return;
                }
            };
            for path in paths.unwrap_or_default() {
                let still_current = view
                    .read_with(cx, |view, _| view.generation == generation)
                    .unwrap_or(false);
                if !still_current {
                    break;
                }
                let Some(path) = path.to_str().map(str::to_owned) else {
                    let _ = view.update(cx, |view, cx| {
                        if view.generation == generation {
                            view.error = Some("Selected filename is not valid Unicode.".into());
                            view.changed(cx);
                        }
                    });
                    break;
                };
                let service = service.clone();
                let task_id = task_id.clone();
                let scope = scope.clone();
                let task = cx.background_executor().spawn(async move {
                    // Read only the path explicitly selected in the native picker.
                    let file = service.read_attachment_file(&path)?;
                    let encoded = file.data_base64.as_str();
                    if encoded.len() > ((MAX_FILE + 2) / 3) * 4 + 4 {
                        return Err("Attachments are limited to 20 MiB.".into());
                    }
                    let preview = inline_preview(&file.mime_type, encoded);
                    let preview_url = preview.as_ref().map(|(url, _)| url.clone());
                    let stored = service.store_selected_attachment(
                        &task_id,
                        &scope.host_id,
                        &scope.cwd,
                        file,
                        preview_url,
                    )?;
                    let stored = serde_json::to_value(stored).map_err(|error| error.to_string())?;
                    stored_draft(stored, preview.map(|(_, image)| image))
                });
                let result = task.await;
                let failed = result.is_err();
                let _ = view.update(cx, |view, cx| {
                    if view.generation == generation {
                        match result {
                            Ok(draft) => {
                                view.drafts.push(draft);
                                view.touched = true;
                            }
                            Err(error) => view.error = Some(error),
                        }
                        view.changed(cx);
                    }
                });
                if failed {
                    break;
                }
            }
            let _ = view.update(cx, |view, cx| {
                if view.generation == generation {
                    view.pending = false;
                    view.changed(cx);
                }
            });
        })
        .detach();
        self.changed(cx);
    }
    pub(super) fn can_choose(&self) -> bool {
        !self.pending && !self.scope.host_id.is_empty() && !self.scope.cwd.is_empty()
    }
}
impl Render for ChatAttachments {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| crate::design::palette(&Value::Null, window.appearance()));
        let font_ratio = self.interface_font_ratio;
        let mut panel = div().flex().flex_col().gap(px(8.));
        let mut strip = div().flex().flex_wrap().gap(px(8.)).min_w_0().my(px(8.));
        for draft in &self.drafts {
            let id = draft.id.clone();
            let mut chip = div()
                .p(px(6.))
                .border_1()
                .border_color(rgb(palette.line))
                .rounded(px(8.))
                .bg(crate::design::surface_color(palette.panel))
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w_0()
                .max_w_full();
            if let Some(image) = &draft.preview {
                chip = chip.child(
                    Button::new(format!("thumbnail-attachment-{id}"))
                        .ghost()
                        .p_0()
                        .h_auto()
                        .accessibility_label(format!("View full-size {}", draft.name))
                        .child(
                            img(image.clone())
                                .max_w(px(96.))
                                .max_h(px(72.))
                                .rounded_md(),
                        )
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |view, _, window, cx| {
                                if let Err(error) =
                                    open_image_modal(view.service.clone(), id.clone(), window, cx)
                                {
                                    view.error = Some(error);
                                    view.changed(cx);
                                }
                            }
                        })),
                );
            }
            if draft.preview.is_none() {
                chip = chip.child(
                    div()
                        .w(px(34.))
                        .h(px(42.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .bg(crate::design::surface_color(palette.soft))
                        .child(
                            Icon::new(IconName::FileText)
                                .with_size(px(23.))
                                .text_color(rgb(palette.muted)),
                        ),
                );
            }
            if matches!(
                draft.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp" | "image/gif"
            ) {
                let preview_id = id.clone();
                chip = chip.child(
                    Button::new(SharedString::from(format!("preview-attachment-{id}")))
                        .compact()
                        .icon(
                            Icon::new(IconName::Eye)
                                .with_size(px(14.))
                                .text_color(rgb(palette.muted)),
                        )
                        .accessibility_label(format!("Preview attachment {}", draft.name))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Err(error) = open_image_modal(
                                view.service.clone(),
                                preview_id.clone(),
                                window,
                                cx,
                            ) {
                                view.error = Some(error);
                                view.changed(cx);
                            }
                        })),
                );
            }
            chip = chip
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .min_w_0()
                        .max_w(px(180.))
                        .child(
                            div()
                                .text_size(px(11. * font_ratio))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.ink))
                                .truncate()
                                .child(draft.name.clone()),
                        )
                        .child(
                            div()
                                .font_family(crate::design::FONT_MONO)
                                .text_size(px(9. * font_ratio))
                                .text_color(rgb(palette.muted))
                                .child(if draft.size >= 1024 * 1024 {
                                    format!("{:.1} MB", draft.size as f64 / 1024. / 1024.)
                                } else {
                                    format!(
                                        "{} KB",
                                        ((draft.size as f64 / 1024.).round() as usize).max(1)
                                    )
                                }),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("remove-attachment-{id}")))
                        .compact()
                        .ghost()
                        .w(px(22.))
                        .min_w(px(22.))
                        .h(px(24.))
                        .p_0()
                        .flex_shrink_0()
                        .child(
                            Icon::new(IconName::X)
                                .with_size(px(13.))
                                .text_color(rgb(palette.muted)),
                        )
                        .accessibility_label(format!("Remove attachment {}", draft.name))
                        .disabled(self.pending)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.remove(&id, cx);
                        })),
                );
            strip = strip.child(chip);
        }
        // An empty attachment strip still contributes its vertical margins.
        // Only real attachments reserve space above the composer input row.
        if !self.drafts.is_empty() {
            panel = panel.child(strip);
        }
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_size(px(12. * font_ratio))
                    .text_color(rgb(0xf87171))
                    .child(error.clone()),
            );
        }
        if let Some(notice) = &self.notice {
            panel = panel.child(notice.clone());
        }
        panel
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn clipboard_raster_images_normalize_to_png() {
        for (gpui_format, image_format) in [
            (ImageFormat::Png, image::ImageFormat::Png),
            (ImageFormat::Tiff, image::ImageFormat::Tiff),
        ] {
            let image = image::DynamicImage::new_rgba8(2, 3);
            let mut encoded = std::io::Cursor::new(Vec::new());
            image.write_to(&mut encoded, image_format).unwrap();
            let file = clipboard_image_file(&Image::from_bytes(gpui_format, encoded.into_inner()))
                .unwrap();
            assert_eq!(file.filename, "Pasted image.png");
            assert_eq!(file.mime_type, "image/png");
            let decoded =
                image::load_from_memory(&STANDARD.decode(file.data_base64).unwrap()).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (2, 3));
        }
        assert!(
            clipboard_image_file(&Image::from_bytes(ImageFormat::Png, b"invalid".to_vec()))
                .is_err()
        );
        assert!(
            clipboard_image_file(&Image::from_bytes(ImageFormat::Svg, b"<svg/>".to_vec())).is_err()
        );
    }
    #[::core::prelude::v1::test]
    fn invalid_backend_metadata_is_not_a_draft() {
        assert!(stored_draft(json!({"id":"a","name":"file","size":MAX_FILE+1}), None).is_err());
        assert!(stored_draft(json!({"name":"file","size":1}), None).is_err());
    }
    #[::core::prelude::v1::test]
    fn unsafe_or_invalid_preview_is_omitted() {
        assert!(inline_preview("image/svg+xml", "abcd").is_none());
        assert!(inline_preview("image/png", "x").is_none());
    }
    #[::core::prelude::v1::test]
    fn message_text_attachment_is_named_bounded_markdown() {
        let filename = message_text_filename("A/Very: Unsafe Author", "visible text").unwrap();
        let attachment = message_text_attachment(filename, "visible text");
        assert_eq!(attachment.filename, "Reply to AVery Unsafe Author.md");
        assert_eq!(attachment.mime_type, "text/markdown");
        assert_eq!(
            STANDARD.decode(attachment.data_base64).unwrap().as_slice(),
            b"visible text"
        );
        assert!(message_text_filename("Alex", "x".repeat(MAX_REPLY_TEXT + 1).as_str()).is_err());
    }
    #[::core::prelude::v1::test]
    fn attachment_persistence_requires_the_same_task_and_exact_destination() {
        let current = AttachmentScope {
            host_id: "host-a".into(),
            cwd: "/work/project".into(),
        };
        let saved = NativeAttachmentDraft {
            host_id: "host-a".into(),
            cwd: "/work/project".into(),
            attachments: vec![],
        };
        assert!(persistence_scope_matches(
            "task-a", "task-a", &current, &saved
        ));
        assert!(!persistence_scope_matches(
            "task-b", "task-a", &current, &saved
        ));
        assert!(!persistence_scope_matches(
            "task-a",
            "task-a",
            &AttachmentScope {
                host_id: "host-b".into(),
                cwd: current.cwd.clone(),
            },
            &saved,
        ));
        assert!(!persistence_scope_matches(
            "task-a",
            "task-a",
            &AttachmentScope {
                host_id: current.host_id.clone(),
                cwd: "/work/other".into(),
            },
            &saved,
        ));
        assert!(!persistence_scope_matches(
            "task-a",
            "task-a",
            &AttachmentScope {
                host_id: String::new(),
                cwd: current.cwd,
            },
            &saved,
        ));
    }
}
