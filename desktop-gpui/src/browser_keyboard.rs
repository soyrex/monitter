//! Route address and standard editing shortcuts while the native browser owns focus.
//! Web content receives no host IPC bridge.
#[cfg(target_os = "macos")]
mod native {
    use futures::channel::mpsc::{self, UnboundedReceiver};
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use wry::WebViewExtMacOS;

    pub(super) struct Monitor(Retained<AnyObject>);
    impl Drop for Monitor {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![class!(NSEvent), removeMonitor: &*self.0];
            }
        }
    }
    pub(super) fn install(
        view: &wry::WebView,
        cancel_pending_hold: Option<Box<dyn Fn()>>,
    ) -> (Option<Monitor>, UnboundedReceiver<()>) {
        let (sender, receiver) = mpsc::unbounded();
        let native = view.webview();
        let callback = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            unsafe {
                if event.is_null() {
                    return event;
                }
                let window: *mut AnyObject = msg_send![&*native, window];
                let event_window: *mut AnyObject = msg_send![event, window];
                if window.is_null() || window != event_window {
                    return event;
                }
                let responder: *mut AnyObject = msg_send![window, firstResponder];
                if responder.is_null() {
                    return event;
                }
                let is_view: bool = msg_send![responder, isKindOfClass: class!(NSView)];
                if !is_view {
                    return event;
                }
                let belongs: bool = msg_send![responder, isDescendantOf: &*native];
                if !belongs {
                    return event;
                }
                if let Some(cancel) = &cancel_pending_hold {
                    cancel();
                }

                let flags: usize = msg_send![event, modifierFlags];
                // Cmd or Ctrl, with no Option/Shift; leave all page typing alone.
                if flags & ((1 << 20) | (1 << 18)) == 0 || flags & ((1 << 17) | (1 << 19)) != 0 {
                    return event;
                }
                let chars: *mut AnyObject = msg_send![event, charactersIgnoringModifiers];
                if chars.is_null() {
                    return event;
                }
                let is_l: bool =
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str("l")];
                let is_copy: bool =
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str("c")];
                let is_paste: bool =
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str("v")];
                let is_cut: bool =
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str("x")];
                let is_select_all: bool =
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str("a")];
                // Editing selectors are native macOS commands and only use Command.
                // Leave Control-C/V to the page (for example, terminal-like web apps).
                let command_down = flags & (1 << 20) != 0;
                let control_down = flags & (1 << 18) != 0;
                let edit_action = if command_down && !control_down && is_copy {
                    Some(objc2::sel!(copy:))
                } else if command_down && !control_down && is_paste {
                    Some(objc2::sel!(paste:))
                } else if command_down && !control_down && is_cut {
                    Some(objc2::sel!(cut:))
                } else if command_down && !control_down && is_select_all {
                    Some(objc2::sel!(selectAll:))
                } else {
                    None
                };
                if !is_l && edit_action.is_none() {
                    return event;
                }
                if let Some(action) = edit_action {
                    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                    // Dispatch through AppKit's responder action path so WKWebView's
                    // focused content view handles editing. Consume only when AppKit
                    // reports a target, preventing a second WebKit key equivalent.
                    let handled: bool = msg_send![
                        app,
                        sendAction: action,
                        to: responder,
                        from: event
                    ];
                    if handled { std::ptr::null_mut() } else { event }
                } else if sender.unbounded_send(()).is_ok() {
                    std::ptr::null_mut()
                } else {
                    event
                }
            }
        });
        let monitor = unsafe {
            let raw: *mut AnyObject = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: (1usize << 10), handler: &*callback];
            Retained::retain(raw).map(Monitor)
        };
        (monitor, receiver)
    }
}

pub(super) struct AddressShortcut {
    #[cfg(target_os = "macos")]
    _monitor: Option<native::Monitor>,
}
impl AddressShortcut {
    pub(super) fn install(
        view: &wry::WebView,
        cancel_pending_hold: Option<Box<dyn Fn()>>,
    ) -> (Self, futures::channel::mpsc::UnboundedReceiver<()>) {
        #[cfg(target_os = "macos")]
        {
            let (monitor, receiver) = native::install(view, cancel_pending_hold);
            (Self { _monitor: monitor }, receiver)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (view, cancel_pending_hold);
            let (_, receiver) = futures::channel::mpsc::unbounded();
            (Self {}, receiver)
        }
    }
}
