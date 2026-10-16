//! Retained, read-only attachment presentation for transcript messages.
use crate::design::{self, ui_scaled_px as px};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, WindowExt as _, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::Value;
use std::{collections::HashSet, io::Cursor, sync::Arc, thread};

const MAX_MESSAGE_ATTACHMENTS: usize = 4;
const MAX_PREVIEW_DATA_URL: usize = 512 * 1024;
const MAX_IMAGE_SIDE: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct AttachmentData {
    identity: String,
    id: String,
    name: String,
    mime_type: String,
    size: u64,
    generated: bool,
    preview_data_url: Option<String>,
}

struct DecodedPreview {
    image: Arc<Image>,
    width: u32,
    height: u32,
}

#[derive(Clone)]
enum PreviewState {
    Pending,
    Decoding,
    Ready(Arc<DecodedPreview>),
    Failed(String),
}

#[derive(Clone)]
struct AttachmentItem {
    data: AttachmentData,
    preview: Option<PreviewState>,
    preview_generation: u64,
}

struct DecodeRequest {
    identity: String,
    data_url: String,
    generation: u64,
    original_id: Option<String>,
}

struct DecodeResult {
    identity: String,
    data_url: String,
    generation: u64,
    decoded: Result<DecodedPreview, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentLayout { Cards, PortraitLeft, LandscapeBottom }

fn image_layout(width: u32, height: u32) -> AttachmentLayout {
    if height > width { AttachmentLayout::PortraitLeft } else { AttachmentLayout::LandscapeBottom }
}
fn portrait_geometry(width: u32, height: u32, bubble_width: f32) -> (f32, f32) {
    let bubble_width = if bubble_width.is_finite() { bubble_width.max(1.) } else { 300. };
    let maximum_width = (bubble_width * 0.3).min(180.);
    let minimum_width = 40_f32.min(maximum_width);
    let aspect = height.max(1) as f32 / width.max(1) as f32;
    let left_width = (240. / aspect).clamp(minimum_width, maximum_width);
    let min_height = (left_width * aspect).clamp(180., 320.);
    (left_width, min_height)
}
fn hero_height(width: f32) -> f32 { (width.max(0.) * 0.48).clamp(120., 220.) }
fn wide_strip(width: u32, height: u32) -> bool { width >= height.max(1).saturating_mul(3) }
fn strip_height(width: u32, height: u32, bubble_width: f32) -> f32 {
    bubble_width.max(1.).min(width.max(1) as f32) * height.max(1) as f32 / width.max(1) as f32
}

pub struct MessageAttachments {
    service: DesktopService,
    attachments: Vec<AttachmentItem>,
    omitted: usize,
    generation: u64,
    error: Option<String>,
    palette: Option<design::Palette>,
    list_item: Option<(ListState, usize)>,
    bubble_width: Option<f32>,
    bubble_layout: AttachmentLayout,
    compact_images: bool,
    presentation_scale: f32,
}

fn short_string(value: &Value, key: &str, max_chars: usize) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(max_chars)
        .collect()
}

fn preview_mime(data_url: &str) -> Option<image::ImageFormat> {
    if data_url.len() > MAX_PREVIEW_DATA_URL {
        return None;
    }
    let (prefix, encoded) = data_url.split_once(',')?;
    if encoded.is_empty() {
        return None;
    }
    match prefix {
        "data:image/png;base64" => Some(image::ImageFormat::Png),
        "data:image/jpeg;base64" => Some(image::ImageFormat::Jpeg),
        "data:image/webp;base64" => Some(image::ImageFormat::WebP),
        _ => None,
    }
}

