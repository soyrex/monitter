//! Settings-derived styling for retained dock panes.
use crate::design::Palette;
use gpui_kit::{prelude::*, *};
use monitter_lib::experimental_desktop::NativeTileTheme;
use serde_json::Value;

/// Shared corner radius for pane clipping, borders and active glow.
pub(crate) const PANE_RADIUS: f32 = 15.;

/// The small subset of workspace settings that affects pane rendering and
/// pointer focus. Construct once from the current settings snapshot and pass
/// it to the dock renderer; it has no persistence or global state of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneAppearance {
    pub tile_theme: NativeTileTheme,
    pub tile_backdrop_color: u32,
    pub paper_color: u32,
    pub pane_wash_tint: u32,
    pub dark: bool,
    pub border_opacity: u16,
    pub show_active_border: bool,
    /// Active pane outline thickness in physical UI pixels.
    pub active_border_width: f32,
    pub active_border_color: u32,
    pub dim_inactive: bool,
    /// Opacity in the same 0.1–0.9 range as Tauri's percentage slider.
    pub inactive_opacity: f32,
    pub focus_follows_mouse: bool,
}

impl PaneAppearance {
    pub fn from_settings(settings: &Value, palette: Palette) -> Self {
        let configured_opacity = settings["inactivePaneOpacity"].as_f64().unwrap_or(0.6) as f32;
        let configured_border_width = settings["activePaneBorderWidth"].as_f64().unwrap_or(1.) as f32;
        Self {
            tile_theme: crate::design::tile_theme(),
            tile_backdrop_color: tile_backdrop(palette),
            paper_color: palette.paper,
            pane_wash_tint: palette.tint(),
            dark: palette.dark,
            border_opacity: crate::design::border_opacity(),
            show_active_border: settings["showActivePaneBorder"].as_bool().unwrap_or(true),
            active_border_width: if configured_border_width.is_finite() {
                configured_border_width.clamp(1., 8.)
            } else {
                1.
            },
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
                color.a = self.resolved_active_border_opacity();
            }
            panel = panel.child(div().absolute().inset_0().border_1()
                .border_color(color));
        } else if !active && self.dim_inactive {
            panel = panel.opacity(self.inactive_opacity);
        }
        panel
    }

    /// Color used by the active pane outline and its matching header divider.
    pub(crate) fn resolved_active_border_color(&self) -> u32 {
        if self.tile_theme == NativeTileTheme::Tern {
            quiet_active_color(self.active_border_color, self.tile_backdrop_color)
        } else {
            self.active_border_color
        }
    }

    /// Keep the active-pane highlight legible when regular border opacity is low.
    pub(crate) fn resolved_active_border_opacity(&self) -> f32 {
        (self.border_opacity as f32 / 100.).max(0.5)
    }
}

/// Background-only emphasis, separate from cached content opacity and the
/// immediate native focus outline. The focused endpoint is the existing fill.
#[derive(Clone, Copy, Debug, Default)]
struct BackgroundEmphasis {
    current: f32,
    from: f32,
    target: f32,
    seeded: bool,
}
impl BackgroundEmphasis {
    fn target(active: bool, hovered: bool, appearance: PaneAppearance) -> f32 {
        let _ = appearance;
        if active { 1. } else if hovered { 0.5 } else { 0. }
    }
    fn observe(&mut self, target: f32, visible: bool, allowed: bool, progress: f32) -> bool {
        self.current = self.from + (self.target - self.from) * progress;
        if !self.seeded || !visible || !allowed {
            self.current = target; self.from = target; self.target = target; self.seeded = true;
            return false;
        }
        if (self.target - target).abs() < 0.0001 { return false; }
        self.from = self.current;
        self.target = target;
        true
    }
    fn sample(&mut self, progress: f32) -> f32 {
        self.current = self.from + (self.target - self.from) * progress;
        self.current
    }
    fn snap(&mut self) { self.current = self.target; self.from = self.target; }
}

