//! Monitter source tokens: app-theme.ts, AppSurface.svelte and reference.html.
//!
//! First cohort owns shell + design. Keep names aligned with `--app-{mode}-{key}`
//! CSS variables published by AppSurface.svelte's `applyAppearance`, plus the
//! AppThemePalette contract (`src/lib/app-theme.ts`). Status colors track the
//! Tauri status bullets and accent-ink helpers; sidebar geometry matches the
//! reference.html sidebar (252px wide, ~52px header, ~32px tabs row).
//!
//! The historical `card` slot is retained as an alias for `panel` so other
//! modules that pre-date the rename still resolve.
use super::field;
use gpui_kit::{Pixels, Rgba, WindowAppearance, px, rgba};
use monitter_lib::experimental_desktop::NativeTileTheme;
use serde_json::Value;
use std::cell::Cell;

/// Shared layout rhythm in design pixels. Scale at the rendering boundary with
/// `ui_scaled_px`; GPUI's rem-based `_1` / `_2` helpers are not these tokens.
/// See `desktop-gpui/UX_STYLE_GUIDE.md` for roles and deliberate exceptions.
pub mod space {
    pub const TIGHT: f32 = 5.;
    pub const INSET: f32 = 10.;
    pub const GROUP: f32 = 15.;
    pub const SECTION: f32 = 20.;
    pub const PAGE: f32 = 30.;
}

pub mod radius {
    pub const CONTROL: f32 = 8.;
    pub const CARD: f32 = 12.;
    /// Floating settings/edit surfaces; ordinary cards retain their own radius.
    pub const MODAL: f32 = 14.;
}

// Geometry — mirrors `AppSurface.svelte` sidebar and reference.html layout.
pub const SIDEBAR_MAX_WIDTH: f32 = 360.;
pub const SIDEBAR_WIDTH: f32 = 252.;
/// Native traffic lights do not grow with the application interface scale.
/// 12pt leading inset + 56pt control cluster + 12pt trailing inset.
pub const COMPACT_SIDEBAR_WIDTH_POINTS: f32 = 80.;
pub fn compact_sidebar_width_design(interface_scale: u16) -> f32 {
    COMPACT_SIDEBAR_WIDTH_POINTS / (normalize_interface_scale(interface_scale) as f32 / 100.)
}
pub const HEADER_HEIGHT: f32 = 58.;
/// Named workspace pills and Tern's three sidebar controls share these metrics.
/// The theme font is already interface-scaled; scale only the extra insets.
pub struct WorkspaceTabMetrics {
    pub height: Pixels,
    pub icon_size: Pixels,
    pub editor_size: Pixels,
    pub text_size: Pixels,
    pub row_height: Pixels,
}
pub fn workspace_tab_metrics(font_size: Pixels) -> WorkspaceTabMetrics {
    let height = font_size + ui_scaled_px(18.);
    WorkspaceTabMetrics {
        height,
        icon_size: font_size + ui_scaled_px(2.),
        editor_size: font_size + ui_scaled_px(8.),
        text_size: (font_size - ui_scaled_px(4.)).max(ui_scaled_px(8.)),
        // Retain the native titlebar centre at normal sizes, but allow larger
        // accessibility fonts/scales to fit without clipping either row.
        row_height: px(HEADER_HEIGHT).max(height + ui_scaled_px(8.)),
    }
}
pub const SIDEBAR_TABS_ROW_MIN_HEIGHT: f32 = 32.;
/// Classic TabBar's native component owns a fixed 32pt strip; Modern accepts
/// scaled geometry. Sidebar/header alignment must use the actual strip.
pub fn workspace_tabbar_height_points(settings: &Value, scale: u16) -> f32 {
    if field(settings, "tabStyle") != "modern" { return 32.; }
    let native_height = match field(settings, "interfaceDensity").as_str() {
        "tight" => 52_f32,
        "spacious" => 68_f32,
        _ => 60_f32,
    };
    let factor = normalize_interface_scale(scale) as f32 / 100.;
    ((native_height / factor).max(36.) - 4.).max(30.) * factor
}
pub const SIDEBAR_TABS_ROW_PAD_X: f32 = 6.;
pub const SIDEBAR_TABS_ROW_PAD_TOP: f32 = 6.;
pub const SIDEBAR_TAB_HEIGHT: f32 = 25.;

