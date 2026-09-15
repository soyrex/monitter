use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager, Url, WebviewWindow, Wry};

pub const LOAD_MENU_ID: &str = "load-dev-ui";
pub const PACKAGED_MENU_ID: &str = "use-packaged-ui";
pub const ERROR_EVENT: &str = "monitter-dev-ui-error";
pub const DEV_UI_ORIGIN: &str = "http://127.0.0.1:18420";
const DEV_UI_PATH: &str = "/monitter-app-ui/";
const DEV_UI_MARKER: &str = "/monitter-app-ui/__monitter_dev__";
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const WATCH_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_PROBE_FAILURES: u8 = 3;
const MAX_PROBE_BYTES: u64 = 4 * 1024;

#[derive(Clone)]
pub struct DevUiState {
    packaged_url: Arc<Url>,
    navigation_generation: Arc<AtomicU64>,
}

impl DevUiState {
    pub fn capture(app: &AppHandle<Wry>) -> Result<Self, String> {
        let window = main_window(app)?;
        let packaged_url = window
            .url()
            .map_err(|error| format!("Could not remember the packaged interface URL: {error}"))?;
        Ok(Self {
            packaged_url: Arc::new(packaged_url),
            navigation_generation: Arc::new(AtomicU64::new(0)),
        })
    }

    fn start_watchdog(&self, app: AppHandle<Wry>) {
        let generation = self.navigation_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let state = self.clone();
        thread::spawn(move || {
            let mut failures = 0;
            loop {
                thread::sleep(WATCH_INTERVAL);
                if state.navigation_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                failures = if probe().is_ok() { 0 } else { failures + 1 };
                if failures < MAX_CONSECUTIVE_PROBE_FAILURES {
                    continue;
                }
                if state
                    .navigation_generation
                    .compare_exchange(
                        generation,
                        generation + 1,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    let _ = main_window(&app).and_then(|window| {
                        window
                            .navigate((*state.packaged_url).clone())
                            .map_err(|error| {
                                format!("Could not restore the packaged interface: {error}")
                            })
                    });
                }
                return;
            }
        });
    }

    fn cancel_watchdog(&self) {
        self.navigation_generation.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn enter(app: &AppHandle<Wry>) -> Result<(), String> {
    probe()?;
    let url = Url::parse(&format!("{DEV_UI_ORIGIN}{DEV_UI_PATH}?monitter-dev-ui=1"))
        .map_err(|error| format!("The hot-reload interface URL is invalid: {error}"))?;
    main_window(app)?
        .navigate(url)
        .map_err(|error| format!("Could not load the hot-reload interface: {error}"))?;
    app.state::<DevUiState>().start_watchdog(app.clone());
    Ok(())
}

pub fn leave(app: &AppHandle<Wry>, state: &DevUiState) -> Result<(), String> {
    state.cancel_watchdog();
    main_window(app)?
        .navigate((*state.packaged_url).clone())
        .map_err(|error| format!("Could not restore the packaged interface: {error}"))
}

fn main_window(app: &AppHandle<Wry>) -> Result<WebviewWindow<Wry>, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "The main Monitter window is unavailable.".to_string())
}

fn probe() -> Result<(), String> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 18_420);
    let mut stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT).map_err(|_| {
        "Start Monitter's Vite bridge with `npm run dev:app-ui`, then try again.".to_string()
    })?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT)).ok();
    stream.set_write_timeout(Some(PROBE_TIMEOUT)).ok();
    stream
        .write_all(
            format!(
                "GET {DEV_UI_MARKER} HTTP/1.1\r\nHost: 127.0.0.1:18420\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .map_err(|_| "Could not query the Monitter Vite server.".to_string())?;
    let mut response = String::new();
    stream
        .take(MAX_PROBE_BYTES)
        .read_to_string(&mut response)
        .map_err(|_| "Could not read the Monitter Vite server marker.".to_string())?;
    if is_compatible_marker_response(&response) {
        Ok(())
    } else {
        Err("Port 18420 is not serving a compatible Monitter hot-reload interface.".into())
    }
}

fn is_compatible_marker_response(response: &str) -> bool {
    let Some((headers, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    let valid_status = headers.starts_with("HTTP/1.1 200 ") || headers.starts_with("HTTP/1.0 200 ");
    valid_status
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .is_some_and(|marker| {
                marker.get("app").and_then(serde_json::Value::as_str) == Some("monitter")
                    && marker
                        .get("hotUiProtocol")
                        .and_then(serde_json::Value::as_u64)
                        == Some(1)
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_ui_origin_is_exact_loopback() {
        let url = Url::parse(DEV_UI_ORIGIN).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(18_420));
        assert_eq!(DEV_UI_PATH, "/monitter-app-ui/");
    }

    #[test]
    fn accepts_only_a_successful_compatible_marker() {
        assert!(is_compatible_marker_response(
            "HTTP/1.1 200 OK\r\n\r\n{\"app\":\"monitter\",\"hotUiProtocol\":1}"
        ));
        assert!(!is_compatible_marker_response(
            "HTTP/1.1 404 Not Found\r\n\r\n{\"app\":\"monitter\",\"hotUiProtocol\":1}"
        ));
        assert!(!is_compatible_marker_response(
            "HTTP/1.1 200 OK\r\n\r\n{\"app\":\"another-app\",\"hotUiProtocol\":1}"
        ));
        assert!(!is_compatible_marker_response(
            "HTTP/1.1 200 OK\r\nX-Fake: {\"app\":\"monitter\",\"hotUiProtocol\":1}\r\n\r\n{}"
        ));
        assert!(!is_compatible_marker_response(
            "HTTP/1.1 200 OK\r\n\r\n{\"app\":\"monitter\",\"hotUiProtocol\":1} trailing"
        ));
    }
}