fn emphasized_background(fill: Background, appearance: Option<PaneAppearance>, emphasis: f32) -> Background {
    let emphasis = emphasis.clamp(0., 1.);
    let inactive = appearance.filter(|appearance| appearance.dim_inactive).map_or(1., |appearance| appearance.inactive_opacity);
    let opacity = inactive + (1. - inactive) * emphasis;
    // Preserve the focused fill exactly; a quiet 2.5-point lightness shift
    // remains visible when users disable dimming or choose opacity=1.
    if let Some(mut color) = fill.as_solid() {
        color.l = (color.l - 0.025 * (1. - emphasis)).clamp(0., 1.);
        let mut surface: Background = color.into();
        surface.blend_mode = fill.blend_mode;
        surface.opacity(opacity)
    } else { fill.opacity(opacity) }
}

/// Resolve the reference pane's quiet top wash into a single background.
/// Call only for originally opaque panes. The resulting stops retain the
/// current inactive emphasis alpha without adding another paint layer.
pub(crate) fn pane_wash_background(fill: Background, tint: u32) -> Background {
    let Some(solid) = fill.as_solid() else { return fill; };
    let base: Rgba = solid.into();
    let tint: Rgba = rgb(tint).into();
    let top = Rgba {
        r: base.r * 0.86 + tint.r * 0.14,
        g: base.g * 0.86 + tint.g * 0.14,
        b: base.b * 0.86 + tint.b * 0.14,
        a: base.a,
    };
    let mut wash = linear_gradient(
        180.,
        linear_color_stop(top, 0.),
        linear_color_stop(base, 0.65),
    );
    wash.blend_mode = fill.blend_mode;
    wash
}

