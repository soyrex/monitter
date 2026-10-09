//! Device-local wallpaper accent sampling. Native URL lookup stays on AppKit's
//! thread; file metadata, ImageIO decoding, and colour analysis run in a worker.
use gpui_kit::{Context, Window};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accents {
    pub highlight: u32,
    pub mood: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    display: u64,
    path: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    identity: Identity,
    modified: Option<SystemTime>,
    bytes: u64,
}

pub struct WallpaperAccent {
    accents: Option<Accents>,
    identity: Option<Identity>,
    fingerprint: Option<Fingerprint>,
    generation: u64,
    loading: bool,
    last_check: Option<Instant>,
    checked_display: Option<u64>,
}
impl WallpaperAccent {
    pub fn new() -> Self {
        Self {
            accents: None,
            identity: None,
            fingerprint: None,
            generation: 0,
            loading: false,
            last_check: None,
            checked_display: None,
        }
    }
    pub fn accents(&self) -> Option<Accents> {
        self.accents
    }
    pub fn refresh(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(display) = window.display(cx).map(|d| u64::from(d.id())) else {
            return;
        };
        // Moving/resizing sends many native bounds events. Avoid querying
        // NSWorkspace or touching files on every such event.
        if self.checked_display == Some(display)
            && self
                .last_check
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.last_check = Some(Instant::now());
        self.checked_display = Some(display);
        let Some(path) = wallpaper_path(display) else {
            self.generation = self.generation.wrapping_add(1);
            self.loading = false;
            self.identity = None;
            self.fingerprint = None;
            if self.accents.take().is_some() {
                cx.notify();
            }
            return;
        };
        let identity = Identity { display, path };
        if self.loading && self.identity.as_ref() == Some(&identity) {
            return;
        }
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity.clone());
            self.fingerprint = None;
            if self.accents.take().is_some() {
                cx.notify();
            }
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let previous = self.fingerprint.clone();
        self.loading = true;
        let worker = cx.background_executor().spawn(async move {
            let metadata = std::fs::metadata(&identity.path).map_err(|e| e.to_string())?;
            let fingerprint = Fingerprint {
                identity,
                modified: metadata.modified().ok(),
                bytes: metadata.len(),
            };
            if previous.as_ref() == Some(&fingerprint) {
                return Ok::<_, String>((fingerprint, None));
            }
            let rgba = thumbnail(&fingerprint.identity.path)?;
            Ok((fingerprint, Some(extract(&rgba))))
        });
        cx.spawn(async move |entity, cx| {
            let result = worker.await;
            let _ = entity.update(cx, |state, cx| {
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match result {
                    Ok((fingerprint, sample)) => {
                        state.fingerprint = Some(fingerprint);
                        if let Some(sample) = sample {
                            if state.accents != Some(sample) {
                                state.accents = Some(sample);
                                cx.notify();
                            }
                        }
                    }
                    Err(_) => {
                        state.fingerprint = None;
                        if state.accents.take().is_some() {
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }
}

const BRAND_BLUE: u32 = 0x00aaf0;
fn hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta < 0.0001 {
        0.
    } else if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    (hue, if max > 0. { delta / max } else { 0. }, max)
}
fn rgb(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let x = c * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let channel = |n: f32| ((n + m) * 255.).round().clamp(0., 255.) as u32;
    (channel(r) << 16) | (channel(g) << 8) | channel(b)
}
/// Quantized populations reject isolated specks while favouring a saturated
/// representative highlight over broad black/white/grey wallpaper areas.
fn extract(rgba: &[u8]) -> Accents {
    let mut bins = vec![(0_f32, 0_f32, 0_f32, 0_f32); 4096];
    let mut population = 0_f32;
    let mut warmth = 0_f32;
    let mut chroma_weight = 0_f32;
    for p in rgba.chunks_exact(4) {
        let alpha = p[3] as f32 / 255.;
        if alpha < 0.1 {
            continue;
        }
        // ImageIO's bitmap context returns premultiplied RGBA.
        let r = (p[0] as f32 / 255. / alpha).min(1.);
        let g = (p[1] as f32 / 255. / alpha).min(1.);
        let b = (p[2] as f32 / 255. / alpha).min(1.);
        let (_, sat, value) = hsv(r, g, b);
        population += alpha;
        if sat < 0.12 || value < 0.10 {
            continue;
        }
        let weight = alpha * sat * value;
        warmth += (r - b) * weight;
        chroma_weight += weight;
        let key = (((r * 15.).round() as usize) << 8)
            | (((g * 15.).round() as usize) << 4)
            | ((b * 15.).round() as usize);
        let bin = &mut bins[key];
        bin.0 += alpha;
        bin.1 += r * alpha;
        bin.2 += g * alpha;
        bin.3 += b * alpha;
    }
    let mut best = None;
    let mut score = 0_f32;
    for (count, r, g, b) in bins {
        if count < (population * 0.01).max(2.) {
            continue;
        }
        let (h, s, v) = hsv(r / count, g / count, b / count);
        let candidate = count.sqrt() * s * (1. - (v - 0.65).abs() * 0.7);
        if candidate > score {
            score = candidate;
            best = Some((h, s, v));
        }
    }
    let Some((h, s, v)) = best else {
        return Accents {
            highlight: BRAND_BLUE,
            mood: BRAND_BLUE,
        };
    };
    let warm = if chroma_weight > 0. {
        warmth / chroma_weight
    } else {
        0.
    };
    Accents {
        highlight: rgb(h, s.clamp(0.45, 0.90), v.clamp(0.72, 0.95)),
        mood: rgb(197.5 + warm.clamp(-1., 1.) * 18., 0.82, 0.92),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn wallpaper_path(display: u64) -> Option<PathBuf> {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSString;
    unsafe {
        let class = AnyClass::get(c"NSScreen")?;
        let screens: *mut AnyObject = msg_send![class, screens];
        let count: usize = msg_send![screens, count];
        let key = NSString::from_str("NSScreenNumber");
        for index in 0..count {
            let screen: *mut AnyObject = msg_send![screens,objectAtIndex:index];
            let description: *mut AnyObject = msg_send![screen, deviceDescription];
            let number: *mut AnyObject = msg_send![description,objectForKey:&*key];
            let id: u32 = msg_send![number, unsignedIntValue];
            if id as u64 != display {
                continue;
            }
            let class = AnyClass::get(c"NSWorkspace")?;
            let workspace: *mut AnyObject = msg_send![class, sharedWorkspace];
            let url: *mut AnyObject = msg_send![workspace,desktopImageURLForScreen:screen];
            if url.is_null() {
                return None;
            }
            let path: *const std::ffi::c_char = msg_send![url, fileSystemRepresentation];
            if path.is_null() {
                return None;
            }
            use std::os::unix::ffi::OsStrExt;
            return Some(PathBuf::from(std::ffi::OsStr::from_bytes(
                std::ffi::CStr::from_ptr(path).to_bytes(),
            )));
        }
    }
    None
}
#[cfg(not(target_os = "macos"))]
fn wallpaper_path(_: u64) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn thumbnail(path: &std::path::Path) -> Result<Vec<u8>, String> {
    thumbnail_rgba(path, 64).map(image::RgbaImage::into_raw)
}

#[cfg(target_os = "macos")]
pub(crate) fn thumbnail_rgba(path: &std::path::Path, max_edge: u32) -> Result<image::RgbaImage, String> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    type Ref = *const c_void;
    #[repr(C)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: Ref,
            bytes: *const u8,
            len: isize,
            is_directory: u8,
        ) -> Ref;
        fn CFNumberCreate(allocator: Ref, kind: isize, value: *const c_void) -> Ref;
        fn CFDictionaryCreate(
            allocator: Ref,
            keys: *const Ref,
            values: *const Ref,
            len: isize,
            key_callbacks: Ref,
            value_callbacks: Ref,
        ) -> Ref;
        fn CFRelease(value: Ref);
        static kCFBooleanTrue: Ref;
    }
    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageSourceCreateWithURL(url: Ref, options: Ref) -> Ref;
        fn CGImageSourceCreateThumbnailAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
        static kCGImageSourceCreateThumbnailFromImageAlways: Ref;
        static kCGImageSourceThumbnailMaxPixelSize: Ref;
        static kCGImageSourceCreateThumbnailWithTransform: Ref;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> Ref;
        fn CGImageGetWidth(image: Ref) -> usize;
        fn CGImageGetHeight(image: Ref) -> usize;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits: usize,
            row_bytes: usize,
            space: Ref,
            bitmap_info: u32,
        ) -> Ref;
        fn CGContextDrawImage(context: Ref, rect: Rect, image: Ref);
    }
    struct Owned(Ref);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }
    fn own(value: Ref, what: &str) -> Result<Owned, String> {
        if value.is_null() {
            Err(format!("Wallpaper {what} unavailable"))
        } else {
            Ok(Owned(value))
        }
    }
    unsafe {
        let bytes = path.as_os_str().as_bytes();
        let url = own(
            CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                0,
            ),
            "URL",
        )?;
        let source = own(CGImageSourceCreateWithURL(url.0, std::ptr::null()), "image")?;
        let limit = max_edge.clamp(1, 1280) as i32;
        let number = own(
            CFNumberCreate(std::ptr::null(), 9, (&limit as *const i32).cast()),
            "thumbnail size",
        )?;
        let keys = [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceThumbnailMaxPixelSize,
            kCGImageSourceCreateThumbnailWithTransform,
        ];
        let values = [kCFBooleanTrue, number.0, kCFBooleanTrue];
        let options = own(
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                3,
                std::ptr::null(),
                std::ptr::null(),
            ),
            "thumbnail options",
        )?;
        let image = own(
            CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.0),
            "thumbnail",
        )?;
        let width = CGImageGetWidth(image.0);
        let height = CGImageGetHeight(image.0);
        if width == 0 || height == 0 || width > limit as usize || height > limit as usize {
            return Err("Wallpaper thumbnail dimensions invalid".into());
        }
        let bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= 1280 * 1280 * 4).ok_or("Wallpaper thumbnail exceeds budget")?;
        let mut rgba = vec![0_u8; bytes];
        let space = own(CGColorSpaceCreateDeviceRGB(), "colour space")?;
        let context = own(
            CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                space.0,
                1 | (4 << 12),
            ),
            "bitmap",
        )?;
        CGContextDrawImage(
            context.0,
            Rect {
                origin: Point { x: 0., y: 0. },
                size: Size {
                    width: width as f64,
                    height: height as f64,
                },
            },
            image.0,
        );
        // Bitmap context stores premultiplied RGBA; image processing uses straight RGBA.
        for pixel in rgba.chunks_exact_mut(4) {
            if pixel[3] > 0 && pixel[3] < 255 {
                for c in 0..3 { pixel[c] = ((pixel[c] as u32 * 255) / pixel[3] as u32).min(255) as u8; }
            }
        }
        image::RgbaImage::from_raw(width as u32, height as u32, rgba).ok_or("Invalid wallpaper bitmap".into())
    }
}
#[cfg(not(target_os = "macos"))]
fn thumbnail(_: &std::path::Path) -> Result<Vec<u8>, String> {
    Err("Wallpaper sampling requires macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(parts: &[([u8; 4], usize)]) -> Vec<u8> {
        parts
            .iter()
            .flat_map(|(p, n)| std::iter::repeat_n(*p, *n).flatten())
            .collect()
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn wallpaper_native_thumbnail_decodes_and_downsamples() {
        let path = std::env::temp_dir().join(format!("monitter-wallpaper-test-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(256, 128, image::Rgba([220, 95, 30, 255])).save(&path).unwrap();
        let result = thumbnail(&path);
        std::fs::remove_file(&path).unwrap();
        let rgba = result.unwrap();
        assert!(rgba.len() <= 64 * 64 * 4);
        assert!(rgba.len() >= 32 * 32 * 4);
        let accent = extract(&rgba).highlight;
        assert!((accent >> 16) > ((accent >> 8) & 255));
        assert!(((accent >> 8) & 255) > (accent & 255));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn wallpaper_backdrop_native_decode_preserves_orientation_and_budget() {
        let path = std::env::temp_dir().join(format!("monitter-wallpaper-orientation-{}.png", std::process::id()));
        let mut input = image::RgbaImage::from_pixel(128, 96, image::Rgba([255, 0, 0, 255]));
        for y in 48..96 { for x in 0..128 { input.put_pixel(x, y, image::Rgba([0, 0, 255, 255])); } }
        input.save(&path).unwrap();
        let out = thumbnail_rgba(&path, 32).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(out.dimensions(), (32, 24));
        assert!(out.get_pixel(16, 2)[0] > 240);
        assert!(out.get_pixel(16, 21)[2] > 240);
    }
    #[test]
    fn wallpaper_neutral_and_transparent_fall_back_to_brand() {
        assert_eq!(
            extract(&pixels(&[
                ([230, 230, 230, 255], 100),
                ([255, 0, 0, 0], 20)
            ])),
            Accents {
                highlight: BRAND_BLUE,
                mood: BRAND_BLUE
            }
        );
    }
    #[test]
    fn wallpaper_highlight_prefers_representative_chroma_over_grey_and_specks() {
        let sample = extract(&pixels(&[
            ([180, 180, 180, 255], 900),
            ([220, 95, 30, 255], 100),
            ([255, 0, 255, 255], 1),
        ]));
        let (h, s, v) = hsv(
            ((sample.highlight >> 16) & 255) as f32 / 255.,
            ((sample.highlight >> 8) & 255) as f32 / 255.,
            (sample.highlight & 255) as f32 / 255.,
        );
        assert!((10. ..35.).contains(&h));
        assert!(s >= 0.45);
        assert!((0.71..0.96).contains(&v));
    }
    #[test]
    fn wallpaper_mood_stays_blue_and_distinguishes_warm_and_cool() {
        let warm = extract(&pixels(&[([220, 70, 30, 255], 100)]));
        let cool = extract(&pixels(&[([30, 70, 220, 255], 100)]));
        let hue = |c: u32| {
            hsv(
                ((c >> 16) & 255) as f32 / 255.,
                ((c >> 8) & 255) as f32 / 255.,
                (c & 255) as f32 / 255.,
            )
            .0
        };
        assert!(hue(warm.mood) > hue(cool.mood));
        assert!((178. ..218.).contains(&hue(warm.mood)));
        assert!((178. ..218.).contains(&hue(cool.mood)));
        assert_eq!(warm, extract(&pixels(&[([220, 70, 30, 255], 100)])));
    }
}
