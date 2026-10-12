//! Native motion preference and activity-animation policy for the experimental profile.
//!
//! System motion is sampled from GPUI Kit's pinned `gpui-base` initialization,
//! which reads `NSWorkspace.accessibilityDisplayShouldReduceMotion` on macOS.
//! We retain that value for this process so rendering never performs AppKit or
//! preference-file I/O. Activity animation names are modeled here; the native
//! full-pane renderer remains a separate integration task.

use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, AtomicU8, Ordering},
    },
    thread,
};

use gpui_kit::{
    App, Global,
};

const PREFERENCE_FILE: &str = "native-motion.v1";
const MAX_PREFERENCE_BYTES: usize = 32;
const STACK_SIZE: usize = 8 * 1024 * 1024;

/// User-facing policy, matching the Tauri `motion.ts` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MotionPreference {
    System = 0,
    Subtle = 1,
    On = 2,
    Off = 3,
}

impl MotionPreference {
    pub const ALL: [Self; 4] = [Self::System, Self::Subtle, Self::On, Self::Off];

    pub const fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Subtle => "subtle",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Subtle => "Subtle",
            Self::On => "On",
            Self::Off => "Off",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "system" => Some(Self::System),
            "subtle" => Some(Self::Subtle),
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// GPUI global for views that prefer reading the selected mode from context.
#[derive(Clone, Copy, Debug)]
pub struct MotionSelection(pub MotionPreference);
impl Global for MotionSelection {}

static CURRENT: OnceLock<AtomicU8> = OnceLock::new();
static SYSTEM_REDUCED: OnceLock<bool> = OnceLock::new();
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

fn preference_from_byte(value: u8) -> MotionPreference {
    match value {
        1 => MotionPreference::Subtle,
        2 => MotionPreference::On,
        3 => MotionPreference::Off,
        _ => MotionPreference::System,
    }
}

fn parse_saved_preference(bytes: &[u8]) -> MotionPreference {
    if bytes.len() > MAX_PREFERENCE_BYTES {
        return MotionPreference::System;
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(MotionPreference::parse)
        .unwrap_or(MotionPreference::System)
}

fn read_saved_preference() -> MotionPreference {
    let Ok(directory) = monitter_lib::experimental_desktop::profile_dir() else {
        return MotionPreference::System;
    };
    let path = directory.join(PREFERENCE_FILE);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return MotionPreference::System;
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PREFERENCE_BYTES as u64
    {
        return MotionPreference::System;
    }
    std::fs::read(path)
        .map(|bytes| parse_saved_preference(&bytes))
        .unwrap_or(MotionPreference::System)
}

fn selected() -> MotionPreference {
    preference_from_byte(
        CURRENT
            .get_or_init(|| AtomicU8::new(read_saved_preference() as u8))
            .load(Ordering::Acquire),
    )
}

/// Current GUI-local preference. The small profile file is read only once.
pub fn current() -> MotionPreference {
    selected()
}

/// Effective reduced-motion policy, matching Tauri: On overrides the OS,
/// Off always disables motion, and System/Subtle follow the sampled OS value.
pub fn motion_allowed() -> bool {
    allowed_for(selected(), SYSTEM_REDUCED.get().copied().unwrap_or(false))
}

const fn allowed_for(mode: MotionPreference, system_reduced: bool) -> bool {
    match mode {
        MotionPreference::On => true,
        MotionPreference::Off => false,
        MotionPreference::System | MotionPreference::Subtle => !system_reduced,
    }
}

/// A visual intensity multiplier for consumers that support graded motion.
pub fn motion_scale() -> f32 {
    if !motion_allowed() {
        0.0
    } else {
        1.0
    }
}

/// Call after `gpui_kit::init`. It captures the system preference once, loads
/// this GUI profile's choice, updates GPUI's shared reduced-motion policy, and
/// publishes the chosen mode as a GPUI global.
pub fn initialize(cx: &mut App) {
    gpui_kit::base::apply_system_reduce_motion(cx);
    let system_reduced = cx.reduce_motion();
    let _ = SYSTEM_REDUCED.set(system_reduced);
    let mode = selected();
    apply_gpui_policy(mode, cx);
    cx.set_global(MotionSelection(mode));
}

/// Deterministic, in-memory motion policy for the isolated native fixture.
#[cfg(feature = "performance-profiling")]
pub(crate) fn initialize_for_ui_fixture(cx: &mut App) {
    let mode = MotionPreference::On;
    let _ = CURRENT.set(AtomicU8::new(mode as u8));
    let _ = SYSTEM_REDUCED.set(false);
    apply_gpui_policy(mode, cx);
    cx.set_global(MotionSelection(mode));
}

/// Select a preference, apply it to GPUI immediately, and queue an atomic
/// write to the experimental profile. No backend settings or snapshots change.
pub fn set_preference(mode: MotionPreference, cx: &mut App) {
    let current = CURRENT.get_or_init(|| AtomicU8::new(read_saved_preference() as u8));
    current.store(mode as u8, Ordering::Release);
    apply_gpui_policy(mode, cx);
    cx.set_global(MotionSelection(mode));
    crate::activity_effects::sync_motion(cx);
    save_async(mode);
}

fn apply_gpui_policy(mode: MotionPreference, cx: &mut App) {
    let system_reduced = SYSTEM_REDUCED.get().copied().unwrap_or_else(|| cx.reduce_motion());
    cx.set_reduce_motion(match mode {
        MotionPreference::On => false,
        MotionPreference::Off => true,
        MotionPreference::System | MotionPreference::Subtle => system_reduced,
    });
}

fn save_async(mode: MotionPreference) {
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = thread::Builder::new()
        .name("gpui-native-motion-preference".into())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let Ok(_guard) = SAVE_LOCK.lock() else { return };
            if SAVE_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            let Ok(directory) = monitter_lib::experimental_desktop::profile_dir() else {
                return;
            };
            let temp = directory.join(format!(
                ".{PREFERENCE_FILE}.{}.{}.tmp",
                std::process::id(),
                generation
            ));
            let target = directory.join(PREFERENCE_FILE);
            let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&temp) else {
                return;
            };
            let written = file.write_all(format!("{}\n", mode.id()).as_bytes()).is_ok()
                && file.sync_all().is_ok();
            drop(file);
            if written {
                if std::fs::rename(&temp, target).is_err() {
                    let _ = std::fs::remove_file(temp);
                }
            } else {
                let _ = std::fs::remove_file(temp);
            }
        });
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityAnimation {
    #[default]
    Sparkles,
    Grid,
    Matrix,
}

