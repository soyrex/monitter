//! Per-view HTTP Basic authentication. Credentials never enter browser metadata,
//! URLs, logs, JavaScript, or persistent credential storage.
#[cfg(target_os = "macos")]
mod native {
    use block2::{Block, RcBlock};
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send, sel,
    };
    use objc2::{
        rc::{Allocated, Retained},
        runtime::{AnyObject, NSObject, Sel},
    };
    use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
    use std::{
        cell::{Cell, RefCell},
        ptr,
        rc::Rc,
    };
    use wry::WebViewExtMacOS;

    type Reply = dyn Fn(isize, *mut AnyObject);
    const USE_CREDENTIAL: isize = 0;
    const DEFAULT_HANDLING: isize = 1;
    const CANCEL: isize = 2;
    const ALERT_FIRST_BUTTON: isize = 1000;

    struct Pending {
        id: u64,
        reply: RcBlock<Reply>,
        alert: Retained<AnyObject>,
        parent: Retained<AnyObject>,
        username: Retained<AnyObject>,
        password: Retained<AnyObject>,
    }

    #[derive(Default)]
    struct Session {
        pending: RefCell<Option<Pending>>,
        next_id: Cell<u64>,
        closed: Cell<bool>,
    }

    impl Session {
        fn cancel(&self) {
            // Take before invoking either framework: completion can reenter.
            let pending = self.pending.borrow_mut().take();
            if let Some(pending) = pending {
                pending.reply.call((CANCEL, ptr::null_mut()));
                unsafe {
                    let _: () =
                        msg_send![&*pending.password, setStringValue: &*NSString::from_str("")];
                    let sheet: *mut AnyObject = msg_send![&*pending.alert, window];
                    let _: () = msg_send![&*pending.parent, endSheet: sheet, returnCode: 1001isize];
                }
            }
        }

        fn finish(&self, id: u64, response: isize) {
            let pending = {
                let mut slot = self.pending.borrow_mut();
                if slot.as_ref().is_none_or(|pending| pending.id != id) {
                    return;
                }
                slot.take().expect("checked pending authentication")
            };
            unsafe {
                if response == ALERT_FIRST_BUTTON && !self.closed.get() {
                    let username: Retained<NSString> = msg_send![&*pending.username, stringValue];
                    let password: Retained<NSString> = msg_send![&*pending.password, stringValue];
                    // NSURLCredentialPersistenceNone = 0: no Keychain/disk save.
                    let credential: Retained<AnyObject> = msg_send![class!(NSURLCredential),
                        credentialWithUser: &*username, password: &*password, persistence: 0usize];
                    let _: () =
                        msg_send![&*pending.password, setStringValue: &*NSString::from_str("")];
                    pending
                        .reply
                        .call((USE_CREDENTIAL, Retained::as_ptr(&credential).cast_mut()));
                } else {
                    let _: () =
                        msg_send![&*pending.password, setStringValue: &*NSString::from_str("")];
                    pending.reply.call((CANCEL, ptr::null_mut()));
                }
            }
        }
    }

    struct DelegateState {
        original: Retained<AnyObject>,
        session: Rc<Session>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = DelegateState]
        struct MonitterBrowserAuthDelegate;

        unsafe impl NSObjectProtocol for MonitterBrowserAuthDelegate {}

        impl MonitterBrowserAuthDelegate {
            #[unsafe(method(respondsToSelector:))]
            fn responds(&self, selector: Sel) -> bool {
                unsafe {
                    let own: bool = msg_send![super(self), respondsToSelector: selector];
                    own || { let original: bool = msg_send![&*self.ivars().original, respondsToSelector: selector]; original }
                }
            }

            #[unsafe(method(forwardingTargetForSelector:))]
            fn forwarding_target(&self, selector: Sel) -> *mut AnyObject {
                unsafe {
                    let responds: bool = msg_send![&*self.ivars().original, respondsToSelector: selector];
                    if responds { Retained::as_ptr(&self.ivars().original).cast_mut() } else { ptr::null_mut() }
                }
            }

            #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
            fn navigation(&self, view: &AnyObject, action: &AnyObject, reply: &Block<dyn Fn(isize)>) {
                self.ivars().session.cancel();
                // Preserve Wry's URL policy, downloads, and decision completion.
                unsafe {
                    let _: () = msg_send![&*self.ivars().original,
                        webView: view, decidePolicyForNavigationAction: action, decisionHandler: reply];
                }
            }

            #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
            fn challenge(&self, view: &AnyObject, challenge: &AnyObject, reply: &Block<Reply>) {
                unsafe { self.prompt(view, challenge, reply); }
            }
        }
    );

    impl MonitterBrowserAuthDelegate {
        unsafe fn prompt(&self, view: &AnyObject, challenge: &AnyObject, reply: &Block<Reply>) {
            let session = &self.ivars().session;
            // Only Basic is implemented. In particular, server trust, client
            // certificates, NTLM and proxy auth never receive our credentials.
            let space: Retained<AnyObject> = unsafe { msg_send![challenge, protectionSpace] };
            let method: Retained<NSString> = unsafe { msg_send![&*space, authenticationMethod] };
            let proxy: bool = unsafe { msg_send![&*space, isProxy] };
            if method.to_string() != "NSURLAuthenticationMethodHTTPBasic" || proxy {
                reply.call((DEFAULT_HANDLING, ptr::null_mut()));
                return;
            }
            let hidden: bool = unsafe { msg_send![view, isHiddenOrHasHiddenAncestor] };
            let parent: Option<Retained<AnyObject>> = unsafe { msg_send![view, window] };
            if session.closed.get() || hidden || parent.is_none() {
                reply.call((CANCEL, ptr::null_mut()));
                return;
            }
            session.cancel();
            let parent = parent.expect("checked browser window");
            // Do not stack an authentication prompt onto an unrelated sheet.
            let existing: *mut AnyObject = unsafe { msg_send![&*parent, attachedSheet] };
            if !existing.is_null() {
                reply.call((CANCEL, ptr::null_mut()));
                return;
            }
            let host: Retained<NSString> = unsafe { msg_send![&*space, host] };
            let port: isize = unsafe { msg_send![&*space, port] };
            let realm: Option<Retained<NSString>> = unsafe { msg_send![&*space, realm] };
            let failures: isize = unsafe { msg_send![challenge, previousFailureCount] };
            let protocol: Option<Retained<NSString>> = unsafe { msg_send![&*space, protocol] };
            let origin = format!(
                "{}://{}:{}",
                protocol
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "http".into()),
                host,
                port
            );
            let description = format!(
                "{}{}{}",
                origin,
                realm.map(|r| format!("\nRealm: {}", r)).unwrap_or_default(),
                if failures > 0 {
                    "\nThe previous credentials were rejected. Please try again."
                } else {
                    "\nEnter the username and password for this site."
                }
            );
            let alert: Retained<AnyObject> = unsafe { msg_send![class!(NSAlert), new] };
            unsafe {
                let _: () =
                    msg_send![&*alert, setMessageText: &*NSString::from_str("Sign in to website")];
                let _: () =
                    msg_send![&*alert, setInformativeText: &*NSString::from_str(&description)];
                let _: *mut AnyObject =
                    msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("Sign In")];
                let _: *mut AnyObject =
                    msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("Cancel")];
            }
            let accessory: Allocated<AnyObject> = unsafe { msg_send![class!(NSView), alloc] };
            let accessory: Retained<AnyObject> = unsafe {
                msg_send![accessory, initWithFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(300., 64.))]
            };
            let username: Allocated<AnyObject> = unsafe { msg_send![class!(NSTextField), alloc] };
            let username: Retained<AnyObject> = unsafe {
                msg_send![username, initWithFrame: NSRect::new(NSPoint::new(0., 36.), NSSize::new(300., 24.))]
            };
            let password: Allocated<AnyObject> =
                unsafe { msg_send![class!(NSSecureTextField), alloc] };
            let password: Retained<AnyObject> = unsafe {
                msg_send![password, initWithFrame: NSRect::new(NSPoint::new(0., 4.), NSSize::new(300., 24.))]
            };
            unsafe {
                let _: () =
                    msg_send![&*username, setPlaceholderString: &*NSString::from_str("Username")];
                let _: () =
                    msg_send![&*password, setPlaceholderString: &*NSString::from_str("Password")];
                let _: () = msg_send![&*username, setNextKeyView: &*password];
                let _: () = msg_send![&*accessory, addSubview: &*username];
                let _: () = msg_send![&*accessory, addSubview: &*password];
                let _: () = msg_send![&*alert, setAccessoryView: &*accessory];
                let sheet: *mut AnyObject = msg_send![&*alert, window];
                let _: () = msg_send![sheet, setInitialFirstResponder: &*username];
            }
            let id = session.next_id.get().wrapping_add(1);
            session.next_id.set(id);
            *session.pending.borrow_mut() = Some(Pending {
                id,
                reply: reply.copy(),
                alert: alert.clone(),
                parent: parent.clone(),
                username,
                password,
            });
            // The sheet owns its block. Capture only a weak Session so the
            // retained alert and pending challenge cannot form a reference cycle.
            let weak = Rc::downgrade(session);
            let completion = RcBlock::new(move |response: isize| {
                if let Some(session) = weak.upgrade() {
                    session.finish(id, response);
                }
            });
            unsafe {
                let _: () = msg_send![&*alert, beginSheetModalForWindow: &*parent, completionHandler: &*completion];
            }
        }
    }

    pub(super) struct Guard {
        browser: Retained<AnyObject>,
        delegate: Retained<MonitterBrowserAuthDelegate>,
        session: Rc<Session>,
    }

    impl Guard {
        pub(super) fn install(view: &wry::WebView) -> Result<Self, String> {
            let mtm = MainThreadMarker::new()
                .ok_or("Browser authentication requires the main thread.")?;
            unsafe {
                let browser: Retained<AnyObject> = Retained::cast_unchecked(view.webview());
                let original: Option<Retained<AnyObject>> =
                    msg_send![&*browser, navigationDelegate];
                let original =
                    original.ok_or("Native browser navigation delegate is unavailable.")?;
                let selector = sel!(webView:decidePolicyForNavigationAction:decisionHandler:);
                let compatible: bool = msg_send![&*original, respondsToSelector: selector];
                if !compatible {
                    return Err("Native browser navigation delegate is incompatible.".into());
                }
                let session = Rc::new(Session::default());
                let delegate = MonitterBrowserAuthDelegate::alloc(mtm).set_ivars(DelegateState {
                    original,
                    session: session.clone(),
                });
                let delegate: Retained<MonitterBrowserAuthDelegate> = msg_send![super(delegate), init];
                let _: () = msg_send![&*browser, setNavigationDelegate: &*delegate];
                Ok(Self {
                    browser,
                    delegate,
                    session,
                })
            }
        }

        pub(super) fn cancel(&self) {
            self.session.cancel();
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.session.closed.set(true);
            self.cancel();
            unsafe {
                let current: *mut AnyObject = msg_send![&*self.browser, navigationDelegate];
                if current == Retained::as_ptr(&self.delegate).cast_mut().cast() {
                    let _: () = msg_send![&*self.browser, setNavigationDelegate: &*self.delegate.ivars().original];
                }
            }
        }
    }
}

pub(super) struct BrowserAuth {
    #[cfg(target_os = "macos")]
    guard: native::Guard,
}

impl BrowserAuth {
    pub(super) fn install(view: &wry::WebView) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                guard: native::Guard::install(view)?,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = view;
            Ok(Self {})
        }
    }

    pub(super) fn cancel(&self) {
        #[cfg(target_os = "macos")]
        self.guard.cancel();
    }
}
