//! Settings-derived styling for retained dock panes.
use crate::design::Palette;
use gpui_kit::{prelude::*, *};
use monitter_lib::experimental_desktop::NativeTileTheme;
use serde_json::Value;

/// The small subset of workspace settings that affects pane rendering and
/// pointer focus. Construct once from the current settings snapshot and pass
/// it to the dock renderer; it has no persistence or global state of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneAppearance {
    pub tile_theme: NativeTileTheme,
    pub tile_backdrop_color: u32,
    pub border_opacity: u16,
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
            tile_theme: crate::design::tile_theme(),
            tile_backdrop_color: tile_backdrop(palette),
            border_opacity: crate::design::border_opacity(),
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
            let mut color = rgb(self.active_border_color);
            if self.tile_theme == NativeTileTheme::Tern {
                color.a = self.border_opacity as f32 / 100.;
            }
            panel = panel.child(div().absolute().inset_0().border_1()
                .border_color(color));
        } else if !active && self.dim_inactive {
            panel = panel.opacity(self.inactive_opacity);
        }
        panel
    }
}

/// Keep the selected palette, but give inset tiles a quieter workspace behind
/// them. This is local native tile presentation, not a theme-token override.
pub(crate) fn tile_backdrop(palette: Palette) -> u32 {
    let factor = if palette.dark { 0.68 } else { 0.97 };
    let mut color = 0;
    for shift in [16, 8, 0] {
        color |= ((((palette.paper >> shift) & 255) as f32 * factor).round() as u32) << shift;
    }
    color
}

pub(crate) fn quiet_active_color(accent: u32, backdrop: u32) -> u32 {
    let mut color = 0;
    for shift in [16, 8, 0] {
        color |= (((accent >> shift) & 255) + ((backdrop >> shift) & 255)) / 2 << shift;
    }
    color
}

/// The same inset 1px rounded outline above GPUI and native child views.
/// Fixed storage avoids allocating a contour on every focused-pane frame.
pub(crate) fn rounded_contour(bounds: Bounds<Pixels>, radius: Pixels) -> [Point<Pixels>; 36] {
    let left = bounds.left() + gpui_kit::px(0.5);
    let top = bounds.top() + gpui_kit::px(0.5);
    let right = (bounds.right() - gpui_kit::px(0.5)).max(left);
    let bottom = (bounds.bottom() - gpui_kit::px(0.5)).max(top);
    let radius = (radius - gpui_kit::px(0.5)).max(gpui_kit::px(0.))
        .min((right - left) / 2.).min((bottom - top) / 2.);
    let centers = [
        point(left + radius, top + radius),
        point(right - radius, top + radius),
        point(right - radius, bottom - radius),
        point(left + radius, bottom - radius),
    ];
    const ARC: [(f32, f32); 9] = [
        (1., 0.), (0.9807853, 0.19509032), (0.9238795, 0.38268343),
        (0.8314696, 0.55557024), (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2),
        (0.55557024, 0.8314696), (0.38268343, 0.9238795), (0.19509032, 0.9807853), (0., 1.),
    ];
    std::array::from_fn(|ix| {
        let corner = ix / 9;
        let (cos, sin) = ARC[ix % 9];
        let (x, y) = match corner {
            0 => (-cos, -sin),
            1 => (sin, -cos),
            2 => (cos, sin),
            _ => (-sin, cos),
        };
        centers[corner] + point(radius * x, radius * y)
    })
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

    #[::core::prelude::v1::test]
    fn rounded_outline_stays_inset_and_clamps_small_tiles() {
        for (width, height) in [(100., 80.), (6., 4.), (1., 1.)] {
            let bounds = Bounds::new(point(gpui_kit::px(10.), gpui_kit::px(20.)),
                size(gpui_kit::px(width), gpui_kit::px(height)));
            let contour = rounded_contour(bounds, gpui_kit::px(8.));
            for p in contour {
                assert!(p.x >= bounds.left() + gpui_kit::px(0.5));
                assert!(p.x <= bounds.right() - gpui_kit::px(0.5));
                assert!(p.y >= bounds.top() + gpui_kit::px(0.5));
                assert!(p.y <= bounds.bottom() - gpui_kit::px(0.5));
            }
            if width == 100. {
                assert_eq!(contour[0], point(gpui_kit::px(10.5), gpui_kit::px(28.)));
                assert_eq!(contour[8], point(gpui_kit::px(18.), gpui_kit::px(20.5)));
            }
        }
    }
}