/// Tern layout tokens in design pixels. Apply interface scale at the boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TernSpacing {
    pub gutter: f32,
    pub outer_margin: f32,
}
impl TernSpacing {
    /// Each tile owns half a gutter; the shell supplies only the extra perimeter.
    pub fn outer_padding(self) -> f32 { self.outer_margin - self.gutter / 2. }
}
pub fn tern_spacing(settings: &Value) -> TernSpacing {
    let gutter = match field(settings, "interfaceDensity").as_str() {
        "tight" => 4., "spacious" => 8., _ => 6.,
    };
    TernSpacing { gutter, outer_margin: gutter + 2. }
}
pub fn workspace_route_needs_fill(tile_theme: NativeTileTheme, has_dock: bool) -> bool {
    tile_theme != NativeTileTheme::Tern || !has_dock
}

#[cfg(test)]
mod tern_spacing_tests {
    use super::*;
    #[test]
    fn density_preserves_a_wider_perimeter_without_doubling_tile_insets() {
        for (density, gutter, margin) in [("tight",4.,6.), ("normal",6.,8.), ("spacious",8.,10.)] {
            let spacing = tern_spacing(&serde_json::json!({"interfaceDensity":density}));
            assert_eq!(spacing.gutter, gutter);
            assert_eq!(spacing.outer_margin, margin);
            assert!(spacing.outer_margin > spacing.gutter);
            assert_eq!(spacing.outer_padding() + spacing.gutter / 2., margin);
            assert_eq!(spacing.gutter / 2. + spacing.gutter / 2., gutter);
        }
    }
    #[test]
    fn tern_dock_routes_keep_one_backdrop_while_empty_states_keep_their_fill() {
        assert!(!workspace_route_needs_fill(NativeTileTheme::Tern, true));
        assert!(workspace_route_needs_fill(NativeTileTheme::Tern, false));
        assert!(workspace_route_needs_fill(NativeTileTheme::Monitter, true));
    }
}

// Typography — font families come from `desktop-gpui/src/fonts.rs` and the
// `--interface-font` / `--chat-font` / `--terminal-font` tokens in Tauri.
pub const FONT_SANS: &str = "IBM Plex Sans";
pub const FONT_MONO: &str = "IBM Plex Mono";
pub const INTERFACE_FONT_SIZE: f32 = 14.;
pub const CHAT_FONT_SIZE: f32 = 13.;
pub const CHAT_MAX_WIDTH: f32 = 700.;
pub const COMPOSER_CONTROL_SIZE: f32 = 32.;

/// Native interface scale is persisted in five-point increments.
pub const INTERFACE_SCALE_MIN: u16 = 80;
pub const INTERFACE_SCALE_MAX: u16 = 200;
pub const INTERFACE_SCALE_STEP: u16 = 5;
pub const INTERFACE_SCALE_DEFAULT: u16 = 125;

thread_local! {
    static ACTIVE_INTERFACE_SCALE: Cell<u16> = const { Cell::new(INTERFACE_SCALE_DEFAULT) };
    static ACTIVE_SURFACE_ALPHA: Cell<f32> = const { Cell::new(1.0) };
    static ACTIVE_TILE_THEME: Cell<NativeTileTheme> = const { Cell::new(NativeTileTheme::Monitter) };
    static ACTIVE_BORDER_OPACITY: Cell<u16> = const { Cell::new(100) };
}

/// Native tile presentation is independent of the light/dark colour palette.
pub fn set_tile_theme(theme: NativeTileTheme) {
    ACTIVE_TILE_THEME.with(|value| value.set(theme));
}

pub fn tile_theme() -> NativeTileTheme {
    ACTIVE_TILE_THEME.with(Cell::get)
}

pub fn set_border_opacity(opacity: u16) {
    ACTIVE_BORDER_OPACITY.with(|value| value.set(opacity.min(100)));
}

pub fn border_opacity() -> u16 {
    ACTIVE_BORDER_OPACITY.with(Cell::get)
}

/// Tern outlines retain their quiet base contrast and follow the native border control.
pub fn tile_border_color(palette: Palette) -> Rgba {
    let mut color = gpui_kit::rgb(palette.ink);
    color.a = 0.20 * border_opacity() as f32 / 100.;
    color
}

