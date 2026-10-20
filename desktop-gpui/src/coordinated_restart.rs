//! A detached coordinator owns restart after the old backend has acknowledged it.
//! Receipts are deliberately separate from the backend's durable restart journal.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPlan {
    pub old_pid: u32,
    pub old_instance: String,
    pub backend_executable: PathBuf,
    pub gui_app: PathBuf,
    pub profile: PathBuf,
    pub receipt: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    ClosingGui,
    ReadyToReplace,
    StartingBackend,
    OpeningGui,
    Complete,
    Failed(String),
}

pub struct RestartHandle {
    pub receipt: PathBuf,
}

impl RestartHandle {
    pub fn phase(&self) -> Result<Phase, String> {
        read_phase(&self.receipt)
    }
}

fn private_file(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("Cannot open restart file {}: {e}", path.display()))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() > 65536
    {
        return Err(format!(
            "Restart file must be an owned, private regular file: {}",
            path.display()
        ));
    }
    Ok(file)
}

pub fn read_phase(path: &Path) -> Result<Phase, String> {
    serde_json::from_reader(private_file(path)?)
        .map_err(|e| format!("Cannot read restart receipt: {e}"))
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    serde_json::to_writer(&mut file, value).map_err(|e| e.to_string())?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn publish(path: &Path, phase: &Phase) -> Result<(), String> {
    // Atomic replacement means the polling parent never sees partial JSON.
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temporary, phase)?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}

fn executable(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Restart executable paths must be absolute.".into());
    }
    let metadata =
        fs::metadata(path).map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!("Not an executable file: {}", path.display()));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn gui_executable(app: &Path) -> Result<PathBuf, String> {
    if app.extension().and_then(|s| s.to_str()) != Some("app") || !app.is_dir() {
        return Err("Restart GUI must be an existing .app bundle.".into());
    }
    let info =
        plist::Value::from_file(app.join("Contents/Info.plist")).map_err(|e| e.to_string())?;
    let name = info
        .as_dictionary()
        .and_then(|d| d.get("CFBundleExecutable"))
        .and_then(plist::Value::as_string)
        .ok_or("GUI bundle lacks CFBundleExecutable.")?;
    if Path::new(name).components().count() != 1 || name == "." || name == ".." {
        return Err("Invalid GUI bundle executable name.".into());
    }
    let path = app.join("Contents/MacOS").join(name);
    executable(&path)?;
    Ok(path)
}

#[cfg(not(target_os = "macos"))]
fn gui_executable(app: &Path) -> Result<PathBuf, String> {
    executable(app)?;
    Ok(app.to_owned())
}

fn validate(plan: &RestartPlan, helper: bool) -> Result<(), String> {
    if plan.old_pid <= 1
        || (helper && plan.old_pid == std::process::id())
        || plan.old_instance.is_empty()
    {
        return Err("Invalid old backend restart identity.".into());
    }
    for path in [&plan.profile, &plan.receipt, &plan.gui_app] {
        if !path.is_absolute() {
            return Err("Restart paths must be absolute.".into());
        }
    }
    let metadata = fs::metadata(&plan.profile).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(
            "Restart profile must be an owned directory without group/world write access.".into(),
        );
    }
    if plan.receipt.parent() != Some(plan.profile.as_path()) {
        return Err("Restart receipt must be directly inside its profile.".into());
    }
    executable(&plan.backend_executable)?;
    gui_executable(&plan.gui_app)?;
    Ok(())
}

fn detached(command: &mut Command) {
    // A new session survives the GUI/backend process group and terminal closing.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn log_file(profile: &Path, id: &str) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(profile.join(format!("restart-{id}.log")))
        .map_err(|e| e.to_string())
}

