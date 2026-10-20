//! Lazy, loopback-only Laya service. Never downloads models or replaces an external server.
use std::{
    fs::{self, OpenOptions},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

struct OwnedService {
    child: Child,
    last_used: Instant,
}
static SERVICE: OnceLock<Mutex<Option<OwnedService>>> = OnceLock::new();
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

fn service() -> &'static Mutex<Option<OwnedService>> {
    SERVICE.get_or_init(|| {
        // Only the Child handle we created can be stopped. External listeners are never killed.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(30));
            if let Some(state) = SERVICE.get() {
                if let Ok(mut slot) = state.lock() {
                    if let Some(owned) = slot.as_mut() {
                        let exited = matches!(owned.child.try_wait(), Ok(Some(_)));
                        if exited || owned.last_used.elapsed() >= IDLE_TIMEOUT {
                            if !exited {
                                let _ = owned.child.kill();
                                let _ = owned.child.wait();
                            }
                            *slot = None;
                        }
                    }
                }
            }
        });
        Mutex::new(None)
    })
}

fn listening() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], 8080));
    TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
}

fn health_model() -> Result<String, String> {
    let output = Command::new("curl")
        .args([
            "-q", "--noproxy", "*", "--proto", "=http", "--fail", "--silent",
            "--connect-timeout", "1", "--max-time", "2", "--max-filesize", "65536",
            "http://127.0.0.1:8080/health",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("Could not check local Laya readiness: {error}"))?;
    if !output.status.success() {
        return Err("Local Laya health check failed.".into());
    }
    let body: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Port 8080 did not return Laya health JSON.")?;
    let model = body.get("model").and_then(|value| value.as_str()).unwrap_or_default();
    if body.get("status").and_then(|value| value.as_str()) != Some("ok")
        || !matches!(body.get("variant").and_then(|v| v.as_str()), Some("english" | "multilingual" | "typed-decisions"))
        || body.get("backend").and_then(|value| value.as_str()).is_none()
        || model.is_empty()
    {
        return Err("Port 8080 is occupied by a service without a valid Laya health response.".into());
    }
    Ok(model.to_owned())
}

/// Reuse an existing Laya server or start the user's installed CLI, waiting at most 60s.
/// Returns the actual served model, since Laya rejects arbitrary model aliases.
pub(super) fn ensure_running(api_key: Option<&str>) -> Result<String, String> {
    let mut slot = service().lock().map_err(|_| "Local Laya service lock failed.")?;
    if let Some(owned) = slot.as_mut() {
        if owned.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            *slot = None;
        } else {
            owned.last_used = Instant::now();
        }
    }
    if listening() {
        return health_model();
    }
    // An owned process that lost its listener must stop before a replacement starts.
    if let Some(mut owned) = slot.take() {
        let _ = owned.child.kill();
        let _ = owned.child.wait();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)
        .ok_or("Cannot locate the local Laya installation without HOME.")?;
    let root = home.join("code/laya.cpp");
    let cli = std::env::var_os("LAYA_CLI_PATH").map(PathBuf::from)
        .unwrap_or_else(|| root.join("build-coreml/bin/laya-cli"));
    let model = std::env::var_os("LAYA_MODEL_DIR").map(PathBuf::from)
        .unwrap_or_else(|| root.join("models/laya"));
    if !cli.is_absolute() || !cli.is_file() || !model.is_absolute() || !model.join("rl_agent_config.json").is_file() {
        return Err("Local Laya is not installed. Set absolute LAYA_CLI_PATH and LAYA_MODEL_DIR paths; Monitter does not download a binary or model.".into());
    }
    let backend = std::env::var("LAYA_RUNTIME_BACKEND")
        .unwrap_or_else(|_| if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "coreml" } else { "cpu" }.into());
    if !matches!(backend.as_str(), "coreml" | "cpu" | "cuda" | "vulkan") {
        return Err("LAYA_RUNTIME_BACKEND must be coreml, cpu, cuda, or vulkan.".into());
    }
    let log_dir = home.join(".monitter/laya-service");
    fs::create_dir_all(&log_dir).map_err(|e| format!("Could not create Laya log directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&log_dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(log_dir.join("server.log")).map_err(|e| e.to_string())?;
    let error_log = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(&cli);
    command.arg(format!("--{backend}"))
        .args(["--server", "--host", "127.0.0.1", "--port", "8080", "--model"])
        .arg(&model).stdin(Stdio::null()).stdout(log).stderr(error_log);
    if let Some(key) = api_key {
        command.env("LAYA_API_KEY", key);
    } else {
        command.env_remove("LAYA_API_KEY");
    }
    let child = command.spawn().map_err(|e| format!("Could not start installed Laya: {e}"))?;
    *slot = Some(OwnedService { child, last_used: Instant::now() });
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let owned = slot.as_mut().ok_or("Local Laya startup lost its process handle.")?;
        if let Some(status) = owned.child.try_wait().map_err(|e| e.to_string())? {
            *slot = None;
            return Err(format!("Local Laya exited ({status}); see {}.", log_dir.join("server.log").display()));
        }
        if listening() {
            if let Ok(model) = health_model() {
                owned.last_used = Instant::now();
                return Ok(model);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if let Some(mut owned) = slot.take() {
        let _ = owned.child.kill();
        let _ = owned.child.wait();
    }
    Err(format!("Local Laya did not become ready within 60 seconds; see {}.", log_dir.join("server.log").display()))
}
