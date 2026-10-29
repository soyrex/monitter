//! GUI-local composer auto-hide preference. No resident-backend upgrade is needed.
//! Rendering reads memory only; bounded profile I/O runs on the background executor.
use gpui_kit::{App, AppContext as _, Global};
use std::{
    fs::OpenOptions,
    io::{Read as _, Write as _},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const FILE: &str = "composer-autohide.v1";
const MAX_BYTES: u64 = 16;
static ENABLED: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    pub(crate) enabled: bool,
    pub(crate) error: Option<String>,
}
impl Global for Selection {}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

fn publish(enabled: bool, error: Option<String>, cx: &mut App) {
    ENABLED.store(enabled, Ordering::Release);
    cx.set_global(Selection { enabled, error });
}

fn load() -> Result<bool, String> {
    let path = monitter_lib::experimental_desktop::profile_dir()?.join(FILE);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Could not read composer auto-hide preference.".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("Composer auto-hide preference must be a small regular file.".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| "Could not read composer auto-hide preference.".to_owned())?;
    match bytes.as_slice() {
        b"on\n" | b"on" => Ok(true),
        b"off\n" | b"off" => Ok(false),
        _ => Err("Composer auto-hide preference is invalid; auto-hide is off.".into()),
    }
}

/// The absence of a saved selection always means off, including existing installs.
pub(crate) fn initialize(cx: &mut App) {
    publish(false, None, cx);
    let generation = GENERATION.load(Ordering::SeqCst);
    let read = cx.background_executor().spawn(async { load() });
    cx.spawn(async move |cx| {
        let result = read.await;
        let _ = cx.update(|cx| {
            // A toggle while loading wins over the older disk selection.
            if GENERATION.load(Ordering::SeqCst) == generation {
                match result {
                    Ok(enabled) => publish(enabled, None, cx),
                    Err(error) => publish(false, Some(error), cx),
                }
            }
        });
    })
    .detach();
}

pub(crate) fn set(enabled: bool, cx: &mut App) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    publish(enabled, None, cx);
    let write = cx.background_executor().spawn(async move {
        let _guard = SAVE_LOCK.lock().map_err(|_| "Could not save composer auto-hide preference.".to_owned())?;
        if GENERATION.load(Ordering::SeqCst) != generation { return Ok(()); }
        let directory = monitter_lib::experimental_desktop::profile_dir()?;
        let target = directory.join(FILE);
        let temp = directory.join(format!(".{FILE}.{}.{generation}.tmp", std::process::id()));
        let result = (|| {
            let mut open = OpenOptions::new();
            open.write(true).create_new(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt as _; open.mode(0o600); }
            let mut file = open.open(&temp)?;
            file.write_all(if enabled { b"on\n" } else { b"off\n" })?;
            file.sync_all()?;
            std::fs::rename(&temp, &target)
        })();
        if result.is_err() { let _ = std::fs::remove_file(temp); }
        result.map_err(|_| "Could not save composer auto-hide preference. Your selection applies until this app closes.".to_owned())
    });
    cx.spawn(async move |cx| {
        let result = write.await;
        let _ = cx.update(|cx| {
            if GENERATION.load(Ordering::SeqCst) == generation {
                publish(enabled, result.err(), cx);
            }
        });
    })
    .detach();
}
