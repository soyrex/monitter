//! Route browser and standard editing shortcuts while the native browser owns focus.
//! Web content receives no host IPC bridge.
#[derive(Clone, Copy, Debug)]
pub(super) enum Shortcut {
    FocusAddress,
    Reload,
    Back,
    Forward,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
}

#[cfg(target_os = "macos")]
mod native {
    use super::Shortcut;
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
    ) -> (Option<Monitor>, UnboundedReceiver<Shortcut>) {
        let (sender, receiver) = mpsc::unbounded();
        let native = view.webview();
        let callback = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            unsafe {
                if event.is_null() {
                    return event;
                }
                let window: *mut AnyObject = msg_send![&*native, window];
                let event_window: *mut AnyObject = msg_send![event, window];
                let hidden: bool = msg_send![&*native, isHiddenOrHasHiddenAncestor];
                if window.is_null() || window != event_window || hidden {
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
                let command_down = flags & (1 << 20) != 0;
                let control_down = flags & (1 << 18) != 0;
                let shift_down = flags & (1 << 17) != 0;
                let option_down = flags & (1 << 19) != 0;
                let key: u16 = msg_send![event, keyCode];
                let chars: *mut AnyObject = msg_send![event, charactersIgnoringModifiers];
                if chars.is_null() {
                    return event;
                }
                let matches = |value: &str| -> bool {
                    msg_send![chars, isEqualToString: &*objc2_foundation::NSString::from_str(value)]
                };
                // Match the host's browser bindings exactly. Modified combinations
                // outside this set remain available to the page.
                let shortcut =
                    if !shift_down && !option_down && (command_down ^ control_down) && matches("l")
                    {
                        Some(Shortcut::FocusAddress)
                    } else if command_down && !control_down && !shift_down && !option_down {
                        if matches("r") {
                            Some(Shortcut::Reload)
                        } else if matches("[") {
                            Some(Shortcut::Back)
                        } else if matches("]") {
                            Some(Shortcut::Forward)
                        } else if matches("t") {
                            Some(Shortcut::NewTab)
                        } else if matches("w") {
                            Some(Shortcut::CloseTab)
                        } else {
                            None
                        }
                    } else if option_down && !command_down && !control_down && !shift_down {
                        match key {
                            123 => Some(Shortcut::Back),
                            124 => Some(Shortcut::Forward),
                            _ => None,
                        }
                    } else if control_down && !command_down && !option_down && key == 48 {
                        Some(if shift_down {
                            Shortcut::PreviousTab
                        } else {
                            Shortcut::NextTab
                        })
                    } else {
                        None
                    };
                if let Some(shortcut) = shortcut {
                    return if sender.unbounded_send(shortcut).is_ok() {
                        std::ptr::null_mut()
                    } else {
                        event
                    };
                }
                if shift_down || option_down {
                    return event;
                }
                let is_copy = matches("c");
                let is_paste = matches("v");
                let is_cut = matches("x");
                let is_select_all = matches("a");
                // Editing selectors are native macOS commands and only use Command.
                // Leave Control-C/V to the page (for example, terminal-like web apps).
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
                if edit_action.is_none() {
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
                    if handled {
                        std::ptr::null_mut()
                    } else {
                        event
                    }
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
    ) -> (Self, futures::channel::mpsc::UnboundedReceiver<Shortcut>) {
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
