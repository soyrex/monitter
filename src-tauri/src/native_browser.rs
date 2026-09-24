//! Local-owner native browser tabs.
//!
//! Each tab is a Tauri child webview backed by the operating system webview
//! (WKWebView, WebView2, or WebKitGTK). This module deliberately does not
//! embed Chromium. On macOS 15.4+, browser children receive an app-private
//! WebExtension controller; extension loading and UI remain separate concerns.
//! External child webviews are not included in Monitter's Tauri capability, so
//! remote pages cannot invoke application commands even if they discover
//! Tauri's JavaScript globals.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[cfg(target_os = "macos")]
use std::{sync::mpsc, time::Duration};
#[cfg(any(target_os = "linux", windows))]
use std::{sync::mpsc, time::Duration};
use tauri::{AppHandle, Emitter, State, Url, Webview, WebviewBuilder, WebviewUrl};
use uuid::Uuid;

const MAX_TAB_ID_LEN: usize = 80;
const MAX_DIMENSION: u32 = 32_768;
const MACOS_PROFILE_UNSUPPORTED: &str = "Native browser tabs require macOS 14 or later for an isolated WKWebView profile; macOS 13 would share the Monitter shell profile.";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub tab_id: String,
    pub url: String,
    /// Empty only while the platform has not reported a document title.
    pub title: String,
    /// `None` means the OS webview does not expose its navigation stack through
    /// Tauri's portable API. It is never guessed from renderer-managed state.
    pub can_go_back: Option<bool>,
    /// See `can_go_back`.
    pub can_go_forward: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionState {
    pub path: String,
    pub display_name: Option<String>,
}

struct BrowserTab {
    webview: Webview<tauri::Wry>,
    state: BrowserState,
}

#[derive(Clone)]
pub struct NativeBrowserState {
    tabs: Arc<Mutex<HashMap<String, BrowserTab>>>,
    profile_dir: PathBuf,
}

impl NativeBrowserState {
    pub fn new(profile_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&profile_dir)
            .map_err(|error| format!("Cannot create native browser profile directory: {error}"))?;
        Ok(Self {
            tabs: Arc::new(Mutex::new(HashMap::new())),
            profile_dir,
        })
    }

    fn tab(
        &self,
        tab_id: &str,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, BrowserTab>>, String> {
        validate_tab_id(tab_id)?;
        self.tabs
            .lock()
            .map_err(|_| "Native browser state is unavailable.".into())
    }
}

fn validate_tab_id(tab_id: &str) -> Result<(), String> {
    if tab_id.is_empty()
        || tab_id.len() > MAX_TAB_ID_LEN
        || !tab_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        || Uuid::parse_str(tab_id).is_err()
    {
        return Err(
            "Browser tab ID must be a UUID using ASCII letters, numbers, and hyphens.".into(),
        );
    }
    Ok(())
}

fn parse_browser_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| {
        "Browser URL must be about:blank or a valid absolute HTTP(S) URL.".to_string()
    })?;
    if url.as_str() == "about:blank" {
        return Ok(url);
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Browser URL must be about:blank or a valid absolute HTTP(S) URL.".into());
    }
    Ok(url)
}

fn validate_bounds(bounds: &BrowserBounds) -> Result<(), String> {
    if bounds.width == 0
        || bounds.height == 0
        || bounds.width > MAX_DIMENSION
        || bounds.height > MAX_DIMENSION
    {
        return Err("Browser bounds must have a non-zero width and height no larger than 32768 logical pixels.".into());
    }
    if bounds.x.unsigned_abs() > MAX_DIMENSION || bounds.y.unsigned_abs() > MAX_DIMENSION {
        return Err("Browser bounds position is outside the supported logical-pixel range.".into());
    }
    Ok(())
}

fn child_label(tab_id: &str) -> String {
    format!("external-browser-{tab_id}")
}

