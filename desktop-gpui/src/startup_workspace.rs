//! Startup timing for the retained native workspace handoff.
use std::{sync::OnceLock, time::Instant};

static STARTED: OnceLock<Instant> = OnceLock::new();
pub fn start() { STARTED.get_or_init(Instant::now); }
pub fn mark(stage: &str) {
    let elapsed_ms = STARTED.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.;
    eprintln!("[native-startup] stage={stage} elapsed_ms={elapsed_ms:.2}");
    #[cfg(not(test))]
    {
        static STAGES: std::sync::Mutex<Vec<(String, f64)>> = std::sync::Mutex::new(Vec::new());
        let Ok(mut stages) = STAGES.lock() else { return; };
        if stages.iter().any(|(name, _)| name == stage) { return; }
        stages.push((stage.to_owned(), elapsed_ms));
        if stage != "live-workspace-frame" { return; }
        let stages = stages.clone();
        // Persist only timing metadata, after the live paint and off the UI
        // thread, so launches from Finder are measurable without a terminal.
        std::thread::spawn(move || {
            let save = || -> Result<(), Box<dyn std::error::Error>> {
                let profile = monitter_lib::experimental_desktop::profile_dir()?;
                let path = profile.join("startup-timing.json");
                let temporary = profile.join(format!("startup-timing.{}.tmp", std::process::id()));
                let data = serde_json::json!({"pid":std::process::id(), "recordedAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(), "stages":stages});
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
                use std::io::Write;
                let mut file = options.open(&temporary)?;
                file.write_all(&serde_json::to_vec(&data)?)?;
                std::fs::rename(temporary, path)?;
                Ok(())
            };
            if let Err(error) = save() { eprintln!("[native-startup] timing_write={error}"); }
        });
    }
}
