//! Local native browser surface. External documents receive no Monitter IPC.
//!
//! Host integration must call `hide` before unmounting this retained pane and
//! `close` before closing its window. Render one frame without this pane before
//! removing the window: gpui-wry frame elements retain native Rc handles.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::IconName,
    base::Selectable,
    component::{
        ActiveTheme as _, Disableable, Icon, Sizable, ThemeStyled,
        button::*,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use gpui_wry::WebView;
pub(crate) use monitter_lib::experimental_desktop::{
    NativeBrowserMetadata as BrowserMetadataSnapshot, NativeBrowserTab as BrowserTabMetadata,
};
use std::{collections::HashSet, time::Duration};
#[cfg(feature = "performance-profiling")]
use std::sync::atomic::{AtomicU64, Ordering};
use url::Url;
use futures::StreamExt as _;
#[path = "browser_omnibar.rs"]
mod omnibar;
#[path = "browser_snapshot.rs"]
mod snapshot;
#[path = "browser_keyboard.rs"]
mod keyboard;
#[path = "browser_controls.rs"]
mod controls;
#[path = "browser_auth.rs"]
mod auth;
#[path = "browser_context.rs"]
mod browser_context;

#[cfg(feature = "performance-profiling")]
static SHOW_REQUESTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static HIDE_REQUESTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static SUCCESSFUL_SHOW_TRANSITIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static SUCCESSFUL_HIDE_TRANSITIONS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "performance-profiling")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BrowserPerformanceCounts {
    pub show_requests: u64,
    pub hide_requests: u64,
    pub successful_show_transitions: u64,
    pub successful_hide_transitions: u64,
}

#[cfg(feature = "performance-profiling")]
pub(crate) fn performance_counts() -> BrowserPerformanceCounts {
    BrowserPerformanceCounts {
        show_requests: SHOW_REQUESTS.load(Ordering::Relaxed),
        hide_requests: HIDE_REQUESTS.load(Ordering::Relaxed),
        successful_show_transitions: SUCCESSFUL_SHOW_TRANSITIONS.load(Ordering::Relaxed),
        successful_hide_transitions: SUCCESSFUL_HIDE_TRANSITIONS.load(Ordering::Relaxed),
    }
}

#[cfg(target_os = "macos")]
use wry::{WebViewBuilderExtDarwin, WebViewExtMacOS};

gpui_kit::actions!(
    native_browser,
    [
        FocusAddress,
        ReloadTab,
        BackTab,
        ForwardTab,
        NewTab,
        CloseTab,
        NextTab,
        PreviousTab
    ]
);

const STORE_ID: [u8; 16] = *b"monitter-gpui-v1";
const MAX_BROWSER_TABS: usize = 64;
const MAX_BROWSER_URL_BYTES: usize = 8192;
const MAX_BROWSER_TITLE_BYTES: usize = 512;
const MAX_BROWSER_METADATA_BYTES: usize = 256 * 1024;

fn normalized_font_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(8. / 14., 32. / 14.)
    } else {
        1.
    }
}

#[derive(Clone, Debug)]
struct RestoredTabMetadata {
    id: u64,
    requested_url: String,
    actual_url: Option<String>,
    title: String,
}

#[derive(Clone, Debug)]
struct BrowserRestorePlan {
    tabs: Vec<RestoredTabMetadata>,
    selected_id: Option<u64>,
    next_id: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum BrowserPaneEvent {
    OpenInNewPane { source_id: u64, url: String },
    MetadataChanged(BrowserMetadataSnapshot),
    WorkspaceAction {
        source_id: u64,
        action: BrowserWorkspaceAction,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserWorkspaceAction {
    NewTab,
    CloseTab,
    ToggleZoom,
    NextTab,
    PreviousTab,
}

#[derive(Clone, Debug)]
enum BrowserTabEvent {
    OpenInNewPane(String),
    MetadataChanged,
    AddressEdited,
    WorkspaceAction(BrowserWorkspaceAction),
}

fn bounded_title(value: &str) -> String {
    let mut title = String::new();
    for character in value.chars().filter(|character| !character.is_control()) {
        if title.len() + character.len_utf8() > MAX_BROWSER_TITLE_BYTES {
            break;
        }
        title.push(character);
    }
    title
}

fn restore_revision_is_current(current: u64, expected: u64) -> bool {
    current == expected
}

fn is_stale_initial_blank(pending: bool, location: &str, actual_url: Option<&str>) -> bool {
    pending && location == "about:blank" && actual_url.is_none()
}

fn is_stale_initial_title(pending: bool, actual_url: Option<&str>) -> bool {
    pending && actual_url.is_none()
}

fn restore_plan(snapshot: &BrowserMetadataSnapshot) -> Result<BrowserRestorePlan, String> {
    if snapshot.tabs.len() > MAX_BROWSER_TABS {
        return Err("Saved browser tabs exceed their limits.".into());
    }
    if snapshot.tabs.is_empty() {
        if snapshot.selected_id.is_some() {
            return Err("Saved browser tab selection is invalid.".into());
        }
        return Ok(BrowserRestorePlan {
            tabs: Vec::new(),
            selected_id: None,
            next_id: 1,
        });
    }
    let mut ids = HashSet::with_capacity(snapshot.tabs.len());
    let mut tabs = Vec::with_capacity(snapshot.tabs.len());
    let mut byte_count = 0usize;
    let mut max_id = 0u64;
    for tab in &snapshot.tabs {
        if tab.id == 0 || !ids.insert(tab.id) {
            return Err("Saved browser tab IDs are invalid or duplicated.".into());
        }
        if tab.requested_url.len() > MAX_BROWSER_URL_BYTES
            || tab
                .actual_url
                .as_ref()
                .is_some_and(|url| url.len() > MAX_BROWSER_URL_BYTES)
            || tab.title.len() > MAX_BROWSER_TITLE_BYTES
            || tab.title.chars().any(char::is_control)
        {
            return Err("Saved browser metadata exceeds its size limits.".into());
        }
        let requested_url = validated_url(&tab.requested_url)?.to_string();
        let actual_url = tab
            .actual_url
            .as_deref()
            .map(validated_url)
            .transpose()?
            .map(|url| url.to_string());
        byte_count = byte_count
            .checked_add(requested_url.len())
            .and_then(|size| size.checked_add(actual_url.as_ref().map_or(0, String::len)))
            .and_then(|size| size.checked_add(tab.title.len()))
            .ok_or("Saved browser metadata exceeds its size limits.")?;
        if byte_count > MAX_BROWSER_METADATA_BYTES {
            return Err("Saved browser metadata exceeds its aggregate size limit.".into());
        }
        max_id = max_id.max(tab.id);
        tabs.push(RestoredTabMetadata {
            id: tab.id,
            requested_url,
            actual_url,
            title: tab.title.clone(),
        });
    }
    let selected_id = snapshot
        .selected_id
        .filter(|selected| ids.contains(selected))
        .ok_or("Saved browser tab selection is invalid.")?;
    let next_id = max_id
        .checked_add(1)
        .ok_or("Saved browser tab IDs are exhausted.")?;
    Ok(BrowserRestorePlan {
        tabs,
        selected_id: Some(selected_id),
        next_id,
    })
}

enum BrowserEvent {
    PageLoad { loading: bool, location: String },
    Location(String),
    Title(String),
    Error(String),
}

fn validated_url(text: &str) -> Result<Url, String> {
    monitter_lib::browser_links::validate_saved_url(text)?;
    Url::parse(text.trim()).map_err(|error| error.to_string())
}

/// WKWebView needs an explicit, narrow read grant for file pages and their
/// sibling assets. Wry's generic load_url uses loadRequest on macOS.
fn load_browser_url(view: &wry::WebView, url: &Url) -> Result<(), String> {
    if url.scheme() != "file" {
        return view.load_url(url.as_str()).map_err(|error| error.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::{msg_send, runtime::AnyObject};
        use objc2_foundation::{NSURL, NSString};
        let path = url.to_file_path().map_err(|_| "Invalid local HTML path.")?;
        let parent = path.parent().ok_or("Local HTML has no parent folder.")?;
        let directory = Url::from_directory_path(parent).map_err(|_| "Invalid HTML folder.")?;
        let file = NSURL::URLWithString(&NSString::from_str(url.as_str())).ok_or("Invalid HTML URL.")?;
        let scope = NSURL::URLWithString(&NSString::from_str(directory.as_str())).ok_or("Invalid HTML folder URL.")?;
        let native = view.webview();
        // The retained Wry view is owned and used on GPUI's main thread.
        let navigation: *mut AnyObject = unsafe {
            msg_send![&*native, loadFileURL: &*file, allowingReadAccessToURL: &*scope]
        };
        if navigation.is_null() { return Err("Native browser rejected the HTML file.".into()); }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    { Err("Local HTML browsing currently requires macOS.".into()) }
}

// Used only when the optional installed-Safari metadata is unavailable or
// slow. Retain Safari's complete browser identity rather than block first use.
const FALLBACK_SAFARI_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

#[cfg(target_os = "macos")]
fn native_browser_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        // A cached in-process sysctl, never a child process or filesystem read.
        let mut version = [0u8; 64];
        let mut length = version.len();
        let status = unsafe { libc::sysctlbyname(c"kern.osproductversion".as_ptr(),
            version.as_mut_ptr().cast(), &mut length, std::ptr::null_mut(), 0) };
        status == 0 && std::str::from_utf8(&version[..length.min(version.len())])
            .ok().and_then(|version| version.split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 14)
    })
}

/// Match the installed desktop Safari rather than WKWebView's incomplete
/// browser identity. Safari deliberately freezes its macOS/WebKit tokens;
/// the Version token follows the local Safari app, including after upgrades.
#[cfg(target_os = "macos")]
fn prepare_native_browser() -> Result<&'static str, String> {
    static USER_AGENT: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    USER_AGENT.get_or_init(|| {
        // Called only by the background executor. Cache both checks across all
        // tabs, without spawning sw_vers or PlistBuddy on the foreground thread.
        let _ = native_browser_supported();
        let info = plist::Value::from_file("/Applications/Safari.app/Contents/Info.plist")
            .map_err(|error| format!("Cannot read the local Safari version: {error}"))?;
        let version = info.as_dictionary()
            .and_then(|info| info.get("CFBundleShortVersionString"))
            .and_then(plist::Value::as_string)
            .ok_or("Cannot identify the installed Safari version.")?;
        if version.is_empty() || version.split('.').any(|part| part.is_empty()
            || !part.bytes().all(|c| c.is_ascii_digit())) {
            return Err("Cannot identify the installed Safari version.".into());
        }
        Ok(format!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{version} Safari/605.1.15"))
    }).as_ref().map(String::as_str).map_err(Clone::clone)
}

#[cfg(not(target_os = "macos"))]
fn prepare_native_browser() -> Result<&'static str, String> {
    Err("This experimental browser currently supports macOS only.".into())
}

fn create_native(
    window: &Window,
    events: futures::channel::mpsc::UnboundedSender<BrowserEvent>,
    user_agent: &str,
) -> Result<wry::WebView, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, events, user_agent);
        return Err("This experimental browser currently supports macOS only.".into());
    }
    #[cfg(target_os = "macos")]
    {
        // Preparation normally warms this cached API check in the background.
        // Always enforce it before using the isolated data-store identifier.
        if !native_browser_supported() {
            return Err("The isolated browser requires macOS 14 or later.".into());
        }
        let navigation = events.clone();
        let title = events.clone();
        wry::WebViewBuilder::new()
            .with_user_agent(user_agent)
            .with_url("about:blank")
            .with_visible(false)
            .with_focused(false)
            .with_data_store_identifier(STORE_ID)
            .with_navigation_handler(move |location| match validated_url(&location) {
                Ok(_) => {
                    // Policy precedes didCommit, so clicked links glow during
                    // provisional network loading as well as document loading.
                    if location != "about:blank" {
                        let _ = navigation.unbounded_send(BrowserEvent::PageLoad { loading: true, location });
                    }
                    true
                }
                Err(error) => {
                    let _ = navigation.unbounded_send(BrowserEvent::Error(error));
                    false
                }
            })
            .with_on_page_load_handler(move |event, location| {
                let loading = matches!(event, wry::PageLoadEvent::Started);
                let _ = events.unbounded_send(BrowserEvent::PageLoad { loading, location: location.clone() });
                let _ = events.unbounded_send(BrowserEvent::Location(location));
            })
            .with_document_title_changed_handler(move |value| {
                let _ = title.unbounded_send(BrowserEvent::Title(value));
            })
            // Popup-to-tab routing is not wired yet; never open an untracked
            // native view outside the retained tab manager.
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .build_as_child(window)
            .map_err(|error| format!("Cannot create native browser: {error}"))
    }
}

/// Retain the destination while native ownership changes. A raw AppKit view
/// alone is insufficient: a detached window may still be opening or closing.
#[cfg(target_os = "macos")]
fn reparent_target(window: &Window) -> Result<objc2::rc::Retained<objc2::runtime::AnyObject>, String> {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("Browser transfer requires a native AppKit window.".into());
    };
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let native: *mut AnyObject = msg_send![view, window];
        let native = Retained::retain(native).ok_or("The destination browser window is unavailable.")?;
        let content: *mut AnyObject = msg_send![&*native, contentView];
        if content.is_null() { return Err("The destination browser content view is unavailable.".into()); }
        Ok(native)
    }
}

#[cfg(feature = "performance-profiling")]
fn create_performance_native(window: &Window) -> Result<wry::WebView, String> {
    create_private_static_native(window, "<!doctype html><meta charset=utf-8><title>Fixture</title><body>Static browser fixture</body>")
}

#[cfg(feature = "performance-profiling")]
fn create_private_static_native(window: &Window, html: &'static str) -> Result<wry::WebView, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, html);
        Err("The native browser performance fixture requires macOS.".into())
    }
    #[cfg(target_os = "macos")]
    {
        // In-memory/private WKWebView with static content; it never touches
        // Monitter's browser datastore or requests a network resource.
        wry::WebViewBuilder::new()
            .with_html(html)
            .with_incognito(true)
            .with_visible(false)
            .with_focused(false)
            .with_navigation_handler(|location| location == "about:blank")
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .build_as_child(window)
            .map_err(|error| format!("Cannot create private performance browser: {error}"))
    }
}

