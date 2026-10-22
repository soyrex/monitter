//! Device-local area preferences, separated from palette selection so themes can
//! supply area defaults later. Surface fills and explicitly marked separator borders carry independent modes.
use gpui_kit::*;
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Workspace,
    Sidebar,
    Pane,
    Bubbles,
    Composer,
    Separators,
}
impl Area {
    pub const ALL: [Self; 6] = [Self::Workspace, Self::Sidebar, Self::Pane, Self::Bubbles, Self::Composer, Self::Separators];
    pub fn id(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Sidebar => "sidebar",
            Self::Pane => "pane",
            Self::Bubbles => "bubbles",
            Self::Composer => "composer",
            Self::Separators => "separators",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Sidebar => "Sidebar",
            Self::Pane => "Chat panes",
            Self::Bubbles => "Chat bubbles",
            Self::Composer => "Message composer",
            Self::Separators => "Separator lines",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Screen,
    Multiply,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Screen, Self::Multiply];
    pub fn id(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Screen => "screen",
            Self::Multiply => "multiply",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Screen => "Screen",
            Self::Multiply => "Multiply",
        }
    }
    fn parse(value: &str) -> Self {
        match value {
            "screen" => Self::Screen,
            "multiply" => Self::Multiply,
            _ => Self::Normal,
        }
    }
    pub(crate) fn shader(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Screen => 1,
            Self::Multiply => 2,
        }
    }
}
static THEME_DARK: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_theme_dark(dark: bool) {
    THEME_DARK.store(dark, Ordering::Relaxed);
}

fn effective_mode(mode: Mode, dark: bool) -> Mode {
    if !dark && mode == Mode::Multiply { Mode::Screen } else { mode }
}

/// Resolve surface blending for the current theme without rewriting preferences.
pub(crate) fn effective_shader(area: Area) -> u32 {
    let mode = current(area);
    if area == Area::Separators { mode.shader() }
    else { effective_mode(mode, THEME_DARK.load(Ordering::Relaxed)).shader() }
}

