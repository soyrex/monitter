//! Native motion preference and activity-animation policy for the experimental profile.
//!
//! GPUI Base observes the operating system on its supported UI path. We retain
//! its latest reading atomically so rendering never performs AppKit or preference
//! file I/O, and apply the selected mode to both GPUI and activity effects.

use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread,
};

use gpui_kit::{App, Global, Subscription};

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
static SYSTEM_REDUCED: AtomicBool = AtomicBool::new(false);
struct SystemMotionObservation {
    _subscription: Subscription,
}
impl Global for SystemMotionObservation {}
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
/// Off always disables motion, and System/Subtle follow the latest OS value.
pub fn motion_allowed() -> bool {
    allowed_for(selected(), SYSTEM_REDUCED.load(Ordering::Acquire))
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
    if !motion_allowed() { 0.0 } else { 1.0 }
}

/// Call after `gpui_kit::init`. Load the GUI profile and follow Base's raw OS
/// preference independently of the explicit On/Off override.
pub fn initialize(cx: &mut App) {
    gpui_kit::base::apply_system_reduce_motion(cx);
    let system_reduced = cx
        .try_global::<gpui_kit::base::SystemMotionPreference>()
        .map_or(cx.reduce_motion(), |value| value.0);
    SYSTEM_REDUCED.store(system_reduced, Ordering::Release);
    let mode = selected();
    apply_gpui_policy(mode, cx);
    cx.set_global(MotionSelection(mode));
    observe_system_motion(cx);
}

fn observe_system_motion(cx: &mut App) {
    if cx.try_global::<SystemMotionObservation>().is_none() {
        let subscription = cx.observe_global::<gpui_kit::base::SystemMotionPreference>(|cx| {
            let reduced = cx.global::<gpui_kit::base::SystemMotionPreference>().0;
            let previous = SYSTEM_REDUCED.swap(reduced, Ordering::AcqRel);
            let mode = selected();
            apply_gpui_policy(mode, cx);
            if allowed_for(mode, previous) != allowed_for(mode, reduced) {
                // Consumers can settle retained transitions on this notification.
                cx.set_global(MotionSelection(mode));
                crate::activity_effects::sync_motion(cx);
            }
        });
        cx.set_global(SystemMotionObservation {
            _subscription: subscription,
        });
    }
}

/// Deterministic, in-memory motion policy for the isolated native fixture.
#[cfg(feature = "performance-profiling")]
pub(crate) fn initialize_for_ui_fixture(cx: &mut App) {
    let mode = MotionPreference::On;
    CURRENT
        .get_or_init(|| AtomicU8::new(mode as u8))
        .store(mode as u8, Ordering::Release);
    set_preference_for_ui_fixture(mode, cx);
    observe_system_motion(cx);
}

/// Fixture-only policy changes never persist a profile preference.
#[cfg(feature = "performance-profiling")]
pub(crate) fn set_preference_for_ui_fixture(mode: MotionPreference, cx: &mut App) {
    CURRENT
        .get_or_init(|| AtomicU8::new(mode as u8))
        .store(mode as u8, Ordering::Release);
    let system = cx
        .try_global::<gpui_kit::base::SystemMotionPreference>()
        .map_or(false, |v| v.0);
    SYSTEM_REDUCED.store(system, Ordering::Release);
    apply_gpui_policy(mode, cx);
    cx.set_global(MotionSelection(mode));
    crate::activity_effects::sync_motion(cx);
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
    gpui_kit::base::set_system_reduce_motion_override(
        match mode {
            MotionPreference::On => Some(false),
            MotionPreference::Off => Some(true),
            MotionPreference::System | MotionPreference::Subtle => None,
        },
        cx,
    );
    // Deterministic fixtures and unsupported platforms may have no OS global.
    cx.set_reduce_motion(!allowed_for(mode, SYSTEM_REDUCED.load(Ordering::Acquire)));
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
            let written = file
                .write_all(format!("{}\n", mode.id()).as_bytes())
                .is_ok()
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
    Liquid,
    Grid,
    Matrix,
}

impl ActivityAnimation {
    pub const ALL: [Self; 4] = [Self::Sparkles, Self::Liquid, Self::Grid, Self::Matrix];

    /// Parse a snapshot setting; unknown or absent values use Tauri's default.
    pub fn from_snapshot(value: Option<&str>) -> Self {
        match value {
            Some("liquid") => Self::Liquid,
            Some("grid") => Self::Grid,
            Some("matrix") => Self::Matrix,
            _ => Self::Sparkles,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Sparkles => "sparkles",
            Self::Liquid => "liquid",
            Self::Grid => "grid",
            Self::Matrix => "matrix",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sparkles => "Sparkles",
            Self::Liquid => "Liquid wave",
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
        assert_eq!(
            MotionPreference::ALL.map(MotionPreference::id),
            ["system", "subtle", "on", "off"]
        );
        for preference in MotionPreference::ALL {
            assert_eq!(MotionPreference::parse(preference.id()), Some(preference));
        }
        assert_eq!(
            ActivityAnimation::from_snapshot(Some("liquid")),
            ActivityAnimation::Liquid
        );
        assert_eq!(
            ActivityAnimation::from_snapshot(Some("grid")),
            ActivityAnimation::Grid
        );
        assert_eq!(
            ActivityAnimation::from_snapshot(Some("matrix")),
            ActivityAnimation::Matrix
        );
        assert_eq!(
            ActivityAnimation::from_snapshot(Some("unknown")),
            ActivityAnimation::Sparkles
        );
        assert_eq!(
            ActivityAnimation::from_snapshot(None),
            ActivityAnimation::Sparkles
        );
    }

    #[::core::prelude::v1::test]
    fn malformed_saved_motion_defaults_to_system() {
        assert_eq!(
            parse_saved_preference(b"not-a-policy\n"),
            MotionPreference::System
        );
        assert_eq!(
            parse_saved_preference(&[b'x'; MAX_PREFERENCE_BYTES + 1]),
            MotionPreference::System
        );
        assert_eq!(parse_saved_preference(b"off\n"), MotionPreference::Off);
    }

    #[::core::prelude::v1::test]
    fn live_os_changes_only_affect_following_modes() {
        for mode in MotionPreference::ALL {
            let readings = [false, true, true, false];
            let allowed = readings.map(|reduced| allowed_for(mode, reduced));
            match mode {
                MotionPreference::System | MotionPreference::Subtle => {
                    assert_eq!(allowed, [true, false, false, true])
                }
                MotionPreference::On => assert_eq!(allowed, [true; 4]),
                MotionPreference::Off => assert_eq!(allowed, [false; 4]),
            }
        }
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
