//! Device-local area preferences, separated from palette selection so themes can
//! supply area defaults later. Surface fills and explicitly marked separator borders carry independent modes.
use gpui_kit::*;
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};

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
    fill.blend_mode = current(area).shader();
    fill
}

pub fn raised_background(area: Area, color: u32) -> Background {
    let mut fill: Background = crate::design::raised_surface_color(color).into();
    fill.blend_mode = current(area).shader();
    fill
}

/// Floating chat controls blur the content under a darker multiply fill.
pub(crate) fn floating_control_background(color: u32) -> Background {
    floating_control_tint(color, 0.72)
}
pub(crate) fn floating_control_hover_background(color: u32) -> Background {
    floating_control_tint(color, 0.50)
}
fn floating_control_tint(color: u32, factor: f32) -> Background {
    let channel = |shift: u32| (((color >> shift) & 0xff) as f32 * factor).round() as u32;
    let tint = (channel(16) << 16) | (channel(8) << 8) | channel(0);
    let mut fill: Background = crate::design::raised_surface_color(tint).into();
    fill.blend_mode = Mode::Multiply.shader();
    fill.backdrop_blur(20.)
}

/// Independent border setting; callers opt in separator rails and dividers.
pub(crate) fn separator_mode() -> u32 { current(Area::Separators).shader() }
pub(crate) fn separator(color: u32) -> Background {
    let mut line: Background = gpui::rgb(color).into();
    line.blend_mode = separator_mode();
    line
}
