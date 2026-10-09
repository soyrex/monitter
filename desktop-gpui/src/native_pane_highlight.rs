//! A noninteractive AppKit outline above GPUI and native browser child views.
//! The caller clears it once per frame, then paints the active tab-group bounds.
use gpui_kit::{Bounds, Pixels, Point, Window};

#[cfg(not(target_os = "macos"))]
pub fn clear(_: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn paint(_: Bounds<Pixels>, _: u32, _: &[Point<Pixels>], _: &Window) {}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::{Allocated, Retained, Weak};
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr};

    // CGColorRef is a pointer to the opaque CoreGraphics CGColor struct.
    // Preserve its Objective-C encoding rather than passing a generic void
    // pointer, which debug selector checks would reject despite identical ABI.
    #[repr(C)]
    struct NativeCGColor {
        _opaque: [u8; 0],
    }
    unsafe impl objc2::Encode for NativeCGColor {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct("CGColor", &[]);
    }
    unsafe impl objc2::RefEncode for NativeCGColor {
        const ENCODING_REF: objc2::Encoding =
            objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
    }

    #[repr(C)]
    struct NativeCGPath {
        _opaque: [u8; 0],
    }
    unsafe impl objc2::Encode for NativeCGPath {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct("CGPath", &[]);
    }
    unsafe impl objc2::RefEncode for NativeCGPath {
        const ENCODING_REF: objc2::Encoding =
            objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut NativeCGPath;
        fn CGPathMoveToPoint(path: *mut NativeCGPath, transform: *const c_void, x: f64, y: f64);
        fn CGPathAddLineToPoint(path: *mut NativeCGPath, transform: *const c_void, x: f64, y: f64);
        fn CGPathCloseSubpath(path: *mut NativeCGPath);
        fn CGPathRelease(path: *const NativeCGPath);
    }

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = ()]
        struct PaneHighlightView;

        impl PaneHighlightView {
            #[unsafe(method_id(initWithFrame:))]
            fn init(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
                let this = this.set_ivars(());
                unsafe { msg_send![super(this), initWithFrame: frame] }
            }
            // AppKit must continue searching siblings, including WKWebView.
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _: NSPoint) -> *mut NSView { ptr::null_mut() }
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool { false }
            #[unsafe(method(isOpaque))]
            fn is_opaque(&self) -> bool { false }
            #[unsafe(method(isFlipped))]
            fn is_flipped(&self) -> bool { true }
            #[unsafe(method(isAccessibilityElement))]
            fn accessibility_element(&self) -> bool { false }
        }
    );

    struct Overlay {
        view: Weak<PaneHighlightView>,
        bounds: Option<Bounds<Pixels>>,
        color: Option<u32>,
        contour: Vec<Point<Pixels>>,
    }
    thread_local! {
        static OVERLAYS: RefCell<HashMap<usize, Overlay>> = RefCell::new(HashMap::new());
    }

    fn host(window: &Window) -> Option<*mut NSView> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr().cast()),
            _ => None,
        }
    }

    pub fn clear(window: &Window) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Some(host) = host(window) else {
            return;
        };
        OVERLAYS.with(|state| {
            let mut state = state.borrow_mut();
            // The view does not retain its parent. Release detached overlays
            // left behind by closed windows on the next live frame.
            state.retain(|_, overlay| overlay.view.load().is_some());
            if let Some(view) = state
                .get(&(host as usize))
                .and_then(|overlay| overlay.view.load())
            {
                unsafe {
                    let _: () = msg_send![class!(CATransaction), begin];
                    let _: () = msg_send![class!(CATransaction), setDisableActions: true];
                    view.setHidden(true);
                    let _: () = msg_send![class!(CATransaction), commit];
                }
            }
        });
    }

    pub fn paint(bounds: Bounds<Pixels>, color: u32, contour: &[Point<Pixels>], window: &Window) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(host_ptr) = host(window) else {
            return;
        };
        let width: f64 = bounds.size.width.into();
        let height: f64 = bounds.size.height.into();
        if width <= 0. || height <= 0. || contour.len() < 3 {
            return;
        }
        // WindowHandle borrows this live GPUIView for the duration of the call.
        let host = unsafe { &*host_ptr };
        let x: f64 = bounds.origin.x.into();
        let top: f64 = bounds.origin.y.into();
        let y = if host.isFlipped() {
            top
        } else {
            host.bounds().size.height - top - height
        };
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        OVERLAYS.with(|state| {
            let mut state = state.borrow_mut();
            state.retain(|_, overlay| overlay.view.load().is_some());
            let overlay = state.entry(host_ptr as usize).or_insert_with(|| {
                let view: Retained<PaneHighlightView> =
                    unsafe { msg_send![PaneHighlightView::alloc(mtm), initWithFrame: frame] };
                view.setWantsLayer(true);
                view.setHidden(true);
                unsafe {
                    let shape: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
                    let _: () = msg_send![&*view, setLayer: shape];
                    let _: () = msg_send![shape, setFillColor: ptr::null::<NativeCGColor>()];
                    let _: () = msg_send![shape, setLineWidth: 1.0f64];
                    let _: () = msg_send![shape, setGeometryFlipped: true];
                }
                // The native parent owns the overlay. The registry is weak, so
                // destroying the host releases the overlay without a callback.
                unsafe {
                    let _: () = msg_send![host, addSubview: &*view
                        positioned: 1isize relativeTo: ptr::null::<NSView>()];
                }
                Overlay {
                    view: Weak::from_retained(&view),
                    bounds: None,
                    color: None,
                    contour: Vec::new(),
                }
            });
            let Some(view) = overlay.view.load() else {
                return;
            };
            unsafe {
                // QuartzCore is already linked by GPUI. Dynamic selectors keep
                // this helper within the existing AppKit/objc2 dependencies.
                let _: () = msg_send![class!(CATransaction), begin];
                let _: () = msg_send![class!(CATransaction), setDisableActions: true];
                let bounds_changed = overlay.bounds != Some(bounds);
                if bounds_changed {
                    view.setFrame(frame);
                    overlay.bounds = Some(bounds);
                }
                let layer: *mut AnyObject = msg_send![&*view, layer];
                if !layer.is_null() {
                    let _: () =
                        msg_send![layer, setContentsScale: f64::from(window.scale_factor())];
                    if bounds_changed || overlay.contour != contour {
                        let path = CGPathCreateMutable();
                        if !path.is_null() {
                            for (index, point) in contour.iter().enumerate() {
                                let point_x: f64 = point.x.into();
                                let point_y: f64 = point.y.into();
                                if index == 0 {
                                    CGPathMoveToPoint(
                                        path,
                                        ptr::null(),
                                        point_x - x,
                                        point_y - top,
                                    );
                                } else {
                                    CGPathAddLineToPoint(
                                        path,
                                        ptr::null(),
                                        point_x - x,
                                        point_y - top,
                                    );
                                }
                            }
                            CGPathCloseSubpath(path);
                            // CAShapeLayer copies the supplied CGPath; release
                            // our creation reference after assigning the path.
                            let _: () = msg_send![layer, setPath: path.cast_const()];
                            CGPathRelease(path);
                            overlay.contour.clear();
                            overlay.contour.extend_from_slice(contour);
                        }
                    }
                    if overlay.color != Some(color) {
                        let red = f64::from((color >> 16) & 255) / 255.;
                        let green = f64::from((color >> 8) & 255) / 255.;
                        let blue = f64::from(color & 255) / 255.;
                        let native_color: *mut AnyObject = msg_send![class!(NSColor),
                            colorWithSRGBRed: red green: green blue: blue alpha: 1.0f64];
                        let cg_color: *const NativeCGColor = msg_send![native_color, CGColor];
                        let _: () = msg_send![layer, setStrokeColor: cg_color];
                        overlay.color = Some(color);
                    }
                }
                // Above every sibling, including browser views created later.
                // Parent supplies the inset stroke contour: no layout gap.
                let subviews: *mut AnyObject = msg_send![host, subviews];
                let last: *mut AnyObject = msg_send![subviews, lastObject];
                if last.cast_const() != Retained::as_ptr(&view).cast::<AnyObject>() {
                    let _: () = msg_send![host, addSubview: &*view
                        positioned: 1isize relativeTo: ptr::null::<NSView>()];
                }
                view.setHidden(false);
                let _: () = msg_send![class!(CATransaction), commit];
            }
        });
    }
}
#[cfg(target_os = "macos")]
pub use macos::{clear, paint};
