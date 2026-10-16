//! AppKit controls above the native browser, never injected into page content.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrowserControlAction {
    Close,
    ToggleZoom,
}

const COMMAND: usize = 1 << 20;
const CONTROL: usize = 1 << 18;
const PRIMARY_MODIFIERS: usize = COMMAND | CONTROL;
const HELD_MODIFIERS: usize = PRIMARY_MODIFIERS | (1 << 17) | (1 << 19) | (1 << 23);
const HOLD_DELAY: Duration = Duration::from_millis(200);

#[derive(Default)]
struct ModifierHold {
    flags: usize,
    pending: Option<(usize, Instant)>,
}

impl ModifierHold {
    fn flags_changed(&mut self, flags: usize, eligible: bool, now: Instant) -> bool {
        let previous = self.flags;
        self.flags = flags & HELD_MODIFIERS;
        self.pending = None;
        // A fresh, lone Ctrl or Cmd press. Once a chord is used, require release
        // before arming again, including after Shift/Option is released.
        if eligible && previous & PRIMARY_MODIFIERS == 0
            && matches!(self.flags, COMMAND | CONTROL)
        {
            self.pending = Some((self.flags, now));
        }
        self.pending.is_some()
    }

    fn cancel(&mut self) { self.pending = None; }

    fn ready(&mut self, flags: usize, eligible: bool, now: Instant) -> bool {
        let Some((modifier, started)) = self.pending else { return false; };
        if !eligible || flags & HELD_MODIFIERS != modifier {
            self.cancel();
            return false;
        }
        if now.duration_since(started) < HOLD_DELAY { return false; }
        self.pending = None;
        true
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.pending.map(|(_, started)| HOLD_DELAY.saturating_sub(now.duration_since(started)))
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{BrowserControlAction, ModifierHold, HOLD_DELAY};
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send, sel,
    };
    use objc2::{rc::Retained, runtime::AnyObject};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::{
        cell::{Cell, RefCell},
        ptr,
        time::{Duration, Instant},
    };
    use wry::WebViewExtMacOS;
    use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};

    // Match CoreGraphics' pointer encoding for objc2's checked message sends.
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

    const WIDTH: f64 = 184.;
    const HEIGHT: f64 = 44.;
    const BOTTOM: f64 = 20.;
    const CLOSE_SIZE: f64 = 44.;
    const PANE_WIDTH: f64 = 88.;
    const CLOSE_TOP: f64 = 8.;
    const CLOSE_RIGHT: f64 = 12.;
    const TOP_HOVER_HEIGHT: f64 = 64.;

    // NSView's default mouse handlers forward through the responder chain to
    // WKWebView. Keep clicks in the toolbar, including its empty padding and
    // disabled controls, from reaching the document behind it.
    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        struct BrowserToolbarPanel;

        impl BrowserToolbarPanel {
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool { false }
            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _: *mut AnyObject) -> bool { true }
            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _: *mut AnyObject) {}
            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, _: *mut AnyObject) {}
            #[unsafe(method(mouseDragged:))]
            fn mouse_dragged(&self, _: *mut AnyObject) {}
            #[unsafe(method(rightMouseDown:))]
            fn right_mouse_down(&self, _: *mut AnyObject) {}
            #[unsafe(method(rightMouseUp:))]
            fn right_mouse_up(&self, _: *mut AnyObject) {}
            #[unsafe(method(rightMouseDragged:))]
            fn right_mouse_dragged(&self, _: *mut AnyObject) {}
            #[unsafe(method(otherMouseDown:))]
            fn other_mouse_down(&self, _: *mut AnyObject) {}
            #[unsafe(method(otherMouseUp:))]
            fn other_mouse_up(&self, _: *mut AnyObject) {}
            #[unsafe(method(otherMouseDragged:))]
            fn other_mouse_dragged(&self, _: *mut AnyObject) {}
        }
    );

    struct State {
        browser: Retained<AnyObject>,
        panel: RefCell<Option<Retained<NSView>>>,
        close_panel: RefCell<Option<Retained<NSView>>>,
        close_shown: Cell<bool>,
        action_sender: UnboundedSender<BrowserControlAction>,
        zoom_button: RefCell<Option<Retained<AnyObject>>>,
        zoomed: Cell<bool>,
        top_hovered: Cell<bool>,
        keyboard_shown: Cell<bool>,
        tracking_area: RefCell<Option<Retained<AnyObject>>>,
        buttons: RefCell<Vec<Retained<AnyObject>>>,
        shown: Cell<bool>,
        modifier_hold: RefCell<ModifierHold>,
        reveal_timer: RefCell<Option<Retained<AnyObject>>>,
    }

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = State]
        struct BrowserControlsView;

        impl BrowserControlsView {
            #[unsafe(method(isFlipped))]
            fn is_flipped(&self) -> bool { true }
            #[unsafe(method(isOpaque))]
            fn is_opaque(&self) -> bool { false }
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool { false }
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, point: NSPoint) -> *mut NSView {
                if !self.ivars().shown.get() && !self.ivars().close_shown.get() { return ptr::null_mut(); }
                unsafe {
                    let hit: *mut NSView = msg_send![super(self), hitTest: point];
                    if hit == self as *const Self as *mut NSView { ptr::null_mut() } else { hit }
                }
            }
            #[unsafe(method(updateTrackingAreas))]
            fn update_tracking_areas(&self) {
                unsafe {
                    let _: () = msg_send![super(self), updateTrackingAreas];
                    if let Some(old) = self.ivars().tracking_area.borrow_mut().take() {
                        let _: () = msg_send![self, removeTrackingArea: &*old];
                    }
                    let bounds = self.bounds();
                    let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(bounds.size.width, TOP_HOVER_HEIGHT.min(bounds.size.height)));
                    let area: objc2::rc::Allocated<AnyObject> = msg_send![class!(NSTrackingArea), alloc];
                    // Mouse entered/exited, active for any pane in the active app.
                    let area: Retained<AnyObject> = msg_send![area, initWithRect: rect, options: (1usize | 64 | 1024), owner: self, userInfo: ptr::null::<AnyObject>()];
                    let _: () = msg_send![self, addTrackingArea: &*area];
                    *self.ivars().tracking_area.borrow_mut() = Some(area);
                }
            }
            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _: *mut AnyObject) {
                self.set_top_hovered(true);
            }
            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _: *mut AnyObject) {
                self.set_top_hovered(false);
            }
            #[unsafe(method(browserClose:))]
            fn close(&self, _: *mut AnyObject) {
                if self.ivars().close_shown.get() {
                    let _ = self.ivars().action_sender.unbounded_send(BrowserControlAction::Close);
                }
            }
            #[unsafe(method(browserZoom:))]
            fn zoom(&self, _: *mut AnyObject) {
                if self.ivars().close_shown.get() {
                    let _ = self.ivars().action_sender.unbounded_send(BrowserControlAction::ToggleZoom);
                }
            }
            #[unsafe(method(browserModifierHeld:))]
            fn modifier_held(&self, _: *mut AnyObject) {
                self.invalidate_reveal_timer();
                let flags: usize = unsafe { msg_send![class!(NSEvent), modifierFlags] };
                let now = Instant::now();
                if self.ivars().modifier_hold.borrow_mut().ready(flags, self.focused(), now) {
                    self.ivars().keyboard_shown.set(true);
                    self.set_shown(true, true);
                    self.set_close_shown(true, true);
                } else {
                    let remaining = self.ivars().modifier_hold.borrow().remaining(now);
                    if let Some(delay) = remaining { self.schedule_reveal(delay); }
                }
            }
            #[unsafe(method(browserBack:))]
            fn back(&self, _: *mut AnyObject) {
                unsafe { let _: *mut AnyObject = msg_send![&*self.ivars().browser, goBack]; }
                self.update_buttons();
            }
            #[unsafe(method(browserReload:))]
            fn reload(&self, _: *mut AnyObject) {
                unsafe { let _: *mut AnyObject = msg_send![&*self.ivars().browser, reload]; }
            }
            #[unsafe(method(browserStop:))]
            fn stop(&self, _: *mut AnyObject) {
                unsafe { let _: () = msg_send![&*self.ivars().browser, stopLoading]; }
            }
            #[unsafe(method(browserForward:))]
            fn forward(&self, _: *mut AnyObject) {
                unsafe { let _: *mut AnyObject = msg_send![&*self.ivars().browser, goForward]; }
                self.update_buttons();
            }
        }
    );

    impl BrowserControlsView {
        fn focused(&self) -> bool {
            unsafe {
                let window: *mut AnyObject = msg_send![&*self.ivars().browser, window];
                if window.is_null() { return false; }
                let key: bool = msg_send![window, isKeyWindow];
                let hidden: bool = msg_send![&*self.ivars().browser, isHiddenOrHasHiddenAncestor];
                if !key || hidden { return false; }
                let responder: *mut AnyObject = msg_send![window, firstResponder];
                let is_view: bool = !responder.is_null() && msg_send![responder, isKindOfClass: class!(NSView)];
                is_view && msg_send![responder, isDescendantOf: &*self.ivars().browser]
            }
        }

        fn invalidate_reveal_timer(&self) {
            if let Some(timer) = self.ivars().reveal_timer.borrow_mut().take() {
                unsafe { let _: () = msg_send![&*timer, invalidate]; }
            }
        }

        fn cancel_hold(&self) {
            self.invalidate_reveal_timer();
            self.ivars().modifier_hold.borrow_mut().cancel();
        }

        fn schedule_reveal(&self, delay: Duration) {
            self.invalidate_reveal_timer();
            unsafe {
                let timer: Retained<AnyObject> = msg_send![class!(NSTimer), scheduledTimerWithTimeInterval: delay.as_secs_f64(), target: self, selector: sel!(browserModifierHeld:), userInfo: ptr::null::<AnyObject>(), repeats: false];
                *self.ivars().reveal_timer.borrow_mut() = Some(timer);
            }
        }

        fn set_top_hovered(&self, hovered: bool) {
            self.ivars().top_hovered.set(hovered);
            self.set_close_shown(hovered || self.ivars().keyboard_shown.get(), true);
        }

        fn dismiss(&self, animated: bool) {
            self.cancel_hold();
            self.ivars().keyboard_shown.set(false);
            self.ivars().top_hovered.set(false);
            self.set_shown(false, animated);
            self.set_close_shown(false, animated);
        }

        fn set_zoomed(&self, zoomed: bool) {
            if self.ivars().zoomed.replace(zoomed) == zoomed { return; }
            let button = self.ivars().zoom_button.borrow();
            let Some(button) = button.as_ref() else { return; };
            let (symbol, label) = if zoomed {
                ("arrow.down.right.and.arrow.up.left", "Restore browser pane")
            } else {
                ("arrow.up.left.and.arrow.down.right", "Expand browser pane")
            };
            unsafe {
                let label = NSString::from_str(label);
                let image: *mut AnyObject = msg_send![class!(NSImage), imageWithSystemSymbolName: &*NSString::from_str(symbol), accessibilityDescription: &*label];
                let _: () = msg_send![&**button, setImage: image];
                let _: () = msg_send![&**button, setToolTip: &*label];
                let _: () = msg_send![&**button, setAccessibilityLabel: &*label];
            }
        }

        fn set_close_shown(&self, shown: bool, animated: bool) {
            if self.ivars().close_shown.replace(shown) == shown { return; }
            let panel = self.ivars().close_panel.borrow();
            let Some(panel) = panel.as_ref() else { return; };
            let origin = NSPoint::new(
                (self.bounds().size.width - CLOSE_RIGHT - PANE_WIDTH).max(0.),
                if shown { CLOSE_TOP } else { -CLOSE_SIZE - 4. },
            );
            unsafe {
                if animated {
                    let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
                    let context: *mut AnyObject = msg_send![class!(NSAnimationContext), currentContext];
                    let _: () = msg_send![context, setDuration: 0.18f64];
                    let animator: *mut AnyObject = msg_send![&**panel, animator];
                    let _: () = msg_send![animator, setFrameOrigin: origin];
                    let _: () = msg_send![animator, setAlphaValue: if shown { 1.0f64 } else { 0.0f64 }];
                    let _: () = msg_send![class!(NSAnimationContext), endGrouping];
                } else {
                    panel.setFrameOrigin(origin);
                    panel.setAlphaValue(if shown { 1. } else { 0. });
                }
            }
        }

        fn update_buttons(&self) {
            let buttons = self.ivars().buttons.borrow();
            if buttons.len() != 4 {
                return;
            }
            unsafe {
                let back: bool = msg_send![&*self.ivars().browser, canGoBack];
                let forward: bool = msg_send![&*self.ivars().browser, canGoForward];
                let _: () = msg_send![&*buttons[0], setEnabled: back];
                let _: () = msg_send![&*buttons[3], setEnabled: forward];
            }
        }

        fn set_shown(&self, shown: bool, animated: bool) {
            if shown {
                self.update_buttons();
            }
            if self.ivars().shown.replace(shown) == shown {
                return;
            }
            let panel = self.ivars().panel.borrow();
            let Some(panel) = panel.as_ref() else {
                return;
            };
            let bounds = self.bounds();
            let origin = NSPoint::new(
                ((bounds.size.width - WIDTH) / 2.).max(0.),
                if shown {
                    (bounds.size.height - BOTTOM - HEIGHT).max(0.)
                } else {
                    bounds.size.height + 4.
                },
            );
            unsafe {
                if animated {
                    let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
                    let context: *mut AnyObject =
                        msg_send![class!(NSAnimationContext), currentContext];
                    let _: () = msg_send![context, setDuration: 0.18f64];
                    let animator: *mut AnyObject = msg_send![&**panel, animator];
                    let _: () = msg_send![animator, setFrameOrigin: origin];
                    let _: () =
                        msg_send![animator, setAlphaValue: if shown { 1.0f64 } else { 0.0f64 }];
                    let _: () = msg_send![class!(NSAnimationContext), endGrouping];
                } else {
                    panel.setFrameOrigin(origin);
                    panel.setAlphaValue(if shown { 1. } else { 0. });
                }
            }
        }
    }

    fn add_surface(panel: &NSView, width: f64, height: f64) {
        unsafe {
            let background: Retained<AnyObject> = msg_send![class!(NSVisualEffectView), new];
            let _: () = msg_send![&*background, setFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(width, height))];
            let appearance: *mut AnyObject = msg_send![class!(NSAppearance), appearanceNamed: &*NSString::from_str("NSAppearanceNameVibrantDark")];
            let _: () = msg_send![&*background, setAppearance: appearance];
            let _: () = msg_send![&*background, setMaterial: 13isize]; // HUD window
            let _: () = msg_send![&*background, setBlendingMode: 1isize]; // Within window
            let _: () = msg_send![&*background, setState: 1isize]; // Active
            let _: () = msg_send![&*background, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![&*background, layer];
            let _: () = msg_send![layer, setCornerRadius: 14.0f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
            // Keep the blur at full strength: fading NSVisualEffectView itself
            // composites sharp page pixels back over the blurred backdrop.
            let _: () = msg_send![panel, addSubview: &*background];
            let tint: Retained<NSView> = msg_send![class!(NSView), new];
            tint.setFrame(NSRect::new(NSPoint::new(0., 0.), NSSize::new(width, height)));
            tint.setWantsLayer(true);
            tint.setAlphaValue(0.7);
            let tint_layer: *mut AnyObject = msg_send![&*tint, layer];
            let color: *mut AnyObject = msg_send![class!(NSColor), colorWithWhite: 0.10f64, alpha: 1.0f64];
            let cg_color: *const NativeCGColor = msg_send![color, CGColor];
            let _: () = msg_send![tint_layer, setBackgroundColor: cg_color];
            let _: () = msg_send![tint_layer, setCornerRadius: 14.0f64];
            panel.addSubview(&tint);
            // The 70% background tint is separate from the opaque buttons.
        }
    }

    pub(super) struct Controls {
        overlay: Retained<BrowserControlsView>,
        monitor: Option<Retained<AnyObject>>,
    }

    impl Controls {
        pub(super) fn install(view: &wry::WebView) -> (Self, UnboundedReceiver<BrowserControlAction>) {
            let mtm = MainThreadMarker::new().expect("Browser controls use the AppKit main thread");
            let browser = view.webview();
            let (action_sender, action_receiver) = mpsc::unbounded();
            unsafe {
                let bounds: NSRect = msg_send![&*browser, bounds];
                let state = State {
                    browser: Retained::cast_unchecked(browser.clone()),
                    panel: RefCell::new(None),
                    close_panel: RefCell::new(None),
                    close_shown: Cell::new(false),
                    action_sender,
                    zoom_button: RefCell::new(None),
                    zoomed: Cell::new(false),
                    top_hovered: Cell::new(false),
                    keyboard_shown: Cell::new(false),
                    tracking_area: RefCell::new(None),
                    buttons: RefCell::new(Vec::new()),
                    shown: Cell::new(false),
                    modifier_hold: RefCell::new(ModifierHold::default()),
                    reveal_timer: RefCell::new(None),
                };
                let overlay = BrowserControlsView::alloc(mtm).set_ivars(state);
                let overlay: Retained<BrowserControlsView> =
                    msg_send![super(overlay), initWithFrame: bounds];
                overlay.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions(2 | 16));
                let panel = BrowserToolbarPanel::alloc(mtm).set_ivars(());
                let panel: Retained<BrowserToolbarPanel> = msg_send![
                    super(panel),
                    initWithFrame: NSRect::new(
                        NSPoint::new((bounds.size.width - WIDTH) / 2., bounds.size.height + 4.),
                        NSSize::new(WIDTH, HEIGHT),
                    )
                ];
                let panel: Retained<NSView> = Retained::cast_unchecked(panel);
                // Keep the panel centered and anchored to the bottom on resize.
                panel.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions(1 | 4 | 8));
                panel.setAlphaValue(0.);
                add_surface(&panel, WIDTH, HEIGHT);

                for (index, (symbol, title, action)) in [
                    ("chevron.left", "Back", sel!(browserBack:)),
                    ("arrow.clockwise", "Refresh", sel!(browserReload:)),
                    ("xmark", "Stop loading", sel!(browserStop:)),
                    ("chevron.right", "Forward", sel!(browserForward:)),
                ]
                .into_iter()
                .enumerate()
                {
                    let button: Retained<AnyObject> = msg_send![class!(NSButton), new];
                    let _: () = msg_send![&*button, setFrame: NSRect::new(NSPoint::new(8. + index as f64 * 42., 4.), NSSize::new(42., 36.))];
                    let _: () = msg_send![&*button, setBordered: false];
                    let _: () = msg_send![&*button, setButtonType: 0usize];
                    let _: () = msg_send![&*button, setImagePosition: 1usize];
                    let image: *mut AnyObject = msg_send![class!(NSImage), imageWithSystemSymbolName: &*NSString::from_str(symbol), accessibilityDescription: &*NSString::from_str(title)];
                    let _: () = msg_send![&*button, setImage: image];
                    let color: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
                    let _: () = msg_send![&*button, setContentTintColor: color];
                    let _: () = msg_send![&*button, setToolTip: &*NSString::from_str(title)];
                    let _: () =
                        msg_send![&*button, setAccessibilityLabel: &*NSString::from_str(title)];
                    let _: () = msg_send![&*button, setTarget: &*overlay];
                    let _: () = msg_send![&*button, setAction: action];
                    let _: () = msg_send![&*panel, addSubview: &*button];
                    overlay.ivars().buttons.borrow_mut().push(button);
                }
                overlay.addSubview(&panel);
                *overlay.ivars().panel.borrow_mut() = Some(panel);
                let close_panel = BrowserToolbarPanel::alloc(mtm).set_ivars(());
                let close_panel: Retained<BrowserToolbarPanel> = msg_send![super(close_panel), initWithFrame: NSRect::new(
                    NSPoint::new((bounds.size.width - CLOSE_RIGHT - PANE_WIDTH).max(0.), -CLOSE_SIZE - 4.),
                    NSSize::new(PANE_WIDTH, CLOSE_SIZE))];
                let close_panel: Retained<NSView> = Retained::cast_unchecked(close_panel);
                close_panel.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions(1 | 8));
                close_panel.setAlphaValue(0.);
                add_surface(&close_panel, PANE_WIDTH, CLOSE_SIZE);
                for (index, (symbol, label, action)) in [
                    ("arrow.up.left.and.arrow.down.right", "Expand browser pane", sel!(browserZoom:)),
                    ("xmark", "Close browser pane", sel!(browserClose:)),
                ].into_iter().enumerate() {
                let close: Retained<AnyObject> = msg_send![class!(NSButton), new];
                let _: () = msg_send![&*close, setFrame: NSRect::new(NSPoint::new(4. + index as f64 * CLOSE_SIZE, 4.), NSSize::new(36., 36.))];
                let _: () = msg_send![&*close, setBordered: false];
                let _: () = msg_send![&*close, setButtonType: 0usize];
                let _: () = msg_send![&*close, setImagePosition: 1usize];
                let image: *mut AnyObject = msg_send![class!(NSImage), imageWithSystemSymbolName: &*NSString::from_str(symbol), accessibilityDescription: &*NSString::from_str(label)];
                let _: () = msg_send![&*close, setImage: image];
                let color: *mut AnyObject = msg_send![class!(NSColor), whiteColor];
                let _: () = msg_send![&*close, setContentTintColor: color];
                let _: () = msg_send![&*close, setToolTip: &*NSString::from_str(label)];
                let _: () = msg_send![&*close, setAccessibilityLabel: &*NSString::from_str(label)];
                let _: () = msg_send![&*close, setTarget: &*overlay];
                let _: () = msg_send![&*close, setAction: action];
                let _: () = msg_send![&*close_panel, addSubview: &*close];
                if index == 0 { *overlay.ivars().zoom_button.borrow_mut() = Some(close); }
                }
                overlay.addSubview(&close_panel);
                *overlay.ivars().close_panel.borrow_mut() = Some(close_panel);
                let _: () = msg_send![&*browser, addSubview: &*overlay];
                let _: () = msg_send![&*overlay, updateTrackingAreas];

                let target = overlay.clone();
                let callback = block2::RcBlock::new(
                    move |event: *mut AnyObject| -> *mut AnyObject {
                        if event.is_null() {
                            return event;
                        }
                        let kind: usize = msg_send![event, type];
                        let flags: usize = msg_send![event, modifierFlags];
                        // Cancel chords before focus/window filtering. Track releases
                        // globally so switching panes cannot leave an armed hold.
                        if matches!(kind, 10 | 1 | 3 | 25 | 22) { target.cancel_hold(); }
                        let window: *mut AnyObject = msg_send![&*target.ivars().browser, window];
                        let event_window: *mut AnyObject = msg_send![event, window];
                        let hidden: bool =
                            msg_send![&*target.ivars().browser, isHiddenOrHasHiddenAncestor];
                        let eligible = !window.is_null() && window == event_window && !hidden;
                        if kind == 12 {
                            target.invalidate_reveal_timer();
                            let arm = target.ivars().modifier_hold.borrow_mut().flags_changed(flags, eligible && target.focused(), Instant::now());
                            if arm { target.schedule_reveal(HOLD_DELAY); }
                            return event;
                        }
                        if !eligible {
                            return event;
                        }
                        if matches!(kind, 1 | 3 | 5 | 6 | 7 | 25 | 27) {
                            let location: NSPoint = msg_send![event, locationInWindow];
                            let point: NSPoint = msg_send![&*target, convertPoint: location, fromView: ptr::null::<NSView>()];
                            let bounds = target.bounds();
                            target.set_top_hovered(point.x >= 0. && point.x <= bounds.size.width
                                && point.y >= 0. && point.y <= TOP_HOVER_HEIGHT.min(bounds.size.height));
                            return event;
                        }
                        let responder: *mut AnyObject = msg_send![window, firstResponder];
                        let is_view: bool = !responder.is_null()
                            && msg_send![responder, isKindOfClass: class!(NSView)];
                        let focused: bool = is_view
                            && msg_send![responder, isDescendantOf: &*target.ivars().browser];
                        if kind == 10 {
                            // keyDown
                            let key: u16 = msg_send![event, keyCode];
                            if key == 53 && focused && (target.ivars().shown.get() || target.ivars().close_shown.get()) {
                                target.dismiss(true);
                                return ptr::null_mut();
                            }
                        } else if kind == 22 {
                            // scrollWheel: dismiss without eating the scroll.
                            let location: NSPoint = msg_send![event, locationInWindow];
                            let point: NSPoint = msg_send![&*target, convertPoint: location, fromView: ptr::null::<NSView>()];
                            let bounds = target.bounds();
                            if point.x >= 0.
                                && point.y >= 0.
                                && point.x <= bounds.size.width
                                && point.y <= bounds.size.height
                            {
                                target.dismiss(true);
                            }
                        }
                        event
                    },
                );
                let raw: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: ((1usize << 12) | (1usize << 10) | (1usize << 22) | (1usize << 1) | (1usize << 3) | (1usize << 5) | (1usize << 6) | (1usize << 7) | (1usize << 25) | (1usize << 27)), handler: &*callback];
                (Self {
                    overlay,
                    monitor: Retained::retain(raw),
                }, action_receiver)
            }
        }

        pub(super) fn refresh(&self) {
            if self.overlay.ivars().shown.get() {
                self.overlay.update_buttons();
            }
        }

        pub(super) fn hide(&self) {
            self.overlay.dismiss(false);
        }

        pub(super) fn set_zoomed(&self, zoomed: bool) { self.overlay.set_zoomed(zoomed); }

        pub(super) fn shortcut_canceller(&self) -> Box<dyn Fn()> {
            let overlay = self.overlay.clone();
            Box::new(move || overlay.cancel_hold())
        }
    }

    impl Drop for Controls {
        fn drop(&mut self) {
            self.overlay.cancel_hold();
            unsafe {
                if let Some(monitor) = self.monitor.take() {
                    let _: () = msg_send![class!(NSEvent), removeMonitor: &*monitor];
                }
                self.overlay.removeFromSuperview();
            }
        }
    }
}

