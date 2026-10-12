//! GUI-local presentation preference, independent of agent execution and IPC.
use gpui_kit::{App, AppContext as _, Global};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const FILE: &str = "streaming-display.v1.json";
const MAX_BYTES: u64 = 256;
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    AsReceived,
    Typing,
}
impl Mode {
    pub const ALL: [Self; 2] = [Self::AsReceived, Self::Typing];
    pub fn id(self) -> &'static str {
        match self {
            Self::AsReceived => "as-received",
            Self::Typing => "typing",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::AsReceived => "As received",
            Self::Typing => "Typing",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub mode: Mode,
    pub characters_per_second: u16,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::AsReceived,
            characters_per_second: 600,
        }
    }
}
impl Options {
    pub fn interval(self) -> Duration {
        // Paint at a bounded cadence; reveal multiple sequential characters per
        // frame rather than limiting throughput to one UI callback per character.
        Duration::from_millis(16)
    }
    fn encode(self) -> Vec<u8> {
        serde_json::to_vec(
            &json!({"mode":self.mode.id(),"charactersPerSecond":self.characters_per_second}),
        )
        .unwrap()
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Streaming display preference is too large.".into());
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|_| "Streaming display preference is invalid.".to_owned())?;
        let mode = match value["mode"].as_str() {
            Some("as-received") => Mode::AsReceived,
            Some("typing") => Mode::Typing,
            _ => return Err("Unknown streaming display mode.".into()),
        };
        let speed = value["charactersPerSecond"]
            .as_u64()
            .filter(|value| (10..=3600).contains(value))
            .ok_or("Typing speed must be between 10 and 3600 characters per second.")?;
        let speed = match speed {
            20 => 120,
            45 => 600,
            90 => 1800,
            speed => speed,
        };
        Ok(Self {
            mode,
            characters_per_second: speed as u16,
        })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub options: Options,
    pub error: Option<String>,
}
impl Global for Selection {}
pub fn current(cx: &App) -> Options {
    cx.try_global::<Selection>()
        .map(|selection| selection.options)
        .unwrap_or_default()
}
fn load() -> Result<Options, String> {
    let path = monitter_lib::experimental_desktop::profile_dir()?.join(FILE);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Options::default()),
        Err(_) => return Err("Could not read streaming display preference.".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("Streaming display preference must be a small regular file.".into());
    }
    Options::decode(
        &std::fs::read(path)
            .map_err(|_| "Could not read streaming display preference.".to_owned())?,
    )
}
pub fn initialize(cx: &mut App) {
    let selection = match load() {
        Ok(options) => Selection {
            options,
            error: None,
        },
        Err(error) => Selection {
            options: Options::default(),
            error: Some(error),
        },
    };
    cx.set_global(selection);
}
pub fn set(options: Options, cx: &mut App) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    cx.set_global(Selection {
        options,
        error: None,
    });
    let write = cx.background_executor().spawn(async move {
        let _guard = SAVE_LOCK.lock().map_err(|_| "Could not save streaming display preference.".to_owned())?;
        if GENERATION.load(Ordering::SeqCst) != generation { return Ok(()); }
        let dir = monitter_lib::experimental_desktop::profile_dir()?;
        let target = dir.join(FILE);
        let temp = dir.join(format!(".{FILE}.{}.{generation}.tmp", std::process::id()));
        let result = (|| {
            let mut open = OpenOptions::new(); open.write(true).create_new(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; open.mode(0o600); }
            let mut file = open.open(&temp)?;
            file.write_all(&options.encode())?; file.sync_all()?;
            std::fs::rename(&temp, &target)
        })();
        if result.is_err() { let _ = std::fs::remove_file(temp); }
        result.map_err(|_| "Could not save streaming display preference. Your selection applies until this app closes.".to_owned())
    });
    cx.spawn(async move |cx| {
        let result = write.await;
        let _ = cx.update(|cx| {
            if GENERATION.load(Ordering::SeqCst) == generation {
                cx.set_global(Selection {
                    options,
                    error: result.err(),
                });
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_roundtrip_and_bound_speed() {
        for mode in Mode::ALL {
            let value = Options {
                mode,
                characters_per_second: 60,
            };
            assert_eq!(Options::decode(&value.encode()).unwrap(), value);
        }
        assert!(Options::decode(br#"{"mode":"typing","charactersPerSecond":0}"#).is_err());
        assert!(Options::decode(br#"{"mode":"typing","charactersPerSecond":3601}"#).is_err());
        assert!(Options::decode(br#"{"mode":"unknown","charactersPerSecond":45}"#).is_err());
        assert!(Options::decode(&vec![b' '; 257]).is_err());
        assert_eq!(Options::default().mode, Mode::AsReceived);
    }
    #[test]
    fn legacy_speed_choices_upgrade_to_fast_frame_based_reveal() {
        for (old, new) in [(20, 120), (45, 600), (90, 1800)] {
            let bytes =
                serde_json::to_vec(&json!({"mode":"typing","charactersPerSecond":old})).unwrap();
            let options = Options::decode(&bytes).unwrap();
            assert_eq!(options.characters_per_second, new);
            assert_eq!(options.interval(), Duration::from_millis(16));
        }
    }
}
