use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Manager, Url, WebviewWindow, Wry};

pub const LOAD_MENU_ID: &str = "load-dev-ui";
pub const PACKAGED_MENU_ID: &str = "use-packaged-ui";
pub const ERROR_EVENT: &str = "monitter-dev-ui-error";
pub const DEV_UI_ORIGIN: &str = "http://127.0.0.1:18420";
const DEV_UI_MARKER: &str = "/__monitter_dev__";
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const MAX_PROBE_BYTES: u64 = 4 * 1024;

#[derive(Clone)]
pub struct DevUiState {
    packaged_url: Arc<Url>,
}

impl DevUiState {
    pub fn capture(app: &AppHandle<Wry>) -> Result<Self, String> {
        let window = main_window(app)?;
        let packaged_url = window
            .url()
            .map_err(|error| format!("Could not remember the packaged interface URL: {error}"))?;
        Ok(Self {
            packaged_url: Arc::new(packaged_url),
        })
    }
}

pub fn enter(app: &AppHandle<Wry>) -> Result<(), String> {
    probe()?;
    let url = Url::parse(&format!("{DEV_UI_ORIGIN}/?monitter-dev-ui=1"))
        .map_err(|error| format!("The hot-reload interface URL is invalid: {error}"))?;
    main_window(app)?
        .navigate(url)
        .map_err(|error| format!("Could not load the hot-reload interface: {error}"))
}

pub fn leave(app: &AppHandle<Wry>, state: &DevUiState) -> Result<(), String> {
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
        "Start Monitter's Vite server with `npm run dev`, then try again.".to_string()
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
    valid_status && body.contains("\"app\":\"monitter\"") && body.contains("\"hotUiProtocol\":1")
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
    }
}