/// Double the composer outline's RGB contrast against its theme surface,
/// retaining the one-pixel stroke and configured surface transparency.
pub fn composer_border_color(palette: Palette) -> Rgba {
    let mut line = surface_color(palette.line);
    let surface = gpui_kit::rgb(palette.card);
    line.r = (surface.r + 2. * (line.r - surface.r)).clamp(0., 1.);
    line.g = (surface.g + 2. * (line.g - surface.g)).clamp(0., 1.);
    line.b = (surface.b + 2. * (line.b - surface.b)).clamp(0., 1.);
    line
}

/// Set the native background alpha from the same settings as Tauri's
/// `--window-transparency` CSS mix. Only translucent and glass surfaces use
/// the percentage; opaque surfaces stay fully opaque. The alpha is intended
/// for background fills and bubble outlines, never foreground text or icons.
pub fn set_surface(settings: &Value) {
    let surface = field(settings, "windowSurface");
    let transparency = settings["windowTransparency"]
        .as_u64()
        .unwrap_or(18)
        .min(100) as f32;
    let alpha = if matches!(surface.as_str(), "translucent" | "glass") {
        (100.0 - transparency) / 100.0
    } else {
        1.0
    };
    ACTIVE_SURFACE_ALPHA.with(|value| value.set(alpha));
}

/// Apply the current native surface alpha to an RGB palette color.
pub fn surface_color(color: u32) -> Rgba {
    let alpha = ACTIVE_SURFACE_ALPHA.with(Cell::get);
    let alpha_byte = (alpha * 255.0).round().clamp(0.0, 255.0) as u32;
    rgba(((color & 0x00ff_ffff) << 8) | alpha_byte)
}

/// Raised surfaces deliberately share the exact same alpha.
/// Their palette colors and the normal compositing of stacked surfaces provide
/// separation. Foreground colors never go through these background helpers.
pub fn raised_surface_color(color: u32) -> Rgba { surface_color(color) }
pub fn strong_surface_color(color: u32) -> Rgba { surface_color(color) }

/// Modals use one quarter of the configured background transparency.
pub fn modal_surface_color(color: u32) -> Rgba {
    let mut fill = surface_color(color);
    fill.a = 1.0 - (1.0 - fill.a) * 0.25;
    fill
}

/// Popup copy needs a steadier surface than the panes behind its blur.
/// Keep the theme hue, deepen dark menus, and attenuate wallpaper bleed.
pub fn menu_surface_color(palette: Palette) -> Rgba {
    let mut fill = modal_surface_color(palette.panel);
    if palette.dark {
        fill.r *= 0.55;
        fill.g *= 0.55;
        fill.b *= 0.55;
    }
    fill
}

/// Normalize stale or malformed stored values to the nearest supported scale.
pub const fn normalize_interface_scale(value: u16) -> u16 {
    let clamped = if value < INTERFACE_SCALE_MIN {
        INTERFACE_SCALE_MIN
    } else if value > INTERFACE_SCALE_MAX {
        INTERFACE_SCALE_MAX
    } else {
        value
    };
    let rounded =
        ((clamped + INTERFACE_SCALE_STEP / 2) / INTERFACE_SCALE_STEP) * INTERFACE_SCALE_STEP;
    if rounded < INTERFACE_SCALE_MIN {
        INTERFACE_SCALE_MIN
    } else if rounded > INTERFACE_SCALE_MAX {
        INTERFACE_SCALE_MAX
    } else {
        rounded
    }
}

/// Scale a design-token pixel value without changing the operating-system DPI.
pub fn ui_px(value: f32, interface_scale: u16) -> Pixels {
    px(value * normalize_interface_scale(interface_scale) as f32 / 100.)
}

/// Set the canonical interface scale for the current GPUI thread.
pub fn set_interface_scale(interface_scale: u16) {
    ACTIVE_INTERFACE_SCALE.with(|scale| scale.set(normalize_interface_scale(interface_scale)));
}

/// Return the canonical interface scale currently active on this GPUI thread.
pub fn interface_scale() -> u16 {
    ACTIVE_INTERFACE_SCALE.with(Cell::get)
}

/// Scale a token using the active native preference; suitable as `px` in a
/// module that explicitly imports `design::ui_scaled_px as px`.
pub fn ui_scaled_px(value: f32) -> Pixels {
    ui_px(value, interface_scale())
}