const FILE: &str = "surface-blend.json";
static STATE: OnceLock<Mutex<Value>> = OnceLock::new();
/// Install fixture defaults before any surface is rendered, without profile IO.
#[cfg(feature = "performance-profiling")]
pub(crate) fn initialize_for_ui_fixture() {
    let _ = STATE.set(Mutex::new(json!({})));
}
fn preferences() -> &'static Mutex<Value> {
    STATE.get_or_init(|| {
        Mutex::new(
            monitter_lib::experimental_desktop::profile_dir()
                .ok()
                .and_then(|dir| std::fs::read(dir.join(FILE)).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({})),
        )
    })
}
pub fn current(area: Area) -> Mode {
    preferences()
        .lock()
        .ok()
        .map(|value| Mode::parse(value["areas"][area.id()].as_str().unwrap_or("normal")))
        .unwrap_or(Mode::Normal)
}
pub fn set(area: Area, mode: Mode) -> Result<(), String> {
    let mut value = preferences()
        .lock()
        .map_err(|_| "Surface preference lock is unavailable".to_owned())?;
    let mut next = value.clone();
    next["version"] = json!(1);
    next["areas"][area.id()] = json!(mode.id());
    let directory = monitter_lib::experimental_desktop::profile_dir().map_err(|e| e.to_string())?;
    let temporary = directory.join(format!(".{FILE}.{}.tmp", std::process::id()));
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, directory.join(FILE)).map_err(|e| e.to_string())?;
    *value = next;
    Ok(())
}
// Sidebar surfaces share one tint throughout their frame and selected tabs.
// Darken RGB only, retaining the user's surface opacity and blend preference.
fn area_tint(area: Area, color: u32) -> u32 {
    if area != Area::Sidebar { return color; }
    let channel = |shift: u32| (((color >> shift) & 0xff_u32) as f32 * 0.95).round() as u32;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
pub fn background(area: Area, color: u32) -> Background {
    let mut fill: Background = crate::design::surface_color(area_tint(area, color)).into();
    fill.blend_mode = effective_shader(area);
    fill
}

/// Tint pane RGB without changing transparency or adding another surface layer.
pub fn tinted_pane_background(color: u32, dark: bool, amount: f32) -> Background {
    let mut fill = background(Area::Pane, pane_tint(color, dark, amount));
    // A fully opaque pane has no backdrop to blend with. Multiply makes a
    // dark theme's paper nearly black, while Screen washes out light themes.
    // Keep the saved blend preference for translucent and glass surfaces.
    if fill.as_solid().is_some_and(|color| color.a >= 0.999) {
        fill.blend_mode = Mode::Normal.shader();
    }
    fill
}

fn pane_tint(color: u32, dark: bool, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 255) as f32;
        if dark { (value * (1. - amount)).round() as u32 }
        else { (value + (255. - value) * amount).round() as u32 }
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

pub fn raised_background(area: Area, color: u32) -> Background {
    let mut fill: Background = crate::design::raised_surface_color(color).into();
    fill.blend_mode = effective_shader(area);
    fill
}

/// Working panes share an equally strong dark or light composer and Git tint.
/// RGB changes preserve the user's transparency and blend preferences.
pub(crate) fn working_composer_color(color: u32, dark: bool) -> u32 {
    theme_tint_color(color, dark, 0.52)
}

/// Floating controls mirror their multiply tint with Screen in light themes.
pub(crate) fn floating_control_background(color: u32, dark: bool) -> Background {
    floating_control_tint(color, dark, 0.28)
}
pub(crate) fn floating_control_hover_background(color: u32, dark: bool) -> Background {
    floating_control_tint(color, dark, 0.50)
}
fn floating_control_tint(color: u32, dark: bool, amount: f32) -> Background {
    let tint = theme_tint_color(color, dark, amount);
    let mut fill: Background = crate::design::raised_surface_color(tint).into();
    fill.blend_mode = if dark { Mode::Multiply } else { Mode::Screen }.shader();
    fill.backdrop_blur(20.)
}

/// Independent border setting; callers opt in separator rails and dividers.
pub(crate) fn separator_mode() -> u32 { current(Area::Separators).shader() }
pub(crate) fn separator(color: u32) -> Background {
    let mut line: Background = gpui::rgb(color).into();
    line.blend_mode = separator_mode();
    line
}

/// The pane already paints beneath both fixed and sliding detail rails.
/// Tint that resolved surface once, rather than compositing its fill again.
pub(crate) fn secondary_sidebar_background(dark: bool) -> Background {
    let mut fill: Background = gpui_kit::rgb(if dark { 0x000000 } else { 0xffffff }).opacity(0.25).into();
    fill.blend_mode = if dark { Mode::Multiply } else { Mode::Screen }.shader();
    fill
}

/// Black Multiply and white Screen have mirrored channel distances.
pub(crate) fn theme_tint_color(color: u32, dark: bool, amount: f32) -> u32 {
    pane_tint(color, dark, amount)
}

/// Git controls are five percentage points more transparent than the input.
pub(crate) fn git_surface_color(color: u32) -> Rgba {
    let mut fill = crate::design::raised_surface_color(color);
    fill.a = (fill.a - 0.05).max(0.);
    fill
}

#[cfg(test)]
mod theme_tint_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn multiply_surfaces_use_screen_in_light_themes() {
        assert_eq!(effective_mode(Mode::Multiply, false), Mode::Screen);
        assert_eq!(effective_mode(Mode::Multiply, true), Mode::Multiply);
        assert_eq!(effective_mode(Mode::Normal, false), Mode::Normal);
        assert_eq!(effective_mode(Mode::Screen, false), Mode::Screen);
    }

    #[::core::prelude::v1::test]
    fn theme_tints_mirror_distance_to_black_and_white() {
        assert_eq!(theme_tint_color(0x6496c8, true, 0.10), 0x5a87b4);
        assert_eq!(theme_tint_color(0x6496c8, false, 0.10), 0x74a1ce);
        assert_eq!(theme_tint_color(0x000000, false, 0.25), 0x404040);
        assert_eq!(theme_tint_color(0xffffff, true, 0.25), 0xbfbfbf);
    }

    #[::core::prelude::v1::test]
    fn working_composer_tint_uses_the_same_strength_in_both_themes() {
        assert_eq!(working_composer_color(0xffffff, true), 0x7a7a7a);
        assert_eq!(working_composer_color(0x000000, false), 0x858585);
    }
}
