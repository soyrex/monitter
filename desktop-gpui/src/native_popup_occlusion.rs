//! Cut popup-sized holes in native child surfaces and route those clicks to GPUI.
//! The rest of each browser remains live; closing a popup restores its layer mask.
use gpui_kit::{App, Window};

// Even-odd masks need disjoint holes: overlapping menus must not expose the
// native view again where two popup rectangles intersect.
fn disjoint_holes(rectangles: impl IntoIterator<Item = [f64; 4]>) -> Vec<[f64; 4]> {
    let mut holes: Vec<[f64; 4]> = Vec::new();
    for rectangle in rectangles {
        let mut pieces = vec![rectangle];
        for &[x, y, right, bottom] in &holes {
            pieces = pieces
                .into_iter()
                .flat_map(|[a, b, c, d]| {
                    let left = a.max(x);
                    let top = b.max(y);
                    let r = c.min(right);
                    let end = d.min(bottom);
                    if left >= r || top >= end {
                        return vec![[a, b, c, d]];
                    }
                    [
                        [a, b, c, top],
                        [a, end, c, d],
                        [a, top, left, end],
                        [r, top, c, end],
                    ]
                    .into_iter()
                    .filter(|v| v[0] < v[2] && v[1] < v[3])
                    .collect()
                })
                .collect();
        }
        holes.extend(pieces);
    }
    holes
}

