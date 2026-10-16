//! Native window backdrop selection supported by pinned GPUI 0.3.7.
use gpui_kit::{Window, WindowBackgroundAppearance};

/// Apply the persisted surface mode through the same GPUI mapping on every OS.
/// Both transparent modes request `Blurred`: macOS uses AppKit vibrancy (not
/// Liquid Glass), Windows requests acrylic on supported Windows versions, and
/// Wayland requests blur when the compositor advertises the KWin blur protocol.
/// X11 and Wayland without that protocol retain alpha transparency without a
/// blur guarantee. Native effects also depend on compositor/system settings.
/// GPUI exposes no per-window opacity percentage; `design::set_surface`
/// separately applies saved transparency to painted surfaces, leaving text opaque.
pub fn apply(window: &Window, setting: &str) -> bool {
    let Some(appearance) = appearance_for(setting) else {
        return false;
    };
    window.set_background_appearance(appearance);
    true
}

#[cfg(test)]
mod tests {
    #[::core::prelude::v1::test]
    fn known_surface_names_are_explicit() {
        assert!(matches!(
            super::appearance_for("opaque"),
            Some(gpui_kit::WindowBackgroundAppearance::Opaque)
        ));
        assert!(matches!(
            super::appearance_for("translucent"),
            Some(gpui_kit::WindowBackgroundAppearance::Blurred)
        ));
        assert!(matches!(
            super::appearance_for("glass"),
            Some(gpui_kit::WindowBackgroundAppearance::Blurred)
        ));
        assert!(super::appearance_for("liquid-glass").is_none());
    }
}

fn appearance_for(setting: &str) -> Option<WindowBackgroundAppearance> {
    match setting {
        "opaque" => Some(WindowBackgroundAppearance::Opaque),
        "translucent" => Some(WindowBackgroundAppearance::Blurred),
        "glass" => Some(WindowBackgroundAppearance::Blurred),
        _ => None,
    }
}
