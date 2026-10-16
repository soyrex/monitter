//! A noninteractive AppKit outline and outward glow above GPUI/browser child views.
//! Active-pane painting keeps the view visible; a frame finalizer hides it only
//! after a rendered workspace observes that no active pane owns it.
use gpui_kit::{Bounds, Pixels, Point, Rgba, Window};
#[cfg(any(test, feature = "performance-profiling"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[path = "pane_highlight_state.rs"]
mod pane_highlight_state;
#[cfg(target_os = "macos")]
use pane_highlight_state::OverlayFrameState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HighlightPerformanceCounts {
    pub shows: u64,
    pub hides: u64,
    pub geometry_paints: u64,
}

#[cfg(any(test, feature = "performance-profiling"))]
static SHOWS: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "performance-profiling"))]
static HIDES: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "performance-profiling"))]
static GEOMETRY_PAINTS: AtomicU64 = AtomicU64::new(0);

#[cfg(any(test, feature = "performance-profiling"))]
pub(crate) fn performance_counts() -> HighlightPerformanceCounts {
    HighlightPerformanceCounts {
        shows: SHOWS.load(Ordering::Relaxed),
        hides: HIDES.load(Ordering::Relaxed),
        geometry_paints: GEOMETRY_PAINTS.load(Ordering::Relaxed),
    }
}