pub(super) struct BrowserControls {
    #[cfg(target_os = "macos")]
    native: native::Controls,
}

impl BrowserControls {
    pub(super) fn install(view: &wry::WebView) -> (Self, futures::channel::mpsc::UnboundedReceiver<BrowserControlAction>) {
        #[cfg(target_os = "macos")]
        {
            let (native, receiver) = native::Controls::install(view);
            (Self { native }, receiver)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = view;
            let (_, receiver) = futures::channel::mpsc::unbounded();
            (Self {}, receiver)
        }
    }

    pub(super) fn refresh(&self) {
        #[cfg(target_os = "macos")]
        self.native.refresh();
    }

    pub(super) fn hide(&self) {
        #[cfg(target_os = "macos")]
        self.native.hide();
    }

    pub(super) fn set_zoomed(&self, zoomed: bool) {
        #[cfg(target_os = "macos")]
        self.native.set_zoomed(zoomed);
        #[cfg(not(target_os = "macos"))]
        let _ = zoomed;
    }

    pub(super) fn shortcut_canceller(&self) -> Box<dyn Fn()> {
        #[cfg(target_os = "macos")]
        { self.native.shortcut_canceller() }
        #[cfg(not(target_os = "macos"))]
        { Box::new(|| {}) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_modifier_reveals_once_after_200ms() {
        for modifier in [COMMAND, CONTROL] {
            let start = Instant::now();
            let mut hold = ModifierHold::default();
            assert!(hold.flags_changed(modifier, true, start));
            assert!(!hold.ready(modifier, true, start + Duration::from_millis(199)));
            assert!(hold.ready(modifier, true, start + HOLD_DELAY));
            assert!(!hold.ready(modifier, true, start + HOLD_DELAY));
        }
    }

    #[test]
    fn releasing_early_cancels_the_pending_reveal() {
        let start = Instant::now();
        let mut hold = ModifierHold::default();
        assert!(hold.flags_changed(COMMAND, true, start));
        assert!(!hold.flags_changed(0, true, start + Duration::from_millis(100)));
        assert!(!hold.ready(COMMAND, true, start + HOLD_DELAY));
    }

    #[test]
    fn a_shortcut_key_cancels_until_the_modifier_is_released() {
        let start = Instant::now();
        let mut hold = ModifierHold::default();
        assert!(hold.flags_changed(COMMAND, true, start));
        hold.cancel();
        assert!(!hold.ready(COMMAND, true, start + HOLD_DELAY));
        assert!(!hold.flags_changed(COMMAND, true, start + HOLD_DELAY));
        hold.flags_changed(0, true, start + HOLD_DELAY);
        assert!(hold.flags_changed(COMMAND, true, start + HOLD_DELAY));
        assert!(hold.ready(COMMAND, true, start + HOLD_DELAY * 2));
    }

    #[test]
    fn modifier_chords_never_arm_or_rearm_on_partial_release() {
        let start = Instant::now();
        for extra in [CONTROL, 1 << 17, 1 << 19, 1 << 23] {
            let mut hold = ModifierHold::default();
            assert!(!hold.flags_changed(COMMAND | extra, true, start));
            assert!(!hold.flags_changed(COMMAND, true, start + HOLD_DELAY));
            assert!(!hold.ready(COMMAND, true, start + HOLD_DELAY * 2));
            hold.flags_changed(0, true, start + HOLD_DELAY * 2);
            assert!(hold.flags_changed(COMMAND, true, start + HOLD_DELAY * 2));
            assert!(!hold.flags_changed(COMMAND | extra, true, start + HOLD_DELAY * 2));
            assert!(!hold.ready(COMMAND, true, start + HOLD_DELAY * 3));
        }
    }

    #[test]
    fn focus_loss_and_hidden_panes_cancel_the_reveal() {
        let start = Instant::now();
        let mut hold = ModifierHold::default();
        assert!(!hold.flags_changed(CONTROL, false, start));
        hold.flags_changed(0, true, start);
        assert!(hold.flags_changed(CONTROL, true, start));
        assert!(!hold.ready(CONTROL, false, start + HOLD_DELAY));
        assert!(!hold.ready(CONTROL, true, start + HOLD_DELAY));
    }

    #[test]
    fn latched_caps_lock_is_not_a_held_shortcut_modifier() {
        let start = Instant::now();
        let mut hold = ModifierHold::default();
        assert!(hold.flags_changed(COMMAND | (1 << 16), true, start));
        assert!(hold.ready(COMMAND | (1 << 16), true, start + HOLD_DELAY));
    }
}