#[cfg(not(target_os = "macos"))]
pub fn finish_frame(_: &mut Window, _: &mut App) {}
#[cfg(not(target_os = "macos"))]
pub fn clear(_: &Window) {}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::{Allocated, Retained, Weak};
    use objc2::runtime::AnyObject;
    use objc2::{
        ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    };
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr};

    #[repr(C)]
    struct CGPath {
        _opaque: [u8; 0],
    }
    unsafe impl objc2::Encode for CGPath {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct("CGPath", &[]);
    }
    unsafe impl objc2::RefEncode for CGPath {
        const ENCODING_REF: objc2::Encoding =
            objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut CGPath;
        fn CGPathAddRect(path: *mut CGPath, transform: *const c_void, rect: NSRect);
        fn CGPathRelease(path: *const CGPath);
    }

    #[derive(Default)]
    struct GuardState {
        rectangles: RefCell<Vec<NSRect>>,
    }
    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = GuardState]
        struct PopupInputGuard;
        impl PopupInputGuard {
            #[unsafe(method_id(initWithFrame:))]
            fn init(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
                let this = this.set_ivars(GuardState::default());
                unsafe { msg_send![super(this), initWithFrame: frame] }
            }
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, point: NSPoint) -> *mut NSView {
                if self.ivars().rectangles.borrow().iter().any(|rect| {
                    point.x >= rect.origin.x && point.x < rect.origin.x + rect.size.width
                        && point.y >= rect.origin.y && point.y < rect.origin.y + rect.size.height
                }) {
                    unsafe { self.superview() }.map_or(ptr::null_mut(), |view| Retained::as_ptr(&view).cast_mut())
                } else { ptr::null_mut() }
            }
            #[unsafe(method(isOpaque))]
            fn is_opaque(&self) -> bool { false }
            #[unsafe(method(isAccessibilityElement))]
            fn accessibility_element(&self) -> bool { false }
        }
    );

    struct MaskedView {
        view: Weak<NSView>,
        layer: Retained<AnyObject>,
        original: Option<Retained<AnyObject>>,
        mask: Retained<AnyObject>,
        geometry: Option<(NSRect, Vec<NSRect>)>,
    }
    impl MaskedView {
        fn restore(&self) {
            unsafe {
                let current: *mut AnyObject = msg_send![&*self.layer, mask];
                if current == Retained::as_ptr(&self.mask).cast_mut() {
                    let _: () = msg_send![&*self.layer, setMask: self.original.as_deref()];
                }
            }
        }
    }
    struct HostState {
        host: Weak<NSView>,
        guard: Weak<PopupInputGuard>,
        views: HashMap<usize, MaskedView>,
    }
    thread_local! { static HOSTS: RefCell<HashMap<usize, HostState>> = RefCell::new(HashMap::new()); }

    fn restore_host(state: HostState) {
        unsafe {
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            for view in state.views.values() {
                view.restore();
            }
            if let Some(guard) = state.guard.load() {
                guard.removeFromSuperview();
            }
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }

    pub fn clear(window: &Window) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        HOSTS.with(|registry| {
            if let Some(state) = registry
                .borrow_mut()
                .remove(&(handle.ns_view.as_ptr() as usize))
            {
                restore_host(state);
            }
        });
    }

    pub fn finish_frame(window: &mut Window, cx: &mut App) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        let host = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        let rectangles = gpui_kit::component::surface_backdrop::popup_bounds(window, cx);
        HOSTS.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.retain(|_, state| state.host.load().is_some());
            let key = host as *const NSView as usize;
            if rectangles.is_empty() {
                if let Some(state) = registry.remove(&key) {
                    restore_host(state);
                }
                return;
            }
            let state = registry.entry(key).or_insert_with(|| {
                let guard: Retained<PopupInputGuard> = unsafe {
                    msg_send![PopupInputGuard::alloc(mtm), initWithFrame: host.bounds()]
                };
                // A non-layer-backed child makes AppKit redraw a Metal host
                // through its display path. Keep the transparent input guard
                // in Core Animation, like the existing native pane outline.
                guard.setWantsLayer(true);
                unsafe {
                    let layer: *mut AnyObject = msg_send![class!(CALayer), layer];
                    let _: () = msg_send![&*guard, setLayer: layer];
                }
                host.addSubview(&guard);
                HostState { host: Weak::new(host), guard: Weak::from_retained(&guard), views: HashMap::new() }
            });
            let Some(guard) = state.guard.load() else { return; };
            if guard.frame() != host.bounds() { guard.setFrame(host.bounds()); }
            let host_rects: Vec<NSRect> = rectangles.iter().map(|bounds| {
                let x = f64::from(f32::from(bounds.origin.x));
                let top = f64::from(f32::from(bounds.origin.y));
                let width = f64::from(f32::from(bounds.size.width));
                let height = f64::from(f32::from(bounds.size.height));
                NSRect::new(NSPoint::new(x, if host.isFlipped() { top } else { host.bounds().size.height - top - height }), NSSize::new(width, height))
            }).collect();
            *guard.ivars().rectangles.borrow_mut() = host_rects.clone();
            let holes = disjoint_holes(host_rects.iter().map(|r| [r.origin.x, r.origin.y,
                r.origin.x + r.size.width, r.origin.y + r.size.height]));
            unsafe {
                let _: () = msg_send![class!(CATransaction), begin];
                let _: () = msg_send![class!(CATransaction), setDisableActions: true];
                // Dynamic NSArray access keeps the existing dependency features.
                let children: *mut AnyObject = msg_send![host, subviews];
                let count: usize = msg_send![children, count];
                let mut guard_is_last = false;
                for index in 0..count {
                    let pointer: *mut NSView = msg_send![children, objectAtIndex: index];
                    if pointer == Retained::as_ptr(&guard).cast_mut().cast() {
                        guard_is_last = index + 1 == count;
                        continue;
                    }
                    let view = &*pointer;
                    let layer: *mut AnyObject = msg_send![view, layer];
                    if layer.is_null() { continue; }
                    if state.views.get(&(pointer as usize)).is_some_and(|entry| {
                        Retained::as_ptr(&entry.layer).cast_mut() != layer
                    }) {
                        if let Some(previous) = state.views.remove(&(pointer as usize)) { previous.restore(); }
                    }
                    let entry = state.views.entry(pointer as usize).or_insert_with(|| {
                        let original: *mut AnyObject = msg_send![layer, mask];
                        let mask: Retained<AnyObject> = msg_send![class!(CAShapeLayer), new];
                        let original = Retained::retain(original);
                        let rule = objc2_foundation::NSString::from_str("even-odd");
                        let _: () = msg_send![&*mask, setFillRule: &*rule];
                        let _: () = msg_send![&*mask, setMask: original.as_deref()];
                        MaskedView { view: Weak::new(view), layer: Retained::retain(layer).unwrap(), original, mask, geometry: None }
                    });
                    let bounds = view.bounds();
                    let current_mask: *mut AnyObject = msg_send![layer, mask];
                    let owns_mask = current_mask == Retained::as_ptr(&entry.mask).cast_mut();
                    if !owns_mask && entry.geometry.is_some() {
                        entry.original = Retained::retain(current_mask);
                        let _: () = msg_send![&*entry.mask, setMask: entry.original.as_deref()];
                    }
                    let local_holes: Vec<NSRect> = holes.iter().map(|&[x, y, right, bottom]| {
                        let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(right-x, bottom-y));
                        let local: NSRect = msg_send![view, convertRect: rect fromView: host];
                        local
                    }).collect();
                    if owns_mask && entry.geometry.as_ref() == Some(&(bounds, local_holes.clone())) { continue; }
                    let path = CGPathCreateMutable();
                    CGPathAddRect(path, ptr::null(), bounds);
                    for rect in &local_holes {
                        CGPathAddRect(path, ptr::null(), *rect);
                    }
                    let _: () = msg_send![&*entry.mask, setFrame: bounds];
                    let _: () = msg_send![&*entry.mask, setBounds: bounds];
                    let _: () = msg_send![&*entry.mask, setPath: path];
                    let _: () = msg_send![&*entry.layer, setMask: &*entry.mask];
                    CGPathRelease(path);
                    entry.geometry = Some((bounds, local_holes));
                }
                state.views.retain(|_, view| view.view.load().is_some());
                // New native children can be inserted after the guard was created.
                if !guard_is_last {
                    let _: () = msg_send![host, addSubview: &*guard positioned: 1isize relativeTo: ptr::null::<NSView>()];
                }
                let _: () = msg_send![class!(CATransaction), commit];
            }
        });
    }
}
#[cfg(target_os = "macos")]
pub use macos::{clear, finish_frame};

#[cfg(test)]
mod tests {
    use super::disjoint_holes;
    #[test]
    fn overlapping_popup_holes_preserve_union_without_double_cutting() {
        let holes = disjoint_holes([[0., 0., 10., 10.], [5., 5., 15., 15.], [2., 2., 4., 4.]]);
        let area: f64 = holes.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum();
        assert_eq!(area, 175.);
        for x in 0..15 {
            for y in 0..15 {
                let x = x as f64 + 0.5;
                let y = y as f64 + 0.5;
                let covered = holes
                    .iter()
                    .filter(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
                    .count();
                assert_eq!(
                    covered,
                    usize::from((x < 10. && y < 10.) || (x >= 5. && y >= 5.))
                );
            }
        }
    }
}