pub fn launch(mut plan: RestartPlan) -> Result<RestartHandle, String> {
    let id = uuid::Uuid::new_v4().to_string();
    plan.receipt = plan.profile.join(format!("restart-{id}.receipt.json"));
    validate(&plan, false)?;
    if plan.old_pid != std::process::id() {
        return Err("Only the running backend may launch its own restart coordinator.".into());
    }
    let plan_path = plan.profile.join(format!("restart-{id}.plan.json"));
    write_new(&plan_path, &plan)?;
    write_new(&plan.receipt, &Phase::ClosingGui)?;
    let log = log_file(&plan.profile, &id)?;
    let mut command = Command::new(&plan.backend_executable);
    command
        .arg("--restart-helper")
        .arg(&plan_path)
        .env("MONITTER_GPUI_PROFILE_DIR", &plan.profile)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    detached(&mut command);
    let child = command.spawn().map_err(|e| {
        let message = format!("Cannot launch restart coordinator: {e}");
        let _ = publish(&plan.receipt, &Phase::Failed(message.clone()));
        message
    })?;
    let _ = std::thread::Builder::new()
        .name("restart-helper-reaper".into())
        .spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
    Ok(RestartHandle {
        receipt: plan.receipt,
    })
}

trait RestartOperations {
    fn close_gui(&mut self) -> Result<(), String>;
    fn wait_old_backend(&mut self) -> Result<(), String>;
    fn start_and_verify_backend(&mut self) -> Result<(), String>;
    fn open_gui(&mut self) -> Result<(), String>;
}

fn coordinate(
    operations: &mut impl RestartOperations,
    mut report: impl FnMut(Phase) -> Result<(), String>,
) -> Result<(), String> {
    let result = (|| {
        report(Phase::ClosingGui)?;
        operations.close_gui()?;
        report(Phase::ReadyToReplace)?;
        operations.wait_old_backend()?;
        report(Phase::StartingBackend)?;
        operations.start_and_verify_backend()?;
        report(Phase::OpeningGui)?;
        operations.open_gui()?;
        report(Phase::Complete)
    })();
    if let Err(error) = &result {
        let _ = report(Phase::Failed(error.clone()));
    }
    result
}

fn alive(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

fn wait_until(
    timeout: Duration,
    mut done: impl FnMut() -> Result<bool, String>,
    description: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if done()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("Timed out waiting for {description}."));
        }
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn exact_gui_target(
    bundle: Option<&Path>,
    argv0: Option<&Path>,
    app: &Path,
    executable: &Path,
) -> bool {
    bundle == Some(app) || argv0 == Some(executable)
}

#[derive(Debug)]
struct ProcessIdentity {
    argv0: PathBuf,
    profile: Option<PathBuf>,
}

fn parse_process_identity(bytes: &[u8]) -> Option<ProcessIdentity> {
    use std::os::unix::ffi::OsStrExt;
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    if !(1..65536).contains(&argc) {
        return None;
    }
    let tail = bytes.get(4..)?;
    let end = tail.iter().position(|b| *b == 0)?;
    let tail = &tail[end..];
    let start = tail.iter().position(|b| *b != 0)?;
    let mut tail = &tail[start..];
    let end = tail.iter().position(|b| *b == 0)?;
    let argv0 = PathBuf::from(std::ffi::OsStr::from_bytes(&tail[..end]));
    for _ in 0..argc {
        let end = tail.iter().position(|b| *b == 0)?;
        tail = &tail[end + 1..];
    }
    let mut home = None;
    let mut explicit_profile = None;
    for field in tail.split(|b| *b == 0).filter(|field| !field.is_empty()) {
        if let Some(value) = field.strip_prefix(b"MONITTER_GPUI_PROFILE_DIR=") {
            explicit_profile = Some(PathBuf::from(std::ffi::OsStr::from_bytes(value)));
        }
        if let Some(value) = field.strip_prefix(b"HOME=") {
            home = Some(PathBuf::from(std::ffi::OsStr::from_bytes(value)));
        }
    }
    let profile = explicit_profile
        .or_else(|| {
            home.map(|home| home.join("Library/Application Support/com.monitter.gpui.experimental"))
        })
        .filter(|profile| profile.is_absolute());
    Some(ProcessIdentity { argv0, profile })
}

#[cfg(target_os = "macos")]
fn process_identity(pid: i32) -> Option<ProcessIdentity> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = 0;
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
            || size > 4 * 1024 * 1024
        {
            return None;
        }
        let mut bytes = vec![0u8; size];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        bytes.truncate(size);
        parse_process_identity(&bytes)
    }
}