fn decode_preview(data_url: &str) -> Result<DecodedPreview, String> {
    if data_url.len() > MAX_PREVIEW_DATA_URL {
        return Err("Inline image preview exceeds the 512 KiB limit.".into());
    }
    let (prefix, encoded) = data_url
        .split_once(',')
        .ok_or("Inline image preview is not a data URL.")?;
    let format = match prefix {
        "data:image/png;base64" => image::ImageFormat::Png,
        "data:image/jpeg;base64" => image::ImageFormat::Jpeg,
        "data:image/webp;base64" => image::ImageFormat::WebP,
        _ => return Err("Inline previews support PNG, JPEG, and WebP images only.".into()),
    };
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Inline image preview contains invalid base64.")?;
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes.as_slice()), format)
        .into_dimensions()
        .map_err(|error| format!("Cannot read preview dimensions: {error}"))?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err("Inline image preview exceeds the 8192 px / 16 MP dimension limit.".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("Cannot decode inline image preview: {error}"))?
        .thumbnail(width.min(600), height.min(600));
    let width = decoded.width();
    let height = decoded.height();
    let mut output = Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot prepare inline image preview: {error}"))?;
    Ok(DecodedPreview {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, output.into_inner())),
        width,
        height,
    })
}

fn attachment_data(values: &[Value]) -> (Vec<AttachmentData>, usize) {
    let mut seen = HashSet::new();
    let mut retained = Vec::with_capacity(MAX_MESSAGE_ATTACHMENTS);
    let mut omitted = 0usize;
    for (index, value) in values.iter().enumerate() {
        let id = short_string(value, "id", 256);
        let source_id = short_string(value, "sourceId", 256);
        let identity = if !source_id.is_empty() {
            source_id
        } else if !id.is_empty() {
            id.clone()
        } else {
            format!("attachment-index-{index}")
        };
        if !seen.insert(identity.clone()) {
            continue;
        }
        if retained.len() >= MAX_MESSAGE_ATTACHMENTS {
            omitted += 1;
            continue;
        }
        let path = short_string(value, "path", 256);
        let preview_data_url = value
            .get("previewDataUrl")
            .and_then(Value::as_str)
            .filter(|data_url| preview_mime(data_url).is_some())
            .map(str::to_owned);
        retained.push(AttachmentData {
            identity,
            id,
            name: short_string(value, "name", 256),
            mime_type: short_string(value, "mimeType", 128),
            size: value.get("size").and_then(Value::as_u64).unwrap_or(0),
            generated: path.to_ascii_lowercase().starts_with("generated by "),
            preview_data_url,
        });
    }
    (retained, omitted)
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024. / 1024.)
    } else {
        format!("{} KB", ((bytes as f64 / 1024.).round() as u64).max(1))
    }
}

fn fit_dimensions(width: u32, height: u32, max_width: f32, max_height: f32) -> (f32, f32) {
    let scale = (max_width / width.max(1) as f32)
        .min(max_height / height.max(1) as f32)
        .min(1.);
    (width as f32 * scale, height as f32 * scale)
}

impl MessageAttachments {
    pub fn new(service: DesktopService, attachments: &[Value], cx: &mut Context<Self>) -> Self {
        let mut pane = Self {
            service,
            attachments: Vec::new(),
            omitted: 0,
            generation: 0,
            error: None,
            palette: None,
            list_item: None,
            bubble_width: None,
            bubble_layout: AttachmentLayout::Cards,
            compact_images: false,
            presentation_scale: 1.,
        };
        pane.set_attachments(attachments, cx);
        pane
    }

    /// Independent hover presentation retains decoded images without sharing
    /// list invalidation or attachment layout with the live transcript row.
    pub(super) fn preview_snapshot(&self, scale: f32) -> Self {
        let mut attachments = self.attachments.clone();
        for item in &mut attachments {
            if matches!(item.preview, Some(PreviewState::Decoding)) {
                item.preview = Some(PreviewState::Pending);
            }
        }
        Self { service: self.service.clone(), attachments, omitted: self.omitted,
            generation: self.generation, error: self.error.clone(), palette: self.palette,
            list_item: None, bubble_width: self.bubble_width, bubble_layout: self.bubble_layout,
            compact_images: self.compact_images, presentation_scale: scale }
    }