#[cfg(test)]
mod interface_scale_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn interface_scale_is_clamped_and_snapped_to_five_percent() {
        assert_eq!(normalize_interface_scale(0), 80);
        assert_eq!(normalize_interface_scale(80), 80);
        assert_eq!(normalize_interface_scale(126), 125);
        assert_eq!(normalize_interface_scale(128), 130);
        assert_eq!(normalize_interface_scale(200), 200);
        assert_eq!(normalize_interface_scale(u16::MAX), 200);
    }

    #[::core::prelude::v1::test]
    fn thread_local_scale_drives_single_argument_pixel_helper() {
        set_interface_scale(150);
        assert_eq!(interface_scale(), 150);
        set_interface_scale(127);
        assert_eq!(interface_scale(), 125);
        assert_eq!(ui_px(16., interface_scale()), px(20.));
        set_interface_scale(INTERFACE_SCALE_DEFAULT);
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn compact_rail_tracks_native_control_width_at_every_interface_scale() {
        for scale in [80, 100, 125, 150, 200] {
            let width = ui_px(compact_sidebar_width_design(scale), scale);
            assert!((f32::from(width) - COMPACT_SIDEBAR_WIDTH_POINTS).abs() < 0.001);
        }
    }

    #[::core::prelude::v1::test]
    fn surface_alpha_matches_tauri_transparency_mix() {
        set_surface(&serde_json::json!({
            "windowSurface": "translucent",
            "windowTransparency": 18
        }));
        assert_eq!(u32::from(surface_color(0x123456)), 0x123456d1);

        set_surface(&serde_json::json!({
            "windowSurface": "glass",
            "windowTransparency": 70
        }));
        assert_eq!(u32::from(surface_color(0x123456)), 0x1234564d);
    }

    #[::core::prelude::v1::test]
    fn all_surface_roles_share_window_transparency() {
        for surface in ["glass", "translucent", "opaque"] {
            for transparency in [0, 18, 70] {
                set_surface(&serde_json::json!({"windowSurface":surface,"windowTransparency":transparency}));
                for color in [0x123456, 0xf0f0f0, 0x242424] {
                    assert_eq!(u32::from(surface_color(color)), u32::from(raised_surface_color(color)));
                    assert_eq!(u32::from(surface_color(color)), u32::from(strong_surface_color(color)));
                }
            }
        }
        set_surface(&serde_json::json!({}));
    }

    #[::core::prelude::v1::test]
    fn opaque_surface_ignores_transparency_and_defaults_to_opaque() {
        set_surface(&serde_json::json!({
            "windowSurface": "opaque",
            "windowTransparency": 70
        }));
        assert_eq!(u32::from(surface_color(0xabcdef)), 0xabcdefff);
        set_surface(&serde_json::json!({}));
        assert_eq!(u32::from(surface_color(0xabcdef)), 0xabcdefff);
    }
}

// Sidebar tab strip — `AppSurface.svelte` `.sidebar-tab`.
pub const SIDEBAR_TAB_FONT_SIZE: f32 = 9.;
pub const SIDEBAR_TAB_LETTER_SPACING_EM: f32 = 0.08;

// Icon sizes — match Lucide usage in the Svelte components.
pub const ICON_SIZE_XS: f32 = 12.;
pub const ICON_SIZE_SM: f32 = 14.;
pub const ICON_SIZE_MD: f32 = 16.;

// Status dot sizes.
pub const STATUS_DOT: f32 = 6.;

// Spacing helpers — match `gap_1`/`py_2` defaults but readable in tokens.
pub const SIDEBAR_TABS_GAP: f32 = 3.;
pub const SIDEBAR_TAB_ENTRY_PADDING: f32 = 3.;
pub const SIDEBAR_TAB_GAP: f32 = 3.;
pub const SIDEBAR_TAB_RADIUS: f32 = 5.;

// Avatar / row avatar / status pill defaults.
pub const AGENT_AVATAR_SIZE: f32 = 32.;
pub const TASK_ROW_HEIGHT: f32 = 32.;
pub const TASK_INDENT: f32 = 16.;

// --- Palette ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub paper: u32,
    pub sidebar: u32,
    pub panel: u32,
    /// Historical alias for `panel` retained so modules that pre-date the
    /// rename still resolve. Tracks `panel` whenever the palette is built.
    pub card: u32,
    pub ink: u32,
    pub muted: u32,
    pub line: u32,
    pub soft: u32,
    pub code: u32,
    pub accent: u32,
}

