//! Native terminal palette selection, kept private to the experimental GPUI
//! profile. This mirrors the client-local Tauri selector without adding a
//! field to the authoritative Settings snapshot.

use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

const PREFERENCE_FILE: &str = "terminal-theme.v1";
const MAX_PREFERENCE_BYTES: usize = 64;
const DEFAULT_INDEX: usize = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalTheme {
    pub id: &'static str,
    pub label: &'static str,
    /// Index into `terminal.rs::PALETTES`; the catalog order is intentionally
    /// identical to that native renderer table and Tauri `terminalThemes`.
    pub palette_index: usize,
}

#[derive(Clone, Copy)]
pub struct Selection(pub usize);
impl gpui_kit::Global for Selection {}

pub const THEMES: [TerminalTheme; 10] = [
    TerminalTheme { id: "monitter", label: "Monitter", palette_index: 0 },
    TerminalTheme { id: "catppuccin-mocha", label: "Catppuccin Mocha", palette_index: 1 },
    TerminalTheme { id: "dracula", label: "Dracula", palette_index: 2 },
    TerminalTheme { id: "gruvbox-dark", label: "Gruvbox Dark", palette_index: 3 },
    TerminalTheme { id: "molokai", label: "Molokai", palette_index: 4 },
    TerminalTheme { id: "nord", label: "Nord", palette_index: 5 },
    TerminalTheme { id: "one-dark", label: "One Dark", palette_index: 6 },
    TerminalTheme { id: "solarized-dark", label: "Solarized Dark", palette_index: 7 },
    TerminalTheme { id: "wombat", label: "Wombat", palette_index: 8 },
    TerminalTheme { id: "xterm", label: "XTerm", palette_index: 9 },
];

static CURRENT: OnceLock<AtomicUsize> = OnceLock::new();
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

fn index_for_id(id: &str) -> Option<usize> {
    THEMES.iter().position(|theme| theme.id == id)
}

fn parse_saved_theme(bytes: &[u8]) -> usize {
    if bytes.len() > MAX_PREFERENCE_BYTES {
        return DEFAULT_INDEX;
    }
    let Ok(value) = std::str::from_utf8(bytes) else {
        return DEFAULT_INDEX;
    };
    index_for_id(value.trim()).unwrap_or(DEFAULT_INDEX)
}

fn read_saved_theme() -> usize {
    let Ok(directory) = monitter_lib::experimental_desktop::profile_dir() else {
        return DEFAULT_INDEX;
    };
    let path = directory.join(PREFERENCE_FILE);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return DEFAULT_INDEX;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_PREFERENCE_BYTES as u64 {
        return DEFAULT_INDEX;
    }
    std::fs::read(path)
        .map(|bytes| parse_saved_theme(&bytes))
        .unwrap_or(DEFAULT_INDEX)
}

fn selected_index() -> usize {
    CURRENT
        .get_or_init(|| AtomicUsize::new(read_saved_theme()))
        .load(Ordering::Acquire)
}

/// The selected native theme. Reads the tiny GUI-local preference file only
/// once, on first access; subsequent reads use the in-memory value.
pub fn current() -> &'static TerminalTheme {
    &THEMES[selected_index()]
}

/// Resolve the Tauri-compatible theme identifier to a native renderer index.
/// Unknown identifiers safely fall back to the Monitter palette.
pub fn palette_index_for(id: &str) -> usize {
    index_for_id(id).unwrap_or(DEFAULT_INDEX)
}

/// Resolve a native renderer index to the catalog entry, with Monitter as the
/// fallback for stale or invalid indexes.
pub fn theme_for_palette_index(index: usize) -> &'static TerminalTheme {
    THEMES.get(index).unwrap_or(&THEMES[DEFAULT_INDEX])
}

/// Change the GUI-local selection immediately and queue an atomic preference
/// write. Returns `None` for an unknown ID and never blocks the calling UI.
pub fn choose(id: &str) -> Option<&'static TerminalTheme> {
    let index = index_for_id(id)?;
    CURRENT
        .get_or_init(|| AtomicUsize::new(read_saved_theme()))
        .store(index, Ordering::Release);

    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let id = THEMES[index].id;
    let _ = thread::Builder::new()
        .name("gpui-terminal-theme-preference".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let Ok(_guard) = SAVE_LOCK.lock() else { return; };
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
            let written = file.write_all(format!("{id}\n").as_bytes()).is_ok()
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

    Some(&THEMES[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn catalog_matches_native_palette_order_and_tauri_ids() {
        let ids: Vec<_> = THEMES.iter().map(|theme| theme.id).collect();
        assert_eq!(
            ids,
            [
                "monitter",
                "catppuccin-mocha",
                "dracula",
                "gruvbox-dark",
                "molokai",
                "nord",
                "one-dark",
                "solarized-dark",
                "wombat",
                "xterm",
            ]
        );
        for (index, theme) in THEMES.iter().enumerate() {
            assert_eq!(theme.palette_index, index);
            assert_eq!(palette_index_for(theme.id), index);
            assert_eq!(theme_for_palette_index(index).id, theme.id);
        }
    }

    #[::core::prelude::v1::test]
    fn invalid_or_oversized_saved_choice_uses_monitter() {
        assert_eq!(parse_saved_theme(b"not-a-theme\n"), DEFAULT_INDEX);
        assert_eq!(parse_saved_theme(&vec![b'x'; MAX_PREFERENCE_BYTES + 1]), DEFAULT_INDEX);
        assert_eq!(palette_index_for("unknown"), DEFAULT_INDEX);
        assert_eq!(theme_for_palette_index(usize::MAX).id, "monitter");
    }
}
