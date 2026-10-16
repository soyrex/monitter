//! One in-memory snapshot per overlay opening. Never screen capture or persistence.
//! Native child WebViews are not part of the GPUI frame; overlays hide them.
use gpui_kit::{App, Bounds, Pixels, RenderImage, Task, Window};
#[cfg(target_os = "macos")]
use gpui_kit::component::ActiveTheme as _;
use std::sync::Arc;

#[derive(Default)]
pub struct BlurredBackdrop {
    image: Option<Arc<RenderImage>>,
    requested: bool,
    generation: u64,
}
impl BlurredBackdrop {
    pub fn image(&self) -> Option<Arc<RenderImage>> {
        self.image.clone()
    }
    pub fn clear(&mut self, window: &mut Window) {
        self.generation = self.generation.wrapping_add(1);
        self.requested = false;
        if let Some(image) = self.image.take() {
            let _ = window.drop_image(image);
        }
    }
    pub fn finish(&mut self, generation: u64, image: Option<Arc<RenderImage>>) -> bool {
        if generation != self.generation || !self.requested {
            return false;
        }
        self.image = image;
        self.image.is_some()
    }
    #[cfg(target_os = "macos")]
    pub fn request(
        &mut self,
        window: &Window,
        bounds: Option<Bounds<Pixels>>,
        cx: &App,
    ) -> Option<(u64, Task<Option<Arc<RenderImage>>>)> {
        if self.requested {
            return None;
        }
        self.requested = true;
        Some((self.generation, capture(window, bounds, cx)?))
    }
    #[cfg(not(target_os = "macos"))]
    pub fn request(
        &mut self,
        _: &Window,
        _: Option<Bounds<Pixels>>,
        _: &App,
    ) -> Option<(u64, Task<Option<Arc<RenderImage>>>)> {
        None
    }
}

/// Capture the last completed GPUI scene and prepare a bounded blurred image.
/// Call before mounting the modal so its contents cannot enter the snapshot.
#[cfg(target_os = "macos")]
pub fn capture(
    window: &Window,
    bounds: Option<Bounds<Pixels>>,
    cx: &App,
) -> Option<Task<Option<Arc<RenderImage>>>> {
    let viewport = window.viewport_size();
    let frame = match window.render_to_image() {
        Ok(frame) => frame,
        Err(error) => {
            eprintln!("Workspace backdrop capture failed: {error}");
            return None;
        }
    };
    let rect = bounds.map(|bounds| {
        let sx = frame.width() as f32 / f32::from(viewport.width);
        let sy = frame.height() as f32 / f32::from(viewport.height);
        (
            f32::from(bounds.origin.x) * sx,
            f32::from(bounds.origin.y) * sy,
            f32::from(bounds.size.width) * sx,
            f32::from(bounds.size.height) * sy,
        )
    });
    let device_scale = frame.width() as f32 / f32::from(viewport.width).max(1.);
    let background: gpui_kit::Rgba = cx.theme().background.into();
    let base = [background.r, background.g, background.b].map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
    Some(cx.background_executor().spawn(async move {
        process_frame(frame, rect, device_scale, base)
            .map(|frame| Arc::new(RenderImage::new(vec![image::Frame::new(frame)])))
    }))
}

#[cfg(not(target_os = "macos"))]
pub fn capture(
    _: &Window,
    _: Option<Bounds<Pixels>>,
    _: &App,
) -> Option<Task<Option<Arc<RenderImage>>>> {
    None
}

fn process_frame(
    mut frame: image::RgbaImage,
    rect: Option<(f32, f32, f32, f32)>,
    device_scale: f32,
    base: [u8; 3],
) -> Option<image::RgbaImage> {
    if frame.width() == 0 || frame.height() == 0 {
        return None;
    }
    if let Some((x, y, w, h)) = rect {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || w <= 0. || h <= 0. {
            return None;
        }
        let left = x.max(0.).floor() as u32;
        let top = y.max(0.).floor() as u32;
        let right = (x + w).max(0.).ceil().min(frame.width() as f32) as u32;
        let bottom = (y + h).max(0.).ceil().min(frame.height() as f32) as u32;
        if right <= left || bottom <= top {
            return None;
        }
        frame = image::imageops::crop_imm(&frame, left, top, right - left, bottom - top).to_image();
    }
    // Readback RGB is already premultiplied by the renderer. Flatten once
    // before blurring: a translucent blurred copy would otherwise let the
    // sharp live text beneath it show through. Foreground menu text stays live.
    for pixel in frame.pixels_mut() {
        let inverse_alpha = 1. - pixel[3] as f32 / 255.;
        for channel in 0..3 {
            pixel[channel] = (pixel[channel] as f32 + base[channel] as f32 * inverse_alpha)
                .round().min(255.) as u8;
        }
        pixel[3] = 255;
    }
    // At most ~3.7 MiB retained, independent of Retina/fullscreen resolution.
    let ratio = (960. / frame.width().max(frame.height()) as f32).min(1.);
    let w = (frame.width() as f32 * ratio).round().max(1.) as u32;
    let h = (frame.height() as f32 * ratio).round().max(1.) as u32;
    let frame = image::imageops::resize(&frame, w, h, image::imageops::FilterType::Triangle);
    let mut frame = image::imageops::blur(&frame, (40. * device_scale * ratio).max(1.));
    // Renderer readback is RGBA; GPUI's direct RenderImage upload uses BGRA.
    for pixel in frame.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_bounded_dimensions_and_correct_channels() {
        let frame = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([200, 30, 10, 255]));
        let out = process_frame(frame, None, 2., [0, 0, 0]).unwrap();
        assert_eq!(out.dimensions(), (960, 540));
        assert_eq!(out.get_pixel(100, 100).0, [10, 30, 200, 255]);
    }
    #[test]
    fn crop_is_pane_local_and_high_frequency_content_is_blurred() {
        let mut frame = image::RgbaImage::from_pixel(80, 40, image::Rgba([255, 0, 0, 255]));
        for y in 0..40 {
            for x in 40..80 {
                frame.put_pixel(
                    x,
                    y,
                    image::Rgba([0, if x % 2 == 0 { 255 } else { 0 }, 0, 255]),
                );
            }
        }
        let out = process_frame(frame, Some((40., 0., 40., 40.)), 1., [0, 0, 0]).unwrap();
        assert_eq!(out.dimensions(), (40, 40));
        let px = out.get_pixel(20, 20).0;
        assert_eq!(px[0], 0);
        assert_eq!(px[2], 0);
        assert!(px[1] > 60 && px[1] < 200);
        assert!(process_frame(out, Some((100., 100., 10., 10.)), 1., [0, 0, 0]).is_none());
    }
    #[test]
    fn translucent_capture_is_flattened_to_hide_sharp_underlying_text() {
        let frame = image::RgbaImage::from_pixel(100, 100, image::Rgba([20, 10, 5, 128]));
        let out = process_frame(frame, None, 1., [100, 80, 60]).unwrap();
        assert_eq!(out.get_pixel(50, 50).0, [35, 50, 70, 255]);
        assert!(out.pixels().all(|pixel| pixel[3] == 255));
    }
    #[test]
    fn closed_or_reopened_overlay_rejects_old_completion() {
        let mut backdrop = BlurredBackdrop::default();
        backdrop.requested = true;
        backdrop.generation = 2;
        let img = Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )]));
        assert!(!backdrop.finish(1, Some(img.clone())));
        assert!(backdrop.image().is_none());
        assert!(backdrop.finish(2, Some(img)));
    }
}