#[cfg(target_os = "macos")]
async fn set_macos_basic_auth_bridge(
    webview: &Webview<tauri::Wry>,
    attach: bool,
    tab_id: Option<&str>,
    app: Option<&AppHandle>,
) -> Result<(), String> {
    let tab_id = if attach {
        Some(
            std::ffi::CString::new(tab_id.ok_or("Native browser tab ID is required.")?)
                .map_err(|_| "Native browser tab ID contains an interior NUL byte.")?,
        )
    } else {
        None
    };
    let app = app.cloned();
    let (sender, receiver) = mpsc::sync_channel(1);
    webview
        .with_webview(move |platform| {
            let installed = if attach {
                match (tab_id.as_deref(), app.as_ref()) {
                    (Some(tab_id), Some(app)) => {
                        crate::native_browser_auth_macos::attach(platform.inner(), tab_id, app)
                    }
                    _ => false,
                }
            } else {
                crate::native_browser_auth_macos::detach(platform.inner());
                true
            };
            let _ = sender.send(installed);
        })
        .map_err(|error| format!("Cannot schedule native browser authentication hook: {error}"))?;
    let installed =
        tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)))
            .await
            .map_err(|error| {
                format!("Native browser authentication hook did not complete: {error}")
            })?
            .map_err(|_| "Native browser authentication hook timed out.".to_string())?;
    if installed {
        Ok(())
    } else {
        Err("Could not attach the native Basic Auth handler to this browser tab.".into())
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum HistoryOperation {
    Read,
    Back,
    Forward,
}

#[cfg(any(target_os = "linux", windows))]
#[derive(Clone, Copy)]
enum HistoryOperation {
    Read,
    Back,
    Forward,
}

#[cfg(target_os = "macos")]
async fn macos_history(
    webview: &Webview<tauri::Wry>,
    operation: HistoryOperation,
) -> Result<(bool, bool), String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    webview
        .with_webview(move |platform| {
            let native = platform.inner();
            let accepted = match operation {
                HistoryOperation::Read => true,
                HistoryOperation::Back => crate::native_browser_auth_macos::go_back(native),
                HistoryOperation::Forward => crate::native_browser_auth_macos::go_forward(native),
            };
            let result = crate::native_browser_auth_macos::history_state(native)
                .map(|flags| (accepted, flags));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("Cannot schedule native browser history operation: {error}"))?;
    let result =
        tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)))
            .await
            .map_err(|error| format!("Native browser history operation did not complete: {error}"))?
            .map_err(|_| "Native browser history operation timed out.".to_string())?
            .ok_or("Native browser history is unavailable for this view.")?;
    // A disabled Back/Forward action is a normal state, not a failed command.
    Ok(result.1)
}

#[cfg(target_os = "macos")]
async fn browser_history_state(
    state: &NativeBrowserState,
    tab_id: &str,
    operation: HistoryOperation,
) -> Result<BrowserState, String> {
    let webview = state
        .tab(tab_id)?
        .get(tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    let (can_go_back, can_go_forward) = macos_history(&webview, operation).await?;
    let native_url = webview
        .url()
        .ok()
        .filter(|url| parse_browser_url(url.as_str()).is_ok());
    let mut tabs = state.tab(tab_id)?;
    let tab = tabs
        .get_mut(tab_id)
        .ok_or("Native browser tab was closed while reading its history.")?;
    tab.state.can_go_back = Some(can_go_back);
    tab.state.can_go_forward = Some(can_go_forward);
    if let Some(url) = native_url {
        tab.state.url = url.to_string();
    }
    Ok(state_for(tab))
}

#[cfg(any(target_os = "linux", windows))]
async fn portable_history(
    webview: &Webview<tauri::Wry>,
    operation: HistoryOperation,
) -> Result<(bool, bool), String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    webview
        .with_webview(move |platform| {
            let operation = match operation {
                HistoryOperation::Read => {
                    crate::native_browser_history_portable::HistoryOperation::Read
                }
                HistoryOperation::Back => {
                    crate::native_browser_history_portable::HistoryOperation::Back
                }
                HistoryOperation::Forward => {
                    crate::native_browser_history_portable::HistoryOperation::Forward
                }
            };
            let result = crate::native_browser_history_portable::history(platform, operation)
                .map(|flags| (flags.can_go_back, flags.can_go_forward));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("Cannot schedule native browser history operation: {error}"))?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)))
        .await
        .map_err(|error| format!("Native browser history operation did not complete: {error}"))?
        .map_err(|_| "Native browser history operation timed out.".to_string())?
}