pub(crate) struct BrowserTab {
    focus: FocusHandle,
    address: Entity<InputState>,
    webview: Option<Entity<WebView>>,
    host_window: AnyWindowHandle,
    native_deferred: bool,
    _native_init: Option<Task<()>>,
    pending_navigation: Option<(String, Option<String>)>,
    #[cfg(target_os = "macos")]
    applied_corner_radius: Option<f64>,
    location: String,
    requested_url: String,
    actual_url: Option<String>,
    title: String,
    error: Option<String>,
    visible: bool,
    #[cfg(feature = "performance-profiling")]
    native_visible: bool,
    pending_address: Option<String>,
    dirty_address: bool,
    omnibar_open: bool,
    initial_address_pending: bool,
    omnibar_generation: u64,
    omnibar_snapshot: Option<std::sync::Arc<RenderImage>>,
    unloaded: bool,
    loading: bool,
    _loading_watch: Option<Task<()>>,
    docked_mode: bool,
    dock_group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
    pane_toolbar_hovered: bool,
    page_edge_colour: Option<u32>,
    page_edge_generation: u64,
    _page_edge_watch: Option<Task<()>>,
    _omnibar_snapshot_task: Option<Task<()>>,
    palette: Option<crate::design::Palette>,
    interface_font_ratio: f32,
    restoring_metadata: bool,
    ignore_initial_blank_event: bool,
    _enter: Subscription,
    _release: Subscription,
    _quit: Subscription,
    _events: Task<()>,
    address_shortcut: Option<keyboard::AddressShortcut>,
    browser_controls: Option<controls::BrowserControls>,
    browser_auth: Option<auth::BrowserAuth>,
    context_click: Option<browser_context::ContextClick>,
    _context_clicks: Task<()>,
    context_menu: Option<Option<String>>,
    context_generation: u64,
    context_point: (f32, f32),
    context_pane_size: std::rc::Rc<std::cell::Cell<Size<Pixels>>>,
    context_snapshot: Option<std::sync::Arc<RenderImage>>,
    context_capture_cancel: Option<snapshot::Cancellation>,
    _address_shortcuts: Task<()>,
    _control_actions: Task<()>,
}

impl EventEmitter<BrowserTabEvent> for BrowserTab {}

impl Focusable for BrowserTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BrowserTab {
    pub(crate) fn is_loading(&self) -> bool {
        self.loading && !self.unloaded
    }

    fn page_load(&mut self, loading: bool, location: &str, cx: &mut Context<Self>) {
        // Initial about:blank callbacks can arrive after an actual navigation
        // was requested. They must neither start nor finish that navigation.
        if self.unloaded || location == "about:blank" { return; }
        self.set_loading(loading, cx);
    }

    fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        self._loading_watch.take();
        if loading {
            self._page_edge_watch.take();
            self.page_edge_colour = None;
            self.page_edge_generation = self.page_edge_generation.wrapping_add(1);
            // Pinned Wry emits didCommit/didFinish, but exposes no failure
            // callback. Observe WKWebView's real loading state only while a
            // navigation is pending, so failed/cancelled loads also settle.
            if self.visible { self._loading_watch = Some(cx.spawn(async move |tab, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let keep_watching = tab.update(cx, |tab, cx| {
                        if !tab.visible || !tab.loading || tab.unloaded { return false; }
                        let native_loading = tab.webview.as_ref().is_some_and(|view| {
                            #[cfg(target_os = "macos")]
                            { unsafe { view.read(cx).raw().webview().isLoading() } }
                            #[cfg(not(target_os = "macos"))]
                            { let _ = view; false }
                        });
                        if !native_loading {
                            tab.loading = false;
                            cx.notify();
                        }
                        native_loading
                    }).unwrap_or(false);
                    if !keep_watching { break; }
                }
            })); }
        }
        cx.notify();
    }

    pub(crate) fn request_search_text(&self, callback: impl Fn(String) + Send + Sync + 'static, cx: &App) {
        let callback = std::sync::Arc::new(callback);
        let Some(view) = &self.webview else { callback(String::new()); return; };
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = completed.clone();
        let deliver = callback.clone();
        let result = view.read(cx).raw().evaluate_script_with_callback(
            "(() => { try { return (document.body?.innerText || '').slice(0, 524288); } catch (_) { return ''; } })()",
            move |result| {
                if done.swap(true, std::sync::atomic::Ordering::Relaxed) { return; }
                let value = serde_json::from_str::<String>(&result).unwrap_or_default();
                let mut end = value.len().min(512 * 1024);
                while !value.is_char_boundary(end) { end -= 1; }
                deliver(value[..end].to_owned());
            },
        );
        if result.is_err() && !completed.swap(true, std::sync::atomic::Ordering::Relaxed) {
            callback(String::new());
        }
    }
    pub(crate) fn has_native_close_control(&self) -> bool {
        cfg!(target_os = "macos") && self.webview.is_some() && self.browser_controls.is_some()
            && !self.unloaded && !self.omnibar_open && self.context_menu.is_none()
    }

    #[cfg(feature = "performance-profiling")]
    fn record_native_visibility(&mut self, visible: bool, succeeded: bool) {
        if !succeeded || self.native_visible == visible {
            return;
        }
        self.native_visible = visible;
        if visible {
            SUCCESSFUL_SHOW_TRANSITIONS.fetch_add(1, Ordering::Relaxed);
        } else {
            SUCCESSFUL_HIDE_TRANSITIONS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn set_dock_group(
        &mut self,
        group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
        cx: &mut Context<Self>,
    ) {
        if group.is_none() {
            self.pane_toolbar_hovered = false;
        }
        self.dock_group = group;
        cx.notify();
    }

    #[cfg(target_os = "macos")]
    fn apply_native_tile_corners(&mut self, cx: &App) {
        use objc2::{class, msg_send};
        use objc2::runtime::AnyObject;

        let radius = if crate::design::tile_theme()
            == monitter_lib::experimental_desktop::NativeTileTheme::Tern
        {
            f64::from(f32::from(crate::design::ui_scaled_px(8.)))
        } else {
            0.
        };
        if self.applied_corner_radius == Some(radius) {
            return;
        }
        let Some(view) = &self.webview else {
            return;
        };
        let native = view.read(cx).raw().webview();
        // The retained Wry handle owns WKWebView, and GPUI renders on the
        // AppKit main thread. Native child views sit above GPUI's corner masks.
        unsafe {
            let _: () = msg_send![&*native, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![&*native, layer];
            if layer.is_null() {
                // Do not cache a failed application; retry once a layer exists.
                return;
            }
            // The chromeless WebView owns all four pane corners.
            let all_corners: usize = 1 | 2 | 4 | 8;
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            let _: () = msg_send![layer, setMaskedCorners: all_corners];
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: radius > 0.];
            let _: () = msg_send![class!(CATransaction), commit];
        }
        self.applied_corner_radius = Some(radius);
    }

    pub(crate) fn open_omnibar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_address(window, cx);
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible { return; }
        self.context_menu = None;
        self.context_snapshot = None;
        if let Some(cancel) = self.context_capture_cancel.take() { cancel.cancel(); }
        self.context_generation = self.context_generation.wrapping_add(1);
        self.omnibar_generation = self.omnibar_generation.wrapping_add(1);
        let generation = self.omnibar_generation;
        self._omnibar_snapshot_task.take();
        let request = if !self.unloaded && !self.omnibar_open {
            self.webview.as_ref().map(|view| snapshot::request(view.read(cx).raw()))
        } else { None };
        // Focus is independent of WebKit completing (or never completing) a
        // snapshot. Attach the optional backdrop only while this overlay lives.
        self.finish_open_omnibar(window, cx);
        if let Some(request) = request {
            self._omnibar_snapshot_task = Some(cx.spawn(async move |tab, cx| {
                let capture = match futures::future::select(
                    Box::pin(request.receive()),
                    Box::pin(cx.background_executor().timer(Duration::from_millis(150))),
                ).await {
                    futures::future::Either::Left((capture, _)) => capture,
                    futures::future::Either::Right(_) => None,
                };
                let image = cx.background_executor().spawn(async move {
                    capture.and_then(snapshot::decode).map(|mut rgba| {
                        for pixel in rgba.pixels_mut() { pixel.0.swap(0, 2); }
                        std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
                    })
                }).await;
                let _ = tab.update(cx, |tab, cx| {
                    if tab.omnibar_generation != generation || !tab.visible || !tab.omnibar_open { return; }
                    tab.omnibar_snapshot = image;
                    cx.notify();
                });
            }));
        }
    }

    fn finish_open_omnibar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(controls) = &self.browser_controls { controls.hide(); }
        self.omnibar_open = true;
        self._page_edge_watch.take();
        self.restoring_metadata = true;
        let address = if self.location == "about:blank" { String::new() } else { self.location.clone() };
        self.address.update(cx, |input, cx| input.set_value(address, window, cx));
        self.restoring_metadata = false;
        if let Some(view) = &self.webview { view.update(cx, |view, _| view.hide()); }
        self.address.read(cx).focus_handle(cx).focus(window, cx);
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
        cx.notify();
    }

    fn dismiss_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.context_snapshot = None;
        if let Some(cancel) = self.context_capture_cancel.take() { cancel.cancel(); }
        self.context_generation = self.context_generation.wrapping_add(1);
        self.focus.focus(window, cx);
        if self.visible && !self.unloaded && !self.omnibar_open && !crate::session_lock::locked(cx) {
            if let Some(view) = &self.webview { view.update(cx, |view, _| { view.show(); let _ = view.raw().focus(); }); }
        }
        cx.notify();
    }

    fn dismiss_omnibar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.omnibar_open = false;
        self._omnibar_snapshot_task.take();
        self.omnibar_generation = self.omnibar_generation.wrapping_add(1);
        self.omnibar_snapshot = None;
        self.dirty_address = false;
        self.focus.focus(window, cx);
        if self.visible && !self.unloaded && !crate::session_lock::locked(cx) {
            if let Some(view) = &self.webview {
                view.update(cx, |view, _| { view.show(); let _ = view.raw().focus(); });
            }
        }
        cx.notify();
    }

    fn submit_omnibar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(cx);
        if self.error.is_none() { self.dismiss_omnibar(window, cx); }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut tab = Self::new_deferred(window, cx);
        tab.start_native(window, cx);
        tab
    }

    fn new_deferred(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut tab = Self::new_with_native(window, cx, Err(String::new()), None);
        tab.error = None;
        tab.native_deferred = true;
        tab
    }

    fn start_native(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.native_deferred || self._native_init.is_some() { return; }
        self._native_init = Some(cx.spawn_in(window, async move |tab, cx| {
            let metadata = cx.background_executor().spawn(async { prepare_native_browser() });
            let user_agent = match futures::future::select(
                metadata,
                Box::pin(cx.background_executor().timer(Duration::from_millis(100))),
            ).await {
                futures::future::Either::Left((result, _)) => result.unwrap_or(FALLBACK_SAFARI_USER_AGENT),
                futures::future::Either::Right(_) => FALLBACK_SAFARI_USER_AGENT,
            };
            let _ = tab.update_in(cx, |tab, window, cx| {
                if !tab.native_deferred { return; }
                tab.native_deferred = false;
                let (events, receiver) = futures::channel::mpsc::unbounded();
                let native = create_native(window, events, user_agent);
                // Reuse the normal native hook installation while preserving
                // this entity's input, metadata, focus, visibility and dock state.
                let ready = Self::new_with_native(window, cx, native, Some(receiver));
                tab.webview = ready.webview;
                tab.error = ready.error;
                #[cfg(target_os = "macos")]
                { tab.applied_corner_radius = ready.applied_corner_radius; }
                tab.address_shortcut = ready.address_shortcut;
                tab.browser_controls = ready.browser_controls;
                tab.browser_auth = ready.browser_auth;
                tab.context_click = ready.context_click;
                tab._context_clicks = ready._context_clicks;
                tab._address_shortcuts = ready._address_shortcuts;
                tab._control_actions = ready._control_actions;
                tab._events = ready._events;
                if tab.webview.is_some() {
                    if let Some((url, title)) = tab.pending_navigation.take() {
                        let _ = tab.open_url(&url, title.as_deref(), cx);
                    }
                    if tab.visible { tab.show(cx); }
                }
                cx.notify();
            });
        }));
    }

    #[cfg(feature = "performance-profiling")]
    pub(crate) fn performance_fixture(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let native = create_performance_native(window);
        let mut tab = Self::new_with_native(window, cx, native, None);
        tab.unloaded = false;
        tab.title = "Static browser fixture".into();
        tab.docked_mode = true;
        tab
    }

    fn new_with_native(
        window: &mut Window,
        cx: &mut Context<Self>,
        native: Result<wry::WebView, String>,
        receiver: Option<futures::channel::mpsc::UnboundedReceiver<BrowserEvent>>,
    ) -> Self {
        let address = cx.new(|cx| InputState::new(window, cx).placeholder("Address or search"));
        let focus = cx.focus_handle();
        let (webview, mut error) = match native {
            Ok(raw) => (Some(cx.new(|cx| WebView::new(raw, window, cx))), None),
            Err(error) => (None, Some(error)),
        };
        let (context_click, context_receiver) = if let Some(view) = &webview {
            let (hook, receiver) = browser_context::ContextClick::install(view.read(cx).raw());
            (Some(hook), Some(receiver))
        } else { (None, None) };
        let context_clicks = cx.spawn_in(window, async move |tab, cx| {
            let Some(mut receiver) = context_receiver else { return; };
            while let Some((x, y)) = receiver.next().await {
                let (send, receive) = futures::channel::oneshot::channel();
                let send = std::sync::Mutex::new(Some(send));
                let request = tab.update(cx, |tab, cx| {
                    if !tab.visible || tab.unloaded || tab.omnibar_open { return None; }
                    if let Some(cancel) = tab.context_capture_cancel.take() { cancel.cancel(); }
                    tab.context_generation = tab.context_generation.wrapping_add(1);
                    let generation = tab.context_generation;
                    let location = tab.location.clone();
                    let view = tab.webview.as_ref()?;
                    view.read(cx).raw().evaluate_script_with_callback(&browser_context::hit_test(x,y), move |value| { if let Some(send) = send.lock().ok().and_then(|mut send| send.take()) { let _ = send.send(value); } }).ok()?;
                    Some((generation, location))
                }).ok().flatten();
                let Some((generation, location)) = request else { continue; };
                let result = match futures::future::select(
                    Box::pin(receive),
                    Box::pin(cx.background_executor().timer(Duration::from_millis(500))),
                ).await {
                    futures::future::Either::Left((result, _)) => result.ok(),
                    futures::future::Either::Right(_) => None,
                };
                let Some(result) = result else { continue; };
                let snapshot_receiver = tab.update(cx, |tab,cx| {
                    if !tab.visible || tab.unloaded || tab.context_generation != generation || tab.location != location { return None; }
                    let request = tab.webview.as_ref().map(|view| snapshot::request(view.read(cx).raw()))?;
                    tab.context_capture_cancel = Some(request.cancellation());
                    Some(request)
                }).ok().flatten();
                let image = if let Some(receiver) = snapshot_receiver {
                    let capture = match futures::future::select(
                        Box::pin(receiver.receive()),
                        Box::pin(cx.background_executor().timer(Duration::from_millis(150))),
                    ).await {
                        futures::future::Either::Left((capture, _)) => capture,
                        futures::future::Either::Right(_) => None,
                    };
                    cx.background_executor().spawn(async move {
                        capture.and_then(snapshot::decode).map(|mut rgba| {
                            for pixel in rgba.pixels_mut() { pixel.0.swap(0,2); }
                            std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]))
                        })
                    }).await
                } else { None };
                let value = serde_json::from_str::<serde_json::Value>(&result).unwrap_or_default();
                if tab.update_in(cx, |tab, window, cx| {
                    if !browser_context::current(tab.visible && !tab.unloaded && !tab.omnibar_open && !crate::session_lock::locked(cx), tab.context_generation, generation, &tab.location, &location) { return; }
                    let link = match value["kind"].as_str() {
                        Some("page") | Some("blocked") => None,
                        Some("link") => match value["url"].as_str().and_then(|value| browser_context::target_for_page(value, &location)) {
                            Some(url) => Some(url),
                            None => { tab.error = Some("This link cannot be opened safely.".into()); cx.notify(); return; }
                        },
                        _ => { tab.error = Some("Context actions are unavailable inside this frame.".into()); cx.notify(); return; }
                    };
                    if let Some(controls) = &tab.browser_controls { controls.hide(); }
                    if let Some(view) = &tab.webview { view.update(cx, |view, _| view.hide()); }
                    tab.context_snapshot = image;
                    tab.context_point = (x as f32,y as f32);
                    tab.context_menu = Some(link);
                    tab.focus.focus(window, cx);
                    cx.notify();
                }).is_err() { break; }
            }
        });
        let browser_auth = webview.as_ref().and_then(|view| {
            match auth::BrowserAuth::install(view.read(cx).raw()) {
                Ok(auth) => Some(auth),
                Err(message) => { error = Some(message); None }
            }
        });
        let (browser_controls, control_receiver) = if let Some(view) = &webview {
            let (controls, receiver) = controls::BrowserControls::install(view.read(cx).raw());
            (Some(controls), Some(receiver))
        } else { (None, None) };
        let control_actions = cx.spawn(async move |tab, cx| {
            let Some(mut receiver) = control_receiver else { return; };
            while let Some(action) = receiver.next().await {
                if tab.update(cx, |tab, cx| {
                    if !tab.visible {
                        return;
                    }
                    let action = match action {
                        controls::BrowserControlAction::TopHovered(hovered) => {
                            if tab.docked_mode && !tab.omnibar_open && tab.context_menu.is_none()
                                && tab.pane_toolbar_hovered != hovered {
                                tab.pane_toolbar_hovered = hovered;
                                cx.notify();
                            }
                            return;
                        }
                        controls::BrowserControlAction::Close if tab.has_native_close_control() => {
                            BrowserWorkspaceAction::CloseTab
                        }
                        controls::BrowserControlAction::ToggleZoom
                            if tab.docked_mode && tab.browser_controls.is_some() =>
                        {
                            BrowserWorkspaceAction::ToggleZoom
                        }
                        _ => return,
                    };
                    cx.emit(BrowserTabEvent::WorkspaceAction(action));
                }).is_err() { return; }
            }
        });
        let (address_shortcut, shortcut_receiver) = if let Some(view) = &webview {
            let cancel_pending_hold = browser_controls
                .as_ref()
                .map(|controls| controls.shortcut_canceller());
            let (guard, receiver) = keyboard::AddressShortcut::install(
                view.read(cx).raw(),
                cancel_pending_hold,
            );
            (Some(guard), Some(receiver))
        } else { (None, None) };
        let address_shortcuts = cx.spawn_in(window, async move |tab, cx| {
            let Some(mut receiver) = shortcut_receiver else { return; };
            while let Some(shortcut) = receiver.next().await {
                if tab.update_in(cx, |tab, window, cx| {
                    if !tab.visible { return; }
                    match shortcut {
                        keyboard::Shortcut::FocusAddress => tab.open_omnibar(window, cx),
                        keyboard::Shortcut::Reload => {
                            tab.reload(cx);
                            if tab.omnibar_open { tab.dismiss_omnibar(window, cx); }
                        }
                        keyboard::Shortcut::Back | keyboard::Shortcut::Forward => {
                            tab.history_step(matches!(shortcut, keyboard::Shortcut::Forward), cx);
                            if tab.omnibar_open { tab.dismiss_omnibar(window, cx); }
                        }
                        keyboard::Shortcut::NewTab => cx.emit(BrowserTabEvent::WorkspaceAction(BrowserWorkspaceAction::NewTab)),
                        keyboard::Shortcut::CloseTab => cx.emit(BrowserTabEvent::WorkspaceAction(BrowserWorkspaceAction::CloseTab)),
                        keyboard::Shortcut::NextTab => cx.emit(BrowserTabEvent::WorkspaceAction(BrowserWorkspaceAction::NextTab)),
                        keyboard::Shortcut::PreviousTab => cx.emit(BrowserTabEvent::WorkspaceAction(BrowserWorkspaceAction::PreviousTab)),
                    }
                }).is_err() { return; }
            }
        });
        let enter =
            cx.subscribe_in(
                &address,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.submit_omnibar(window, cx),
                    InputEvent::Change if !this.restoring_metadata => {
                        this.dirty_address = true;
                        cx.emit(BrowserTabEvent::AddressEdited);
                        cx.notify();
                    }
                    InputEvent::Change => {}
                    _ => {}
                },
            );
        let release = cx.on_release(|this, cx| this.close(cx));
        let quit = cx.on_app_quit(|this, cx| {
            this.close(cx);
            async {}
        });
        let event_task = if let Some(mut receiver) = receiver {
            cx.spawn(async move |pane, cx| {
                // Dropping the tab cancels this task; no blocked native thread
                // or timer is retained while waiting for WebKit callbacks.
                while let Some(event) = receiver.next().await {
                    let mut pending = vec![event];
                    while let Ok(event) = receiver.try_recv() {
                        pending.push(event);
                    }
                    if pane
                        .update(cx, |this, cx| {
                            if !pending.is_empty() {
                                if let Some(controls) = &this.browser_controls { controls.refresh(); }
                                for event in pending {
                                    match event {
                                        BrowserEvent::PageLoad { loading, location } => {
                                            this.page_load(loading, &location, cx);
                                        }
                                        BrowserEvent::Location(value) => {
                                            if !this.unloaded {
                                                if let Ok(url) = validated_url(&value) {
                                                    let location = url.to_string();
                                                    if !is_stale_initial_blank(
                                                        this.ignore_initial_blank_event,
                                                        &location,
                                                        this.actual_url.as_deref(),
                                                    ) && location.len() <= MAX_BROWSER_URL_BYTES
                                                        && this.actual_url.as_deref() != Some(&location)
                                                    {
                                                        if location != "about:blank" {
                                                            this.ignore_initial_blank_event = false;
                                                        }
                                                        this.actual_url = Some(location.clone());
                                                        this.pending_address = Some(location.clone());
                                                        this.location = location;
                                                        cx.emit(BrowserTabEvent::MetadataChanged);
                                                    }
                                                }
                                            }
                                        }
                                        BrowserEvent::Title(value) => {
                                            if !this.unloaded
                                                && !is_stale_initial_title(
                                                    this.ignore_initial_blank_event,
                                                    this.actual_url.as_deref(),
                                                )
                                            {
                                                let title = bounded_title(&value);
                                                if this.title != title {
                                                    this.title = title;
                                                    cx.emit(BrowserTabEvent::MetadataChanged);
                                                }
                                            }
                                        }
                                        BrowserEvent::Error(value) => {
                                            this.set_loading(false, cx);
                                            this.error = Some(value);
                                        }
                                    }
                                }
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
        } else {
            cx.spawn(async move |_, _| {})
        };
        Self {
            focus,
            address,
            webview,
            host_window: window.window_handle(),
            native_deferred: false,
            _native_init: None,
            pending_navigation: None,
            #[cfg(target_os = "macos")]
            applied_corner_radius: None,
            location: "about:blank".into(),
            requested_url: "about:blank".into(),
            actual_url: None,
            title: String::new(),
            error,
            visible: false,
            #[cfg(feature = "performance-profiling")]
            native_visible: false,
            pending_address: None,
            dirty_address: false,
            omnibar_open: false,
            initial_address_pending: true,
            omnibar_generation: 0,
            omnibar_snapshot: None,
            unloaded: true,
            loading: false,
            _loading_watch: None,
            docked_mode: false,
            dock_group: None,
            pane_toolbar_hovered: false,
            page_edge_colour: None,
            page_edge_generation: 0,
            _page_edge_watch: None,
            _omnibar_snapshot_task: None,
            palette: None,
            interface_font_ratio: 1.,
            restoring_metadata: false,
            ignore_initial_blank_event: true,
            _enter: enter,
            _release: release,
            _quit: quit,
            _events: event_task,
            address_shortcut,
            browser_controls,
            browser_auth,
            context_click,
            _context_clicks: context_clicks,
            context_menu: None,
            context_generation: 0,
            context_point: (0.,0.),
            context_pane_size: std::rc::Rc::new(std::cell::Cell::new(Size::default())),
            context_snapshot: None,
            context_capture_cancel: None,
            _address_shortcuts: address_shortcuts,
            _control_actions: control_actions,
        }
    }

    fn page_edge_eligible(&self, cx: &App) -> bool {
        self.visible && !self.unloaded && !self.loading && !self.omnibar_open
            && self.context_menu.is_none() && !crate::session_lock::locked(cx)
            && self.webview.is_some()
            && (self.page_edge_colour.is_none() || self.pane_toolbar_hovered)
    }

    fn reconcile_page_edge_watch(&mut self, cx: &mut Context<Self>) {
        if !self.page_edge_eligible(cx) {
            self._page_edge_watch.take();
            return;
        }
        if self._page_edge_watch.is_some() { return; }
        self._page_edge_watch = Some(cx.spawn(async move |tab, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(600)).await;
                let sample = tab.update(cx, |tab, cx| {
                    if !tab.page_edge_eligible(cx) { return None; }
                    tab.webview.as_ref().map(|view| (
                        tab.page_edge_generation,
                        snapshot::request_top_edge(view.read(cx).raw()),
                    ))
                }).ok().flatten();
                let Some((generation, request)) = sample else { break; };
                let capture = match futures::future::select(
                    Box::pin(request.receive()),
                    Box::pin(cx.background_executor().timer(Duration::from_millis(250))),
                ).await {
                    futures::future::Either::Left((capture, _)) => capture,
                    futures::future::Either::Right(_) => None,
                };
                let colour = cx.background_executor().spawn(async move {
                    capture.and_then(snapshot::decode).and_then(|image| snapshot::dominant_colour(&image))
                }).await;
                let keep_sampling = tab.update(cx, |tab, cx| {
                    if tab.page_edge_generation != generation || !tab.page_edge_eligible(cx) { return false; }
                    if let Some(colour) = colour {
                        if tab.page_edge_colour != Some(colour) {
                            tab.page_edge_colour = Some(colour);
                            cx.notify();
                        }
                    }
                    tab.page_edge_eligible(cx)
                }).unwrap_or(false);
                if !keep_sampling { break; }
            }
        }));
    }

    fn navigate(&mut self, cx: &mut Context<Self>) {
        let result = omnibar::navigation_target(&self.address.read(cx).value())
            .and_then(|target| self.open_url(&target, None, cx));
        if let Err(error) = result { self.set_loading(false, cx); self.error = Some(error); cx.notify(); }
    }

    pub(crate) fn open_url(&mut self, target: &str, title: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        if self.native_deferred {
            let normalized = if target == "about:blank" { target.to_owned() }
                else { monitter_lib::browser_links::normalize_target(target)? };
            let url = validated_url(&normalized)?;
            if url.as_str().len() > MAX_BROWSER_URL_BYTES {
                return Err("Browser URL exceeds the 8192 byte limit.".into());
            }
            let url = url.to_string();
            self.pending_navigation = Some((url.clone(), title.map(str::to_owned)));
            self.requested_url = url.clone();
            self.actual_url = None;
            self.location = url;
            if let Some(title) = title { self.title = bounded_title(title); }
            self.pending_address = Some(self.location.clone());
            self.dirty_address = false;
            self.error = None;
            cx.update_window_context(self.host_window, |window, cx| self.start_native(window, cx))
                .map_err(|error| error.to_string())?;
            cx.emit(BrowserTabEvent::MetadataChanged);
            cx.notify();
            return Ok(());
        }
        if let Some(auth) = &self.browser_auth { auth.cancel(); }
        let result = (if target == "about:blank" { Ok(target.to_owned()) } else { monitter_lib::browser_links::normalize_target(target) }).and_then(|target| validated_url(&target)).and_then(|url| {
            if url.as_str().len() > MAX_BROWSER_URL_BYTES {
                return Err("Browser URL exceeds the 8192 byte limit.".into());
            }
            let view = self.webview.clone().ok_or("Native browser is unavailable.")?;
            self.set_loading(url.as_str() != "about:blank", cx);
            load_browser_url(view.read(cx).raw(), &url)
                .map(|()| url.to_string())
                .map_err(|error| format!("Navigation failed: {error}"))
        });
        match result {
            Ok(requested_url) => {
                if let Some(title) = title { self.title = bounded_title(title); }
                self.requested_url = requested_url.clone();
                self.actual_url = None;
                self.location = requested_url;
                self.unloaded = false;
                self.dirty_address = false;
                self.pending_address = Some(self.location.clone());
                self.error = None;
                if self.visible && !self.omnibar_open && self.context_menu.is_none() && !crate::session_lock::locked(cx) {
                    if let Some(view) = self.webview.clone() {
                        let shown = view.update(cx, |view, _| {
                            view.set_visible(true).map_err(|error| error.to_string())
                        });
                        #[cfg(feature = "performance-profiling")]
                        self.record_native_visibility(true, shown.is_ok());
                        match shown {
                            Ok(()) => {}
                            Err(error) => {
                                self.set_loading(false, cx);
                                self.error = Some(format!("Cannot show native browser: {error}"));
                            }
                        }
                    }
                }
                // Loading the URL is accepted independently of whether the
                // native host could make the view visible afterward.
                cx.emit(BrowserTabEvent::MetadataChanged);
            }
            Err(error) => { self.set_loading(false, cx); self.error = Some(error.clone()); cx.notify(); return Err(error); }
        }
        cx.notify();
        Ok(())
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(auth) = &self.browser_auth { auth.cancel(); }
        if self.unloaded {
            self.navigate(cx);
            return;
        }
        self.set_loading(self.location != "about:blank", cx);
        self.error = self
            .webview
            .as_ref()
            .ok_or_else(|| "Native browser is unavailable.".to_owned())
            .and_then(|view| {
                view.read(cx)
                    .raw()
                    .reload()
                    .map_err(|error| format!("Reload failed: {error}"))
            })
            .err();
        if self.error.is_some() { self.set_loading(false, cx); }
        cx.notify();
    }

    fn set_appearance(
        &mut self,
        palette: crate::design::Palette,
        interface_font_ratio: f32,
        cx: &mut Context<Self>,
    ) {
        let ratio = normalized_font_ratio(interface_font_ratio);
        if self.palette != Some(palette) || self.interface_font_ratio != ratio {
            self.palette = Some(palette);
            self.interface_font_ratio = ratio;
            cx.notify();
        }
    }

    fn history_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(auth) = &self.browser_auth { auth.cancel(); }
        #[cfg(target_os = "macos")]
        if let Some(view) = self.webview.clone() {
            let native = view.read(cx).raw().webview();
            // Use WKWebView history directly, never page-controlled JavaScript.
            unsafe {
                if forward && native.canGoForward() {
                    self.set_loading(true, cx);
                    let _ = native.goForward();
                } else if !forward && native.canGoBack() {
                    self.set_loading(true, cx);
                    let _ = native.goBack();
                }
            }
        }
        cx.notify();
    }

    /// Call before removing the pane from the render tree; preserves session.
    pub fn hide(&mut self, cx: &mut App) {
        if let Some(auth) = &self.browser_auth { auth.cancel(); }
        if let Some(controls) = &self.browser_controls { controls.hide(); }
        self.context_generation = self.context_generation.wrapping_add(1);
        if let Some(cancel) = self.context_capture_cancel.take() { cancel.cancel(); }
        self.context_menu = None;
        self.context_snapshot = None;
        self.omnibar_generation = self.omnibar_generation.wrapping_add(1);
        self.omnibar_snapshot = None;
        self.omnibar_open = false;
        self.pane_toolbar_hovered = false;
        #[cfg(feature = "performance-profiling")]
        HIDE_REQUESTS.fetch_add(1, Ordering::Relaxed);
        self.visible = false;
        self._page_edge_watch.take();
        self._loading_watch.take();
        self._omnibar_snapshot_task.take();
        self.page_edge_generation = self.page_edge_generation.wrapping_add(1);
        if let Some(view) = self.webview.clone() {
            let visibility_error = view.update(cx, |view, _| view.set_visible(false)).err();
            #[cfg(feature = "performance-profiling")]
            self.record_native_visibility(false, visibility_error.is_none());
            if let Some(error) = visibility_error {
                self.error = Some(format!("Cannot hide native browser: {error}"));
            }
        }
    }

    /// Restore a retained pane when returning to its native browser surface.
    pub fn show(&mut self, cx: &mut App) {
        #[cfg(feature = "performance-profiling")]
        SHOW_REQUESTS.fetch_add(1, Ordering::Relaxed);
        if crate::session_lock::locked(cx) {self.hide(cx);return;}
        self.visible = true;
        if let Some(view) = self.webview.clone() {
            if self.unloaded || self.omnibar_open || self.context_menu.is_some() {
                let result = view.update(cx, |view, _| view.set_visible(false));
                #[cfg(feature = "performance-profiling")]
                self.record_native_visibility(false, result.is_ok());
                if let Err(error) = result {
                    self.error = Some(format!("Cannot hide unloaded browser view: {error}"));
                }
            } else {
                let result = view.update(cx, |view, _| view.set_visible(true));
                #[cfg(feature = "performance-profiling")]
                self.record_native_visibility(true, result.is_ok());
                if let Err(error) = result {
                    self.error = Some(format!("Cannot show native browser: {error}"));
                }
            }
        }
    }

    pub(crate) fn is_hosted_in(&self, window: &Window) -> bool {
        self.host_window == window.window_handle()
    }

    /// Validate without hiding, navigating, changing ownership or clearing a
    /// pending address. Every tab in a batch is checked before any tab moves.
    pub(crate) fn preflight_reparent_to_window(&self, window: &Window) -> Result<(), String> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            return Err("Moving browser tabs between windows is supported only on macOS.".into());
        }
        #[cfg(target_os = "macos")]
        {
            if self.webview.is_none() && !self.native_deferred { return Err("The browser tab has no native view to transfer.".into()); }
            reparent_target(window)?;
            Ok(())
        }
    }

    /// Move the existing native page rather than reload its URL. The owning
    /// dock shows it after layout in the new host, including on a return move.
    pub(crate) fn reparent_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        self.preflight_reparent_to_window(window)?;
        #[cfg(not(target_os = "macos"))]
        { let _ = cx; unreachable!("unsupported transfer was rejected by preflight"); }
        #[cfg(target_os = "macos")]
        {
            if self.webview.is_none() && self.native_deferred {
                self._native_init.take();
                self.host_window = window.window_handle();
                self.dock_group = None;
                self.context_pane_size.set(Size::default());
                let id = cx.entity_id();
                cx.rehost_entity_window(id, window).map_err(|error| error.to_string())?;
                self.address.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
                if self.visible || self.pending_navigation.is_some() { self.start_native(window, cx); }
                cx.notify();
                return Ok(());
            }
            let view = self.webview.clone().ok_or("The browser tab has no native view to transfer.")?;
            let was_visible = self.visible;
            self.hide(cx);
            if let Err(error) = view.update(cx, |view, cx| view.reparent(window, cx)) {
                if was_visible { self.show(cx); }
                return Err(format!("Cannot move the retained browser page: {error}"));
            }
            self.host_window = window.window_handle();
            self.dock_group = None;
            self.context_pane_size.set(Size::default());
            // Pane-owned callbacks resolve their subscriber's current window.
            // Pin hidden children too; they need not wait for their first draw.
            let tab_id = cx.entity_id();
            cx.rehost_entity_window(tab_id, window).map_err(|error| error.to_string())?;
            self.address.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
            cx.rehost_entity_window(view.entity_id(), window).map_err(|error| error.to_string())?;
            cx.notify();
            Ok(())
        }
    }

    /// Drop native ownership before window destruction; see module contract.
    pub fn close(&mut self, cx: &mut App) {
        self.loading = false;
        self.unloaded = true;
        self.native_deferred = false;
        self._native_init.take();
        self.pending_navigation = None;
        self._loading_watch.take();
        self.hide(cx);
        self.address_shortcut.take();
        self.context_click.take();
        self.browser_auth.take();
        self.browser_controls.take();
        self.webview.take();
    }

    fn restore_metadata(
        &mut self,
        metadata: &RestoredTabMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Restored panes retain their saved presentation rather than taking focus.
        self.initial_address_pending = false;
        self.restoring_metadata = true;
        self.requested_url = metadata.requested_url.clone();
        self.actual_url = metadata.actual_url.clone();
        self.location = metadata
            .actual_url
            .as_deref()
            .unwrap_or(&metadata.requested_url)
            .to_owned();
        self.title = metadata.title.clone();
        self.unloaded = true;
        self.set_loading(false, cx);
        self.pending_address = None;
        let address = self
            .actual_url
            .as_deref()
            .unwrap_or(&self.requested_url)
            .to_owned();
        self.address
            .update(cx, |state, cx| state.set_value(address, window, cx));
        self.dirty_address = false;
        self.restoring_metadata = false;
    }
}