#[cfg(target_os = "macos")]
fn matching_gui_pids(plan: &RestartPlan) -> Result<Vec<u32>, String> {
    use objc2_app_kit::NSRunningApplication;
    let executable = gui_executable(&plan.gui_app)?;
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err("Cannot enumerate GUI processes safely.".into());
    }
    let mut processes = vec![0i32; count as usize + 1024];
    let found = unsafe {
        libc::proc_listallpids(
            processes.as_mut_ptr().cast(),
            (processes.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if found <= 0 || found as usize >= processes.len() {
        return Err("Cannot obtain a complete GUI process inventory.".into());
    }
    processes.truncate(found as usize);
    let mut pids = Vec::new();
    for pid in processes.into_iter().filter(|pid| *pid > 1) {
        let identity = process_identity(pid);
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        let bundle = app
            .as_ref()
            .and_then(|app| app.bundleURL())
            .and_then(|url| url.path())
            .map(|s| PathBuf::from(s.to_string()));
        if !exact_gui_target(
            bundle.as_deref(),
            identity.as_ref().map(|identity| identity.argv0.as_path()),
            &plan.gui_app,
            &executable,
        ) {
            continue;
        }
        let profile = identity.as_ref().and_then(|identity| identity.profile.as_ref())
            .ok_or_else(|| format!("Cannot verify the profile of GUI process {pid}; no backend replacement is authorized."))?;
        if profile != &plan.profile {
            continue;
        }
        if pid <= 1 || pid as u32 == plan.old_pid || pid as u32 == std::process::id() {
            return Err("Refusing to terminate a backend as a GUI application.".into());
        }
        pids.push(pid as u32);
    }
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn close_exact_gui(plan: &RestartPlan) -> Result<(), String> {
    let pids = matching_gui_pids(plan)?;
    let requests = pids
        .iter()
        .map(|pid| crate::restart_close::request_close(&plan.profile, *pid))
        .collect::<Result<Vec<_>, _>>()?;
    wait_until(
        Duration::from_secs(20),
        || {
            let mut complete = true;
            for request in &requests {
                let acknowledged = crate::restart_close::acknowledged(&plan.profile, request)?;
                if !alive(request.pid) && !acknowledged {
                    return Err(format!(
                        "GUI process {} exited without acknowledging durable state capture; the backend remains running.",
                        request.pid
                    ));
                }
                complete &= acknowledged && !alive(request.pid);
            }
            Ok(complete)
        },
        "the target GUI to acknowledge durable close and exit; its version may not support coordinated restart, or state capture failed. The backend remains running",
    )?;
    if !matching_gui_pids(plan)?.is_empty() {
        return Err("A target GUI appeared during shutdown; the backend remains running.".into());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn close_exact_gui(_: &RestartPlan) -> Result<(), String> {
    Err("Coordinated GUI restart is only supported on macOS.".into())
}

fn backend_command(plan: &RestartPlan) -> Command {
    let mut command = Command::new(&plan.backend_executable);
    command
        .arg("--gui-app")
        .arg(&plan.gui_app)
        .env("MONITTER_GPUI_PROFILE_DIR", &plan.profile)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    detached(&mut command);
    command
}

fn gui_command(plan: &RestartPlan) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command
        .arg("-n")
        .arg("-a")
        .arg(&plan.gui_app)
        .arg("--env")
        .arg(format!(
            "MONITTER_GPUI_PROFILE_DIR={}",
            plan.profile.display()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

struct NativeOperations<'a> {
    plan: &'a RestartPlan,
    replacement: Option<Child>,
}

impl RestartOperations for NativeOperations<'_> {
    fn close_gui(&mut self) -> Result<(), String> {
        close_exact_gui(self.plan)
    }
    fn wait_old_backend(&mut self) -> Result<(), String> {
        wait_until(
            Duration::from_secs(15),
            || Ok(!alive(self.plan.old_pid)),
            "the old backend to exit",
        )
    }
    fn start_and_verify_backend(&mut self) -> Result<(), String> {
        let mut child = backend_command(self.plan)
            .spawn()
            .map_err(|e| format!("Cannot start replacement backend: {e}"))?;
        let (sender, receiver) = std::sync::mpsc::channel();
        let old = self.plan.old_instance.clone();
        let deadline = Instant::now() + Duration::from_secs(45);
        // IPC may block internally. The coordinator's main thread always keeps
        // its own deadline and child liveness check outside that worker.
        std::thread::Builder::new().name("restart-authenticated-connect".into()).spawn(move || {
            while Instant::now() < deadline {
                match monitter_lib::experimental_desktop::DesktopService::connect() {
                    Ok(service) if service.server_instance_id() != old && !service.server_instance_id().is_empty() => {
                        let _ = sender.send(Ok(()));
                        return;
                    }
                    Ok(_) => {},
                    Err(error) => eprintln!("Restart handshake pending: {error}"),
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = sender.send(Err("Replacement backend did not complete an authenticated fresh-instance handshake.".to_string()));
        }).map_err(|e| e.to_string())?;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "Replacement backend exited before GUI launch: {status}"
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Replacement backend handshake timed out after 45 seconds; the replacement was left running.".into());
            }
            match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(result) => {
                    result?;
                    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                        return Err(format!(
                            "Replacement backend exited during handshake: {status}"
                        ));
                    }
                    self.replacement = Some(child);
                    return Ok(());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(format!("Replacement handshake worker stopped: {error}")),
            }
        }
    }
    fn open_gui(&mut self) -> Result<(), String> {
        let mut child = gui_command(self.plan)
            .spawn()
            .map_err(|e| format!("Cannot reopen GUI: {e}"))?;
        let result = wait_until(
            Duration::from_secs(15),
            || match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => Ok(true),
                Some(status) => Err(format!("Opening the GUI failed: {status}")),
                None => Ok(false),
            },
            "macOS to open the exact GUI bundle",
        );
        if result.is_err() {
            // This is only our /usr/bin/open subprocess, never a GUI/backend.
            let _ = child.kill();
        }
        result?;
        #[cfg(target_os = "macos")]
        wait_until(
            Duration::from_secs(15),
            || {
                if let Some(backend) = self.replacement.as_mut() {
                    if let Some(status) = backend.try_wait().map_err(|e| e.to_string())? {
                        return Err(format!(
                            "Replacement backend exited before GUI appeared: {status}"
                        ));
                    }
                }
                Ok(!matching_gui_pids(self.plan)?.is_empty())
            },
            "the exact replacement GUI process to appear",
        )?;
        Ok(())
    }
}

/// Invoke before creating a backend or UI. The NSWorkspace calls run on main.
pub fn helper_main(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Restart plan path must be absolute.".into());
    }
    let mut bytes = Vec::new();
    private_file(path)?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let plan: RestartPlan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let result = (|| {
        validate(&plan, true)?;
        if path.parent() != Some(plan.profile.as_path()) {
            return Err("Restart plan is outside its profile.".into());
        }
        if std::env::var_os("MONITTER_GPUI_PROFILE_DIR")
            .map(PathBuf::from)
            .as_ref()
            != Some(&plan.profile)
        {
            return Err("Restart helper profile does not match its private plan.".into());
        }
        // Require the receipt created by the launching backend; never overwrite
        // a journal, arbitrary profile file, or another coordinator's receipt.
        let expected = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".plan.json"))
            .ok_or("Invalid restart plan filename.")?;
        if !expected.starts_with("restart-")
            || plan.receipt != plan.profile.join(format!("{expected}.receipt.json"))
        {
            return Err("Restart plan and receipt identities differ.".into());
        }
        if read_phase(&plan.receipt)? != Phase::ClosingGui {
            return Err("Restart receipt is not awaiting GUI shutdown.".into());
        }
        if let Err(error) = verify_old_backend(&plan) {
            publish(&plan.receipt, &Phase::Failed(error.clone()))?;
            show_failure(&plan, &error);
            return Err(error);
        }
        let mut operations = NativeOperations {
            plan: &plan,
            replacement: None,
        };
        let result = coordinate(&mut operations, |phase| {
            eprintln!("Restart phase: {phase:?}");
            publish(&plan.receipt, &phase)
        });
        if let Err(error) = &result {
            show_failure(&plan, error);
        }
        result
    })();
    if let Err(error) = &result {
        eprintln!("Coordinated restart failed: {error}");
        // coordinate persists operational errors. Validation failures must not
        // write to a path from an unvalidated plan.
    }
    result
}