    pub fn set_attachments(&mut self, attachments: &[Value], cx: &mut Context<Self>) {
        let (mut data, omitted) = attachment_data(attachments);
        // Stored attachments may predate persisted inline previews. Resolve
        // thumbnails by registered ID only, never an arbitrary transcript path.
        for item in &mut data {
            if item.preview_data_url.is_none()
                && !item.generated
                && !item.id.is_empty()
                && matches!(
                    item.mime_type.as_str(),
                    "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                )
            {
                item.preview_data_url = Some(format!("attachment-id:{}", item.id));
            }
        }
        if omitted == self.omitted
            && data.len() == self.attachments.len()
            && data
                .iter()
                .zip(&self.attachments)
                .all(|(next, current)| next == &current.data)
        {
            return;
        }

        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let previous = std::mem::take(&mut self.attachments);
        self.attachments = data
            .into_iter()
            .map(|data| {
                let prior = previous.iter().find(|item| {
                    item.data.identity == data.identity
                        && item.data.preview_data_url == data.preview_data_url
                });
                let preview = data.preview_data_url.as_ref().map(|_| {
                    prior
                        .and_then(|item| item.preview.clone())
                        .unwrap_or(PreviewState::Pending)
                });
                let preview_generation = prior
                    .map(|item| item.preview_generation)
                    .unwrap_or(generation);
                AttachmentItem {
                    data,
                    preview,
                    preview_generation,
                }
            })
            .collect();
        self.omitted = omitted;
        self.error = None;
        // The virtual list renders only the viewport. Decoding here would
        // fetch and inflate thumbnails for every retained history message.
        cx.notify();
    }

    fn start_pending_previews(&mut self, cx: &mut Context<Self>) {
        let requests: Vec<_> = self
            .attachments
            .iter_mut()
            .filter_map(|item| {
                let preview = item.preview.as_mut()?;
                if !matches!(preview, PreviewState::Pending) {
                    return None;
                }
                let data_url = item.data.preview_data_url.clone()?;
                *preview = PreviewState::Decoding;
                Some(DecodeRequest {
                    identity: item.data.identity.clone(),
                    data_url,
                    generation: item.preview_generation,
                    original_id: (!item.data.generated && !item.data.id.is_empty()).then(|| item.data.id.clone()),
                })
            })
            .collect();
        if requests.is_empty() {
            return;
        }

        let worker_requests: Vec<_> = requests
            .iter()
            .map(|request| {
                (
                    request.identity.clone(),
                    request.data_url.clone(),
                    request.generation,
                    request.original_id.clone(),
                )
            })
            .collect();
        let (sender, receiver) = oneshot::channel();
        let service = self.service.clone();
        let worker = thread::Builder::new()
            .name("monitter-message-image-preview".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let results = worker_requests
                    .into_iter()
                    .map(|(identity, data_url, generation, original_id)| DecodeResult {
                        decoded: if let Some(id) = data_url.strip_prefix("attachment-id:") {
                            super::attachments::image_thumbnail(service.clone(), id).map(
                                |(image, width, height)| DecodedPreview {
                                    image,
                                    width,
                                    height,
                                },
                            )
                        } else {
                            decode_preview(&data_url).and_then(|preview| {
                                if wide_strip(preview.width, preview.height) {
                                    if let Some(id) = original_id {
                                        return super::attachments::image_thumbnail(service.clone(), &id)
                                            .map(|(image, width, height)| DecodedPreview { image, width, height });
                                    }
                                }
                                Ok(preview)
                            })
                        },
                        identity,
                        data_url,
                        generation,
                    })
                    .collect::<Vec<_>>();
                let _ = sender.send(results);
            });
        if let Err(error) = worker {
            for request in &requests {
                if let Some(item) = self.attachments.iter_mut().find(|item| {
                    item.data.identity == request.identity
                        && item.preview_generation == request.generation
                }) {
                    item.preview = Some(PreviewState::Failed(format!(
                        "Could not start image preview decoder: {error}"
                    )));
                }
            }
            return;
        }

        let pane = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let results = receiver.await.unwrap_or_default();
            let _ = pane.update(cx, |pane, cx| {
                for result in results {
                    let Some(item) = pane.attachments.iter_mut().find(|item| {
                        item.data.identity == result.identity
                            && item.preview_generation == result.generation
                            && item.data.preview_data_url.as_deref() == Some(&result.data_url)
                    }) else {
                        continue;
                    };
                    item.preview = Some(match result.decoded {
                        Ok(decoded) => PreviewState::Ready(Arc::new(decoded)),
                        Err(error) => PreviewState::Failed(error),
                    });
                }
                pane.invalidate_list_item();
                cx.notify();
            });
        })
        .detach();
    }

    /// First decoded image determines placement; pending images retain cards until ready.
    pub fn layout(&self) -> AttachmentLayout {
        if self.compact_images { return AttachmentLayout::Cards; }
        self.attachments.iter().find_map(|item| match item.preview.as_ref() {
            Some(PreviewState::Ready(preview)) => Some(image_layout(preview.width, preview.height)),
            _ => None,
        }).unwrap_or(AttachmentLayout::Cards)
    }
    /// Assistant screenshots share one compact row; full-size viewing keeps
    /// the same original/inline lightbox path as the larger previews.
    pub fn set_compact_images(&mut self, compact: bool) {
        self.compact_images = compact;
    }
    pub fn portrait_dimensions(&self, width_design: f32) -> (f32, f32) {
        self.attachments.iter().find_map(|item| match item.preview.as_ref() {
            Some(PreviewState::Ready(preview)) => Some(portrait_geometry(preview.width, preview.height, width_design)),
            _ => None,
        }).unwrap_or_else(|| portrait_geometry(1, 2, width_design))
    }
    /// Width is in design pixels, before interface scaling.
    pub fn set_bubble_layout(&mut self, width_design: f32, layout: AttachmentLayout, _cx: &mut Context<Self>) {
        let width = if width_design.is_finite() { width_design.max(1.) } else { 300. };
        // Called during the parent's row render: no notify/splice feedback loop.
        self.bubble_width = Some(width);
        self.bubble_layout = layout;
    }

    pub fn set_list_item(&mut self, list: ListState, index: usize) {
        self.list_item = Some((list, index));
    }
    fn invalidate_list_item(&self) {
        if let Some((list, index)) = &self.list_item {
            if *index < list.item_count() {
                list.splice(*index..*index + 1, 1);
            }
        }
    }
    pub fn set_appearance(&mut self, palette: design::Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn build_widget(
        &self,
        palette: design::Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let px = |value: f32| design::ui_scaled_px(value * self.presentation_scale);
        if self.attachments.is_empty() && self.omitted == 0 && self.error.is_none() {
            return div().into_any_element();
        }
        let pane = cx.entity().downgrade();
        let service = self.service.clone();
        let mut body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(if self.compact_images { 2. } else { 8. }))
            .my(px(if self.compact_images { 2. } else { 8. }));
        let hero = self.bubble_width.is_some() && self.bubble_layout != AttachmentLayout::Cards;
        let portrait = self.bubble_layout == AttachmentLayout::PortraitLeft;
        if hero {
            body = div().w_full().min_w_0().flex().flex_col().gap(px(0.));
            if portrait { body = body.h_full(); }
        }
        for (index, item) in self.attachments.iter().enumerate() {
            let mut card = div()
                .max_w(px(300.))
                .min_w_0()
                .p(px(6.))
                .border_1()
                .border_color(rgb(palette.line))
                .rounded(px(8.))
                .bg(crate::design::surface_color(palette.panel));
            if let Some(PreviewState::Ready(preview)) = item.preview.as_ref() {
                let original = !item.data.generated && !item.data.id.is_empty();
                let attachment_id = item.data.id.clone();
                let title = item.data.name.clone();
                let inline_image = preview.image.clone();
                let image_width = preview.width;
                let image_height = preview.height;
                let service = service.clone();
                let pane = pane.clone();
                let (width, height) = fit_dimensions(preview.width, preview.height, 284., 300.);
                let mut preview_image = img(preview.image.clone()).w(px(width)).h(px(height)).max_w(px(284.)).max_h(px(300.)).rounded(px(5.));
                if self.compact_images {
                    preview_image = img(preview.image.clone()).w(px(96.)).h(px(72.))
                        .object_fit(ObjectFit::Contain);
                }
                let strip = !portrait && wide_strip(preview.width, preview.height);
                if hero {
                    preview_image = img(preview.image.clone()).w_full().h_full()
                        .object_fit(if strip { ObjectFit::Contain } else { ObjectFit::Cover });
                }
                let mut preview_button = Button::new(format!("message-attachment-preview-{index}"))
                    .ghost()
                    .w_full()
                    .h_auto()
                    .min_w_0()
                    .p_0()
                    .accessibility_label(format!("View full-size {}", item.data.name))
                    .child(preview_image)
                    .on_click(move |_, window, cx| {
                        if original {
                            if let Err(error) = super::attachments::open_image_modal(
                                service.clone(),
                                attachment_id.clone(),
                                window,
                                cx,
                            ) {
                                let _ = pane.update(cx, |pane, cx| {
                                    pane.error = Some(error);
                                    cx.notify();
                                });
                            }
                        } else {
                            open_inline_lightbox(
                                window,
                                cx,
                                title.clone(),
                                inline_image.clone(),
                                image_width,
                                image_height,
                            );
                        }
                    });
                if hero {
                    preview_button = preview_button.rounded(px(0.)).tooltip(item.data.name.clone());
                    let image_height = if portrait { 180. } else if strip {
                        strip_height(preview.width, preview.height, self.bubble_width.unwrap_or(300.))
                    } else { hero_height(self.bubble_width.unwrap_or(300.)) };
                    card = div().w_full().min_w_0().overflow_hidden().flex().flex_col();
                    if portrait && index == 0 {
                        card = card.flex_1().h_full().min_h(px(120.));
                        preview_button = preview_button.h_full();
                    } else {
                        card = card.h(px(image_height)).flex_shrink_0();
                        preview_button = preview_button.h(px(image_height));
                    }
                    card = card.child(preview_button);
                } else if self.compact_images {
                    preview_button = preview_button.w(px(96.)).h(px(72.))
                        .tooltip(item.data.name.clone());
                    card = card.p_0().w(px(98.)).h(px(74.)).flex_shrink_0()
                        .overflow_hidden().rounded(px(6.)).child(preview_button);
                } else {
                    card = card.flex().flex_col().gap(px(6.)).p(px(8.)).child(preview_button)
                        .child(attachment_info(&item.data, palette, self.presentation_scale));
                }
            } else {
                let image_file = item.data.mime_type.starts_with("image/");
                let can_open_original =
                    image_file && !item.data.generated && !item.data.id.is_empty();
                let icon = Icon::new(IconName::FileText)
                    .with_size(px(23.))
                    .text_color(rgb(palette.muted));
                let mut file_row = div().w_full().min_w_0().flex().items_center().gap(px(8.));
                if can_open_original {
                    let attachment_id = item.data.id.clone();
                    let service = service.clone();
                    let pane = pane.clone();
                    file_row = file_row.child(
                        Button::new(format!("message-attachment-open-{index}"))
                            .ghost()
                            .compact()
                            .w(px(34.))
                            .h(px(42.))
                            .p_0()
                            .icon(icon)
                            .accessibility_label(format!("View full-size {}", item.data.name))
                            .on_click(move |_, window, cx| {
                                if let Err(error) = super::attachments::open_image_modal(
                                    service.clone(),
                                    attachment_id.clone(),
                                    window,
                                    cx,
                                ) {
                                    let _ = pane.update(cx, |pane, cx| {
                                        pane.error = Some(error);
                                        cx.notify();
                                    });
                                }
                            }),
                    );
                } else {
                    file_row = file_row.child(
                        div()
                            .w(px(34.))
                            .h(px(42.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(crate::design::surface_color(palette.soft))
                            .child(icon),
                    );
                }
                card = card.child(file_row.child(attachment_info(&item.data, palette, self.presentation_scale)));
                if let Some(PreviewState::Failed(error)) = item.preview.as_ref() {
                    card = card.child(
                        div()
                            .text_size(px(9.))
                            .text_color(rgb(palette.muted))
                            .child(format!("Image preview unavailable: {error}")),
                    );
                }
            }
            body = body.child(card);
        }
        if self.omitted > 0 {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{} additional attachment{} not shown.",
                        self.omitted,
                        if self.omitted == 1 { "" } else { "s" }
                    )),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(0xd65b5b))
                    .child(error.clone()),
            );
        }
        body.into_any_element()
    }
}