impl Render for BrowserTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visible && self.native_deferred { self.start_native(window, cx); }
        if self.visible && self.loading && self._loading_watch.is_none() { self.set_loading(true, cx); }
        self.reconcile_page_edge_watch(cx);
        #[cfg(target_os = "macos")]
        self.apply_native_tile_corners(cx);
        // Wait for the owning dock to show a newly created pane, then open the
        // normal Cmd-L composer once. Escape and later workspace switches must
        // not reopen it, and links opened directly must keep their page visible.
        if self.visible && self.initial_address_pending {
            self.initial_address_pending = false;
            if self.unloaded && self.requested_url == "about:blank" && self.location == "about:blank" {
                self.finish_open_omnibar(window, cx);
            }
        }
        if !self.dirty_address && !self.address.read(cx).focus_handle(cx).is_focused(window) {
            if let Some(location) = self.pending_address.take() {
                self.address
                    .update(cx, |address, cx| address.set_value(location, window, cx));
                self.dirty_address = false;
            }
        }
        let palette = self.palette.unwrap_or_else(|| {
            crate::design::palette(&serde_json::Value::Null, window.appearance())
        });
        if let Some(controls) = &self.browser_controls {
            let zoomed = self.dock_group.as_ref()
                .and_then(|group| group.upgrade())
                .is_some_and(|group| group.read(cx).is_zoomed());
            controls.set_zoomed(zoomed);
        }
        let font_ratio = self.interface_font_ratio;
        let address = Input::new(&self.address).appearance(false).bordered(false)
            .aria_label("Address or search").text_size(px(14. * font_ratio))
            .text_color(rgb(palette.ink)).w_full();
        let mut content = div().relative().flex().flex_col().size_full().min_w_0().min_h_0()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.context_menu.is_some() && event.keystroke.key == "escape" {
                    this.dismiss_context(window,cx); cx.stop_propagation();
                }
            }));
        let measured_context_size = self.context_pane_size.clone();
        content = content.child(canvas(|_,_,_| (),move |bounds,_,_,_| measured_context_size.set(bounds.size)).absolute().size_full());
        if self.docked_mode {
            content =
                content
                    .key_context("MonitterNativeBrowser")
                    .on_action(cx.listener(|this, _: &FocusAddress, window, cx| {
                        this.focus_address(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ReloadTab, window, cx| {
                        if this.visible {
                            this.reload(cx);
                            if this.omnibar_open { this.dismiss_omnibar(window, cx); }
                        }
                    }))
                    .on_action(cx.listener(|this, _: &BackTab, window, cx| {
                        if this.visible { this.history_step(false, cx); if this.omnibar_open { this.dismiss_omnibar(window, cx); } }
                    }))
                    .on_action(cx.listener(|this, _: &ForwardTab, window, cx| {
                        if this.visible { this.history_step(true, cx); if this.omnibar_open { this.dismiss_omnibar(window, cx); } }
                    }))
                    .on_action(cx.listener(|_, _: &NewTab, _, cx| {
                        cx.emit(BrowserTabEvent::WorkspaceAction(
                            BrowserWorkspaceAction::NewTab,
                        ));
                    }))
                    .on_action(cx.listener(|_, _: &CloseTab, _, cx| {
                        cx.emit(BrowserTabEvent::WorkspaceAction(
                            BrowserWorkspaceAction::CloseTab,
                        ));
                    }))
                    .on_action(cx.listener(|_, _: &NextTab, _, cx| {
                        cx.emit(BrowserTabEvent::WorkspaceAction(
                            BrowserWorkspaceAction::NextTab,
                        ));
                    }))
                    .on_action(cx.listener(|_, _: &PreviousTab, _, cx| {
                        cx.emit(BrowserTabEvent::WorkspaceAction(
                            BrowserWorkspaceAction::PreviousTab,
                        ));
                    }));
        }
        let mut page_content = div().flex_1().min_h_0().min_w_0().flex().flex_col();
        if let Some(error) = self.error.as_ref().filter(|_| !self.omnibar_open) {
            page_content = page_content.child(
                div()
                    .px(px(10.))
                    .py(px(5.))
                    .text_size(px(12. * font_ratio))
                    .text_color(rgb(0xdc2626))
                    .child(error.clone()),
            );
        }
        if self.unloaded {
            let start_message = if self.requested_url == "about:blank" {
                "Press ⌘L / Ctrl+L to enter an address or search."
            } else {
                "Press ⌘L / Ctrl+L to open the saved page or enter an address."
            };
            page_content = page_content.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .p(px(16.))
                    .text_color(rgb(palette.muted))
                    .child(Button::new("browser-open-omnibar").text().label("Open address or search")
                        .on_click(cx.listener(|this, _, window, cx| this.open_omnibar(window, cx))))
                    .child(Icon::new(IconName::Globe).with_size(px(24.)))
                    .child(
                        div()
                            .text_size(px(13. * font_ratio))
                            .text_color(rgb(palette.ink))
                            .truncate()
                            .child(if self.title.is_empty() {
                                "Browser".to_owned()
                            } else {
                                self.title.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12. * font_ratio))
                            .text_center()
                            .max_w(px(300.))
                            .child(start_message),
                    ),
            );
        } else if let Some(view) = self.webview.as_ref().filter(|_| self.visible) {
            // WKWebView is above the GPUI canvas. Reserve the browser-side
            // half of the divider grab band so its real GPUI highlight remains
            // visible and receives pointer events along the entire viewport.
            // Use logical pixels to match the unscaled native divider band.
            page_content = page_content.child(
                div().flex_1().min_h(px(0.)).p(gpui::px(4.)).child(view.clone()),
            );
        }
        let mut pane_toolbar = None;
        if self.docked_mode && self.visible && !self.omnibar_open && self.context_menu.is_none() {
            if let Some(group) = self.dock_group.as_ref().and_then(WeakEntity::upgrade) {
                let context = group.read(cx).context(cx);
                let revealed = self.pane_toolbar_hovered;
                // Match the bottom native navigation HUD: vibrant dark,
                // 70% charcoal tint, rounded corners, and opaque light icons.
                let toolbar_ink = 0xf1f3f4;
                let button_style = ButtonCustomVariant::new(cx)
                    .foreground(rgb(toolbar_ink).into())
                    .hover(rgb(toolbar_ink).opacity(0.08).into())
                    .active(rgb(toolbar_ink).opacity(0.14).into());
                let grip = div()
                    .size(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(3.))
                    .children((0..3).map(|_| {
                        div().flex().gap(px(3.)).children((0..3).map(|_| {
                            div()
                                .size(px(2.))
                                .rounded_full()
                                .bg(rgb(toolbar_ink).opacity(0.5))
                        }))
                    }));
                let drag = gpui_kit::component::dock::pane_drag_region(
                    div()
                        .h_full()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            div()
                                .size(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(toolbar_ink).opacity(0.75))
                                .child(Icon::new(IconName::Move).with_size(px(16.))),
                        )
                        .child(grip),
                    &context,
                    cx,
                );
                let toolbar = div()
                    .id("browser-pane-topbar")
                    .debug_selector(|| "browser-pane-topbar".into())
                    .h(px(36.))
                    .flex_shrink_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .px(px(4.))
                    .gap(px(2.))
                    .bg(Background::from(rgb(0x1a1a1a).opacity(0.7)).backdrop_blur(20.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(rgb(toolbar_ink).opacity(0.10))
                    .child(drag)
                    .children(
                        gpui_kit::component::dock::pane_zoom_control(&context, cx).map(|button| {
                            button
                                .w(px(28.))
                                .h(px(28.))
                                .p_0()
                                .custom(button_style)
                                .debug_selector(|| "browser-pane-maximize".into())
                        }),
                    )
                    .child(
                        gpui_kit::component::dock::pane_close_control(&context, cx)
                            .w(px(28.))
                            .h(px(28.))
                            .p_0()
                            .custom(button_style)
                            .debug_selector(|| "browser-pane-close".into()),
                    );
                let sensor = div()
                    .id("browser-pane-topbar-sensor")
                    .debug_selector(|| "browser-pane-topbar-sensor".into())
                    .relative()
                    .w_full()
                    .h(px(48.))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .items_start()
                    .overflow_hidden();
                let hover = cx.listener(|this, hovered: &bool, _, cx| {
                    if this.pane_toolbar_hovered != *hovered {
                        this.pane_toolbar_hovered = *hovered;
                        cx.notify();
                    }
                });
                let leave = cx.listener(|this, _: &MouseExitEvent, _, cx| {
                    if this.pane_toolbar_hovered {
                        this.pane_toolbar_hovered = false;
                        cx.notify();
                    }
                });
                // Observe a fixed pane-relative band in capture. An occluding
                // toolbar (or the native page) must not make its own sensor lose
                // hover as soon as it appears. This listener never consumes input.
                content = content.child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                    let visible = bounds.intersect(&window.content_mask().bounds);
                    let band = Bounds::new(visible.origin, size(visible.size.width, px(48.).min(visible.size.height)));
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            hover(&band.contains(&event.position), window, cx);
                        }
                    });
                    window.on_mouse_event(move |event: &MouseExitEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture { leave(event, window, cx); }
                    });
                }).absolute().size_full());
                pane_toolbar = Some((revealed, sensor, toolbar));
            }
        }
        let update_top_inset = self.browser_controls.as_ref().map(|controls| controls.top_inset_updater());
        if let Some((revealed, sensor, toolbar)) = pane_toolbar {
            // One spring drives both the visible bar and its reserved space.
            // The native document's actual frame moves and shrinks, keeping
            // fixed website navigation below the controls throughout the slide.
            content = content.child(div().relative().flex_1().min_h_0().min_w_0()
                .bg(rgb(self.page_edge_colour.unwrap_or(palette.card)))
                .flex().flex_col().overflow_hidden().with_spring(
                    "browser-pane-topbar-slide",
                    SpringAnimation::new(SpringConfig::new(310., 30., 1.))
                        .to(if revealed { px(36.) } else { px(0.) })
                        .from(px(0.)).with_epsilon(0.05),
                    move |layout, height| {
                        let height = height.max(px(0.)).min(px(36.));
                        if let Some(update) = update_top_inset { update(height.as_f32()); }
                        layout.child(div().h(height).flex_shrink_0())
                            .child(page_content)
                            // Preserve the pane clip before deferring above the
                            // native page, and keep the bar in normal flow.
                            .child(div().absolute().inset_0().overflow_hidden().flex().flex_col()
                                .child(deferred(sensor.child(toolbar.mt(height - px(36.))))
                                    .with_priority(3)))
                    },
                ));
        } else {
            if let Some(update) = update_top_inset { update(0.); }
            content = content.child(page_content);
        }
        if let Some(link) = self.context_menu.clone() {
            if let Some(image) = &self.context_snapshot {
                content = content.child(div().absolute().inset_0().p(gpui::px(4.)).child(img(image.clone()).size_full()));
            }
            // Keep foreground tied to this pane's palette in every button state;
            // Ghost otherwise takes its foreground from the global GPUI theme.
            let context_button_style = ButtonCustomVariant::new(cx)
                .foreground(rgb(palette.ink).into())
                .hover(rgb(palette.ink).opacity(0.08).into())
                .active(rgb(palette.ink).opacity(0.12).into());
            let mut menu = div().id("browser-context-card").debug_selector(|| "browser-context-card".into()).occlude().flex().flex_col().p(px(6.)).rounded(px(10.))
                .popover_style(cx).border_1().border_color(crate::design::surface_color(palette.line))
                .bg(crate::surface_blend::raised_background(crate::surface_blend::Area::Composer, palette.card))
                .font_family(cx.theme().font_family.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            if let Some(url) = link.clone() {
                menu = menu.child(div().h(px(32.)).debug_selector(|| "browser-context-new-pane".into()).child(Button::new("browser-context-new-pane").w_full().h_full().justify_start().custom(context_button_style).label(browser_context::items(true)[0])
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let url = url.clone(); this.dismiss_context(window, cx);
                        cx.emit(BrowserTabEvent::OpenInNewPane(url));
                    }))));
            }
            let url = link.clone().unwrap_or_else(|| self.location.clone());
            menu = menu.child(div().h(px(32.)).debug_selector(|| "browser-context-system".into()).child(Button::new("browser-context-system").w_full().h_full().justify_start().custom(context_button_style).label(browser_context::SYSTEM)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dismiss_context(window, cx);
                    if let Some(url) = browser_context::target_for_page(&url, &this.location) { cx.open_url(&url); }
                    else { this.error = Some("This page cannot be opened in the system browser.".into()); cx.notify(); }
                }))));
            if let Some(url) = link.clone() {
                menu = menu.child(div().h(px(32.)).debug_selector(|| "browser-context-copy".into()).child(Button::new("browser-context-copy").w_full().h_full().justify_start().custom(context_button_style).label(browser_context::COPY)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone())); this.dismiss_context(window, cx);
                    }))));
            }
            let pane_size = self.webview.as_ref().map(|view| view.read(cx).bounds().size)
                .filter(|size| size.width > gpui::px(0.) && size.height > gpui::px(0.))
                .unwrap_or_else(|| {
                    let size = self.context_pane_size.get();
                    if size.width > gpui::px(0.) && size.height > gpui::px(0.) { size } else { window.viewport_size() }
                });
            let (left,top,width,height) = browser_context::placement(
                self.context_point, (pane_size.width.as_f32(),pane_size.height.as_f32()),
                px(220.).as_f32(), (px(if link.is_some() {108.} else {44.}) + gpui::px(2.)).as_f32(),
            );
            menu = menu.flex_shrink_0().w(gpui::px(width)).h(gpui::px(height)).overflow_hidden();
            // Keep the card in normal flow, like the omnibar. Absolute children
            // in deferred cached panes can lose their pane-relative paint offset.
            content = content.child(div().id("browser-context-overlay").absolute().inset_0().occlude()
                .flex().flex_col().pl(gpui::px(left)).pt(gpui::px(top))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| { this.dismiss_context(window,cx); cx.stop_propagation(); }))
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" { this.dismiss_context(window,cx); cx.stop_propagation(); }
                })).child(menu));
        }
        if self.omnibar_open {
            if let Some(image) = &self.omnibar_snapshot {
                content = content.child(div().absolute().inset_0().p(gpui::px(4.)).child(img(image.clone()).size_full()));
            }
            let mut card = div().id("browser-omnibar-card")
                .debug_selector(|| "browser-omnibar-card".into())
                .w(px(560.)).max_w_full().popover_style(cx)
                .border_color(crate::design::surface_color(palette.line))
                .rounded(px(crate::design::radius::CONTROL))
                .bg(crate::surface_blend::raised_background(
                    crate::surface_blend::Area::Composer, palette.card,
                ))
                .p(px(crate::design::space::INSET))
                .font_family(cx.theme().font_family.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().items_center().gap(px(crate::design::space::TIGHT))
                    .child(div().flex_1().min_w_0().child(address))
                    .child(Button::new("omnibar-enter").outline().compact()
                        .w(px(crate::design::COMPOSER_CONTROL_SIZE))
                        .h(px(crate::design::COMPOSER_CONTROL_SIZE))
                        .border_color(rgb(palette.accent)).text_color(rgb(palette.accent))
                        .icon(Icon::new(IconName::CornerDownLeft))
                        .accessibility_label("Open address or search")
                        .disabled(self.address.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.submit_omnibar(window, cx)))));
            if let Some(error) = &self.error {
                card = card.child(div().pt(px(8.)).text_size(px(12. * font_ratio))
                    .text_color(rgb(0xdc2626)).child(error.clone()));
            }
            // Paint after pane contents/chrome so the empty-state text and
            // native-pane adornments cannot cross the address composer.
            content = content.child(deferred(div().id("browser-omnibar-overlay").absolute().inset_0().occlude()
                .flex().items_center().justify_center().p(px(20.))
                .bg(Background::from(black().opacity(0.10)).backdrop_blur(6.))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    this.dismiss_omnibar(window, cx); cx.stop_propagation();
                }))
                .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if this.address.update(cx, |input, cx| input.marked_text_range(window, cx).is_some()) { return; }
                    match event.keystroke.key.as_str() {
                        "escape" => this.dismiss_omnibar(window, cx),
                        _ => return,
                    }
                    cx.stop_propagation(); cx.notify();
                })).child(card)).with_priority(4));
        }
        content
    }
}

