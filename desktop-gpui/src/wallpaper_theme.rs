//! Device-local wallpaper theme preferences.
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Manual,
    Highlight,
    Mood,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Manual, Self::Highlight, Self::Mood];
    pub fn id(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Highlight => "highlight",
            Self::Mood => "mood",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Highlight => "Wallpaper highlight",
            Self::Mood => "Wallpaper mood",
        }
    }
}
const FILE: &str = "wallpaper-accent.json";
fn preferences() -> &'static Mutex<Value> {
    static STATE: OnceLock<Mutex<Value>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(
            monitter_lib::experimental_desktop::profile_dir()
                .ok()
                .and_then(|dir| std::fs::read(dir.join(FILE)).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({"mode":"highlight","strength":70})),
        )
    })
}
pub fn mode() -> Mode {
    match preferences()
        .lock()
        .ok()
        .and_then(|v| v["mode"].as_str().map(str::to_owned))
        .as_deref()
    {
        Some("highlight") => Mode::Highlight,
        Some("mood") => Mode::Mood,
        _ => Mode::Manual,
    }
}
pub fn strength() -> u16 {
    preferences()
        .lock()
        .ok()
        .and_then(|v| v["strength"].as_u64())
        .unwrap_or(70)
        .min(100) as u16
}
pub fn set_mode(mode: Mode) -> Result<(), String> {
    save("mode", json!(mode.id()))
}
pub fn set_strength(strength: u16) -> Result<(), String> {
    save("strength", json!(strength.min(100)))
}
fn save(key: &str, value: Value) -> Result<(), String> {
    let mut prefs = preferences()
        .lock()
        .map_err(|_| "Wallpaper preferences are unavailable".to_owned())?;
    let mut next = prefs.clone();
    next[key] = value;
    let dir = monitter_lib::experimental_desktop::profile_dir().map_err(|e| e.to_string())?;
    let temp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, dir.join(FILE)).map_err(|e| e.to_string())?;
    *prefs = next;
    Ok(())
}
pub fn mix(base: u32, sampled: u32, strength: u16) -> u32 {
    let amount = u32::from(strength.min(100));
    let component = |shift: u32| {
        (((base >> shift) & 255_u32) * (100 - amount)
            + ((sampled >> shift) & 255_u32) * amount
            + 50)
            / 100
    };
    (component(16) << 16) | (component(8) << 8) | component(0)
}
#[cfg(test)]
mod tests {
    #[test]
    fn wallpaper_strength_endpoints_and_midpoint() {
        assert_eq!(super::mix(0x112233, 0xaabbcc, 0), 0x112233);
        assert_eq!(super::mix(0x112233, 0xaabbcc, 100), 0xaabbcc);
        assert_eq!(super::mix(0, 0xffffff, 50), 0x808080);
        assert_eq!(super::mix(0, 0xffffff, 250), 0xffffff);
    }
}