impl Render for MessageAttachments {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_pending_previews(cx);
        let palette = self
            .palette
            .unwrap_or_else(|| design::palette(&Value::Null, window.appearance()));
        self.build_widget(palette, window, cx)
    }
}

fn attachment_info(data: &AttachmentData, palette: design::Palette, scale: f32) -> AnyElement {
    let px = |value: f32| design::ui_scaled_px(value * scale);
    div()
        .w_full()
        .min_w_0()
        .flex()
        .items_baseline()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(palette.ink))
                .child(data.name.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family(crate::design::FONT_MONO)
                .text_size(px(9.))
                .text_color(rgb(palette.muted))
                .child(format_size(data.size)),
        )
        .into_any_element()
}

fn open_inline_lightbox(
    window: &mut Window,
    cx: &mut App,
    title: String,
    image: Arc<Image>,
    width: u32,
    height: u32,
) {
    let (width, height) = fit_dimensions(width, height, 840., 620.);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .width(px(880.))
            .keyboard(true)
            .close_button(true)
            .child(
                div()
                    .id("inline-image-lightbox")
                    .w_full()
                    .min_w_0()
                    .max_h(px(640.))
                    .overflow_scroll()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(img(image.clone()).w(px(width)).h(px(height))),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn attachment_portrait_dimensions_keep_tall_aspect_and_narrow_bounds() {
        let (width, height) = portrait_geometry(100, 600, 500.);
        assert_eq!((width, height), (40., 240.));
        assert!((height / width - 6.).abs() < 0.01);
        let (width, height) = portrait_geometry(100, 600, 100.);
        assert!(width <= 30.001);
        assert!((180. ..=320.).contains(&height));
        let (width, height) = portrait_geometry(100, 2000, 600.);
        assert_eq!((width, height), (40., 320.));
    }

    #[::core::prelude::v1::test]
    fn attachment_hero_geometry_preserves_orientation_and_bounds() {
        assert_eq!(image_layout(300, 600), AttachmentLayout::PortraitLeft);
        assert_eq!(image_layout(600, 300), AttachmentLayout::LandscapeBottom);
        assert_eq!(image_layout(300, 300), AttachmentLayout::LandscapeBottom);
        assert_eq!(hero_height(100.), 120.);
        assert_eq!(hero_height(1000.), 220.);
        assert_eq!(hero_height(400.), 192.);
        assert!(wide_strip(900, 100));
        assert!(!wide_strip(600, 300));
        assert_eq!(strip_height(900, 100, 450.), 50.);
        assert_eq!(strip_height(900, 100, 1200.), 100.);
    }

    #[::core::prelude::v1::test]
    fn preview_decoder_accepts_small_raster_and_rejects_svg_or_corrupt_input() {
        let source = image::DynamicImage::new_rgba8(3, 2);
        let mut output = Cursor::new(Vec::new());
        source
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let data_url = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(output.into_inner())
        );
        let decoded = decode_preview(&data_url).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert!(decode_preview("data:image/svg+xml;base64,AAAA").is_err());
        assert!(decode_preview("data:image/png;base64,not-base64").is_err());
    }

    #[::core::prelude::v1::test]
    fn preview_decoder_rejects_dimensions_before_full_decode() {
        let source = image::DynamicImage::new_rgba8(8193, 1);
        let mut output = Cursor::new(Vec::new());
        source
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let data_url = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(output.into_inner())
        );
        assert!(
            decode_preview(&data_url)
                .err()
                .unwrap()
                .contains("dimension")
        );
    }

    #[::core::prelude::v1::test]
    fn preview_decoder_rejects_oversized_inline_source() {
        let data_url = format!("data:image/png;base64,{}", "A".repeat(MAX_PREVIEW_DATA_URL));
        assert!(decode_preview(&data_url).err().unwrap().contains("512 KiB"));
    }
}
