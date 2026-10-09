//! Ordered Metal backdrop filtering for native menus and modal scrims.
//! Other platforms retain the existing shared capture fallback.

#[cfg(not(target_os = "macos"))]
use crate::{blurred_backdrop, design};
#[cfg(not(target_os = "macos"))]
use gpui_kit::component::DialogBackdropFactory;
use gpui_kit::{Window, component::WindowExt};
#[cfg(not(target_os = "macos"))]
use std::rc::Rc;

/// Install platform backdrop handling used by all Kit dialogs in this
/// window. Call once after the component root has been initialized.
pub(crate) fn install(window: &mut Window, cx: &mut gpui_kit::App) {
    #[cfg(target_os = "macos")]
    {
        // Metal captures the ordered backdrop during the same frame, before
        // foreground paint. Avoid a late CPU snapshot replacing this result.
        window.configure_dialog_backdrop(cx, None, Some(0.25));
        window.configure_popup_backdrop(cx, None);
        return;
    }
    #[cfg(not(target_os = "macos"))]
    let factory: DialogBackdropFactory =
        Rc::new(|window, cx| blurred_backdrop::capture(window, None, cx));
    #[cfg(not(target_os = "macos"))]
    window.configure_dialog_backdrop(cx, Some(factory), Some(0.25));
    #[cfg(not(target_os = "macos"))]
    let popup_factory = Rc::new(|window: &Window, cx: &gpui_kit::App| {
        if design::surface_color(0).a >= 1. {
            return None;
        }
        blurred_backdrop::capture(window, None, cx)
    });
    #[cfg(not(target_os = "macos"))]
    window.configure_popup_backdrop(cx, Some(popup_factory));
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "dialog_backdrop_tests.rs"]
mod dialog_backdrop_tests;

#[cfg(all(test, feature = "visual-export"))]
#[path = "popup_backdrop_tests.rs"]
mod popup_backdrop_tests;

#[cfg(all(test, feature = "visual-export"))]
#[path = "popup_focus_tests.rs"]
mod popup_focus_tests;

#[cfg(all(test, feature = "visual-export"))]
#[path = "tooltip_delay_tests.rs"]
mod tooltip_delay_tests;