impl Palette {
    /// Read the panel slot under its current canonical name.
    pub fn panel(&self) -> u32 {
        self.card
    }

    /// Activity pane surface levels derived from the selected theme.
    pub fn fill(&self, level: usize) -> u32 {
        const DARK: [f32; 4] = [0.035, 0.07, 0.10, 0.15];
        const LIGHT: [f32; 4] = [0.03, 0.06, 0.09, 0.13];
        let alphas = if self.dark { DARK } else { LIGHT };
        mix_rgb(self.paper, self.ink, alphas[level.min(3)])
    }

    pub fn tint(&self) -> u32 {
        mix_rgb(self.ink, self.accent, 0.35)
    }

    pub fn hairline(&self, level: usize) -> u32 {
        const DARK: [f32; 2] = [0.10, 0.18];
        const LIGHT: [f32; 2] = [0.14, 0.24];
        let alphas = if self.dark { DARK } else { LIGHT };
        mix_rgb(self.paper, self.tint(), alphas[level.min(1)])
    }

    pub fn glow(&self) -> u32 {
        mix_rgb(self.paper, self.accent, if self.dark { 0.38 } else { 0.45 })
    }

    fn status_hue(&self, hue: u32) -> u32 {
        if self.dark { mix_rgb(hue, self.ink, 0.12) }
        else { mix_rgb(hue, self.ink, 0.08) }
    }

    pub fn ok(&self) -> u32 {
        self.status_hue(0x4f8f63)
    }

    pub fn warn(&self) -> u32 {
        self.status_hue(0xb38b45)
    }

    pub fn err(&self) -> u32 {
        self.status_hue(0xbd655b)
    }

    pub fn agent(&self) -> u32 {
        self.accent
    }

    pub fn text(&self, level: usize) -> u32 {
        const DARK: [f32; 4] = [0.92, 0.66, 0.46, 0.30];
        const LIGHT: [f32; 4] = [0.92, 0.74, 0.60, 0.36];
        let alphas = if self.dark { DARK } else { LIGHT };
        mix_rgb(self.paper, self.ink, alphas[level.min(3)])
    }
}

