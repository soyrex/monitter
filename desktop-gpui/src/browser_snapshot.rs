//! Ephemeral, bounded WebKit snapshots. AppKit objects stay on its main thread;
//! immutable CGImages may be retained there and rasterized on a worker.
use futures::channel::oneshot;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[derive(Clone, Default)]
pub(super) struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub(super) fn cancel(&self) { self.0.store(true, Ordering::Relaxed); }
    fn cancelled(&self) -> bool { self.0.load(Ordering::Relaxed) }
}

pub(super) struct Request {
    receiver: Option<oneshot::Receiver<Option<CapturedImage>>>,
    cancellation: Cancellation,
}
impl Request {
    pub(super) fn cancellation(&self) -> Cancellation { self.cancellation.clone() }
    pub(super) async fn receive(mut self) -> Option<CapturedImage> {
        self.receiver.take()?.await.ok().flatten()
    }
}
impl Drop for Request {
    fn drop(&mut self) { self.cancellation.cancel(); }
}

#[cfg(target_os = "macos")]
pub(super) struct CapturedImage(*const std::ffi::c_void);
// CGImage is immutable and Core Graphics permits its use across threads. Only
// the retained CGImage crosses threads, never the AppKit NSImage/WKWebView.
#[cfg(target_os = "macos")]
unsafe impl Send for CapturedImage {}
#[cfg(target_os = "macos")]
impl Drop for CapturedImage {
    fn drop(&mut self) { unsafe { native::CGImageRelease(self.0); } }
}
#[cfg(not(target_os = "macos"))]
pub(super) struct CapturedImage;

pub(super) fn request(view: &wry::WebView) -> Request { request_region(view, false) }
pub(super) fn request_top_edge(view: &wry::WebView) -> Request { request_region(view, true) }

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;
    use objc2_foundation::NSRect;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        pub fn CGImageRetain(image: *const c_void) -> *const c_void;
        pub fn CGImageRelease(image: *const c_void);
        pub fn CGImageGetWidth(image: *const c_void) -> usize;
        pub fn CGImageGetHeight(image: *const c_void) -> usize;
        pub fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
        pub fn CGColorSpaceRelease(space: *const c_void);
        pub fn CGBitmapContextCreate(data: *mut c_void, width: usize, height: usize,
            bits: usize, stride: usize, space: *const c_void, bitmap_info: u32) -> *mut c_void;
        pub fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *const c_void);
        pub fn CGContextRelease(context: *mut c_void);
    }
}

#[cfg(target_os = "macos")]
fn request_region(view: &wry::WebView, top_edge: bool) -> Request {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use wry::WebViewExtMacOS;
    let (sender, receiver) = oneshot::channel();
    let sender = std::cell::RefCell::new(Some(sender));
    let cancellation = Cancellation::default();
    let cancelled = cancellation.clone();
    let callback = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        let Some(sender) = sender.borrow_mut().take() else { return; };
        // A hidden/dismissed pane drops its request before any conversion.
        if cancelled.cancelled() || sender.is_canceled() { return; }
        let capture = unsafe {
            if image.is_null() { None } else {
                let cg_image: *const std::ffi::c_void = msg_send![image,
                    CGImageForProposedRect: std::ptr::null_mut::<NSRect>(),
                    context: std::ptr::null::<AnyObject>(),
                    hints: std::ptr::null::<AnyObject>()];
                if cg_image.is_null() { None } else {
                    let width = native::CGImageGetWidth(cg_image);
                    let height = native::CGImageGetHeight(cg_image);
                    // Retina backing may double snapshotWidth. Bound both
                    // dimensions and allocation even for very tall viewports.
                    if width == 0 || height == 0 || width > 2048 || height > 2048
                        || width.saturating_mul(height) > 4 * 1024 * 1024 { None }
                    else { Some(CapturedImage(native::CGImageRetain(cg_image))) }
                }
            }
        };
        let _ = sender.send(capture);
    });
    unsafe {
        let native = view.webview();
        let bounds: NSRect = msg_send![&*native, bounds];
        if !bounds.size.width.is_finite() || !bounds.size.height.is_finite()
            || bounds.size.width <= 16. || bounds.size.height < 3. { return Request { receiver: Some(receiver), cancellation }; }
        let configuration: Retained<AnyObject> = msg_send![class!(WKSnapshotConfiguration), new];
        let rect = if top_edge {
            NSRect::new(NSPoint::new(8., 0.), NSSize::new(bounds.size.width - 16., 3.))
        } else { bounds };
        let _: () = msg_send![&*configuration, setRect: rect];
        // Limit the longest edge in points; WebKit maintains the aspect ratio.
        let width = if top_edge { rect.size.width.min(512.) }
            else { rect.size.width * (1024. / rect.size.width.max(rect.size.height)).min(1.) };
        let number: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: width];
        let _: () = msg_send![&*configuration, setSnapshotWidth: &*number];
        let _: () = msg_send![&*native, takeSnapshotWithConfiguration: &*configuration, completionHandler: &*callback];
    }
    Request { receiver: Some(receiver), cancellation }
}

