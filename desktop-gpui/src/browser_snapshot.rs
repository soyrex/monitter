//! Ephemeral snapshot of this pane's WebKit content; never persisted or captured
//! from the screen. The live WebView remains retained behind the address overlay.
use std::sync::mpsc::{self, Receiver};

#[cfg(target_os = "macos")]
pub(super) fn request(view: &wry::WebView) -> Receiver<Option<Vec<u8>>> {
    use objc2::{msg_send, runtime::AnyObject};
    use wry::WebViewExtMacOS;
    let (sender, receiver) = mpsc::channel();
    let callback = block2::RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
        let bytes = unsafe {
            if image.is_null() {
                None
            } else {
                let data: *mut AnyObject = msg_send![image, TIFFRepresentation];
                if data.is_null() {
                    None
                } else {
                    let length: usize = msg_send![data, length];
                    let pointer: *const u8 = msg_send![data, bytes];
                    if pointer.is_null() || length == 0 || length > 64 * 1024 * 1024 {
                        None
                    } else {
                        Some(std::slice::from_raw_parts(pointer, length).to_vec())
                    }
                }
            }
        };
        let _ = sender.send(bytes);
    });
    unsafe {
        let native = view.webview();
        let _: () = msg_send![&*native, takeSnapshotWithConfiguration: std::ptr::null::<AnyObject>(), completionHandler: &*callback];
    }
    receiver
}

#[cfg(not(target_os = "macos"))]
pub(super) fn request(_: &wry::WebView) -> Receiver<Option<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(None);
    receiver
}