/// Retained native tabs share only this experimental profile's WK data store.
/// Closing a tab releases its child; hiding the workspace preserves all tabs.
pub struct BrowserPane {
    tabs: Vec<(u64, Entity<BrowserTab>)>,
    active: Option<u64>,
    next_id: u64,
    metadata_revision: u64,
    observations: Vec<(u64, Subscription)>,
    metadata_subscriptions: Vec<(u64, Subscription)>,
    focus: FocusHandle,
    visible: bool,
    docked_mode: bool,
    palette: Option<crate::design::Palette>,
    interface_font_ratio: f32,
    _release: Subscription,
    _quit: Subscription,
}
impl EventEmitter<BrowserPaneEvent> for BrowserPane {}

impl BrowserPane {
    /// A single private, static page for the disconnected native transfer
    /// preview. This bypasses BrowserPane::new and the persistent native store.
    #[cfg(feature = "performance-profiling")]
    pub(crate) fn workspace_window_fixture(window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String> {
        let native = create_private_static_native(window, r#"<!doctype html>
<html lang="en"><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; form-action 'none'; base-uri 'none'">
<title>Retained browser preview</title>
<style>html{color-scheme:dark}body{margin:40px;max-width:760px;font:17px system-ui;background:#14191f;color:#edf2f8}h1{font-size:28px}p{line-height:1.5;color:#b9c7d7}label{display:block;margin-top:28px}input,button{font:inherit;padding:12px;border:1px solid #60758f;border-radius:8px;background:#232f3d;color:#fff}input{display:block;margin-top:10px;width:90%}button{margin-top:24px;cursor:pointer}output{display:block;margin-top:18px;font-size:24px;color:#74d8ea}</style>
<h1>Retained browser preview</h1>
<p>Offline private page. No network, browser profile, backend, or agent connection.</p>
<p>Edit the value and increment the counter. Move the Browser workspace to a new window, return it, or close its detached window. Both values should remain unchanged.</p>
<label for="retained-value">Unsent browser input</label>
<input id="retained-value" value="Retain this browser value" autocomplete="off">
<button id="increment" type="button">Increment local counter</button>
<output id="counter" aria-live="polite">Counter: 0</output>
<script>let count=0;document.getElementById('increment').addEventListener('click',()=>{document.getElementById('counter').textContent='Counter: '+(++count)});</script>
</html>"#)?;
        let tab = cx.new(|cx| {
            let mut tab = BrowserTab::new_with_native(window, cx, Ok(native), None);
            tab.unloaded = false;
            tab.title = "Retained browser preview".into();
            tab.docked_mode = true;
            tab.initial_address_pending = false;
            tab
        });
        let mut manager = Self::with_initial_tab(tab, cx);
        manager.set_docked_mode(true, cx);
        Ok(manager)
    }

    fn new_tab(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<BrowserTab> {
        let palette = self.palette;
        let ratio = self.interface_font_ratio;
        cx.new(|cx| {
            let mut tab = BrowserTab::new(window, cx);
            tab.docked_mode = self.docked_mode;
            if let Some(palette) = palette {
                tab.set_appearance(palette, ratio, cx);
            }
            tab
        })
    }

    pub fn set_appearance(
        &mut self,
        palette: crate::design::Palette,
        interface_font_ratio: f32,
        cx: &mut Context<Self>,
    ) {
        let ratio = normalized_font_ratio(interface_font_ratio);
        if self.palette == Some(palette) && self.interface_font_ratio == ratio {
            return;
        }
        self.palette = Some(palette);
        self.interface_font_ratio = ratio;
        for (_, tab) in &self.tabs {
            tab.update(cx, |tab, cx| tab.set_appearance(palette, ratio, cx));
        }
        cx.notify();
    }

    /// Switch the retained manager from its legacy internal strip to
    /// per-tab DockWorkspace panes. Parent integration calls this before
    /// registering browser tab PaneSpecs.
    pub(crate) fn set_docked_mode(&mut self, docked: bool, cx: &mut Context<Self>) {
        if self.docked_mode == docked {
            return;
        }
        self.docked_mode = docked;
        for (_, tab) in &self.tabs {
            tab.update(cx, |tab, cx| {
                tab.docked_mode = docked;
                cx.notify();
            });
        }
        cx.notify();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("cmd-l", FocusAddress, Some("MonitterNativeBrowser")),
            KeyBinding::new("ctrl-l", FocusAddress, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-[", BackTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-]", ForwardTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("alt-left", BackTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("alt-right", ForwardTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-r", ReloadTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-t", NewTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-w", CloseTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("ctrl-tab", NextTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("MonitterNativeBrowser")),
        ]);
        let tab = cx.new(|cx| BrowserTab::new(window, cx));
        Self::with_initial_tab(tab, cx)
    }

    fn with_initial_tab(tab: Entity<BrowserTab>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&tab, |_, _, cx| cx.notify());
        let metadata_subscription =
            cx.subscribe(&tab, |pane, _, event: &BrowserTabEvent, cx| match event {
                BrowserTabEvent::OpenInNewPane(url) => cx.emit(BrowserPaneEvent::OpenInNewPane { source_id: 1, url: url.clone() }),
                BrowserTabEvent::MetadataChanged => pane.metadata_changed(cx),
                BrowserTabEvent::AddressEdited => {
                    pane.metadata_revision = pane.metadata_revision.saturating_add(1);
                }
                BrowserTabEvent::WorkspaceAction(action) => {
                    cx.emit(BrowserPaneEvent::WorkspaceAction {
                        source_id: 1,
                        action: *action,
                    });
                }
            });
        let release = cx.on_release(|this, cx| this.close(cx));
        let quit = cx.on_app_quit(|this, cx| {
            this.close(cx);
            async {}
        });
        Self {
            tabs: vec![(1, tab)],
            active: Some(1),
            next_id: 2,
            metadata_revision: 0,
            observations: vec![(1, observation)],
            metadata_subscriptions: vec![(1, metadata_subscription)],
            focus: cx.focus_handle(),
            visible: false,
            docked_mode: false,
            palette: None,
            interface_font_ratio: 1.,
            _release: release,
            _quit: quit,
        }
    }

    /// Current safe metadata projection for native preference persistence.
    pub(crate) fn metadata_snapshot(&self, cx: &App) -> BrowserMetadataSnapshot {
        BrowserMetadataSnapshot {
            tabs: self
                .tabs
                .iter()
                .map(|(id, tab)| {
                    let tab = tab.read(cx);
                    BrowserTabMetadata {
                        id: *id,
                        requested_url: tab.requested_url.clone(),
                        actual_url: tab.actual_url.clone(),
                        title: tab.title.clone(),
                    }
                })
                .collect(),
            selected_id: self.active,
        }
    }

    /// Monotonic within this pane lifetime; address edits also advance it so
    /// late startup restores cannot overwrite user input.
    pub(crate) fn metadata_revision(&self) -> u64 {
        self.metadata_revision
    }

    pub(crate) fn ordered_tab_ids(&self) -> Vec<u64> {
        self.tabs.iter().map(|(id, _)| *id).collect()
    }

    pub(crate) fn selected_tab_id(&self) -> Option<u64> {
        self.active
    }

    pub(crate) fn tab_entity(&self, id: u64) -> Option<Entity<BrowserTab>> {
        self.tabs
            .iter()
            .find(|(tab_id, _)| *tab_id == id)
            .map(|(_, tab)| tab.clone())
    }

    fn scoped_tabs(&self, ids: &[u64]) -> Result<Vec<(u64, Entity<BrowserTab>)>, String> {
        let mut seen = HashSet::new();
        ids.iter().map(|id| {
            if !seen.insert(*id) { return Err(format!("Browser tab {id} occurs twice in the transfer scope.")); }
            self.tab_entity(*id).map(|tab| (*id, tab)).ok_or_else(|| format!("Browser tab {id} is unavailable."))
        }).collect()
    }

    pub(crate) fn preflight_tab_transfer(&self, ids: &[u64], window: &Window, cx: &App) -> Result<(), String> {
        for (_, tab) in self.scoped_tabs(ids)? { tab.read(cx).preflight_reparent_to_window(window)?; }
        Ok(())
    }

    /// Keep one global manager and all tab entities/metadata. If a native move
    /// fails, return prior pages to their original live windows before reporting
    /// the error; a rollback failure is explicit and never recreates a page.
    pub(crate) fn transfer_tabs_to_window(&mut self, ids: &[u64], window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        self.preflight_tab_transfer(ids, window, cx)?;
        let plan: Vec<_> = self.scoped_tabs(ids)?.into_iter().map(|(id, tab)| {
            let state = tab.read(cx);
            let source = state.host_window;
            let visible = state.visible;
            (id, tab, source, visible)
        }).collect();
        for (index, (id, tab, _, _)) in plan.iter().enumerate() {
            if let Err(error) = tab.update(cx, |tab, cx| tab.reparent_to_window(window, cx)) {
                let mut failures = Vec::new();
                for (id, tab, source, visible) in plan[..=index].iter().rev() {
                    let restore = if source.window_id() == window.window_handle().window_id() {
                        tab.update(cx, |tab, cx| tab.reparent_to_window(window, cx))
                    } else {
                        source.update(cx, |_, original, cx| tab.update(cx, |tab, cx| tab.reparent_to_window(original, cx)))
                            .map_err(|error| error.to_string()).and_then(|result| result)
                    };
                    if let Err(error) = restore { failures.push(format!("tab {id}: {error}")); }
                    else if *visible { tab.update(cx, |tab, cx| tab.show(cx)); }
                }
                return Err(if failures.is_empty() { format!("Cannot transfer browser tab {id}: {error}") }
                    else { format!("Cannot transfer browser tab {id}: {error}. Rollback incomplete: {}", failures.join("; ")) });
            }
        }
        Ok(())
    }

    /// Window hosts must scope visibility to their own tabs. Manager-wide hide
    /// and close remain intentional application shutdown/security operations.
    pub(crate) fn hide_tabs(&self, ids: &[u64], cx: &mut App) -> Result<(), String> {
        for (_, tab) in self.scoped_tabs(ids)? { tab.update(cx, |tab, cx| tab.hide(cx)); }
        Ok(())
    }

    pub(crate) fn show_tabs(&self, ids: &[u64], cx: &mut App) -> Result<(), String> {
        for (_, tab) in self.scoped_tabs(ids)? { tab.update(cx, |tab, cx| tab.show(cx)); }
        Ok(())
    }

    pub(crate) fn close_tabs(&mut self, ids: &[u64], cx: &mut Context<Self>) -> Result<(), String> {
        self.scoped_tabs(ids)?;
        for id in ids { self.remove_docked(*id, cx); }
        Ok(())
    }

    /// Create a blank, unloaded browser tab for the owning DockWorkspace.
    /// Dock visibility lifecycle—not this manager—shows its native child.
    pub(crate) fn add_docked(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        if self.tabs.len() >= MAX_BROWSER_TABS {
            return Err(format!(
                "A browser can have at most {MAX_BROWSER_TABS} tabs."
            ));
        }
        let id = self.next_id;
        let next_id = id
            .checked_add(1)
            .ok_or_else(|| "Browser tab IDs are exhausted.".to_owned())?;
        self.set_docked_mode(true, cx);
        let tab = self.new_tab(window, cx);
        self.attach_tab(id, &tab, cx);
        self.tabs.push((id, tab));
        self.active = Some(id);
        self.next_id = next_id;
        self.metadata_changed(cx);
        Ok(id)
    }

    /// Close only this DockWorkspace tab. The child hides itself before the
    /// native WebView owner is released; Dock visibility controls other tabs.
    pub(crate) fn remove_docked(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|(tab_id, _)| *tab_id == id) else {
            return;
        };
        let (_, tab) = self.tabs.remove(index);
        self.observations.retain(|(tab_id, _)| *tab_id != id);
        self.metadata_subscriptions
            .retain(|(tab_id, _)| *tab_id != id);
        tab.update(cx, |tab, cx| tab.close(cx));
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|(next_id, _)| *next_id);
        }
        self.metadata_changed(cx);
    }

    /// Reflect DockWorkspace selection in safe browser metadata without
    /// changing the visibility of other panes in a split layout.
    pub(crate) fn select_docked(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.tabs.iter().any(|(tab_id, _)| *tab_id == id) || self.active == Some(id) {
            return;
        }
        self.active = Some(id);
        self.metadata_changed(cx);
    }

    /// Applies validated preference metadata without navigating any saved URL.
    /// Returns `Ok(false)` when the pane changed after the caller captured the
    /// expected revision, allowing the caller to preserve newer user state.
    pub(crate) fn restore_metadata(
        &mut self,
        snapshot: BrowserMetadataSnapshot,
        expected_revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if !restore_revision_is_current(self.metadata_revision, expected_revision) {
            return Ok(false);
        }
        let plan = restore_plan(&snapshot)?;
        for (_, tab) in self.tabs.drain(..) {
            tab.update(cx, |tab, cx| tab.close(cx));
        }
        self.observations.clear();
        self.metadata_subscriptions.clear();
        self.tabs.reserve(plan.tabs.len());
        for metadata in &plan.tabs {
            let palette = self.palette;
            let ratio = self.interface_font_ratio;
            let docked_mode = self.docked_mode;
            let tab = cx.new(|cx| {
                let mut tab = BrowserTab::new_deferred(window, cx);
                tab.restore_metadata(metadata, window, cx);
                tab.docked_mode = docked_mode;
                if let Some(palette) = palette {
                    tab.set_appearance(palette, ratio, cx);
                }
                tab
            });
            self.attach_tab(metadata.id, &tab, cx);
            self.tabs.push((metadata.id, tab));
        }
        self.active = plan.selected_id;
        self.next_id = plan.next_id;
        // Inactive restored tabs retain metadata only. Native views are created
        // on first visibility/action, without loading any saved external URL.
        if self.visible {
            if let Some(tab) = self.active_tab() {
                tab.update(cx, |tab, cx| tab.show(cx));
            }
        }
        cx.notify();
        Ok(true)
    }

    fn attach_tab(&mut self, id: u64, tab: &Entity<BrowserTab>, cx: &mut Context<Self>) {
        let source_id = id;
        self.observations
            .push((id, cx.observe(tab, |_, _, cx| cx.notify())));
        self.metadata_subscriptions.push((
            id,
            cx.subscribe(
                tab,
                move |pane, _, event: &BrowserTabEvent, cx| match event {
                    BrowserTabEvent::OpenInNewPane(url) => cx.emit(BrowserPaneEvent::OpenInNewPane { source_id, url: url.clone() }),
                    BrowserTabEvent::MetadataChanged => pane.metadata_changed(cx),
                    BrowserTabEvent::AddressEdited => {
                        pane.metadata_revision = pane.metadata_revision.saturating_add(1);
                    }
                    BrowserTabEvent::WorkspaceAction(action) => {
                        cx.emit(BrowserPaneEvent::WorkspaceAction {
                            source_id,
                            action: *action,
                        });
                    }
                },
            ),
        ));
    }

    fn metadata_changed(&mut self, cx: &mut Context<Self>) {
        self.metadata_revision = self.metadata_revision.saturating_add(1);
        let snapshot = self.metadata_snapshot(cx);
        if let Err(error) = restore_plan(&snapshot) {
            if let Some(tab) = self.active_tab() {
                tab.update(cx, |tab, cx| {
                    tab.error = Some(format!("Browser metadata was not saved: {error}"));
                    cx.notify();
                });
            }
            return;
        }
        cx.emit(BrowserPaneEvent::MetadataChanged(snapshot));
    }
    pub(crate) fn open_omnibar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_address(window, cx);
    }
    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible { return; }
        if let Some(tab) = self.active_tab() {
            tab.update(cx, |tab, cx| {
                tab.open_omnibar(window, cx);
            });
        }
    }
    fn cycle(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !self.visible || self.tabs.is_empty() {
            return;
        }
        let index = self
            .tabs
            .iter()
            .position(|(id, _)| Some(*id) == self.active)
            .unwrap_or_default();
        let next = if forward {
            (index + 1) % self.tabs.len()
        } else {
            (index + self.tabs.len() - 1) % self.tabs.len()
        };
        self.select(self.tabs[next].0, cx);
    }
    fn active_tab(&self) -> Option<Entity<BrowserTab>> {
        self.tabs
            .iter()
            .find(|(id, _)| Some(*id) == self.active)
            .map(|(_, tab)| tab.clone())
    }
    fn select(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.tabs.iter().any(|(tab_id, _)| *tab_id == id) || self.active == Some(id) {
            return;
        }
        if let Some(tab) = self.active_tab() {
            tab.update(cx, |tab, cx| tab.hide(cx));
        }
        self.active = Some(id);
        if self.visible {
            if let Some(tab) = self.active_tab() {
                tab.update(cx, |tab, cx| tab.show(cx));
            }
        }
        self.metadata_changed(cx);
    }
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() >= MAX_BROWSER_TABS {
            if let Some(tab) = self.active_tab() {
                tab.update(cx, |tab, cx| {
                    tab.error = Some(format!(
                        "A browser can have at most {MAX_BROWSER_TABS} tabs."
                    ));
                    cx.notify();
                });
            }
            return;
        }
        if let Some(tab) = self.active_tab() {
            tab.update(cx, |tab, cx| tab.hide(cx));
        }
        let id = self.next_id;
        let Some(next_id) = self.next_id.checked_add(1) else {
            return;
        };
        self.next_id = next_id;
        let tab = self.new_tab(window, cx);
        if self.visible {
            tab.update(cx, |tab, cx| tab.show(cx));
        }
        self.attach_tab(id, &tab, cx);
        self.tabs.push((id, tab));
        self.active = Some(id);
        self.metadata_changed(cx);
    }
    fn remove(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|(tab_id, _)| *tab_id == id) else {
            return;
        };
        let (_, tab) = self.tabs.remove(index);
        self.observations.retain(|(tab_id, _)| *tab_id != id);
        self.metadata_subscriptions
            .retain(|(tab_id, _)| *tab_id != id);
        // A previous GPUI frame can retain the raw handle until the next draw;
        // hide immediately, and let that frame release it with the window alive.
        tab.update(cx, |tab, cx| tab.close(cx));
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|(id, _)| *id);
            if self.visible {
                if let Some(tab) = self.active_tab() {
                    tab.update(cx, |tab, cx| tab.show(cx));
                }
            }
        }
        if self.tabs.is_empty() {
            self.add(window, cx);
        } else {
            self.metadata_changed(cx);
        }
    }
    pub fn hide(&mut self, cx: &mut App) {
        self.visible = false;
        for (_, tab) in &self.tabs {
            tab.update(cx, |tab, cx| tab.hide(cx));
        }
    }
    pub fn show(&mut self, cx: &mut App) {
        if crate::session_lock::locked(cx) {self.hide(cx);return;}
        self.visible = true;
        if let Some(tab) = self.active_tab() {
            tab.update(cx, |tab, cx| tab.show(cx));
        }
    }
    /// Host must still draw a blank frame before destroying the native window.
    pub fn close(&mut self, cx: &mut App) {
        self.visible = false;
        for (_, tab) in self.tabs.drain(..) {
            tab.update(cx, |tab, cx| tab.close(cx));
        }
        self.active = None;
        self.observations.clear();
        self.metadata_subscriptions.clear();
    }
}
impl Render for BrowserPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.docked_mode {
            return div().size_full();
        }
        let palette = self.palette.unwrap_or_else(|| {
            crate::design::palette(&serde_json::Value::Null, window.appearance())
        });
        let font_ratio = self.interface_font_ratio;
        let tab_scroll = window.use_keyed_state("native-browser-tabs-overflow", cx,
            |_, _| ScrollHandle::new()).read(cx).clone();
        let mut tabs = div()
            .id("native-browser-tabs")
            .flex()
            .flex_none()
            .items_stretch()
            .gap_0()
            .h(px(31.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&tab_scroll)
            .border_b_1()
            .border_color(rgb(palette.line))
            .bg(*cx.theme().tokens.tab_active);
        for (id, tab) in &self.tabs {
            let id = *id;
            let tab_state = tab.read(cx);
            let title = if tab_state.title.is_empty() {
                "Browser".to_owned()
            } else {
                tab_state.title.clone()
            };
            let active = self.active == Some(id);
            let mut tab_entry = div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .h(px(30.))
                .border_r_1()
                .border_color(rgb(palette.line))
                .child(
                    Button::new(SharedString::from(format!("browser-tab-{id}")))
                        .text()
                        .compact()
                        .h(px(30.))
                        .max_w(px(210.))
                        .min_w(px(0.))
                        .px(px(9.))
                        .pr(px(31.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .selected(active)
                        .text_size(px(11.5 * font_ratio))
                        .text_color(rgb(if active { palette.ink } else { palette.muted }))
                        .accessibility_label(format!("Browser tab: {title}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
                        .child(
                            Icon::new(IconName::Globe)
                                .with_size(px(13.))
                                .text_color(rgb(palette.muted)),
                        )
                        .child(div().min_w_0().truncate().child(title.clone())),
                );
            tab_entry = tab_entry.child(
                Button::new(SharedString::from(format!("browser-close-tab-{id}")))
                    .text()
                    .compact()
                    .w(px(22.))
                    .h(px(24.))
                    .p_0()
                    .min_w(px(0.))
                    .flex_shrink_0()
                    .absolute()
                    .right(px(3.))
                    .top(px(3.))
                    .accessibility_label(format!("Close browser tab {title}"))
                    .child(
                        Icon::new(IconName::X)
                            .with_size(px(12.))
                            .text_color(rgb(palette.muted)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.remove(id, window, cx))),
            );
            tabs = tabs.child(tab_entry);
        }
        tabs = tabs.child(
            Button::new("browser-new-tab")
                .text()
                .compact()
                .w(px(30.))
                .h(px(30.))
                .p_0()
                .min_w(px(0.))
                .flex_none()
                .flex_shrink_0()
                .accessibility_label("New browser tab")
                .child(
                    Icon::new(IconName::Plus)
                        .with_size(px(14.))
                        .text_color(rgb(palette.muted)),
                )
                .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
        );
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("MonitterNativeBrowser")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &FocusAddress, window, cx| this.focus_address(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ReloadTab, window, cx| {
                if this.visible { if let Some(tab) = this.active_tab() {
                    tab.update(cx, |tab, cx| { tab.reload(cx); if tab.omnibar_open { tab.dismiss_omnibar(window, cx); } });
                } }
            }))
            .on_action(cx.listener(|this, _: &BackTab, window, cx| {
                if this.visible { if let Some(tab) = this.active_tab() {
                    tab.update(cx, |tab, cx| { tab.history_step(false, cx); if tab.omnibar_open { tab.dismiss_omnibar(window, cx); } });
                } }
            }))
            .on_action(cx.listener(|this, _: &ForwardTab, window, cx| {
                if this.visible { if let Some(tab) = this.active_tab() {
                    tab.update(cx, |tab, cx| { tab.history_step(true, cx); if tab.omnibar_open { tab.dismiss_omnibar(window, cx); } });
                } }
            }))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                if this.visible {
                    this.add(window, cx);
                    this.focus_address(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.visible {
                    if let Some(id) = this.active {
                        this.remove(id, window, cx);
                        this.focus_address(window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle(false, cx)))
            .child(ScrollEdgeFade::new(tabs, tab_scroll, Axis::Horizontal, gpui_kit::px(20.)));
        if let Some(tab) = self.active_tab() {
            content = content.child(div().flex_1().min_h(px(0.)).child(tab));
        }
        content
    }
}

/// Isolated native smoke: uses the real tab and WebKit without any backend.
#[cfg(feature = "visual-export")]
pub(crate) fn open_html_smoke_if_requested(cx: &mut App) -> bool {
    let Some(path) = std::env::var_os("MONITTER_GPUI_HTML_SMOKE_FILE") else { return false; };
    let path = path.to_string_lossy().into_owned();
    gpui_kit::open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(gpui_kit::px(900.), gpui_kit::px(640.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Monitter HTML browser smoke".into()), ..Default::default() }),
        ..Default::default()
    }, cx, |window, cx| {
        let tab = cx.new(|cx| {
            let mut tab = BrowserTab::new(window, cx);
            tab.docked_mode = true;
            tab.open_url(&path, None, cx).expect("open native HTML smoke");
            tab.show(cx);
            cx.spawn_in(window, async move |tab, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let (sender, receiver) = mpsc::channel();
                let _ = tab.update_in(cx, |tab, _, cx| {
                    tab.webview.as_ref().expect("native view").read(cx).raw()
                        .evaluate_script_with_callback(
                            "JSON.stringify({title:document.title, heading:document.querySelector('h1')?.textContent, color:getComputedStyle(document.querySelector('h1')).color, image:document.querySelector('img')?.naturalWidth, url:location.href})",
                            move |result| { let _ = sender.send(result); }
                        ).expect("evaluate native HTML");
                });
                for _ in 0..50 {
                    if let Ok(result) = receiver.try_recv() {
                        eprintln!("MONITTER_HTML_SMOKE_RESULT={result}");
                        let _ = tab.update_in(cx, |_, _, cx| cx.quit());
                        return;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                eprintln!("MONITTER_HTML_SMOKE_TIMEOUT");
                let _ = tab.update_in(cx, |_, _, cx| cx.quit());
            }).detach();
            tab
        });
        tab
    }).expect("open isolated HTML browser smoke");
    cx.activate(true);
    true
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserMetadataSnapshot, BrowserTabMetadata, MAX_BROWSER_TABS, bounded_title,
        is_stale_initial_blank, is_stale_initial_title, restore_plan, restore_revision_is_current,
        validated_url,
    };

    /// Real retained manager/tab entities with no WKWebView, network or profile.
    fn offline_transfer_fixture(cx: &mut gpui_kit::TestAppContext) -> (
        gpui_kit::Entity<super::BrowserPane>,
        [gpui_kit::Entity<super::BrowserTab>; 2],
        &mut gpui_kit::VisualTestContext,
    ) {
        use gpui_kit::{AppContext as _, component::Root};
        use std::{cell::RefCell, rc::Rc};
        cx.update(|cx| gpui_kit::init(cx));
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        let (_, visual) = cx.add_window_view(move |window, cx| {
            let tabs: [_; 2] = std::array::from_fn(|index| cx.new(|cx| {
                let mut tab = super::BrowserTab::new_with_native(window, cx, Err("offline fixture".into()), None);
                tab.error = None;
                tab.docked_mode = true;
                tab.visible = true;
                tab.unloaded = false;
                tab.loading = true;
                tab.requested_url = format!("https://example.invalid/page/{index}");
                tab.location = tab.requested_url.clone();
                tab.actual_url = Some(tab.location.clone());
                tab.title = format!("Retained page {index}");
                tab
            }));
            let manager = cx.new(|cx| {
                let mut manager = super::BrowserPane::with_initial_tab(tabs[0].clone(), cx);
                manager.attach_tab(2, &tabs[1], cx);
                manager.tabs.push((2, tabs[1].clone()));
                manager.next_id = 3;
                manager.docked_mode = true;
                manager
            });
            *output.borrow_mut() = Some((manager.clone(), tabs));
            Root::new(manager, window, cx)
        });
        let (manager, tabs) = slot.borrow_mut().take().unwrap();
        (manager, tabs, visual)
    }

    #[gpui_kit::test]
    fn browser_pane_uses_a_slide_down_dock_drag_topbar(cx: &mut gpui_kit::TestAppContext) {
        use crate::docking::{DockWorkspace, PaneFactory, PaneSpec};
        use gpui_kit::{
            AppContext as _, Focusable as _, Modifiers, MouseButton, component::Root, px,
            size,
        };
        use std::{cell::RefCell, rc::Rc};

        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
        });
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        let (_, visual) = cx.add_window_view(move |window, cx| {
            let factory: PaneFactory = Rc::new(|_, _, _| None);
            let dock = cx.new(|cx| DockWorkspace::new("browser-topbar-test", factory, window, cx));
            dock.update(cx, |dock, cx| {
                dock.apply_layout_preset("columns", window, cx).unwrap();
            });
            let tab = cx.new(|cx| {
                let mut tab = super::BrowserTab::new_with_native(
                    window,
                    cx,
                    Err("offline browser topbar fixture".into()),
                    None,
                );
                tab.error = None;
                tab.docked_mode = true;
                tab.visible = true;
                tab.unloaded = false;
                tab.initial_address_pending = false;
                tab.title = "Browser topbar".into();
                tab
            });
            let focus = tab.read(cx).focus_handle(cx);
            let origin = dock.read(cx).group_for_pane("empty:preset-1", cx).unwrap();
            dock.update(cx, |dock, cx| {
                dock.add_browser_beside(
                    PaneSpec {
                        id: "browser:topbar-test".into(),
                        title: "Browser".into(),
                        content: tab.clone().into(),
                        focus,
                        lifecycle: None,
                        closable: true,
                    },
                    Some(origin),
                    window,
                    cx,
                )
                .unwrap();
            });
            *output.borrow_mut() = Some((dock.clone(), tab));
            Root::new(dock, window, cx)
        });
        let (_dock, tab) = slot.borrow_mut().take().unwrap();
        visual.simulate_resize(size(px(1200.), px(720.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));

        let sensor = visual
            .debug_bounds("browser-pane-topbar-sensor")
            .expect("browser pane mounts its top-edge reveal sensor");
        let hidden = visual
            .debug_bounds("browser-pane-topbar")
            .expect("browser pane mounts its own topbar instead of dock corner chrome");
        assert!(
            hidden.size.height >= px(36.) && hidden.size.height <= px(52.),
            "browser pane topbar stays compact"
        );
        assert!(visual.debug_bounds("browser-pane-close").is_some());
        assert!(visual.debug_bounds("browser-pane-maximize").is_some());
        let drag = visual
            .debug_bounds("dock-pane-header-drag-region")
            .expect("topbar body uses the dock's native DragPanel lifecycle");
        assert!(drag.size.width >= px(56.) && drag.size.width <= px(64.));

        visual.simulate_mouse_move(sensor.center(), None, Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|_, cx| assert!(tab.read(cx).pane_toolbar_hovered));
        for _ in 0..14 {
            visual
                .executor()
                .advance_clock(std::time::Duration::from_millis(16));
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
        }
        let revealed = visual.debug_bounds("browser-pane-topbar").unwrap();
        assert!(
            revealed.top() > hidden.top() + px(5.),
            "topbar slides down on top-edge hover: hidden={hidden:?}, revealed={revealed:?}"
        );

        // The controls remain outside the drag region; beginning a pane drag
        // from its body must not trigger close or maximize.
        let start = visual.debug_bounds("dock-pane-header-drag-region").unwrap().center();
        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        visual.simulate_mouse_up(start, MouseButton::Left, Modifiers::none());
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("browser-pane-topbar").is_some());
    }

    #[gpui_kit::test]
    fn browser_transfer_preflight_rejects_batch_without_mutating_retained_pages(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let (manager, tabs, visual) = offline_transfer_fixture(cx);
        visual.update(|window, cx| {
            let before = manager.read(cx).metadata_snapshot(cx);
            let revision = manager.read(cx).metadata_revision();
            for ids in [&[1, 999][..], &[1, 1][..], &[1, 2][..]] {
                assert!(manager.update(cx, |manager, cx| manager.transfer_tabs_to_window(ids, window, cx)).is_err());
                assert_eq!(manager.read(cx).metadata_snapshot(cx), before);
                assert_eq!(manager.read(cx).metadata_revision(), revision);
                for (index, tab) in tabs.iter().enumerate() {
                    let tab_state = tab.read(cx);
                    assert!(tab_state.visible && tab_state.loading && !tab_state.unloaded);
                    assert_eq!(manager.read(cx).tab_entity(index as u64 + 1).unwrap().entity_id(), tab.entity_id());
                    assert_eq!(tab_state.host_window.window_id(), window.window_handle().window_id());
                }
            }
        });
    }

    #[gpui_kit::test]
    fn browser_scoped_visibility_and_close_preserve_unselected_tab(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let (manager, tabs, visual) = offline_transfer_fixture(cx);
        visual.update(|_, cx| {
            // Invalid scope is rejected before an earlier valid tab can hide.
            assert!(manager.update(cx, |manager, cx| manager.hide_tabs(&[1, 999], cx)).is_err());
            assert!(tabs[0].read(cx).visible && tabs[1].read(cx).visible);
            let peer_url = tabs[1].read(cx).location.clone();
            manager.update(cx, |manager, cx| manager.hide_tabs(&[1], cx)).unwrap();
            assert!(!tabs[0].read(cx).visible && tabs[1].read(cx).visible);
            manager.update(cx, |manager, cx| manager.show_tabs(&[1], cx)).unwrap();
            assert!(tabs[0].read(cx).visible && tabs[1].read(cx).visible);
            manager.update(cx, |manager, cx| manager.close_tabs(&[1], cx)).unwrap();
            assert!(manager.read(cx).tab_entity(1).is_none());
            assert_eq!(manager.read(cx).tab_entity(2).unwrap().entity_id(), tabs[1].entity_id());
            assert!(!tabs[0].read(cx).loading);
            let peer = tabs[1].read(cx);
            assert!(peer.visible && peer.loading && !peer.unloaded);
            assert_eq!(peer.location, peer_url);
            assert_eq!(manager.read(cx).tabs.len(), 1, "closing one host creates no replacement native tab");
        });
    }

    #[gpui_kit::test]
    fn empty_browser_opens_focused_address_once_on_first_display(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _, component::{Root, Theme, ThemeMode}};
        use std::{cell::{Cell, RefCell}, rc::Rc};
        for light in [true, false] {
            cx.update(|cx| {
                gpui_kit::init(cx);
                crate::surface_blend::initialize_for_ui_fixture();
                Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
            });
            let slot = Rc::new(RefCell::new(None));
            let output = slot.clone();
            let (_, visual) = cx.add_window_view(move |window, cx| {
                let tab = cx.new(|cx| {
                    let mut tab = super::BrowserTab::new_with_native(window, cx, Err("offline fixture".into()), None);
                    tab.error = None;
                    tab.docked_mode = true;
                    tab
                });
                *output.borrow_mut() = Some(tab.clone());
                Root::new(tab, window, cx)
            });
            let tab = slot.borrow().as_ref().unwrap().clone();
            let edits = Rc::new(Cell::new(0));
            let observed = edits.clone();
            let _events = visual.update(|_, cx| cx.subscribe(&tab, move |_, event: &super::BrowserTabEvent, _| {
                if matches!(event, super::BrowserTabEvent::AddressEdited) { observed.set(observed.get() + 1); }
            }));
            visual.simulate_resize(gpui_kit::size(gpui::px(280.), gpui::px(320.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.update(|window, cx| {
                assert!(!tab.read(cx).omnibar_open, "hidden creation must not open or steal focus");
                assert!(!tab.read(cx).address.read(cx).focus_handle(cx).is_focused(window));
                tab.update(cx, |tab, cx| { tab.show(cx); cx.notify(); });
            });
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.update(|_, cx| assert!(tab.read(cx).omnibar_open, "first visible render opens address state"));
            let card = visual.debug_bounds("browser-omnibar-card").expect("new empty pane shows address menu");
            assert!(card.left() >= gpui::px(0.) && card.right() <= gpui::px(280.5));
            visual.update(|window, cx| {
                let tab = tab.read(cx);
                assert!(tab.omnibar_open);
                assert!(tab.address.read(cx).value().is_empty());
                assert!(tab.address.read(cx).focus_handle(cx).is_focused(window), "ready to type without Cmd-L");
            });
            assert_eq!(edits.get(), 0, "automatic opening must not invalidate metadata restore as a user edit");
            visual.simulate_input("example.com");
            visual.update(|_, cx| assert_eq!(tab.read(cx).address.read(cx).value().as_str(), "example.com"));
            assert!(edits.get() > 0);
            visual.simulate_keystrokes("escape");
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.update(|_, cx| tab.update(cx, |tab, cx| {
                assert!(!tab.omnibar_open, "Escape dismisses the menu");
                tab.hide(cx); tab.show(cx); cx.notify();
            }));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("browser-omnibar-card").is_none(), "returning to a pane must not reopen a dismissed menu");
            visual.update(|window, cx| tab.update(cx, |tab, cx| tab.open_omnibar(window, cx)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("browser-omnibar-card").is_some(), "explicit address action still opens it");
            visual.update(|window, _| window.remove_window());
        }
    }

    #[gpui_kit::test]
    fn restored_and_direct_url_browser_panes_do_not_auto_open_address(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _, component::Root};
        use std::{cell::RefCell, rc::Rc};
        cx.update(|cx| { gpui_kit::init(cx); crate::surface_blend::initialize_for_ui_fixture(); });
        for restored_url in [Some("https://example.com/saved"), Some("about:blank"), None] {
            let slot = Rc::new(RefCell::new(None));
            let output = slot.clone();
            let (_, visual) = cx.add_window_view(move |window, cx| {
                let tab = cx.new(|cx| {
                    let mut tab = super::BrowserTab::new_with_native(window, cx, Err("offline fixture".into()), None);
                    tab.error = None;
                    if let Some(url) = restored_url {
                        tab.restore_metadata(&super::RestoredTabMetadata { id: 1, requested_url: url.into(), actual_url: None, title: "Saved".into() }, window, cx);
                    } else {
                        // State after a successful direct URL open, before its first frame.
                        tab.unloaded = false;
                        tab.requested_url = "https://example.com/direct".into();
                        tab.location = tab.requested_url.clone();
                    }
                    tab.show(cx);
                    tab
                });
                *output.borrow_mut() = Some(tab.clone());
                Root::new(tab, window, cx)
            });
            let tab = slot.borrow().as_ref().unwrap().clone();
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.update(|window, cx| {
                assert!(!tab.read(cx).omnibar_open);
                assert!(!tab.read(cx).address.read(cx).focus_handle(cx).is_focused(window));
            });
            assert!(visual.debug_bounds("browser-omnibar-card").is_none());
            visual.update(|window, _| window.remove_window());
        }
    }

    #[gpui_kit::test]
    fn page_load_activity_ignores_blank_and_unloaded_callbacks_and_clears_on_errors(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (tab, visual) = cx.add_window_view(|window, cx| {
            super::BrowserTab::new_with_native(window, cx, Err("No native view in state test".into()), None)
        });
        visual.update(|_, cx| {
            tab.update(cx, |tab, cx| {
                assert!(!tab.is_loading());
                tab.page_load(true, "https://example.com/", cx);
                assert!(!tab.is_loading(), "unloaded panes ignore late native callbacks");
                tab.unloaded = false;
                tab.page_load(true, "about:blank", cx);
                assert!(!tab.is_loading());
                tab.page_load(true, "https://example.com/", cx);
                assert!(tab.is_loading());
                tab.page_load(false, "about:blank", cx);
                assert!(tab.is_loading(), "late initial blank must not finish a real load");
                tab.page_load(false, "https://example.com/", cx);
                assert!(!tab.is_loading());
                tab.page_load(true, "https://example.com/", cx);
                assert!(tab.open_url("javascript:alert(1)", None, cx).is_err());
                assert!(!tab.is_loading(), "blocked navigation clears loading");
                tab.page_load(true, "https://example.com/", cx);
                tab.reload(cx);
                assert!(!tab.is_loading(), "native reload errors clear loading");
                tab.page_load(true, "https://example.com/", cx);
                tab.close(cx);
                assert!(!tab.is_loading());
                assert!(tab._loading_watch.is_none());
                tab.page_load(true, "https://example.com/", cx);
                assert!(!tab.is_loading(), "closed panes ignore late page events");
            });
        });
    }

    #[test]
    fn native_callback_stream_wakes_without_timer_and_closes_on_teardown() {
        use futures::{FutureExt as _, StreamExt as _};
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        assert!(receiver.next().now_or_never().is_none());
        sender
            .unbounded_send(super::BrowserEvent::Location("https://example.com".into()))
            .unwrap();
        sender
            .unbounded_send(super::BrowserEvent::Title("Example".into()))
            .unwrap();
        assert!(matches!(
            receiver.next().now_or_never(),
            Some(Some(super::BrowserEvent::Location(_)))
        ));
        assert!(matches!(
            receiver.next().now_or_never(),
            Some(Some(super::BrowserEvent::Title(_)))
        ));
        drop(sender);
        assert!(matches!(receiver.next().now_or_never(), Some(None)));
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        drop(receiver);
        assert!(
            sender
                .unbounded_send(super::BrowserEvent::Title("late callback".into()))
                .is_err()
        );
    }

    fn tab(id: u64, requested_url: &str) -> BrowserTabMetadata {
        BrowserTabMetadata {
            id,
            requested_url: requested_url.to_owned(),
            actual_url: None,
            title: format!("Tab {id}"),
        }
    }

    #[::core::prelude::v1::test]
    fn browser_url_policy() {
        for url in [
            "about:blank",
            "https://example.com/path?q=1",
            "http://localhost:19436",
        ] {
            assert!(validated_url(url).is_ok(), "{url}");
        }
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,test",
            "about:blank#x",
            "https://user:pass@example.com",
            "example.com",
        ] {
            assert!(validated_url(url).is_err(), "{url}");
        }
    }

    #[::core::prelude::v1::test]
    fn browser_restore_plan_preserves_order_and_selection_without_loading() {
        let snapshot = BrowserMetadataSnapshot {
            tabs: vec![tab(9, "https://example.com/first"), tab(4, "about:blank")],
            selected_id: Some(4),
        };
        let plan = restore_plan(&snapshot).unwrap();
        assert_eq!(
            plan.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            [9, 4]
        );
        assert_eq!(plan.selected_id, Some(4));
        assert_eq!(plan.next_id, 10);
        assert_eq!(plan.tabs[0].requested_url, "https://example.com/first");

        let empty_plan = restore_plan(&BrowserMetadataSnapshot::default()).unwrap();
        assert!(empty_plan.tabs.is_empty());
        assert_eq!(empty_plan.selected_id, None);
        assert_eq!(empty_plan.next_id, 1);
    }

    #[::core::prelude::v1::test]
    fn browser_restore_plan_rejects_unbounded_or_unsafe_metadata() {
        for snapshot in [
            BrowserMetadataSnapshot {
                tabs: vec![tab(1, "https://example.com"), tab(1, "https://other.test")],
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: (1..=MAX_BROWSER_TABS as u64 + 1)
                    .map(|id| tab(id, "about:blank"))
                    .collect(),
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![tab(1, "https://user:secret@example.com")],
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![tab(1, &format!("https://example.com/{}", "x".repeat(8192)))],
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![BrowserTabMetadata {
                    id: 1,
                    requested_url: "about:blank".into(),
                    actual_url: None,
                    title: "x".repeat(513),
                }],
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![BrowserTabMetadata {
                    id: 1,
                    requested_url: "about:blank".into(),
                    actual_url: None,
                    title: "bad\ntitle".into(),
                }],
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![tab(1, "about:blank")],
                selected_id: None,
            },
            BrowserMetadataSnapshot {
                tabs: vec![tab(1, "about:blank")],
                selected_id: Some(2),
            },
            BrowserMetadataSnapshot {
                tabs: (1..=40)
                    .map(|id| tab(id, &format!("https://e.test/{}", "x".repeat(8150))))
                    .collect(),
                selected_id: Some(1),
            },
            BrowserMetadataSnapshot {
                tabs: vec![tab(u64::MAX, "about:blank")],
                selected_id: Some(u64::MAX),
            },
        ] {
            assert!(restore_plan(&snapshot).is_err());
        }
    }

    #[::core::prelude::v1::test]
    fn browser_restore_revision_guard_and_strict_selection() {
        assert!(restore_revision_is_current(7, 7));
        assert!(!restore_revision_is_current(8, 7));
        assert!(
            restore_plan(&BrowserMetadataSnapshot {
                tabs: vec![tab(5, "about:blank")],
                selected_id: Some(99),
            })
            .is_err()
        );
    }

    #[::core::prelude::v1::test]
    fn initial_blank_callback_guard_survives_explicit_navigation_until_commit() {
        assert!(is_stale_initial_blank(true, "about:blank", None));
        assert!(!is_stale_initial_blank(true, "https://example.com/", None));
        assert!(!is_stale_initial_blank(
            true,
            "about:blank",
            Some("https://example.com/")
        ));
        assert!(!is_stale_initial_blank(false, "about:blank", None));
        assert!(is_stale_initial_title(true, None));
        assert!(!is_stale_initial_title(false, None));
        assert!(!is_stale_initial_title(true, Some("https://example.com/")));
        assert_eq!(bounded_title("safe\nmalicious\u{7f}"), "safemalicious");
    }
}

#[cfg(test)]
#[path = "browser_context_tests.rs"]
mod browser_context_tests;