impl ActivityAnimation {
    /// Parse a snapshot setting; unknown or absent values use Tauri's default.
    pub fn from_snapshot(value: Option<&str>) -> Self {
        match value {
            Some("grid") => Self::Grid,
            Some("matrix") => Self::Matrix,
            _ => Self::Sparkles,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Sparkles => "sparkles",
            Self::Grid => "grid",
            Self::Matrix => "matrix",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sparkles => "Sparkles",
            Self::Grid => "Pulse grid",
            Self::Matrix => "Letter stream",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn preference_values_and_snapshot_animation_names_match_tauri() {
        assert_eq!(MotionPreference::ALL.map(MotionPreference::id), ["system", "subtle", "on", "off"]);
        for preference in MotionPreference::ALL {
            assert_eq!(MotionPreference::parse(preference.id()), Some(preference));
        }
        assert_eq!(ActivityAnimation::from_snapshot(Some("grid")), ActivityAnimation::Grid);
        assert_eq!(ActivityAnimation::from_snapshot(Some("matrix")), ActivityAnimation::Matrix);
        assert_eq!(ActivityAnimation::from_snapshot(Some("unknown")), ActivityAnimation::Sparkles);
        assert_eq!(ActivityAnimation::from_snapshot(None), ActivityAnimation::Sparkles);
    }

    #[::core::prelude::v1::test]
    fn malformed_saved_motion_defaults_to_system() {
        assert_eq!(parse_saved_preference(b"not-a-policy\n"), MotionPreference::System);
        assert_eq!(parse_saved_preference(&[b'x'; MAX_PREFERENCE_BYTES + 1]), MotionPreference::System);
        assert_eq!(parse_saved_preference(b"off\n"), MotionPreference::Off);
    }

    #[::core::prelude::v1::test]
    fn system_and_subtle_follow_accessibility_but_on_and_off_override() {
        for mode in [MotionPreference::System, MotionPreference::Subtle] {
            assert!(allowed_for(mode, false));
            assert!(!allowed_for(mode, true));
        }
        assert!(allowed_for(MotionPreference::On, true));
        assert!(!allowed_for(MotionPreference::Off, false));
    }
}
