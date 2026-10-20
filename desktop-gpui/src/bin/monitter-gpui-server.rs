//! Persistent owner of the experimental backend. No workspace window is opened.
#[cfg(target_os = "macos")]
#[path = "../server_menu.rs"]
mod server_menu;
#[cfg(target_os = "macos")]
#[path = "../coordinated_restart.rs"]
mod coordinated_restart;
#[cfg(target_os = "macos")]
#[path = "../restart_close.rs"]
mod restart_close;
use monitter_lib::experimental_desktop::DesktopServer;
#[cfg(target_os = "macos")]
use monitter_lib::experimental_desktop::RestartMode;
#[cfg(target_os = "macos")]
use std::{path::{Path, PathBuf}, sync::Arc, time::Duration};

#[cfg(target_os = "macos")]
fn gui_path(initial: &std::path::Path) -> PathBuf {
    // Every GUI build is immutable; the menu opens the latest packaged client.
    let latest = initial
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("latest.json"));
    latest
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["app"].as_str().map(PathBuf::from))
        .filter(|path| path.join("Contents/MacOS/monitter-gpui-desktop").is_file())
        .unwrap_or_else(|| initial.to_path_buf())
}

#[cfg(target_os = "macos")]
fn main() {
    let helper_arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if helper_arguments.first().is_some_and(|arg| arg == "--restart-error") {
        if helper_arguments.len() != 2 {
            std::process::exit(2);
        }
        if let Err(error) = coordinated_restart::failure_alert(Path::new(&helper_arguments[1])) {
            eprintln!("Could not show restart failure: {error}");
            std::process::exit(1);
        }
        return;
    }
    if helper_arguments.first().is_some_and(|arg| arg == "--restart-helper") {
        if helper_arguments.len() != 2 {
            eprintln!("Usage: monitter-gpui-server --restart-helper PLAN_PATH");
            std::process::exit(2);
        }
        if let Err(error) = coordinated_restart::helper_main(Path::new(&helper_arguments[1])) {
            eprintln!("Monitter restart failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(debug_assertions)]
    if (std::env::args_os().count() == 2
        && std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--menu-preview")))
        || std::env::current_exe().ok().as_ref().and_then(|path|path.file_name())
            == Some(std::ffi::OsStr::new("monitter-server-menu-preview"))
    {
        menu_preview();
        return;
    }
    let mut arguments = std::env::args_os().skip(1);
    let initial_gui = match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--gui-app" => PathBuf::from(path),
        (None, None, None) => {
            let executable = std::env::current_exe().expect("server executable path");
            let directory = executable
                .parent()
                .and_then(|path| path.parent())
                .and_then(|path| path.parent())
                .and_then(|path| path.parent())
                .expect("server bundle directory");
            let installed = directory.join("Monitter.app");
            if installed.join("Contents/MacOS/monitter-gpui-desktop").is_file() {
                installed
            } else {
                directory.join("Monitter GPUI.app")
            }
        }
        _ => {
            eprintln!("Usage: monitter-gpui-server [--gui-app APP_PATH]");
            std::process::exit(2);
        }
    };
    let server = match DesktopServer::start() {
        Ok(server) => Arc::new(server),
        Err(error) => {
            eprintln!("GPUI backend server could not start: {error}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "GPUI backend server ready: pid={} instance={}",
        std::process::id(),
        server.instance_id()
    );
    gpui_kit::application().run(move |cx| {
        let mtm = objc2::MainThreadMarker::new().expect("AppKit main thread");
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        let activity_owner = server.clone();
        let activity_labels = Arc::new(std::sync::Mutex::new(Err("Loading running agents…".to_string())));
        let activity_loading = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let active_reader = Arc::new(move || {
            // Native menu expansion must never wait for a backend writer lock.
            // One worker refreshes the cache; reopen the submenu for its result.
            if !activity_loading.swap(true, std::sync::atomic::Ordering::AcqRel) {
                let owner = activity_owner.clone();
                let labels = activity_labels.clone();
                let loading = activity_loading.clone();
                if let Err(error) = std::thread::Builder::new().name("restart-agent-menu".into()).spawn(move || {
                    let result = owner.active_agents().map(|agents| agents.into_iter()
                        .map(|agent| format!("{} — {}", agent.agent_name, agent.title)).collect::<Vec<_>>());
                    if let Ok(mut labels) = labels.lock() { *labels = result; }
                    loading.store(false, std::sync::atomic::Ordering::Release);
                }) {
                    activity_loading.store(false, std::sync::atomic::Ordering::Release);
                    return Err(error.to_string());
                }
            }
            activity_labels.try_lock().map(|labels| labels.clone())
                .unwrap_or_else(|_| Err("Running agents are being checked…".into()))
        });
        let (menu, events) = server_menu::ServerMenu::install(
            mtm,
            Some("Monitter backend running"),
            active_reader,
            "Monitter",
        );
        let quit_owner = server.clone();
        cx.on_app_quit(move |_| {
            // Explicit normal quit retains the backend's cleanup semantics.
            // Coordinated replacement exits only after transport preparation,
            // bypassing AppKit quit callbacks and blocking Rust destructors.
            quit_owner.shutdown();
            async {}
        })
        .detach();
        cx.spawn(async move |cx| {
            let mut menu = menu;
            let mut restart_pending = false;
            let mut paused_worker = false;
            loop {
                if paused_worker && !RESTART_OPERATION_RUNNING.load(std::sync::atomic::Ordering::Acquire) {
                    paused_worker = false;
                    menu.set_restart_available(true);
                    menu.set_restart_pending(true);
                    menu.set_status_label(mtm, Some("Restart paused — operation finished; cancel or retry"));
                }
                while let Ok(event) = events.try_recv() {
                    match event {
                        server_menu::ServerMenuEvent::OpenMonitter => {
                            let target = gui_path(&initial_gui);
                            if let Err(error) = std::process::Command::new("/usr/bin/open")
                                .arg("-a")
                                .arg(target)
                                .spawn()
                            {
                                eprintln!("Could not open GPUI client: {error}");
                            }
                        }
                        server_menu::ServerMenuEvent::RestartWhenIdle
                        | server_menu::ServerMenuEvent::RestartNow => {
                            let mode = if event == server_menu::ServerMenuEvent::RestartNow {
                                RestartMode::Force
                            } else {
                                RestartMode::Wait
                            };
                            let owner = server.clone();
                            menu.set_status_label(mtm, Some("Preparing Monitter restart…"));
                            let result = bounded_operation(cx.background_executor().clone(),
                                "Recording restart", Duration::from_secs(15),
                                move || owner.begin_restart(mode)).await;
                            match result {
                                Ok(status) => {
                                    restart_pending = true;
                                    menu.set_restart_pending(true);
                                    menu.set_status_label(
                                        mtm,
                                        Some(&format!(
                                            "Restart pending — {} active agents",
                                            status.active_agents.len()
                                        )),
                                    );
                                }
                                Err(error) => menu.set_status_label(
                                    mtm,
                                    Some(&format!("Restart failed: {error}")),
                                ),
                            }
                        }
                        server_menu::ServerMenuEvent::CancelRestart => {
                            let owner = server.clone();
                            match bounded_operation(cx.background_executor().clone(),
                                "Cancelling restart", Duration::from_secs(15),
                                move || owner.cancel_restart()).await
                            {
                                Ok(()) => {
                                    restart_pending = false;
                                    menu.set_restart_pending(false);
                                    menu.set_restart_available(true);
                                    menu.set_status_label(
                                        mtm,
                                        Some("Backend running — restart cancelled"),
                                    );
                                }
                                Err(error) => menu.set_status_label(
                                    mtm,
                                    Some(&format!("Could not cancel restart: {error}")),
                                ),
                            }
                        }
                        server_menu::ServerMenuEvent::QuitServer => {
                            // This is the server's explicit stop action, never a GUI disconnect.
                            server.shutdown();
                            let _ = cx.update(|cx| cx.quit());
                            return;
                        }
                    }
                }
                if restart_pending {
                    let owner = server.clone();
                    match bounded_operation(cx.background_executor().clone(),
                        "Checking active agents", Duration::from_secs(15),
                        move || owner.poll_restart()).await
                    {
                        Ok(Some(status)) if status.ready => {
                            // Resolve the configured install location, not the
                            // running bundle: an update may have moved it into a
                            // rollback directory since this process was launched.
                            let target_gui = gui_path(&initial_gui);
                            let executable = match replacement_executable(&target_gui) {
                                Ok(path) => path,
                                Err(error) => {
                                    restart_pending = false;
                                    menu.set_status_label(mtm, Some(&format!("Restart paused: {error}")));
                                    continue;
                                }
                            };
                            menu.set_restart_available(false);
                            menu.set_status_label(mtm, Some("Preparing agents for restart…"));
                            let owner = server.clone();
                            let preparation = bounded_operation(cx.background_executor().clone(),
                                "Stopping agent transports", Duration::from_secs(30),
                                move || owner.prepare_replacement()).await;
                            if let Err(error) = preparation {
                                // Keep admission fenced and the durable Force
                                // journal intact. Never replace a live run owner.
                                restart_pending = false;
                                menu.set_status_label(mtm, Some(&format!("Restart paused: {error}")));
                                continue;
                            }
                            let plan = monitter_lib::experimental_desktop::profile_dir().and_then(|profile| {
                                Ok(coordinated_restart::RestartPlan {
                                    old_pid: std::process::id(),
                                    old_instance: server.instance_id().to_string(),
                                    backend_executable: executable,
                                    gui_app: target_gui,
                                    receipt: profile.join(format!("restart-{}.json", uuid::Uuid::new_v4())),
                                    profile,
                                })
                            });
                            let launched = plan.and_then(coordinated_restart::launch);
                            match launched {
                                Ok(handle) => {
                                    menu.set_status_label(mtm, Some("Saving and closing Monitter windows…"));
                                    menu.set_restart_pending(false);
                                    let deadline = std::time::Instant::now() + Duration::from_secs(60);
                                    loop {
                                        match handle.phase() {
                                            Ok(coordinated_restart::Phase::ReadyToReplace) => {
                                                // All registered providers released; GUI saved
                                                // and closed; independent helper is waiting.
                                                // Kernel closes IPC and lease descriptors.
                                                // Do not enter unbounded Service::cleanup/Drop.
                                                std::process::exit(0);
                                            }
                                            Ok(coordinated_restart::Phase::Failed(error)) | Err(error) => {
                                                menu.set_status_label(mtm, Some(&format!("Restart paused: {error}")));
                                                break;
                                            }
                                            _ if std::time::Instant::now() >= deadline => {
                                                menu.set_status_label(mtm, Some("Restart paused: window close timed out; see restart log"));
                                                break;
                                            }
                                            _ => {}
                                        }
                                        cx.background_executor().timer(Duration::from_millis(100)).await;
                                    }
                                }
                                Err(error) => menu.set_status_label(mtm, Some(&format!("Restart paused: {error}"))),
                            }
                            restart_pending = false;
                            menu.set_restart_pending(true);
                        }
                        Ok(Some(status)) => menu.set_status_label(
                            mtm,
                            Some(&format!(
                                "Waiting to restart — {} active agents",
                                status.active_agents.len()
                            )),
                        ),
                        Ok(None) => {
                            restart_pending = false;
                            menu.set_restart_pending(false);
                        }
                        Err(error) => {
                            restart_pending = false;
                            menu.set_restart_available(false);
                            menu.set_status_label(mtm, Some(&format!("Restart paused: {error}")));
                        }
                    }
                }
                if RESTART_OPERATION_RUNNING.load(std::sync::atomic::Ordering::Acquire) {
                    // A timed-out operation may still hold backend locks. Do
                    // not race cancellation/retry against its eventual result.
                    paused_worker = true;
                    menu.set_restart_available(false);
                    menu.set_restart_pending(false);
                }
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
            }
        })
        .detach();
    });
}

/// Test the real AppKit menu without opening any profile, network listener,
/// agent transport or server. Restart choices only log their selection.
#[cfg(all(target_os = "macos", debug_assertions))]
fn menu_preview() {
    gpui_kit::application().run(|cx| {
        let mtm = objc2::MainThreadMarker::new().expect("preview main thread");
        objc2_app_kit::NSApplication::sharedApplication(mtm)
            .setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader_count = reads.clone();
        let reader = Arc::new(move || {
            let count = reader_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            eprintln!("menu-preview active-agent reads={count}");
            Ok(Vec::new())
        });
        let (menu, events) = server_menu::ServerMenu::install(
            mtm,
            Some("Menu preview only — no backend or agents"),
            reader,
            "Monitter Test",
        );
        eprintln!(
            "menu-preview ready; active-agent reads={}",
            reads.load(std::sync::atomic::Ordering::SeqCst)
        );
        cx.spawn(async move |cx| {
            let _menu = menu;
            loop {
                while let Ok(event) = events.try_recv() {
                    eprintln!("menu-preview selection: {event:?}");
                    if event == server_menu::ServerMenuEvent::QuitServer {
                        let _ = cx.update(|cx| cx.quit());
                        return;
                    }
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    });
}

/// The installed GUI and backend are siblings. Never relaunch a renamed
/// rollback copy merely because it hosts the old process's executable image.
#[cfg(target_os = "macos")]
fn replacement_executable(gui: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let executable = gui.parent().ok_or("GUI bundle has no parent directory")?
        .join("Monitter GPUI Server.app/Contents/MacOS/monitter-gpui-server");
    for path in [&executable, &gui.join("Contents/MacOS/monitter-gpui-desktop")] {
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{} is not executable. Reinstall Monitter before restarting.", path.display()));
        }
    }
    Ok(executable)
}

/// A blocked backend mutex or transport cancellation must not freeze AppKit.
/// Timeout leaves the owner and restart journal in place; no unsafe fallback.
#[cfg(target_os = "macos")]
static RESTART_OPERATION_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
async fn bounded_operation<T: Send + 'static>(
    executor: gpui_kit::BackgroundExecutor,
    label: &'static str,
    timeout: Duration,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    use std::sync::atomic::Ordering;
    if RESTART_OPERATION_RUNNING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err("A previous restart operation is still finishing; retry after it settles.".into());
    }
    struct RunningGuard;
    impl Drop for RunningGuard {
        fn drop(&mut self) { RESTART_OPERATION_RUNNING.store(false, Ordering::Release); }
    }
    let (send, receive) = futures::channel::oneshot::channel();
    if let Err(error) = std::thread::Builder::new().name("restart-preparation".into())
        .spawn(move || {
            let guard = RunningGuard;
            let result = operation();
            drop(guard);
            let _ = send.send(result);
        }) {
        RESTART_OPERATION_RUNNING.store(false, Ordering::Release);
        return Err(error.to_string());
    }
    match futures::future::select(Box::pin(receive), Box::pin(executor.timer(timeout))).await {
        futures::future::Either::Left((result, _)) => result.map_err(|_| format!("{label} worker stopped"))?,
        futures::future::Either::Right(_) => Err(format!("{label} timed out. Backend retained; cancel or inspect the restart log.")),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod replacement_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn replacement_uses_configured_install_even_when_running_copy_moved() {
        let directory = std::env::temp_dir().join(format!("monitter-target-{}", uuid::Uuid::new_v4()));
        let gui = directory.join("Monitter.app");
        let backend = directory.join("Monitter GPUI Server.app/Contents/MacOS/monitter-gpui-server");
        let gui_exe = gui.join("Contents/MacOS/monitter-gpui-desktop");
        for path in [&backend, &gui_exe] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"fixture").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(replacement_executable(&gui).unwrap(), backend);
        std::fs::remove_file(&backend).unwrap();
        // The old running image being present cannot authorize a fallback.
        assert!(replacement_executable(&gui).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(target_os = "linux")]
fn main() {
    // Accept the same launch interface as macOS. Linux's headless server has no
    // tray menu; users reopen the sibling GUI executable independently.
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(_path), None) if flag == "--gui-app" => {}
        (None, None, None) => {}
        _ => {
            eprintln!("Usage: monitter-gpui-server [--gui-app GUI_EXECUTABLE]");
            std::process::exit(2);
        }
    }
    // Only record shutdown intent in a signal handler; the owner thread performs
    // backend cleanup. Do not block signals inherited by real harness processes.
    if let Err(error) = install_shutdown_signals() {
        eprintln!("Cannot configure GPUI backend shutdown signals: {error}");
        std::process::exit(1);
    }
    let server = match DesktopServer::start() {
        Ok(server) => server,
        Err(error) => {
            eprintln!("GPUI backend server could not start: {error}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "GPUI backend server ready: pid={} instance={} (SIGINT/SIGTERM to stop)",
        std::process::id(),
        server.instance_id()
    );
    while !SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    server.shutdown();
}

#[cfg(target_os = "linux")]
static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
extern "C" fn request_shutdown(_signal: libc::c_int) {
    // AtomicBool is lock-free on supported Linux targets; no allocation, locks,
    // logging, or backend operations run in this asynchronous handler.
    SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn install_shutdown_signals() -> Result<(), std::io::Error> {
    // SAFETY: zero is a valid initialization for sigaction before its fields
    // and empty signal mask are populated. Handler has the required C ABI.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = request_shutdown as *const () as usize;
    action.sa_flags = libc::SA_RESTART;
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: action is initialized; null means no old handler requested.
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("The experimental backend server currently supports macOS and Linux.");
    std::process::exit(1);
}
