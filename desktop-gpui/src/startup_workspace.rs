//! Content-free monotonic startup milestones. The baseline is main entry;
//! external launch-to-window probes additionally cover dyld/OS startup time.
use std::{sync::{Mutex, OnceLock}, time::{Duration, Instant}};

static STARTED: OnceLock<Instant> = OnceLock::new();
static STARTED_WALL: OnceLock<u128> = OnceLock::new();
static STAGES: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());
static SPANS: Mutex<Vec<(&'static str, Option<Duration>)>> = Mutex::new(Vec::new());

pub fn nested_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MONITTER_STARTUP_NESTED_TIMING").is_some())
}

pub fn record_span(phase: &'static str, duration: Option<Duration>) {
    if !nested_enabled() { return; }
    let Ok(mut spans) = SPANS.lock() else { return; };
    spans.push((phase, duration));
}

pub fn start() {
    STARTED.get_or_init(Instant::now);
    STARTED_WALL.get_or_init(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
    mark("main-entry");
}
pub fn mark(stage: &str) {
    let elapsed_ms = STARTED.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.;
    let Ok(mut stages) = STAGES.lock() else { return; };
    if stages.iter().any(|(name, _)| name == stage) { return; }
    stages.push((stage.to_owned(), elapsed_ms));
    eprintln!("[native-startup] stage={stage} elapsed_ms={elapsed_ms:.2}");
    #[cfg(not(test))]
    if matches!(stage, "cached-input-ready" | "live-workspace-frame" | "window-activated"
        | "startup-drafts-hydrated" | "workspace-actions-ready" | "workspace-actions-paint-observed") {
        let stages = stages.clone();
        let spans = SPANS.lock().map(|spans| spans.clone()).unwrap_or_default();
        // A slow/failed backend must still leave first-visible/input timings.
        // Serialized writers reject older snapshots, so the early cache write
        // can never replace the later authoritative timing result.
        std::thread::spawn(move || {
            static WRITTEN: Mutex<usize> = Mutex::new(0);
            let Ok(mut written) = WRITTEN.lock() else { return; };
            if *written >= stages.len() { return; }
            let save = || -> Result<(), Box<dyn std::error::Error>> {
                let profile = monitter_lib::experimental_desktop::profile_dir()?;
                let path = profile.join("startup-timing.json");
                let temporary = profile.join(format!("startup-timing.{}.{}.tmp", std::process::id(), stages.len()));
                let spans: Vec<_> = spans.iter().map(|(phase, duration)| serde_json::json!({
                    "phase": phase,
                    "durationMs": duration.map(|value| value.as_secs_f64() * 1000.),
                    "observed": duration.is_some(),
                })).collect();
                let data = serde_json::json!({"pid":std::process::id(), "clockBaseline":"main-entry",
                    "mainEnteredAt":STARTED_WALL.get().copied(),
                    "recordedAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(), "stages":stages,
                    "spans":spans});
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
                use std::io::Write;
                let mut file = options.open(&temporary)?;
                file.write_all(&serde_json::to_vec(&data)?)?;
                std::fs::rename(temporary, path)?;
                Ok(())
            };
            match save() {
                Ok(()) => *written = stages.len(),
                Err(error) => eprintln!("[native-startup] timing_write={error}"),
            }
        });
    }
}