#[inline]
fn record_show() {
    #[cfg(any(test, feature = "performance-profiling"))]
    SHOWS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn record_hide() {
    #[cfg(any(test, feature = "performance-profiling"))]
    HIDES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn record_geometry_paint() {
    #[cfg(any(test, feature = "performance-profiling"))]
    GEOMETRY_PAINTS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn begin_frame(_: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn observe_candidate(_: &str, _: bool, _: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn finish_frame(_: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn clear(_: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn retain_panes(_: &[String], _: &Window) {}
#[cfg(not(target_os = "macos"))]
pub fn paint(
    _: &str,
    _: Bounds<Pixels>,
    _: Rgba,
    _: Rgba,
    _: &[Point<Pixels>],
    _: &Window,
) -> bool {
    false
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::{Allocated, Retained, Weak};
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};
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
        fn CGPathAddRect(path: *mut NativeCGPath, transform: *const c_void, rect: NSRect);
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
        color: Option<Rgba>,
        glow_color: Option<Rgba>,
        pulsing: bool,
        contour: Vec<Point<Pixels>>,
        glow_layers: Vec<Retained<AnyObject>>,
        glow_mask: Retained<AnyObject>,
        glow_container: Retained<AnyObject>,
        frame: OverlayFrameState,
    }
    thread_local! {
        static OVERLAYS: RefCell<HashMap<(usize, String), Overlay>> = RefCell::new(HashMap::new());
    }

    fn host(window: &Window) -> Option<*mut NSView> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr().cast()),
            _ => None,
        }
    }

    /// Reconcile live dock pane IDs at a topology boundary. Cached pane render
    /// replays remain visible; closed or switched-away panes cannot leave ghosts.
    pub fn retain_panes(pane_ids: &[String], window: &Window) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Some(host) = host(window) else {
            return;
        };
        OVERLAYS.with(|state| {
            state.borrow_mut().retain(|(owner, id), overlay| {
                let Some(view) = overlay.view.load() else {
                    return false;
                };
                if *owner != host as usize || pane_ids.contains(id) {
                    return true;
                }
                if overlay.frame.clear() {
                    record_hide();
                }
                unsafe {
                    let _: () = msg_send![&*view, removeFromSuperview];
                }
                false
            })
        });
    }

    pub fn begin_frame(window: &Window) {
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
            for ((owner, _), overlay) in state.iter_mut() {
                if *owner == host as usize {
                    overlay.frame.begin_frame();
                }
            }
        });
    }

    /// Records whether a native pane in a rendered DockArea should own the
    /// active outline. Called during render, before GPUI's deferred paint pass.
    /// A candidate survives a replayed deferred Canvas even when its callback
    /// is not invoked on that frame.
    pub fn observe_candidate(pane_id: &str, candidate: bool, window: &Window) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Some(host) = host(window) else {
            return;
        };
        OVERLAYS.with(|state| {
            if let Some(overlay) = state
                .borrow_mut()
                .get_mut(&(host as usize, pane_id.to_owned()))
            {
                overlay.frame.observe_candidate(candidate);
            }
        });
    }

    /// Hides an existing outline only when a rendered workspace observed no
    /// active pane during this frame. The caller runs after deferred pane
    /// overlays; an unobserved cached replay preserves the current view.
    pub fn finish_frame(window: &Window) {
        update_window_visibility(window, false);
    }

    /// Hide every pane overlay for lock and other immediate state transitions.
    pub fn clear(window: &Window) {
        update_window_visibility(window, true);
    }

    fn update_window_visibility(window: &Window, clear: bool) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Some(host) = host(window) else {
            return;
        };
        OVERLAYS.with(|state| {
            let mut state = state.borrow_mut();
            state.retain(|_, overlay| overlay.view.load().is_some());
            for ((owner, _), overlay) in state.iter_mut() {
                if *owner != host as usize { continue; }
                let hide = if clear { overlay.frame.clear() } else { overlay.frame.finish_frame() };
                if !hide { continue; }
                let Some(view) = overlay.view.load() else { continue; };
                unsafe {
                    let _: () = msg_send![class!(CATransaction), begin];
                    let _: () = msg_send![class!(CATransaction), setDisableActions: true];
                    let _: () = msg_send![&*overlay.glow_container, removeAnimationForKey: &*NSString::from_str("monitter-activity-glow-pulse")];
                    overlay.pulsing = false;
                    view.setHidden(true);
                    let _: () = msg_send![class!(CATransaction), commit];
                }
                record_hide();
            }
        });
    }

    pub fn paint(
        pane_id: &str,
        bounds: Bounds<Pixels>,
        color: Rgba,
        glow_color: Rgba,
        contour: &[Point<Pixels>],
        window: &Window,
    ) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Some(host_ptr) = host(window) else {
            return false;
        };
        let width: f64 = bounds.size.width.into();
        let height: f64 = bounds.size.height.into();
        if width <= 0. || height <= 0. || contour.len() < 3 {
            return false;
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
        const GLOW_SPREAD: f64 = 20.;
        let frame = NSRect::new(
            NSPoint::new(x - GLOW_SPREAD, y - GLOW_SPREAD),
            NSSize::new(width + GLOW_SPREAD * 2., height + GLOW_SPREAD * 2.),
        );
        OVERLAYS.with(|state| {
            let mut state = state.borrow_mut();
            state.retain(|_, overlay| overlay.view.load().is_some());
            let overlay = state.entry((host_ptr as usize, pane_id.to_owned())).or_insert_with(|| {
                let view: Retained<PaneHighlightView> =
                    unsafe { msg_send![PaneHighlightView::alloc(mtm), initWithFrame: frame] };
                view.setWantsLayer(true);
                view.setHidden(true);
                let mut glow_layers = Vec::with_capacity(20);
                let glow_mask;
                let glow_container;
                unsafe {
                    let shape: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
                    let _: () = msg_send![&*view, setLayer: shape];
                    let _: () = msg_send![shape, setFillColor: ptr::null::<NativeCGColor>()];
                    let _: () = msg_send![shape, setLineWidth: 1.0f64];
                    let _: () = msg_send![shape, setGeometryFlipped: true];
                    // The even-odd mask keeps the outside of the pane contour,
                    // allowing the halo to radiate outward into its 20px margin.
                    let glow: *mut AnyObject = msg_send![class!(CALayer), layer];
                    let mask: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
                    let white: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
                    let white_cg: *const NativeCGColor = msg_send![white, CGColor];
                    let _: () = msg_send![mask, setFillColor: white_cg];
                    let _: () = msg_send![glow, setMask: mask];
                    let _: () = msg_send![shape, addSublayer: glow];
                    glow_container = Retained::retain(glow).expect("new glow container");
                    glow_mask = Retained::retain(mask).expect("new glow mask");
                    for depth in (1..=20).rev() {
                        let band: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
                        let _: () = msg_send![band, setFillColor: ptr::null::<NativeCGColor>()];
                        let _: () = msg_send![band, setLineWidth: f64::from(depth * 2)];
                        let _: () = msg_send![glow, addSublayer: band];
                        glow_layers.push(Retained::retain(band).expect("new glow band"));
                    }
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
                    glow_color: None,
                    pulsing: false,
                    contour: Vec::new(),
                    glow_layers,
                    glow_mask,
                    glow_container,
                    frame: OverlayFrameState::default(),
                }
            });
            let Some(view) = overlay.view.load() else {
                return false;
            };
            let should_show = overlay.frame.mark_painted();
            record_geometry_paint();
            if should_show {
                record_show();
            }
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
                    if bounds_changed {
                        let local = NSRect::new(NSPoint::new(0., 0.), frame.size);
                        let _: () = msg_send![&*overlay.glow_container, setFrame: local];
                        let _: () = msg_send![&*overlay.glow_mask, setFrame: local];
                        for band in &overlay.glow_layers {
                            let _: () = msg_send![&**band, setFrame: local];
                        }
                    }
                    let _: () =
                        msg_send![layer, setContentsScale: f64::from(window.scale_factor())];
                    if bounds_changed || overlay.contour != contour {
                        let path = CGPathCreateMutable();
                        if !path.is_null() {
                            for (index, point) in contour.iter().enumerate() {
                                let point_x: f64 = point.x.into();
                                let point_y: f64 = point.y.into();
                                let local_x = point_x - x + GLOW_SPREAD;
                                let local_y = point_y - top + GLOW_SPREAD;
                                if index == 0 {
                                    CGPathMoveToPoint(
                                        path,
                                        ptr::null(),
                                        local_x,
                                        local_y,
                                    );
                                } else {
                                    CGPathAddLineToPoint(
                                        path,
                                        ptr::null(),
                                        local_x,
                                        local_y,
                                    );
                                }
                            }
                            CGPathCloseSubpath(path);
                            // CAShapeLayer copies the supplied CGPath; release
                            // our creation reference after assigning the path.
                            let _: () = msg_send![layer, setPath: path.cast_const()];
                            let mask_path = CGPathCreateMutable();
                            let outer = NSRect::new(NSPoint::new(0., 0.), frame.size);
                            CGPathAddRect(mask_path, ptr::null(), outer);
                            // Re-add pane contour as a hole so only its exterior glows.
                            for (index, point) in contour.iter().enumerate() {
                                let px: f64 = point.x.into();
                                let py: f64 = point.y.into();
                                let (px, py) = (px - x + GLOW_SPREAD, py - top + GLOW_SPREAD);
                                if index == 0 { CGPathMoveToPoint(mask_path, ptr::null(), px, py); }
                                else { CGPathAddLineToPoint(mask_path, ptr::null(), px, py); }
                            }
                            CGPathCloseSubpath(mask_path);
                            let even_odd = NSString::from_str("evenOdd");
                            let _: () = msg_send![&*overlay.glow_mask, setFillRule: &*even_odd];
                            let _: () = msg_send![&*overlay.glow_mask, setPath: mask_path.cast_const()];
                            CGPathRelease(mask_path);
                            for band in &overlay.glow_layers {
                                let _: () = msg_send![&**band, setPath: path.cast_const()];
                            }
                            CGPathRelease(path);
                            overlay.contour.clear();
                            overlay.contour.extend_from_slice(contour);
                        }
                    }
                    if overlay.color != Some(color) {
                        let native_color: *mut AnyObject = msg_send![class!(NSColor),
                            colorWithSRGBRed: f64::from(color.r)
                            green: f64::from(color.g) blue: f64::from(color.b)
                            alpha: f64::from(color.a)];
                        let cg_color: *const NativeCGColor = msg_send![native_color, CGColor];
                        let _: () = msg_send![layer, setStrokeColor: cg_color];
                        overlay.color = Some(color);
                    }
                    // Compositor-only breathing: a three-second round trip,
                    // with no GPUI timers or chat/content invalidation.
                    let pulse = glow_color.a > 0. && crate::native_motion::motion_allowed();
                    if overlay.pulsing != pulse {
                        let key = NSString::from_str("monitter-activity-glow-pulse");
                        let _: () = msg_send![&*overlay.glow_container, removeAnimationForKey: &*key];
                        if pulse {
                            let animation: *mut AnyObject = msg_send![class!(CABasicAnimation), animationWithKeyPath: &*NSString::from_str("opacity")];
                            let low = NSNumber::new_f32(0.65);
                            let high = NSNumber::new_f32(1.0);
                            let timing: *mut AnyObject = msg_send![class!(CAMediaTimingFunction), functionWithName: &*NSString::from_str("easeInEaseOut")];
                            let _: () = msg_send![animation, setFromValue: &*low];
                            let _: () = msg_send![animation, setToValue: &*high];
                            let _: () = msg_send![animation, setDuration: 1.5f64];
                            let _: () = msg_send![animation, setAutoreverses: true];
                            let _: () = msg_send![animation, setRepeatCount: f32::INFINITY];
                            let _: () = msg_send![animation, setTimingFunction: timing];
                            let _: () = msg_send![&*overlay.glow_container, addAnimation: animation forKey: &*key];
                        }
                        overlay.pulsing = pulse;
                    }
                    if overlay.glow_color != Some(glow_color) {
                        for (index, band) in overlay.glow_layers.iter().enumerate() {
                            // A quadratic falloff reaches transparency at 20px.
                            // Composite narrow bands over wider, fainter bands.
                            let depth = 20 - index;
                            let outer = (20 - depth) as f64 / 20.;
                            let inner = (21 - depth) as f64 / 20.;
                            let outer_alpha = 0.18 * outer * outer;
                            let inner_alpha = 0.18 * inner * inner;
                            let alpha = (inner_alpha - outer_alpha) / (1. - outer_alpha);
                            let tint: *mut AnyObject = msg_send![class!(NSColor),
                                colorWithSRGBRed: f64::from(glow_color.r)
                                green: f64::from(glow_color.g) blue: f64::from(glow_color.b)
                                alpha: f64::from(glow_color.a) * alpha];
                            let tint_cg: *const NativeCGColor = msg_send![tint, CGColor];
                            let _: () = msg_send![&**band, setStrokeColor: tint_cg];
                        }
                        overlay.glow_color = Some(glow_color);
                    }
                }
                // Above every sibling, including browser views created later.
                // The expanded frame reserves room for the outward halo.
                let subviews: *mut AnyObject = msg_send![host, subviews];
                let last: *mut AnyObject = msg_send![subviews, lastObject];
                if last.cast_const() != Retained::as_ptr(&view).cast::<AnyObject>() {
                    let _: () = msg_send![host, addSubview: &*view
                        positioned: 1isize relativeTo: ptr::null::<NSView>()];
                }
                if should_show {
                    view.setHidden(false);
                }
                let _: () = msg_send![class!(CATransaction), commit];
            }
            true
        })
    }
}
#[cfg(target_os = "macos")]
pub use macos::{begin_frame, clear, finish_frame, observe_candidate, paint, retain_panes};

#[cfg(all(test, feature = "visual-export"))]
#[path = "pane_highlight_tests.rs"]
mod regression_tests;
