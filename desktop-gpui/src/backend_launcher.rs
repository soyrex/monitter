//! Connect to the independent experimental server without owning its lifetime.
//! Starting a GUI may start a missing server; closing the GUI never signals it.
use monitter_lib::experimental_desktop::{BackendAvailability, DesktopService};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Exact transport messages only; backend validation errors remain visible.
pub(crate) fn is_transient_connection_error(error: &str) -> bool {
    matches!(
        error,
        "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation."
            | "Private IPC response incomplete; delivery may be ambiguous. Refresh state before retrying a mutation."
            | "Private IPC connection unavailable. Reconnect and try again."
            | "Private IPC is temporarily unavailable after 3 attempts. Refresh or reconnect, then try again."
    )
}

fn server_paths(executable: &std::path::Path) -> Result<(PathBuf, PathBuf), String> {
    #[cfg(target_os = "macos")]
    {
        let gui_app = executable
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .ok_or_else(|| "Cannot locate the GPUI app bundle.".to_string())?
            .to_path_buf();
        let build = gui_app
            .parent()
            .ok_or_else(|| "Cannot locate the GPUI build directory.".to_string())?;
        Ok((
            build.join("Monitter GPUI Server.app/Contents/MacOS/monitter-gpui-server"),
            gui_app,
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let directory = executable
            .parent()
            .ok_or_else(|| "Cannot locate the GPUI executable directory.".to_string())?;
        Ok((
            directory.join("monitter-gpui-server"),
            executable.to_path_buf(),
        ))
    }
}

fn packaged_server() -> Result<(PathBuf, PathBuf), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let (server, gui) = server_paths(&executable)?;
    if !server.is_file() {
        return Err(format!(
            "The persistent backend is not running and its executable is missing: {}. Build both GPUI binaries first.",
            server.display()
        ));
    }
    Ok((server, gui))
}

/// Runs on the existing startup worker, never on the native UI thread.
/// Connection retries cannot replay any task or mutation.
pub fn connect() -> Result<DesktopService, String> {
    let initial_error = match DesktopService::connect() {
        Ok(service) => return Ok(service),
        Err(error) => error,
    };
    if DesktopService::server_availability()? == BackendAvailability::Present {
        // A live server with an incompatible protocol or invalid handshake is
        // not a missing server. Preserve its runs and expose the actual error.
        return Err(initial_error);
    }
    let (executable, gui_app) = packaged_server()?;
    let mut child = Command::new(executable)
        .arg("--gui-app")
        .arg(gui_app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("Could not launch the GPUI server: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last_error = "Backend is starting.".to_string();
    while Instant::now() < deadline {
        match DesktopService::connect() {
            Ok(service) => {
                // Keep a waiter while this GUI remains alive; dropping Child
                // does not terminate the independent backend.
                let _ = std::thread::Builder::new()
                    .name("monitter-server-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return Ok(service);
            }
            Err(error) => last_error = error,
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            // Another simultaneously opening GUI may have won server ownership.
            // Reconnect once; never terminate the winner or remove its socket.
            return DesktopService::connect()
                .map_err(|error| format!("GPUI server exited ({status}): {error}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // A slow server retains its own lifetime. No kill-on-timeout semantics.
    Err(format!(
        "GPUI server connection timed out: {last_error}. The server was not stopped."
    ))
}

#[cfg(test)]
mod tests {
    use super::server_paths;
    use std::path::Path;

    #[cfg(target_os = "macos")]
    #[test]
    fn packaged_macos_pair_uses_sibling_bundles() {
        let (server, gui) = server_paths(Path::new(
            "/build/Monitter GPUI.app/Contents/MacOS/monitter-gpui-desktop",
        ))
        .unwrap();
        assert_eq!(
            server,
            Path::new("/build/Monitter GPUI Server.app/Contents/MacOS/monitter-gpui-server")
        );
        assert_eq!(gui, Path::new("/build/Monitter GPUI.app"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unpackaged_pair_uses_sibling_executables() {
        let (server, gui) = server_paths(Path::new("/build/debug/monitter-gpui-desktop")).unwrap();
        assert_eq!(server, Path::new("/build/debug/monitter-gpui-server"));
        assert_eq!(gui, Path::new("/build/debug/monitter-gpui-desktop"));
    }
}