#[cfg(not(target_os = "macos"))]
fn request_region(_: &wry::WebView, _: bool) -> Request {
    let cancellation = Cancellation::default();
    let (sender, receiver) = oneshot::channel();
    let _ = sender.send(None);
    Request { receiver: Some(receiver), cancellation }
}

/// Run on the background executor. No AppKit access, TIFF encoding or decoding.
#[cfg(target_os = "macos")]
pub(super) fn decode(capture: CapturedImage) -> Option<image::RgbaImage> {
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    unsafe {
        let width = native::CGImageGetWidth(capture.0);
        let height = native::CGImageGetHeight(capture.0);
        if width == 0 || height == 0 || width > 2048 || height > 2048
            || width.saturating_mul(height) > 4 * 1024 * 1024 { return None; }
        let mut pixels = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
        let space = native::CGColorSpaceCreateDeviceRGB();
        if space.is_null() { return None; }
        // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big -> RGBA.
        let context = native::CGBitmapContextCreate(pixels.as_mut_ptr().cast(), width,
            height, 8, width * 4, space, 1 | (4 << 12));
        native::CGColorSpaceRelease(space);
        if context.is_null() { return None; }
        // Both source and destination use Core Graphics' default, unflipped
        // image coordinates. A UIKit/AppKit view transform here would invert
        // the top-to-bottom bitmap rows expected by image::RgbaImage/GPUI.
        native::CGContextDrawImage(context,
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(width as f64, height as f64)), capture.0);
        native::CGContextRelease(context);
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..3] { *channel = ((u32::from(*channel) * 255 / alpha).min(255)) as u8; }
            }
        }
        image::RgbaImage::from_raw(width as u32, height as u32, pixels)
    }
}
#[cfg(not(target_os = "macos"))]
pub(super) fn decode(_: CapturedImage) -> Option<image::RgbaImage> { None }

/// Prefer the dominant opaque colour over text, shadows, or rounded corners.
/// Quantization groups antialiasing noise; return the actual average in that bin.
pub(super) fn dominant_colour(image: &image::RgbaImage) -> Option<u32> {
    let mut bins = std::collections::HashMap::<u16, (u64, [u64; 3])>::new();
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 240 { continue; }
        let key = ((r as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (b as u16 >> 3);
        let bin = bins.entry(key).or_default();
        bin.0 += 1;
        for (sum, value) in bin.1.iter_mut().zip([r, g, b]) { *sum += u64::from(value); }
    }
    let (count, sums) = bins.into_iter().max_by_key(|(key, (count, _))| (*count, *key))?.1;
    Some(((sums[0] / count) as u32) << 16 | ((sums[1] / count) as u32) << 8 | (sums[2] / count) as u32)
}