fn show_failure(plan: &RestartPlan, error: &str) {
    #[cfg(target_os = "macos")]
    {
        // This separate native application may wait for OK. It cannot hold up
        // the critical coordinator or the old backend's receipt poller.
        let _ = error;
        let mut command = Command::new(&plan.backend_executable);
        command
            .arg("--restart-error")
            .arg(&plan.receipt)
            .env("MONITTER_GPUI_PROFILE_DIR", &plan.profile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detached(&mut command);
        let _ = command.spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (plan, error);
}

fn verify_old_backend(plan: &RestartPlan) -> Result<(), String> {
    if !alive(plan.old_pid) {
        return Err("The original backend exited before restart coordination began.".into());
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("restart-old-instance-check".into())
        .spawn(move || {
            let result = monitter_lib::experimental_desktop::DesktopService::connect()
                .map(|service| service.server_instance_id().to_owned());
            let _ = sender.send(result);
        })
        .map_err(|e| e.to_string())?;
    let instance = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| {
            "Timed out authenticating the original backend; no application was closed.".to_string()
        })??;
    if instance != plan.old_instance || !alive(plan.old_pid) {
        return Err("Restart plan does not identify the authenticated running backend; no application was closed.".into());
    }
    Ok(())
}

/// Dispatch this in an isolated `--restart-error` process, before backend init.
pub fn failure_alert(path: &Path) -> Result<(), String> {
    let Phase::Failed(message) = read_phase(path)? else {
        return Err("Restart alert requires a failed private restart receipt.".into());
    };
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAlert, NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::NSString;
        let mtm = MainThreadMarker::new().ok_or("Restart alerts must run on the main thread.")?;
        let application = NSApplication::sharedApplication(mtm);
        application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Monitter restart failed"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "{message}\n\nRestart details: {}",
            path.display()
        )));
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        #[allow(deprecated)]
        application.activateIgnoringOtherApps(true);
        alert.runModal();
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        fail: Option<&'static str>,
        calls: Vec<&'static str>,
    }
    impl Fake {
        fn step(&mut self, name: &'static str) -> Result<(), String> {
            self.calls.push(name);
            if self.fail == Some(name) {
                Err(format!("{name} failed"))
            } else {
                Ok(())
            }
        }
    }
    impl RestartOperations for Fake {
        fn close_gui(&mut self) -> Result<(), String> {
            self.step("close")
        }
        fn wait_old_backend(&mut self) -> Result<(), String> {
            self.step("exit")
        }
        fn start_and_verify_backend(&mut self) -> Result<(), String> {
            self.step("verify")
        }
        fn open_gui(&mut self) -> Result<(), String> {
            self.step("open")
        }
    }
    #[test]
    fn handshake_and_gui_shutdown_gate_replacement_and_opening() {
        let mut operations = Fake {
            fail: None,
            calls: vec![],
        };
        let mut phases = vec![];
        coordinate(&mut operations, |phase| {
            phases.push(phase);
            Ok(())
        })
        .unwrap();
        assert_eq!(operations.calls, ["close", "exit", "verify", "open"]);
        assert_eq!(
            phases,
            [
                Phase::ClosingGui,
                Phase::ReadyToReplace,
                Phase::StartingBackend,
                Phase::OpeningGui,
                Phase::Complete
            ]
        );
    }
    #[test]
    fn refused_gui_close_never_authorizes_old_backend_exit() {
        let mut operations = Fake {
            fail: Some("close"),
            calls: vec![],
        };
        let mut phases = vec![];
        assert!(
            coordinate(&mut operations, |phase| {
                phases.push(phase);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(operations.calls, ["close"]);
        assert_eq!(
            phases,
            [Phase::ClosingGui, Phase::Failed("close failed".into())]
        );
    }
    #[test]
    fn failed_handshake_never_opens_gui() {
        let mut operations = Fake {
            fail: Some("verify"),
            calls: vec![],
        };
        let mut phases = vec![];
        assert!(
            coordinate(&mut operations, |phase| {
                phases.push(phase);
                Ok(())
            })
            .is_err()
        );
        assert!(!operations.calls.contains(&"open"));
        assert!(!phases.contains(&Phase::OpeningGui));
        assert_eq!(phases.last(), Some(&Phase::Failed("verify failed".into())));
    }
    #[test]
    fn timeout_is_bounded_and_records_condition() {
        let start = Instant::now();
        let error = wait_until(Duration::from_millis(2), || Ok(false), "fake process").unwrap_err();
        assert!(error.contains("fake process"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn exact_target_does_not_select_preview_with_same_executable_name() {
        let app = Path::new("/Applications/Monitter GPUI.app");
        let executable = app.join("Contents/MacOS/monitter-gpui-desktop");
        assert!(!exact_gui_target(
            Some(Path::new("/tmp/Preview/Monitter GPUI.app")),
            Some(Path::new(
                "/tmp/Preview/Monitter GPUI.app/Contents/MacOS/monitter-gpui-desktop"
            )),
            app,
            &executable
        ));
        assert!(exact_gui_target(Some(app), None, app, &executable));
        assert!(exact_gui_target(None, Some(&executable), app, &executable));
    }
    #[test]
    fn backend_launch_preserves_spaces_and_profile_as_single_arguments() {
        let plan = RestartPlan {
            old_pid: 42,
            old_instance: "old".into(),
            backend_executable: "/build path/server".into(),
            gui_app: "/build path/My GUI.app".into(),
            profile: "/private/profile path".into(),
            receipt: "/private/profile path/receipt".into(),
        };
        let command = backend_command(&plan);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("--gui-app"), plan.gui_app.as_os_str()]
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "MONITTER_GPUI_PROFILE_DIR"
                    && value == Some(plan.profile.as_os_str()))
        );
        let command = gui_command(&plan);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-n",
                "-a",
                "/build path/My GUI.app",
                "--env",
                "MONITTER_GPUI_PROFILE_DIR=/private/profile path"
            ]
            .map(std::ffi::OsStr::new)
        );
    }
    #[test]
    fn process_arguments_distinguish_profiles_of_the_same_installed_gui() {
        let mut bytes = 2i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/resolved/shared-binary\0\0/Applications/Monitter GPUI.app/Contents/MacOS/gui\0--example\0HOME=/Users/test\0MONITTER_GPUI_PROFILE_DIR=/tmp/isolated profile\0\0");
        let identity = parse_process_identity(&bytes).unwrap();
        assert_eq!(
            identity.argv0,
            Path::new("/Applications/Monitter GPUI.app/Contents/MacOS/gui")
        );
        assert_eq!(
            identity.profile.as_deref(),
            Some(Path::new("/tmp/isolated profile"))
        );
        assert_ne!(
            identity.profile.as_deref(),
            Some(Path::new(
                "/Users/test/Library/Application Support/com.monitter.gpui.experimental"
            ))
        );
        let mut default = 1i32.to_ne_bytes().to_vec();
        default.extend_from_slice(
            b"/resolved/shared-binary\0\0/Applications/GUI\0HOME=/Users/test\0\0",
        );
        assert_eq!(
            parse_process_identity(&default).unwrap().profile.as_deref(),
            Some(Path::new(
                "/Users/test/Library/Application Support/com.monitter.gpui.experimental"
            ))
        );
        assert!(parse_process_identity(&[0u8; 3]).is_none());
    }
    #[test]
    fn helper_rejects_its_own_pid_before_touching_application_paths() {
        let mut plan = RestartPlan {
            old_pid: std::process::id(),
            old_instance: "instance".into(),
            backend_executable: PathBuf::new(),
            gui_app: PathBuf::new(),
            profile: PathBuf::new(),
            receipt: PathBuf::new(),
        };
        assert!(validate(&plan, true).unwrap_err().contains("identity"));
        plan.old_pid = 1;
        assert!(validate(&plan, false).unwrap_err().contains("identity"));
    }
    #[test]
    fn private_receipt_roundtrip_and_atomic_failure() {
        let directory =
            std::env::temp_dir().join(format!("monitter-restart-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let receipt = directory.join("receipt.json");
        write_new(&receipt, &Phase::ClosingGui).unwrap();
        assert_eq!(read_phase(&receipt).unwrap(), Phase::ClosingGui);
        publish(&receipt, &Phase::Failed("not ready".into())).unwrap();
        assert_eq!(
            read_phase(&receipt).unwrap(),
            Phase::Failed("not ready".into())
        );
        fs::set_permissions(&receipt, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_phase(&receipt).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
