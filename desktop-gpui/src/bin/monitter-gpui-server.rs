//! Persistent owner of the experimental backend. No workspace window is opened.
#[cfg(target_os = "macos")]
#[path = "../server_menu.rs"]
mod server_menu;
use monitter_lib::experimental_desktop::DesktopServer;
#[cfg(target_os = "macos")]
use std::{path::PathBuf, sync::Arc, time::Duration};

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
    let mut arguments = std::env::args_os().skip(1);
    let initial_gui = match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--gui-app" => PathBuf::from(path),
        (None, None, None) => {
            let executable = std::env::current_exe().expect("server executable path");
            executable
                .parent()
                .and_then(|path| path.parent())
                .and_then(|path| path.parent())
                .and_then(|path| path.parent())
                .expect("server bundle directory")
                .join("Monitter GPUI.app")
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
        let (menu, events) = server_menu::ServerMenu::install(
            mtm,
            Some("Backend running — GUI can restart independently"),
        );
        let quit_owner = server.clone();
        cx.on_app_quit(move |_| {
            quit_owner.shutdown();
            async {}
        })
        .detach();
        cx.spawn(async move |cx| {
            let _menu_owner = menu;
            loop {
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
                        server_menu::ServerMenuEvent::QuitServer => {
                            // This is the server's explicit stop action, never a GUI disconnect.
                            server.shutdown();
                            let _ = cx.update(|cx| cx.quit());
                            return;
                        }
                    }
                }
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
            }
        })
        .detach();
    });
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