/// Linear blend of two 0xRRGGBB colours; `t` is the weight of `over`.
pub fn mix_rgb(under: u32, over: u32, t: f32) -> u32 {
    let t = t.clamp(0., 1.);
    let channel = |shift: u32| {
        let a = ((under >> shift) & 0xff) as f32;
        let b = ((over >> shift) & 0xff) as f32;
        (a + (b - a) * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

/// Selected text shares the resolved theme accent, including custom and
/// wallpaper accents. Keep the text's own foreground visible through the fill.
/// This opacity is independent of window surface transparency.
pub fn text_selection_color(palette: Palette) -> Rgba {
    gpui_kit::rgb(palette.accent).opacity(if palette.dark { 0.24 } else { 0.20 })
}

#[cfg(test)]
mod text_selection_tests {
    use super::*;
    fn luminance(color: Rgba) -> f32 {
        fn linear(channel: f32) -> f32 {
            if channel <= 0.04045 { channel / 12.92 }
            else { ((channel + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }
    fn contrast(ink: Rgba, fill: Rgba, surface: Rgba) -> f32 {
        let selected = Rgba {
            r: fill.r * fill.a + surface.r * (1. - fill.a),
            g: fill.g * fill.a + surface.g * (1. - fill.a),
            b: fill.b * fill.a + surface.b * (1. - fill.a), a: 1.,
        };
        let a = luminance(ink); let b = luminance(selected);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn selection_preserves_readable_theme_ink_across_preset_surfaces() {
        for (name, light, dark) in PRESETS {
            for palette in [*light, *dark] {
                let fill = text_selection_color(palette);
                for surface in [palette.paper, palette.card, palette.code] {
                    assert!(contrast(gpui_kit::rgb(palette.ink), fill, gpui_kit::rgb(surface)) >= 3.,
                        "{name} selected text must remain readable (dark={})", palette.dark);
                }
            }
        }
    }
    #[test]
    fn selection_tracks_custom_accent_in_both_appearances_without_changing_ink() {
        for theme in ["light", "dark"] {
            let blue = palette(&serde_json::json!({"theme":theme,"accent":"#6594db"}), WindowAppearance::Light);
            let purple = palette(&serde_json::json!({"theme":theme,"accent":"#a78bfa"}), WindowAppearance::Light);
            let blue_fill = text_selection_color(blue); let purple_fill = text_selection_color(purple);
            assert_ne!(blue_fill, purple_fill);
            assert_eq!(blue.ink, purple.ink);
            assert!((0.20..=0.30).contains(&purple_fill.a));
            assert!(contrast(gpui_kit::rgb(purple.ink), purple_fill, gpui_kit::rgb(purple.card)) >= 4.5);
        }
    }
}

const fn light(
    accent: u32,
    paper: u32,
    sidebar: u32,
    panel: u32,
    line: u32,
    soft: u32,
    code: u32,
    ink: u32,
    muted: u32,
) -> Palette {
    Palette {
        dark: false,
        accent,
        paper,
        sidebar,
        panel,
        card: panel,
        line,
        soft,
        code,
        ink,
        muted,
    }
}

const fn dark(
    accent: u32,
    paper: u32,
    sidebar: u32,
    panel: u32,
    line: u32,
    soft: u32,
    code: u32,
    ink: u32,
    muted: u32,
) -> Palette {
    Palette {
        dark: true,
        accent,
        paper,
        sidebar,
        panel,
        card: panel,
        line,
        soft,
        code,
        ink,
        muted,
    }
}

// Presets mirror `appThemes` in src/lib/app-theme.ts. Tauri-side name casing
// is preserved here so a future settings bridge can pass the id verbatim.
pub const PRESETS: &[(&str, Palette, Palette)] = &[
    (
        "monitter",
        light(
            0x00a8f0, 0xf7f7f7, 0xf0f0f0, 0xffffff, 0xd8d8d8, 0xeeeeee, 0xe8e8e8, 0x252525,
            0x747474,
        ),
        dark(
            0x00a8f0, 0x151515, 0x0f0f0f, 0x202020, 0x3d3d3d, 0x272727, 0x292929, 0xeeeeee,
            0xa4a4a4,
        ),
    ),
    (
        "catppuccin",
        light(
            0x1e66f5, 0xeff1f5, 0xe6e9ef, 0xffffff, 0xbcc0cc, 0xe6e9ef, 0xdce0e8, 0x4c4f69,
            0x6c6f85,
        ),
        dark(
            0x89b4fa, 0x1e1e2e, 0x181825, 0x313244, 0x45475a, 0x29293b, 0x313244, 0xcdd6f4,
            0xa6adc8,
        ),
    ),
    (
        "dracula",
        light(
            0x7c4dbd, 0xf8f8f2, 0xeeeff4, 0xffffff, 0xc7c8d1, 0xededf3, 0xe4e4eb, 0x282a36,
            0x62657a,
        ),
        dark(
            0xbd93f9, 0x282a36, 0x21222c, 0x343746, 0x44475a, 0x30323f, 0x343746, 0xf8f8f2,
            0xaab0c5,
        ),
    ),
    (
        "gruvbox",
        light(
            0x79740e, 0xfbf1c7, 0xf2e5bc, 0xfff8d8, 0xd5c4a1, 0xebdbb2, 0xe6d5ae, 0x3c3836,
            0x665c54,
        ),
        dark(
            0xb8bb26, 0x282828, 0x1d2021, 0x32302f, 0x504945, 0x3c3836, 0x3c3836, 0xebdbb2,
            0xa89984,
        ),
    ),
    (
        "nord",
        light(
            0x5e81ac, 0xeceff4, 0xe5e9f0, 0xffffff, 0xd8dee9, 0xe5e9f0, 0xd8dee9, 0x2e3440,
            0x4c566a,
        ),
        dark(
            0x88c0d0, 0x2e3440, 0x242933, 0x3b4252, 0x4c566a, 0x353c4a, 0x3b4252, 0xeceff4,
            0xd8dee9,
        ),
    ),
    (
        "one",
        light(
            0x4078f2, 0xfafafa, 0xf0f0f0, 0xffffff, 0xd3d3d3, 0xeaeaeb, 0xe5e5e6, 0x383a42,
            0x696c77,
        ),
        dark(
            0x61afef, 0x21252b, 0x181a1f, 0x282c34, 0x3e4451, 0x2c313c, 0x2c313c, 0xabb2bf,
            0x8b919d,
        ),
    ),
    (
        "solarized",
        light(
            0x268bd2, 0xfdf6e3, 0xeee8d5, 0xfffbed, 0xd6ceba, 0xeee8d5, 0xe7e0cb, 0x586e75,
            0x657b83,
        ),
        dark(
            0x2aa198, 0x002b36, 0x001e27, 0x073642, 0x586e75, 0x073642, 0x073642, 0x93a1a1,
            0x839496,
        ),
    ),
    (
        "tokyo-night",
        light(
            0x2e7de9, 0xe1e2e7, 0xd5d6db, 0xf1f2f4, 0xb4b5b9, 0xdcdde2, 0xd5d6db, 0x343b58,
            0x6172b0,
        ),
        dark(
            0x7aa2f7, 0x1a1b26, 0x16161e, 0x24283b, 0x414868, 0x292e42, 0x24283b, 0xc0caf5,
            0xa9b1d6,
        ),
    ),
    (
        "wombat",
        light(
            0x5c7a29, 0xf2efe8, 0xe8e4dc, 0xfaf8f2, 0xcbc5ba, 0xe2ddd4, 0xd9d4cb, 0x3a3834,
            0x6f6b64,
        ),
        dark(
            0xb1e969, 0x171717, 0x101010, 0x242424, 0x444444, 0x2d2d2d, 0x2d2d2d, 0xdedacf,
            0xaaa69e,
        ),
    ),
    (
        "xterm",
        light(
            0x0000ee, 0xffffff, 0xf3f3f3, 0xffffff, 0xcccccc, 0xebebeb, 0xe3e3e3, 0x000000,
            0x555555,
        ),
        dark(
            0x5c5cff, 0x000000, 0x080808, 0x111111, 0x444444, 0x1a1a1a, 0x222222, 0xffffff,
            0xbebebe,
        ),
    ),
];

// Static warm-paper reference, separate from the live Monitter preset.
fn reference_palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark: true,
            paper: 0x1a1814,
            sidebar: 0x141210,
            panel: 0x211e19,
            card: 0x211e19,
            line: 0x3a352d,
            soft: 0x25211b,
            code: 0x2a251f,
            ink: 0xf2ede3,
            muted: 0x9a8f7d,
            accent: 0x28764d,
        }
    } else {
        Palette {
            dark: false,
            paper: 0xfbf8f2,
            sidebar: 0xf2ede3,
            panel: 0xffffff,
            card: 0xffffff,
            line: 0xded8ce,
            soft: 0xf2ede3,
            code: 0xf5f1e8,
            ink: 0x332e27,
            muted: 0x736958,
            accent: 0x28764d,
        }
    }
}

fn color(settings: &Value, key: &str, fallback: u32) -> u32 {
    field(settings, key)
        .strip_prefix('#')
        .and_then(|color| u32::from_str_radix(color, 16).ok())
        .unwrap_or(fallback)
}

pub fn palette(settings: &Value, appearance: WindowAppearance) -> Palette {
    palette_for(settings, appearance, "monitter")
}

pub fn palette_for(settings: &Value, appearance: WindowAppearance, preset: &str) -> Palette {
    let is_dark = match field(settings, "theme").as_str() {
        "dark" => true,
        "light" => false,
        _ => matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    };
    let mut p = if preset == "reference" {
        reference_palette(is_dark)
    } else {
        let mut resolved = PRESETS
            .iter()
            .find(|(id, _, _)| *id == preset)
            .map(|(_, light, dark)| if is_dark { *dark } else { *light })
            .unwrap_or_else(|| {
                let monitter = PRESETS[0];
                if is_dark { monitter.2 } else { monitter.1 }
            });
        resolved.dark = is_dark;
        resolved
    };
    p.accent = color(settings, "accent", p.accent);
    p.card = p.panel;
    p
}

pub fn parse_color(value: &str, fallback: u32) -> u32 {
    value
        .strip_prefix('#')
        .and_then(|color| u32::from_str_radix(color, 16).ok())
        .unwrap_or(fallback)
}

// Status colors mirror Tauri sidebar dots / timeline palette: running=accent,
// error=red, completed=green, interrupted=amber, else=muted.
pub fn status_color(status: &str, p: Palette) -> u32 {
    match status {
        "running" => p.accent,
        "error" => 0xbd655b,
        "completed" => 0x4f8f63,
        "interrupted" => 0xb38b45,
        _ => p.muted,
    }
}
