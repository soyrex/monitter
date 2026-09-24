//! Main-thread bridge for the native HTTP Basic Auth navigation delegate.
//! Calls must originate in a Tauri `Webview::with_webview` callback, while the
//! child WKWebView and Wry's navigation delegate are both alive.

use std::{
    ffi::{c_char, c_void},
    sync::OnceLock,
};

use tauri::{AppHandle, Emitter};

type BrowserFocusCallback = unsafe extern "C" fn(*const c_char);

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

unsafe extern "C" {
    fn monitter_browser_auth_attach(
        webview: *mut c_void,
        tab_id: *const c_char,
        focus_callback: BrowserFocusCallback,
    ) -> bool;
    fn monitter_browser_auth_detach(webview: *mut c_void);
    fn monitter_browser_history_state(
        webview: *mut c_void,
        can_go_back: *mut bool,
        can_go_forward: *mut bool,
    ) -> bool;
    fn monitter_browser_go_back(webview: *mut c_void) -> bool;
    fn monitter_browser_go_forward(webview: *mut c_void) -> bool;
}

/// Installs the Basic Auth adapter on a live child WKWebView. Returns false if
/// the pointer is invalid, a delegate is missing, or this is not AppKit's main
/// thread. The caller should abort tab creation if installation fails.
pub fn attach(webview: *mut c_void, tab_id: &std::ffi::CStr, app: &AppHandle) -> bool {
    // The application outlives all native browser webviews. Keeping this clone
    // process-lifetime avoids passing Rust-owned data through AppKit callbacks.
    let _ = APP_HANDLE.set(app.clone());
    unsafe { monitter_browser_auth_attach(webview, tab_id.as_ptr(), emit_browser_focus) }
}

/// Called synchronously by AppKit's local mouse-down monitor. This only emits
/// to the local main webview; no callback or capability is exposed to the
/// external child page.
unsafe extern "C" fn emit_browser_focus(tab_id: *const c_char) {
    if tab_id.is_null() {
        return;
    }
    let Ok(tab_id) = unsafe { std::ffi::CStr::from_ptr(tab_id) }.to_str() else {
        return;
    };
    let Some(app) = APP_HANDLE.get() else {
        return;
    };
    let _ = app.emit_to(
        tauri::EventTarget::webview("main"),
        "monitter:browser-focus",
        serde_json::json!({ "tabId": tab_id }),
    );
}

/// Cancel sheets and restore Wry's delegate before closing the child webview.
pub fn detach(webview: *mut c_void) {
    unsafe { monitter_browser_auth_detach(webview) }
}

/// Read the native WKWebView history flags. `None` means the pointer or call
/// thread was invalid; an empty history is `Some((false, false))`.
pub fn history_state(webview: *mut c_void) -> Option<(bool, bool)> {
    let mut back = false;
    let mut forward = false;
    if unsafe { monitter_browser_history_state(webview, &mut back, &mut forward) } {
        Some((back, forward))
    } else {
        None
    }
}

/// Returns whether WebKit accepted a back navigation. A missing history entry
/// and an invalid call both return false; callers can query `history_state`
/// first when they need to distinguish those conditions.
pub fn go_back(webview: *mut c_void) -> bool {
    unsafe { monitter_browser_go_back(webview) }
}

/// Returns whether WebKit accepted a forward navigation.
pub fn go_forward(webview: *mut c_void) -> bool {
    unsafe { monitter_browser_go_forward(webview) }
}