#[cfg(any(target_os = "linux", windows))]
async fn browser_history_state(
    state: &NativeBrowserState,
    tab_id: &str,
    operation: HistoryOperation,
) -> Result<BrowserState, String> {
    let webview = state
        .tab(tab_id)?
        .get(tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    let (can_go_back, can_go_forward) = portable_history(&webview, operation).await?;
    let native_url = webview
        .url()
        .ok()
        .filter(|url| parse_browser_url(url.as_str()).is_ok());
    let mut tabs = state.tab(tab_id)?;
    let tab = tabs
        .get_mut(tab_id)
        .ok_or("Native browser tab was closed while reading its history.")?;
    tab.state.can_go_back = Some(can_go_back);
    tab.state.can_go_forward = Some(can_go_forward);
    if let Some(url) = native_url {
        tab.state.url = url.to_string();
    }
    Ok(state_for(tab))
}

#[cfg(target_os = "macos")]
fn refresh_history_after_page_load(
    webview: Webview<tauri::Wry>,
    tabs: Arc<Mutex<HashMap<String, BrowserTab>>>,
    tab_id: String,
    app: AppHandle,
) {
    // History changes when WebKit commits a navigation, not when the address
    // is first requested. Reading it after the load event keeps toolbar state
    // grounded in WKWebView rather than reconstructing a stack in JavaScript.
    let _ = webview.with_webview(move |platform| {
        let Some((can_go_back, can_go_forward)) =
            crate::native_browser_auth_macos::history_state(platform.inner())
        else {
            return;
        };
        if let Ok(mut tabs) = tabs.lock() {
            if let Some(tab) = tabs.get_mut(&tab_id) {
                tab.state.can_go_back = Some(can_go_back);
                tab.state.can_go_forward = Some(can_go_forward);
                let _ = app.emit_to(
                    tauri::EventTarget::webview("main"),
                    "monitter:browser-state",
                    state_for(tab),
                );
            }
        }
    });
}

#[cfg(any(target_os = "linux", windows))]
fn refresh_history_after_page_load(
    webview: Webview<tauri::Wry>,
    tabs: Arc<Mutex<HashMap<String, BrowserTab>>>,
    tab_id: String,
    app: AppHandle,
) {
    // WebKitGTK and WebView2 update history on navigation commit. This callback
    // reads their native stacks rather than inferring history from URL changes.
    let url_webview = webview.clone();
    let _ = webview.with_webview(move |platform| {
        let Ok(flags) = crate::native_browser_history_portable::history(
            platform,
            crate::native_browser_history_portable::HistoryOperation::Read,
        ) else {
            return;
        };
        if let Ok(mut tabs) = tabs.lock() {
            if let Some(tab) = tabs.get_mut(&tab_id) {
                tab.state.can_go_back = Some(flags.can_go_back);
                tab.state.can_go_forward = Some(flags.can_go_forward);
                if let Some(url) = url_webview
                    .url()
                    .ok()
                    .filter(|url| parse_browser_url(url.as_str()).is_ok())
                {
                    tab.state.url = url.to_string();
                }
                let _ = app.emit_to(
                    tauri::EventTarget::webview("main"),
                    "monitter:browser-state",
                    state_for(tab),
                );
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn macos_profile_supported() -> bool {
    let Ok(output) = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
    else {
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout);
    version
        .trim()
        .split('.')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .is_some_and(|major| major >= 14)
}

#[cfg(target_os = "macos")]
fn macos_extensions_supported() -> bool {
    let Ok(output) = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
    else {
        return false;
    };
    let version = String::from_utf8_lossy(&output.stdout);
    let mut parts = version
        .trim()
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    major > 15 || (major == 15 && minor >= 4)
}

#[cfg(not(target_os = "macos"))]
fn macos_profile_supported() -> bool {
    true
}

fn ensure_owner(webview: &Webview<tauri::Wry>) -> Result<(), String> {
    if webview.label() == "main" {
        Ok(())
    } else {
        Err("Native browser commands are available only to the local Monitter owner.".into())
    }
}

fn state_for(tab: &BrowserTab) -> BrowserState {
    tab.state.clone()
}

fn apply_layout(
    webview: &Webview<tauri::Wry>,
    bounds: &BrowserBounds,
    visible: bool,
) -> Result<(), String> {
    // Renderer teardown can report a 0x0 rectangle. Hiding must still succeed
    // in that state so a child webview cannot remain as an overlay.
    if !visible && (bounds.width == 0 || bounds.height == 0) {
        return webview
            .hide()
            .map_err(|error| format!("Cannot update native browser tab visibility: {error}"));
    }
    validate_bounds(bounds)?;
    webview
        .set_position(tauri::LogicalPosition::new(
            bounds.x as f64,
            bounds.y as f64,
        ))
        .map_err(|error| format!("Cannot position native browser tab: {error}"))?;
    webview
        .set_size(tauri::LogicalSize::new(
            bounds.width as f64,
            bounds.height as f64,
        ))
        .map_err(|error| format!("Cannot resize native browser tab: {error}"))?;
    if visible {
        webview.show()
    } else {
        webview.hide()
    }
    .map_err(|error| format!("Cannot update native browser tab visibility: {error}"))
}

/// Opens a native child webview. This is async by design: Tauri documents a
/// Windows deadlock risk for synchronous commands that perform webview work.
#[tauri::command]
pub async fn browser_open(
    app: AppHandle,
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
    url: String,
    bounds: BrowserBounds,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    validate_tab_id(&tab_id)?;
    validate_bounds(&bounds)?;
    let url = parse_browser_url(&url)?;
    if !macos_profile_supported() {
        return Err(MACOS_PROFILE_UNSUPPORTED.into());
    }
    {
        let tabs = state.tab(&tab_id)?;
        if tabs.contains_key(&tab_id) {
            return Err("A native browser tab with this ID already exists.".into());
        }
    }
    // The invoke already supplies and authenticates the owner webview. Use
    // its hosting window directly. A separate registry lookup by window label
    // failed during a split/tab smoke test even though this owner was alive.
    let main = owner_webview.window();
    let tabs_for_navigation = Arc::clone(&state.tabs);
    let tab_for_navigation = tab_id.clone();
    let tabs_for_title = Arc::clone(&state.tabs);
    let tab_for_title = tab_id.clone();
    #[cfg(target_os = "macos")]
    let tabs_for_load = Arc::clone(&state.tabs);
    #[cfg(target_os = "macos")]
    let tab_for_load = tab_id.clone();
    #[cfg(any(target_os = "linux", windows))]
    let tabs_for_load = Arc::clone(&state.tabs);
    #[cfg(any(target_os = "linux", windows))]
    let tab_for_load = tab_id.clone();
    let app_for_navigation = app.clone();
    let app_for_title = app.clone();
    #[cfg(target_os = "macos")]
    let app_for_load = app.clone();
    #[cfg(any(target_os = "linux", windows))]
    let app_for_load = app.clone();
    let mut builder = WebviewBuilder::new(child_label(&tab_id), WebviewUrl::External(url.clone()))
        .data_directory(state.profile_dir.clone())
        .on_navigation(move |next_url| {
            // Do not record a rejected navigation: the state is a future
            // restore hint and must never retain a blocked scheme or URL
            // userinfo credential.
            if parse_browser_url(next_url.as_str()).is_err() {
                return false;
            }
            if let Ok(mut tabs) = tabs_for_navigation.lock() {
                if let Some(tab) = tabs.get_mut(&tab_for_navigation) {
                    tab.state.url = next_url.to_string();
                    tab.state.title.clear();
                    let _ = app_for_navigation.emit_to(
                        tauri::EventTarget::webview("main"),
                        "monitter:browser-state",
                        state_for(tab),
                    );
                }
            }
            true
        })
        .on_document_title_changed(move |_webview, title| {
            if let Ok(mut tabs) = tabs_for_title.lock() {
                if let Some(tab) = tabs.get_mut(&tab_for_title) {
                    tab.state.title = title;
                    let _ = app_for_title.emit_to(
                        tauri::EventTarget::webview("main"),
                        "monitter:browser-state",
                        state_for(tab),
                    );
                }
            }
        })
        .on_page_load(move |webview, payload| {
            #[cfg(target_os = "macos")]
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                refresh_history_after_page_load(
                    webview,
                    Arc::clone(&tabs_for_load),
                    tab_for_load.clone(),
                    app_for_load.clone(),
                );
            }
            #[cfg(any(target_os = "linux", windows))]
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                refresh_history_after_page_load(
                    webview,
                    Arc::clone(&tabs_for_load),
                    tab_for_load.clone(),
                    app_for_load.clone(),
                );
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
            let _ = (webview, payload);
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    #[cfg(target_os = "macos")]
    {
        builder = builder.data_store_identifier(*b"monitter-browser");
    }
    #[cfg(target_os = "macos")]
    let webview = if macos_extensions_supported() {
        // WKWebExtensionController and WKWebViewConfiguration are confined to
        // AppKit's main thread. Build the child there; send only Tauri's
        // thread-safe webview handle back to this async command.
        let (sender, receiver) = mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let result = (|| {
                let configuration =
                    crate::native_browser_extensions_macos::child_webview_configuration()?;
                main.add_child(
                    builder.with_webview_configuration(configuration),
                    tauri::LogicalPosition::new(bounds.x as f64, bounds.y as f64),
                    tauri::LogicalSize::new(bounds.width as f64, bounds.height as f64),
                )
                .map_err(|error| format!("Cannot create native browser tab: {error}"))
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| format!("Cannot schedule native browser creation: {error}"))?;
        tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)))
            .await
            .map_err(|error| format!("Native browser creation did not complete: {error}"))?
            .map_err(|_| "Native browser creation timed out.".to_string())??
    } else {
        main.add_child(
            builder,
            tauri::LogicalPosition::new(bounds.x as f64, bounds.y as f64),
            tauri::LogicalSize::new(bounds.width as f64, bounds.height as f64),
        )
        .map_err(|error| format!("Cannot create native browser tab: {error}"))?
    };
    #[cfg(not(target_os = "macos"))]
    let webview = main
        .add_child(
            builder,
            tauri::LogicalPosition::new(bounds.x as f64, bounds.y as f64),
            tauri::LogicalSize::new(bounds.width as f64, bounds.height as f64),
        )
        .map_err(|error| format!("Cannot create native browser tab: {error}"))?;
    // Child webviews default to visible. Hide before installing any bridge or
    // returning state so an in-flight open can never cover another pane's
    // drag/drop target. The renderer explicitly shows the selected child with
    // browser_set_layout once it has left its unloaded state.
    if let Err(error) = webview.hide() {
        let _ = webview.close();
        return Err(format!("Cannot initially hide native browser tab: {error}"));
    }
    #[cfg(target_os = "macos")]
    if let Err(error) = set_macos_basic_auth_bridge(&webview, true, Some(&tab_id), Some(&app)).await
    {
        let _ = webview.close();
        return Err(error);
    }
    let browser_state = BrowserState {
        tab_id: tab_id.clone(),
        url: url.to_string(),
        title: String::new(),
        can_go_back: None,
        can_go_forward: None,
    };
    state.tab(&tab_id)?.insert(
        tab_id,
        BrowserTab {
            webview,
            state: browser_state.clone(),
        },
    );
    Ok(browser_state)
}

#[tauri::command]
pub async fn browser_set_layout(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
    bounds: BrowserBounds,
    visible: bool,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    let webview = state
        .tab(&tab_id)?
        .get(&tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    apply_layout(&webview, &bounds, visible)?;
    state
        .tab(&tab_id)?
        .get(&tab_id)
        .map(state_for)
        .ok_or("Native browser tab was closed while updating its layout.".into())
}

#[tauri::command]
pub async fn browser_navigate(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
    url: String,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    let url = parse_browser_url(&url)?;
    let webview = state
        .tab(&tab_id)?
        .get(&tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    webview
        .navigate(url.clone())
        .map_err(|error| format!("Cannot navigate native browser tab: {error}"))?;
    let mut tabs = state.tab(&tab_id)?;
    let tab = tabs
        .get_mut(&tab_id)
        .ok_or("Native browser tab was closed while navigating.")?;
    tab.state.url = url.to_string();
    tab.state.title.clear();
    Ok(state_for(tab))
}

#[tauri::command]
pub async fn browser_back(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    #[cfg(target_os = "macos")]
    return browser_history_state(&state, &tab_id, HistoryOperation::Back).await;
    #[cfg(any(target_os = "linux", windows))]
    return browser_history_state(&state, &tab_id, HistoryOperation::Back).await;
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = state
            .tab(&tab_id)?
            .get(&tab_id)
            .ok_or("Native browser tab was not found.")?;
        Err("Native browser back is not available on this platform yet.".into())
    }
}

#[tauri::command]
pub async fn browser_forward(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    #[cfg(target_os = "macos")]
    return browser_history_state(&state, &tab_id, HistoryOperation::Forward).await;
    #[cfg(any(target_os = "linux", windows))]
    return browser_history_state(&state, &tab_id, HistoryOperation::Forward).await;
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = state
            .tab(&tab_id)?
            .get(&tab_id)
            .ok_or("Native browser tab was not found.")?;
        Err("Native browser forward is not available on this platform yet.".into())
    }
}

#[tauri::command]
pub async fn browser_reload(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    let webview = state
        .tab(&tab_id)?
        .get(&tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    webview
        .reload()
        .map_err(|error| format!("Cannot reload native browser tab: {error}"))?;
    state
        .tab(&tab_id)?
        .get(&tab_id)
        .map(state_for)
        .ok_or("Native browser tab was closed while reloading.".into())
}

#[tauri::command]
pub async fn browser_close(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
) -> Result<(), String> {
    ensure_owner(&owner_webview)?;
    let webview = state
        .tab(&tab_id)?
        .get(&tab_id)
        .ok_or("Native browser tab was not found.")?
        .webview
        .clone();
    #[cfg(target_os = "macos")]
    set_macos_basic_auth_bridge(&webview, false, None, None).await?;
    webview
        .close()
        .map_err(|error| format!("Cannot close native browser tab: {error}"))?;
    state.tab(&tab_id)?.remove(&tab_id);
    Ok(())
}

#[tauri::command]
pub async fn browser_get_state(
    owner_webview: Webview<tauri::Wry>,
    state: State<'_, NativeBrowserState>,
    tab_id: String,
) -> Result<BrowserState, String> {
    ensure_owner(&owner_webview)?;
    #[cfg(target_os = "macos")]
    return browser_history_state(&state, &tab_id, HistoryOperation::Read).await;
    #[cfg(any(target_os = "linux", windows))]
    return browser_history_state(&state, &tab_id, HistoryOperation::Read).await;
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let tabs = state.tab(&tab_id)?;
        tabs.get(&tab_id)
            .map(state_for)
            .ok_or("Native browser tab was not found.".into())
    }
}

/// Loads an unpacked extension resource directory into the shared macOS
/// browser profile. This is local-owner-only; it never accepts ZIPs, Web Store
/// URLs or a path supplied by a remote browser page.
#[tauri::command]
pub async fn browser_load_unpacked_extension(
    app: AppHandle,
    owner_webview: Webview<tauri::Wry>,
    path: String,
) -> Result<BrowserExtensionState, String> {
    ensure_owner(&owner_webview)?;
    let directory = PathBuf::from(&path);
    if !directory.is_absolute() {
        return Err("Unpacked browser extension path must be absolute.".into());
    }
    #[cfg(target_os = "macos")]
    {
        if !macos_extensions_supported() {
            return Err("Native browser extensions require macOS 15.4 or later.".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let callback_sender = sender.clone();
            if let Err(error) = crate::native_browser_extensions_macos::load_unpacked_extension(
                &directory,
                move |result| {
                    let _ = callback_sender.send(result);
                },
            ) {
                let _ = sender.send(Err(error));
            }
        })
        .map_err(|error| format!("Cannot schedule browser extension load: {error}"))?;
        let result = tauri::async_runtime::spawn_blocking(move || {
            receiver.recv_timeout(Duration::from_secs(60))
        })
        .await
        .map_err(|error| format!("Browser extension load did not complete: {error}"))?
        .map_err(|_| "Browser extension load timed out.".to_string())??;
        return Ok(BrowserExtensionState {
            path: result.directory.to_string_lossy().into_owned(),
            display_name: result.display_name,
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Unpacked browser extensions are not available on this platform yet.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_id_and_url_validation_are_bounded() {
        assert!(validate_tab_id("6ec50cf0-555b-4bd2-89f5-1e2afced5540").is_ok());
        assert!(validate_tab_id("tab_01-a").is_err());
        assert!(validate_tab_id("../tab").is_err());
        assert!(parse_browser_url("https://example.com/a").is_ok());
        assert!(parse_browser_url("about:blank").is_ok());
        assert!(parse_browser_url("file:///tmp/x").is_err());
        assert!(parse_browser_url("https://user:secret@example.com").is_err());
    }
    #[test]
    fn bounds_are_logical_and_nonzero() {
        assert!(validate_bounds(&BrowserBounds {
            x: -12,
            y: 8,
            width: 640,
            height: 480
        })
        .is_ok());
        assert!(validate_bounds(&BrowserBounds {
            x: 0,
            y: 0,
            width: 0,
            height: 480
        })
        .is_err());
    }
    #[test]
    fn browser_state_serializes_camel_case() {
        let json = serde_json::to_value(BrowserState {
            tab_id: "t".into(),
            url: "https://example.com".into(),
            title: String::new(),
            can_go_back: None,
            can_go_forward: None,
        })
        .unwrap();
        assert_eq!(json["tabId"], "t");
        assert!(json["canGoForward"].is_null());
    }
}
