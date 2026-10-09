//! Settings-derived styling for retained dock panes.
use crate::design::Palette;
use gpui_kit::{prelude::*, *};
use serde_json::Value;

/// The small subset of workspace settings that affects pane rendering and
/// pointer focus. Construct once from the current settings snapshot and pass
/// it to the dock renderer; it has no persistence or global state of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneAppearance {
    pub show_active_border: bool,
    pub active_border_color: u32,
    pub dim_inactive: bool,
    /// Opacity in the same 0.1–0.9 range as Tauri's percentage slider.
    pub inactive_opacity: f32,
    pub focus_follows_mouse: bool,
}

impl PaneAppearance {
    pub fn from_settings(settings: &Value, palette: Palette) -> Self {
        let configured_opacity = settings["inactivePaneOpacity"].as_f64().unwrap_or(0.6) as f32;
        Self {
            show_active_border: settings["showActivePaneBorder"].as_bool().unwrap_or(true),
            active_border_color: palette.accent,
            dim_inactive: settings["dimInactivePanes"].as_bool().unwrap_or(true),
            inactive_opacity: if configured_opacity.is_finite() {
                configured_opacity.clamp(0.1, 0.9)
            } else {
                0.6
            },
            focus_follows_mouse: settings["focusFollowsMouse"].as_bool().unwrap_or(false),
        }
    }

    /// Wrap pane content with the active accent border and inactive opacity
    /// selected in settings. The caller owns active-pane identity and should
    /// re-render when that identity or this config changes.
    pub fn style_panel(&self, content: AnyElement, active: bool) -> Div {
        let mut panel = div().size_full().relative().child(content);
        if active && self.show_active_border {
            panel = panel.child(div().absolute().inset_0().border_1()
                .border_color(rgb(self.active_border_color)));
        } else if !active && self.dim_inactive {
            panel = panel.opacity(self.inactive_opacity);
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn palette() -> Palette {
        Palette {
            dark: false,
            paper: 0xffffff,
            sidebar: 0xeeeeee,
            panel: 0xffffff,
            card: 0xffffff,
            ink: 0,
            muted: 0x777777,
            line: 0xdddddd,
            soft: 0xeeeeee,
            code: 0xeeeeee,
            accent: 0x123456,
        }
    }

    #[::core::prelude::v1::test]
    fn defaults_match_tauri_workspace_settings() {
        let appearance = PaneAppearance::from_settings(&json!({}), palette());
        assert!(appearance.show_active_border);
        assert!(appearance.dim_inactive);
        assert_eq!(appearance.inactive_opacity, 0.6);
        assert!(!appearance.focus_follows_mouse);
    }

    #[::core::prelude::v1::test]
    fn opacity_is_clamped_to_the_tauri_slider_range() {
        let appearance = PaneAppearance::from_settings(
            &json!({"inactivePaneOpacity": 3.0, "focusFollowsMouse": true}),
            palette(),
        );
        assert_eq!(appearance.inactive_opacity, 0.9);
        assert!(appearance.focus_follows_mouse);
    }
}
