//! Original raster previews, read only by a registered attachment ID on demand.
use crate::design::ui_scaled_px as px;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::{
    component::{WindowExt as _, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::Arc;
const MAX_SOURCE: usize = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
gpui_kit::actions!(attachment_image, [ClosePreview]);
struct DecodedImage {
    image: Arc<Image>,
    width: u32,
    height: u32,
    filename: String,
    first_frame: bool,
}
fn decode(value: Value) -> Result<DecodedImage, String> {
    decode_image(value, false)
}
fn decode_image(value: Value, thumbnail: bool) -> Result<DecodedImage, String> {
    let mime = value["mimeType"]
        .as_str()
        .ok_or("Image response has no MIME type.")?;
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/webp" => image::ImageFormat::WebP,
        "image/gif" => image::ImageFormat::Gif,
        _ => return Err("Only PNG, JPEG, WebP and GIF raster previews are supported.".into()),
    };
    let encoded = value["dataBase64"]
        .as_str()
        .ok_or("Image response has no data.")?;
    if encoded.len() > ((MAX_SOURCE + 2) / 3) * 4 {
        return Err("Image exceeds the 20 MiB limit.".into());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Image response contains invalid base64.")?;
    if bytes.len() > MAX_SOURCE {
        return Err("Image exceeds the 20 MiB limit.".into());
    }
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|error| format!("Cannot read image dimensions: {error}"))?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(
            "Image dimensions exceed the safe preview limit (8192 px per side, 16 megapixels)."
                .into(),
        );
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    // Decode one bounded frame, then give GPUI only a known PNG representation.
    let image = reader
        .decode()
        .map_err(|error| format!("Cannot decode image: {error}"))?;
    let image = if thumbnail {
        if width >= height.saturating_mul(3) {
            // Text strips need their source pixels, not an enlarged tiny card.
            image.thumbnail(width.min(1600), height.min(600))
        } else {
            image.thumbnail(width.min(600), height.min(600))
        }
    } else {
        image
    };
    let (width, height) = (image.width(), image.height());
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("Cannot prepare image: {error}"))?;
    let filename = value["filename"].as_str().unwrap_or("Image").to_owned();
    Ok(DecodedImage {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, output.into_inner())),
        width,
        height,
        filename,
        first_frame: mime == "image/gif",
    })
}
pub(super) fn open(
    service: DesktopService,
    attachment_id: String,
    cx: &mut App,
) -> Result<(), String> {
    if attachment_id.is_empty() {
        return Err("Attachment has no registered ID.".into());
    }
    cx.bind_keys([KeyBinding::new(
        "escape",
        ClosePreview,
        Some("MonitterAttachmentImage"),
    )]);
    gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(920.), px(720.)),
                cx,
            ))),
            app_owns_titlebar_drag: true,
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.), px(22.))),
            }),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| AttachmentPreview::new(service, attachment_id, window, cx)),
    )
    .map(|_| ())
    .map_err(|error| format!("Cannot open image preview: {error}"))
}
pub(super) fn open_modal(
    service: DesktopService,
    attachment_id: String,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    if attachment_id.is_empty() {
        return Err("Attachment has no registered ID.".into());
    }
    let preview = cx.new(|cx| {
        let mut preview = AttachmentPreview::new(service, attachment_id, window, cx);
        preview.modal = true;
        preview
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Image preview")
            .width(px(900.))
            .keyboard(true)
            .close_button(true)
            .child(div().w_full().h(px(620.)).child(preview.clone()))
    });
    Ok(())
}
pub(super) fn thumbnail(
    service: DesktopService,
    attachment_id: &str,
) -> Result<(Arc<Image>, u32, u32), String> {
    let value = service.invoke(
        "read_attachment_image",
        json!({"attachmentId": attachment_id}),
    )?;
    let decoded = decode_image(value, true)?;
    Ok((decoded.image, decoded.width, decoded.height))
}
struct AttachmentPreview {
    decoded: Option<DecodedImage>,
    error: Option<String>,
    actual_size: bool,
    modal: bool,
    focus: FocusHandle,
}
impl AttachmentPreview {
    fn new(
        service: DesktopService,
        attachment_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let task = cx.background_executor().spawn(async move {
            decode(service.invoke(
                "read_attachment_image",
                json!({"attachmentId":attachment_id}),
            )?)
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(image) => view.decoded = Some(image),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            decoded: None,
            error: None,
            actual_size: false,
            modal: false,
            focus,
        }
    }
}
impl Render for AttachmentPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .key_context("MonitterAttachmentImage")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &ClosePreview, window, cx| {
                if view.modal {
                    window.close_dialog(cx);
                } else {
                    window.remove_window();
                }
            }));
        let title = self
            .decoded
            .as_ref()
            .map(|image| {
                format!(
                    "{} · {} × {}{}",
                    image.filename,
                    image.width,
                    image.height,
                    if image.first_frame {
                        " · first GIF frame"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| "Image preview".into());
        body = body.child(
            div()
                .flex()
                .items_center()
                .h(px(58.))
                .pl(px(82.))
                .pr_3()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_center()
                        .child(title)
                        .on_mouse_down(MouseButton::Left, |event, window, _| {
                            if event.click_count == 2 {
                                window.titlebar_double_click();
                            } else {
                                window.start_window_move();
                            }
                        }),
                )
                .child(
                    Button::new("image-fit")
                        .label(if self.actual_size {
                            "Fit image"
                        } else {
                            "Actual size"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.actual_size = !view.actual_size;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("image-close")
                        .label("Close")
                        .on_click(cx.listener(|view, _, window, cx| {
                            if view.modal {
                                window.close_dialog(cx);
                            } else {
                                window.remove_window();
                            }
                        })),
                ),
        );
        if let Some(image) = &self.decoded {
            let viewport = window.viewport_size();
            let ui_scale = f32::from(px(1.)).max(0.01);
            // The dialog's 620 px child is smaller than the host window. Reserve
            // dialog padding, preview controls and scroll margins for Fit;
            // Actual size intentionally retains the scrollable native size.
            let (fit_width, fit_height) = if self.modal {
                (
                    (f32::from(viewport.width) / ui_scale - 100.)
                        .min(800.)
                        .max(1.),
                    (f32::from(viewport.height) / ui_scale - 200.)
                        .min(510.)
                        .max(1.),
                )
            } else {
                (
                    (f32::from(viewport.width) / ui_scale - 48.).max(1.),
                    (f32::from(viewport.height) / ui_scale - 110.).max(1.),
                )
            };
            let scale = if self.actual_size {
                1.
            } else {
                (fit_width / image.width as f32)
                    .min(fit_height / image.height as f32)
                    .min(1.)
            };
            body = body.child(
                div()
                    .id("original-image-scroll")
                    .mx_3()
                    .mb_3()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_scroll()
                    .child(
                        img(image.image.clone())
                            .w(px(image.width as f32 * scale))
                            .h(px(image.height as f32 * scale)),
                    ),
            );
        } else if let Some(error) = &self.error {
            body = body.child(div().text_color(rgb(0xdc2626)).child(error.clone()));
        } else {
            body = body.child("Loading original image…");
        }
        body
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn thumbnail_decode_bounds_retained_dimensions() {
        let image = image::DynamicImage::new_rgba8(1200, 800);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image(
            json!({"mimeType":"image/png","dataBase64":STANDARD.encode(output.into_inner())}),
            true,
        )
        .unwrap();
        assert_eq!((decoded.width, decoded.height), (600, 400));
    }
    #[::core::prelude::v1::test]
    fn wide_text_strip_keeps_source_resolution() {
        let image = image::DynamicImage::new_rgba8(900, 100);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image(
            json!({"mimeType":"image/png","dataBase64":STANDARD.encode(output.into_inner())}),
            true,
        )
        .unwrap();
        assert_eq!((decoded.width, decoded.height), (900, 100));
    }
    #[::core::prelude::v1::test]
    fn rejects_unsupported_and_corrupt_image_responses() {
        assert!(decode(json!({"mimeType":"image/svg+xml","dataBase64":"AAAA"})).is_err());
        assert!(decode(json!({"mimeType":"image/png","dataBase64":"not-base64"})).is_err());
        assert!(decode(json!({"mimeType":"image/png","dataBase64":"AAAA"})).is_err());
    }
    #[::core::prelude::v1::test]
    fn rejects_dimensions_before_full_decoding() {
        let image = image::DynamicImage::new_rgba8(8193, 1);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let result = decode(
            json!({"mimeType":"image/png","dataBase64":STANDARD.encode(output.into_inner())}),
        );
        assert!(result.err().unwrap().contains("dimensions"));
    }
    #[::core::prelude::v1::test]
    fn decodes_real_small_raster_dimensions() {
        let image = image::DynamicImage::new_rgba8(3, 2);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(json!({"filename":"fixture.png","mimeType":"image/png","dataBase64":STANDARD.encode(output.into_inner())})).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
    }
}