pub(crate) struct PaneBackground {
    emphasis: BackgroundEmphasis,
    clock: crate::chat::feedback::Clock,
    fill: Background,
    wash: bool,
    appearance: Option<PaneAppearance>,
    active: bool,
    hovered: bool,
    visible: bool,
}
impl PaneBackground {
    pub(crate) fn new() -> Self {
        Self { emphasis: Default::default(), clock: crate::chat::feedback::FeedbackClock::new(),
            fill: rgba(0x00000000).into(), wash: false, appearance: None, active: false, hovered: false, visible: false }
    }
    pub(crate) fn set_appearance(&mut self, appearance: PaneAppearance, fill: Background, wash: bool, active: bool, cx: &mut Context<Self>) {
        let changed = self.appearance != Some(appearance) || self.fill != fill || self.wash != wash;
        self.appearance = Some(appearance);
        self.fill = fill;
        self.wash = wash;
        self.active = active;
        self.retarget(cx);
        if changed { cx.notify(); }
    }
    pub(crate) fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered == hovered { return; }
        self.hovered = hovered;
        self.retarget(cx);
        cx.notify();
    }
    fn retarget(&mut self, cx: &mut Context<Self>) {
        let Some(appearance) = self.appearance else { return; };
        let allowed = !cx.reduce_motion() && crate::native_motion::motion_allowed();
        let progress = self.clock.borrow_mut().progress(!allowed);
        let target = BackgroundEmphasis::target(self.active, self.hovered, appearance);
        if self.emphasis.observe(target, self.visible && self.clock.borrow().painted, allowed, progress) {
            self.clock.borrow_mut().start(std::time::Duration::from_millis(140));
            cx.notify();
        }
    }
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible { return; }
        self.visible = visible;
        self.clock.borrow_mut().set_visible(visible);
        if !visible { self.hovered = false; self.emphasis.snap(); }
        cx.notify();
    }
}
impl Render for PaneBackground {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.clock.borrow_mut().progress(cx.reduce_motion() || !crate::native_motion::motion_allowed());
        let emphasis = self.emphasis.sample(progress);
        let fill = emphasized_background(self.fill, self.appearance, emphasis);
        let fill = if self.wash && self.fill.as_solid().is_some_and(|color| color.a >= 0.999) {
            pane_wash_background(fill, self.appearance.map_or(0, |appearance| appearance.pane_wash_tint))
        } else { fill };
        crate::chat::feedback::painted(div().size_full().bg(fill), self.clock.clone())
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

/// The same inset rounded outline above GPUI and native child views.
/// Fixed storage avoids allocating a contour on every focused-pane frame.
pub(crate) fn rounded_contour(
    bounds: Bounds<Pixels>,
    radius: Pixels,
    stroke_width: Pixels,
) -> [Point<Pixels>; 36] {
    let inset = stroke_width / 2.;
    let left = bounds.left() + inset;
    let top = bounds.top() + inset;
    let right = (bounds.right() - inset).max(left);
    let bottom = (bounds.bottom() - inset).max(top);
    let radius = (radius - inset).max(gpui_kit::px(0.))
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
    fn background_reversal_preserves_displayed_emphasis() {
        let mut motion = BackgroundEmphasis::default();
        assert!(!motion.observe(0., false, true, 1.));
        assert!(motion.observe(1., true, true, 1.));
        let middle = motion.sample(0.4);
        assert!(motion.observe(0., true, true, 0.4));
        assert_eq!(motion.sample(0.), middle);
        assert_eq!(motion.sample(1.), 0.);
        assert!(!motion.observe(1., true, false, 1.));
        assert_eq!(motion.current, 1.);
        assert!(!motion.observe(0.5, false, true, 1.));
        assert_eq!(motion.current, 0.5);
    }

    #[::core::prelude::v1::test]
    fn background_focus_hover_and_inactive_remain_distinct_without_dimming() {
        let mut appearance = PaneAppearance::from_settings(&json!({"inactivePaneOpacity":1,"dimInactivePanes":false}), palette());
        appearance.inactive_opacity = 1.;
        let fill: Background = rgb(0x79818a).into();
        let idle = emphasized_background(fill, Some(appearance), BackgroundEmphasis::target(false, false, appearance)).as_solid().unwrap();
        let hover = emphasized_background(fill, Some(appearance), BackgroundEmphasis::target(false, true, appearance)).as_solid().unwrap();
        let focused = emphasized_background(fill, Some(appearance), BackgroundEmphasis::target(true, true, appearance));
        assert!(idle.l < hover.l && hover.l < focused.as_solid().unwrap().l);
        assert_eq!(focused, fill);
        assert_eq!(idle.a, hover.a);
    }

    #[::core::prelude::v1::test]
    fn defaults_match_tauri_workspace_settings() {
        let appearance = PaneAppearance::from_settings(&json!({}), palette());
        assert!(appearance.show_active_border);
        assert_eq!(appearance.active_border_width, 1.);
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
    fn active_outline_width_is_clamped_to_preferences_range() {
        assert_eq!(
            PaneAppearance::from_settings(&json!({"activePaneBorderWidth": 0}), palette())
                .active_border_width,
            1.
        );
        assert_eq!(
            PaneAppearance::from_settings(&json!({"activePaneBorderWidth": 12}), palette())
                .active_border_width,
            8.
        );
    }

    #[::core::prelude::v1::test]
    fn rounded_outline_stays_inset_and_clamps_small_tiles() {
        for (width, height) in [(100., 80.), (6., 4.), (1., 1.)] {
            let bounds = Bounds::new(point(gpui_kit::px(10.), gpui_kit::px(20.)),
                size(gpui_kit::px(width), gpui_kit::px(height)));
            let contour = rounded_contour(bounds, gpui_kit::px(8.), gpui_kit::px(1.));
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
    #[::core::prelude::v1::test]
    fn thick_rounded_outline_is_inset_by_half_its_width() {
        let bounds = Bounds::new(
            point(gpui_kit::px(10.), gpui_kit::px(20.)),
            size(gpui_kit::px(100.), gpui_kit::px(80.)),
        );
        let contour = rounded_contour(bounds, gpui_kit::px(8.), gpui_kit::px(8.));
        assert!(contour.iter().all(|point| point.x >= bounds.left() + gpui_kit::px(4.)));
        assert!(contour.iter().all(|point| point.x <= bounds.right() - gpui_kit::px(4.)));
        assert!(contour.iter().all(|point| point.y >= bounds.top() + gpui_kit::px(4.)));
        assert!(contour.iter().all(|point| point.y <= bounds.bottom() - gpui_kit::px(4.)));
    }
}
