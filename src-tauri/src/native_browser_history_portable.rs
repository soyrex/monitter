//! Native navigation-history bridge for Wry webviews outside macOS.
//!
//! Every function here must be called synchronously from
//! `Webview::with_webview`; the supplied platform handle is only valid for
//! that callback. This intentionally reads the operating-system webview's
//! history rather than maintaining a renderer-side approximation.

use tauri::webview::PlatformWebview;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryState {
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOperation {
    Read,
    Back,
    Forward,
}

/// Reads native history, optionally performs one enabled navigation, and then
/// reads the flags again. An unavailable history entry is normal: `Back` and
/// `Forward` return the unchanged state without asking the engine to navigate.
#[cfg(any(target_os = "linux", windows))]
pub fn history(
    platform: &PlatformWebview,
    operation: HistoryOperation,
) -> Result<HistoryState, String> {
    imp::history(platform, operation)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{HistoryOperation, HistoryState, PlatformWebview};
    use webkit2gtk::WebViewExt;

    fn read(platform: &PlatformWebview) -> HistoryState {
        let webview = platform.inner();
        HistoryState {
            can_go_back: webview.can_go_back(),
            can_go_forward: webview.can_go_forward(),
        }
    }

    pub(super) fn history(
        platform: &PlatformWebview,
        operation: HistoryOperation,
    ) -> Result<HistoryState, String> {
        let before = read(platform);
        let webview = platform.inner();
        match operation {
            HistoryOperation::Read => {}
            HistoryOperation::Back if before.can_go_back => webview.go_back(),
            HistoryOperation::Forward if before.can_go_forward => webview.go_forward(),
            HistoryOperation::Back | HistoryOperation::Forward => return Ok(before),
        }
        Ok(read(platform))
    }
}

#[cfg(windows)]
mod imp {
    use super::{HistoryOperation, HistoryState, PlatformWebview};

    fn webview(
        platform: &PlatformWebview,
    ) -> Result<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2, String> {
        // WebView2 returns HRESULT failures while a controller is being
        // destroyed or before its core has been created. Surface those to the
        // command instead of reporting stale, guessed toolbar state.
        unsafe { platform.controller().CoreWebView2() }
            .map_err(|error| format!("WebView2 history is unavailable: {error}"))
    }

    fn read(platform: &PlatformWebview) -> Result<HistoryState, String> {
        let webview = webview(platform)?;
        // The generated COM bindings use windows_core::BOOL, not a bare i32.
        // Let the pointer parameters infer the type without adding another
        // direct Windows crate dependency to the macOS build.
        let mut can_go_back = Default::default();
        let mut can_go_forward = Default::default();
        unsafe {
            webview
                .CanGoBack(&mut can_go_back)
                .map_err(|error| format!("WebView2 could not read back history: {error}"))?;
            webview
                .CanGoForward(&mut can_go_forward)
                .map_err(|error| format!("WebView2 could not read forward history: {error}"))?;
        }
        Ok(HistoryState {
            can_go_back: can_go_back.0 != 0,
            can_go_forward: can_go_forward.0 != 0,
        })
    }

    pub(super) fn history(
        platform: &PlatformWebview,
        operation: HistoryOperation,
    ) -> Result<HistoryState, String> {
        let before = read(platform)?;
        match operation {
            HistoryOperation::Read => return Ok(before),
            HistoryOperation::Back if !before.can_go_back => return Ok(before),
            HistoryOperation::Forward if !before.can_go_forward => return Ok(before),
            HistoryOperation::Back | HistoryOperation::Forward => {}
        }
        let webview = webview(platform)?;
        unsafe {
            match operation {
                HistoryOperation::Back => webview.GoBack(),
                HistoryOperation::Forward => webview.GoForward(),
                HistoryOperation::Read => unreachable!("returned above"),
            }
            .map_err(|error| format!("WebView2 history navigation failed: {error}"))?;
        }
        read(platform)
    }
}
