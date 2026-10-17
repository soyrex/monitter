//! Cached local wallpaper behind fullscreen translucent surfaces. No screen capture.
use gpui_kit::{Context, RenderImage, Window};
use std::{path::PathBuf, sync::Arc, time::{Duration, Instant, SystemTime}};
const MAX_EDGE: u32 = 960;
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint { display: u64, path: PathBuf, modified: Option<SystemTime>, bytes: u64 }
#[derive(Default)]
pub struct WallpaperBackdrop {
    enabled: bool,
    image: Option<Arc<RenderImage>>,
    fingerprint: Option<Fingerprint>,
    generation: u64,
    loading: bool,
    checked_display: Option<u64>,
    last_check: Option<Instant>,
}
impl WallpaperBackdrop {
    pub fn enabled(&self) -> bool { self.enabled }
    pub fn image(&self) -> Option<Arc<RenderImage>> { self.image.clone() }
    pub fn refresh(&mut self, window: &mut Window, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = enabled;
        if !enabled {
            self.generation = self.generation.wrapping_add(1);
            self.loading = false;
            self.last_check = None;
            self.checked_display = None;
            self.fingerprint = None;
            if let Some(old) = self.image.take() { let _ = window.drop_image(old); cx.notify(); }
            return;
        }
        let Some(display) = window.display(cx).map(|d| u64::from(d.id())) else { return };
        if self.checked_display == Some(display) && (self.loading || self.last_check.is_some_and(|t| t.elapsed() < Duration::from_secs(2))) { return; }
        self.checked_display = Some(display);
        self.last_check = Some(Instant::now());
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let previous = self.fingerprint.clone();
        self.loading = true;
        // AppKit URL lookup must stay on the UI thread. Other platforms resolve on the worker.
        #[cfg(target_os = "macos")]
        let path = crate::wallpaper_accent::wallpaper_path(display).or_else(|| {
            // A macOS fullscreen Space may not expose its own desktop image.
            previous.as_ref().filter(|p| p.display == display).map(|p| p.path.clone())
        });
        let worker = cx.background_executor().spawn(async move {
            #[cfg(target_os = "macos")]
            let path = if crate::wallpaper_macos::aerial_selected() {
                crate::wallpaper_macos::aerial_thumbnail()
            } else { path };
            #[cfg(not(target_os = "macos"))]
            let path = crate::wallpaper_platform::resolve();
            let path = path.ok_or("Wallpaper file unavailable")?;
            let metadata = std::fs::metadata(&path).map_err(|_| "Wallpaper metadata unavailable")?;
            if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 { return Err("Wallpaper file exceeds budget"); }
            let fingerprint = Fingerprint { display, path, modified: metadata.modified().ok(), bytes: metadata.len() };
            if previous.as_ref() == Some(&fingerprint) { return Ok((fingerprint, None)); }
            let rgba = decode(&fingerprint.path).map_err(|_| "Wallpaper image unavailable")?;
            let rgba = prepare(rgba).ok_or("Wallpaper bitmap invalid")?;
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]));
            Ok((fingerprint, Some(image)))
        });
        cx.spawn_in(window, async move |entity, cx| {
            let result = worker.await;
            let _ = entity.update_in(cx, |state, window, cx| {
                if state.generation != generation { return; }
                state.loading = false;
                match result {
                    Ok((fingerprint, image)) => {
                        state.fingerprint = Some(fingerprint);
                        if let Some(image) = image {
                            if let Some(old) = state.image.replace(image) { let _ = window.drop_image(old); }
                            cx.notify();
                        }
                    }
                    Err(_) => {
                        state.fingerprint = None;
                        if let Some(old) = state.image.take() { let _ = window.drop_image(old); cx.notify(); }
                    }
                }
            });
        }).detach();
    }
}
#[cfg(target_os = "macos")]
fn decode(path: &std::path::Path) -> Result<image::RgbaImage, String> {
    crate::wallpaper_accent::thumbnail_rgba(path, MAX_EDGE)
}
#[cfg(not(target_os = "macos"))]
fn decode(path: &std::path::Path) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?
        .with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16384); limits.max_image_height = Some(16384);
    reader.limits(limits);
    Ok(reader.decode().map_err(|e| e.to_string())?.thumbnail(MAX_EDGE, MAX_EDGE).to_rgba8())
}
fn prepare(mut rgba: image::RgbaImage) -> Option<image::RgbaImage> {
    if rgba.width() == 0 || rgba.height() == 0 { return None; }
    let ratio = (MAX_EDGE as f32 / rgba.width().max(rgba.height()) as f32).min(1.);
    if ratio < 1. { rgba = image::imageops::resize(&rgba, (rgba.width() as f32 * ratio).round().max(1.) as u32, (rgba.height() as f32 * ratio).round().max(1.) as u32, image::imageops::FilterType::Triangle); }
    for pixel in rgba.pixels_mut() {
        for c in 0..3 { pixel[c] = ((pixel[c] as u32 * pixel[3] as u32) / 255) as u8; }
        pixel[3] = 255;
    }
    let mut rgba = image::imageops::blur(&rgba, 16.);
    // GPUI direct image upload expects BGRA.
    for pixel in rgba.pixels_mut() { pixel.0.swap(0, 2); }
    Some(rgba)
}
pub fn needed(fullscreen: bool, surface: &str) -> bool {
    // AppKit supplies its native material in fullscreen too. Painting the raw
    // wallpaper above it bypasses the material's tint and luminance treatment,
    // making the same translucent surfaces much brighter after entering a Space.
    needs_wallpaper_fallback(cfg!(target_os = "macos"), fullscreen, surface)
}
fn needs_wallpaper_fallback(native_material: bool, fullscreen: bool, surface: &str) -> bool {
    !native_material && fullscreen && matches!(surface, "glass" | "translucent")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wallpaper_backdrop_preserves_native_material_across_fullscreen_transitions() {
        for surface in ["glass", "translucent", "opaque"] {
            for fullscreen in [false, true] {
                assert!(!needs_wallpaper_fallback(true, fullscreen, surface));
                assert_eq!(needed(fullscreen, surface), needs_wallpaper_fallback(cfg!(target_os = "macos"), fullscreen, surface));
            }
        }
    }
    #[test]
    fn wallpaper_backdrop_retains_other_platform_fullscreen_fallback() {
        assert!(needs_wallpaper_fallback(false, true, "glass"));
        assert!(needs_wallpaper_fallback(false, true, "translucent"));
        assert!(!needs_wallpaper_fallback(false, false, "glass"));
        assert!(!needs_wallpaper_fallback(false, true, "opaque"));
    }
    #[test]
    fn wallpaper_backdrop_is_bounded_blurred_and_bgra() {
        let mut image = image::RgbaImage::from_pixel(1200, 600, image::Rgba([255, 0, 0, 255]));
        for y in 0..600 { for x in 600..1200 { image.put_pixel(x, y, image::Rgba([0, 0, 255, 255])); } }
        let out = prepare(image).unwrap(); assert_eq!(out.dimensions(), (960, 480));
        assert_eq!(out.get_pixel(10, 100).0, [0, 0, 255, 255]);
        let middle = out.get_pixel(480, 100).0; assert!(middle[0] > 50 && middle[2] > 50);
        assert!(out.pixels().all(|p| p[3] == 255));
    }
    #[test]
    fn wallpaper_backdrop_transparent_pixels_never_leave_fullscreen_clear() {
        let out = prepare(image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 0]))).unwrap();
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }
}
