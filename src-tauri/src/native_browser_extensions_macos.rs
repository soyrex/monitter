//! Bounded host for unpacked Safari/WebExtension-compatible directories.
//!
//! This module is deliberately macOS-only. It owns one `WKWebExtensionController`
//! and one app-private `WKWebsiteDataStore` on the AppKit main thread. Callers
//! use `child_webview_configuration` from either a Tauri child webview or a
//! dedicated native `WKWebView` adapter; they must not reuse the Monitter shell
//! configuration.
//! Loading is asynchronous because WebKit parses an unpacked extension directory
//! asynchronously. A successful load proves only WebKit accepted the manifest;
//! popup, login and autofill behaviour require a real extension-specific test.

use std::{cell::RefCell, collections::{HashMap, HashSet}, fs, path::{Path, PathBuf}};

use block2::RcBlock;
use objc2::{define_class, msg_send, rc::{Allocated, Retained}, runtime::{NSObject, NSObjectProtocol, ProtocolObject}, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSPopover;
use objc2_foundation::{NSArray, NSError, NSString, NSURL, NSUUID};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionController,
    WKWebExtensionControllerConfiguration, WKWebExtensionControllerDelegate, WKWebExtensionTab,
    WKWebExtensionWindow, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use sha2::{Digest, Sha256};

const EXTENSION_PROFILE_UUID: &str = "f06d2ea1-32b0-43cd-97f4-2b796a11f87d";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

struct ExtensionHost {
    controller: Retained<WKWebExtensionController>,
    data_store: Retained<WKWebsiteDataStore>,
    /// Contains both an in-flight WebKit parse and a loaded context. This
    /// prevents loading the same directory twice into one controller.
    loaded_or_pending: HashSet<PathBuf>,
    /// Retaining contexts makes default-action lookup explicit and keeps the
    /// host's loaded-extension model independent of WebKit set enumeration.
    contexts: HashMap<PathBuf, Retained<WKWebExtensionContext>>,
    popup_delegate: Retained<ExtensionPopupDelegate>,
    extension_window: Retained<ExtensionWindow>,
    tabs: HashMap<String, Retained<ExtensionTab>>,
    tab_order: Vec<String>,
    focused_tab: Option<String>,
    window_is_open: bool,
}

thread_local! {
    // WebKit's extension controller is MainThreadOnly and must never enter
    // Tauri managed state (which can be accessed off the AppKit main thread).
    static HOST: RefCell<Option<ExtensionHost>> = const { RefCell::new(None) };
    static POPUP_PRESENTER: RefCell<Option<Box<dyn FnMut(Retained<NSPopover>)>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MonitterWebExtensionPopupDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct ExtensionPopupDelegate;

    impl ExtensionPopupDelegate {
        #[unsafe(method_id(init))]
        unsafe fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for ExtensionPopupDelegate {}

    impl ExtensionPopupDelegate {
        #[unsafe(method(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> *mut NSArray<ProtocolObject<dyn WKWebExtensionWindow>> {
            Retained::autorelease_ptr(extension_windows())
        }

        #[unsafe(method(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> *mut ProtocolObject<dyn WKWebExtensionWindow> {
            focused_extension_window().map(Retained::autorelease_ptr).unwrap_or(std::ptr::null_mut())
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(
            &self,
            _controller: &WKWebExtensionController,
            action: &WKWebExtensionAction,
            _context: &WKWebExtensionContext,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            // The native adapter supplies the anchor and presentation policy.
            // We never create a synthetic browser tab or grant permissions here.
            let presented = if let Some(popover) = unsafe { action.popupPopover() } {
                POPUP_PRESENTER.with(|presenter| {
                    if let Some(presenter) = presenter.borrow_mut().as_mut() {
                        presenter(popover);
                        true
                    } else {
                        false
                    }
                })
            } else {
                false
            };
            if presented {
                completion_handler.call((std::ptr::null_mut(),));
            } else {
                let domain = NSString::from_str("com.monitter.browser-extension");
                let error = unsafe { NSError::errorWithDomain_code_userInfo(&domain, 1, None) };
                completion_handler.call((Retained::autorelease_ptr(error),));
            }
        }
    }

    unsafe impl WKWebExtensionControllerDelegate for ExtensionPopupDelegate {}
);

// A single native browser surface is intentionally reported as one normal
// extension window. Tabs are registered separately as the Tauri child views
// are created. Keeping this object free of a retained tab avoids an Objective-C
// reference cycle; the main-thread host owns both objects.
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MonitterWebExtensionWindow"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct ExtensionWindow;

    unsafe impl NSObjectProtocol for ExtensionWindow {}

    impl ExtensionWindow {
        #[unsafe(method(tabsForWebExtensionContext:))]
        fn tabs(
            &self,
            _context: &WKWebExtensionContext,
        ) -> *mut NSArray<ProtocolObject<dyn WKWebExtensionTab>> {
            Retained::autorelease_ptr(extension_tabs())
        }

        #[unsafe(method(activeTabForWebExtensionContext:))]
        fn active_tab(
            &self,
            _context: &WKWebExtensionContext,
        ) -> *mut ProtocolObject<dyn WKWebExtensionTab> {
            focused_extension_tab().map(Retained::autorelease_ptr).unwrap_or(std::ptr::null_mut())
        }
    }

    unsafe impl WKWebExtensionWindow for ExtensionWindow {}
);

struct ExtensionTabIvars {
    web_view: Retained<WKWebView>,
}

// WebKit's real tab representation for one real native WKWebView. Returning
// that WKWebView from the protocol is the critical link that enables content
// script injection; a Wry child-view handle by itself is not sufficient.
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MonitterWebExtensionTab"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ExtensionTabIvars]
    struct ExtensionTab;

    unsafe impl NSObjectProtocol for ExtensionTab {}

    impl ExtensionTab {
        #[unsafe(method(webViewForWebExtensionContext:))]
        fn web_view(
            &self,
            _context: &WKWebExtensionContext,
        ) -> *mut WKWebView {
            Retained::autorelease_ptr(self.ivars().web_view.clone())
        }

        #[unsafe(method(windowForWebExtensionContext:))]
        fn window(
            &self,
            _context: &WKWebExtensionContext,
        ) -> *mut ProtocolObject<dyn WKWebExtensionWindow> {
            extension_window().map(Retained::autorelease_ptr).unwrap_or(std::ptr::null_mut())
        }
    }

    unsafe impl WKWebExtensionTab for ExtensionTab {}
);

impl ExtensionPopupDelegate {
    fn new(mtm: MainThreadMarker) -> Result<Retained<Self>, String> {
        let this = Self::alloc(mtm).set_ivars(());
        Ok(unsafe { msg_send![super(this), init] })
    }
}

impl ExtensionWindow {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

impl ExtensionTab {
    fn new(
        mtm: MainThreadMarker,
        web_view: Retained<WKWebView>,
        _tab_uuid: &str,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ExtensionTabIvars {
            web_view,
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn as_extension_tab(tab: Retained<ExtensionTab>) -> Retained<ProtocolObject<dyn WKWebExtensionTab>> {
    ProtocolObject::from_retained(tab)
}

fn as_extension_window(
    window: Retained<ExtensionWindow>,
) -> Retained<ProtocolObject<dyn WKWebExtensionWindow>> {
    ProtocolObject::from_retained(window)
}

fn extension_tabs() -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
    HOST.with(|host| {
        let tabs = host
            .borrow()
            .as_ref()
            .map(|host| {
                host.tab_order
                    .iter()
                    .filter_map(|id| host.tabs.get(id).cloned())
                    .map(as_extension_tab)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        NSArray::from_retained_slice(&tabs)
    })
}

fn focused_extension_tab() -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref()?;
        host.focused_tab
            .as_ref()
            .and_then(|id| host.tabs.get(id).cloned())
            .map(as_extension_tab)
    })
}

fn extension_window() -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref()?;
        host.window_is_open.then(|| as_extension_window(host.extension_window.clone()))
    })
}

fn extension_windows() -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
    let windows = extension_window().into_iter().collect::<Vec<_>>();
    NSArray::from_retained_slice(&windows)
}

fn focused_extension_window() -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
    HOST.with(|host| {
        host.borrow()
            .as_ref()
            .filter(|host| host.focused_tab.is_some())
            .map(|host| as_extension_window(host.extension_window.clone()))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedExtension {
    pub directory: PathBuf,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionActionState {
    pub label: String,
    pub enabled: bool,
    pub presents_popup: bool,
}

fn main_thread() -> Result<MainThreadMarker, String> {
    MainThreadMarker::new().ok_or_else(|| {
        "macOS browser extensions must be initialized on the AppKit main thread.".into()
    })
}

fn profile_uuid() -> Result<Retained<NSUUID>, String> {
    let value = NSString::from_str(EXTENSION_PROFILE_UUID);
    NSUUID::from_string(&value).ok_or_else(|| "Invalid internal extension profile identifier.".into())
}

fn ensure_host(mtm: MainThreadMarker) -> Result<(), String> {
    HOST.with(|host| {
        if host.borrow().is_some() {
            return Ok(());
        }

        let identifier = profile_uuid()?;
        let data_store = unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&identifier, mtm) };
        let controller_configuration = unsafe {
            WKWebExtensionControllerConfiguration::configurationWithIdentifier(&identifier, mtm)
        };
        // The binding marks None as potentially invalid; this app-private
        // persistent store is always present and is shared with tab configs.
        unsafe { controller_configuration.setDefaultWebsiteDataStore(Some(&data_store)) };
        let controller = unsafe {
            WKWebExtensionController::initWithConfiguration(
                WKWebExtensionController::alloc(mtm),
                &controller_configuration,
            )
        };
        let popup_delegate = ExtensionPopupDelegate::new(mtm)?;
        let extension_window = ExtensionWindow::new(mtm);
        *host.borrow_mut() = Some(ExtensionHost {
            controller,
            data_store,
            loaded_or_pending: HashSet::new(),
            contexts: HashMap::new(),
            popup_delegate,
            extension_window,
            tabs: HashMap::new(),
            tab_order: Vec::new(),
            focused_tab: None,
            window_is_open: false,
        });
        // The delegate also supplies real tab/window discovery. Keep it
        // installed even when no toolbar-popup presenter is configured.
        let host = host.borrow();
        let host = host.as_ref().expect("extension host was just installed");
        let delegate = ProtocolObject::<dyn WKWebExtensionControllerDelegate>::from_ref(&*host.popup_delegate);
        unsafe { host.controller.setDelegate(Some(delegate)) };
        Ok(())
    })
}

/// Returns an app-private configuration for exactly one native browser child.
///
/// The caller must invoke this from the AppKit main thread and use the returned
/// configuration only for an app-private external-browser view.
pub fn child_webview_configuration() -> Result<Retained<WKWebViewConfiguration>, String> {
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        // These setters are marked unsafe by the generated bindings because
        // WebKit requires main-thread confinement. `mtm` establishes that.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        unsafe {
            configuration.setWebsiteDataStore(&host.data_store);
            configuration.setWebExtensionController(Some(&host.controller));
        }
        Ok(configuration)
    })
}

/// Returns another retained handle to the one shared extension controller.
///
/// A dedicated native `WKWebView` adapter can use this when it needs to build
/// a configuration itself. As with the configuration helper, call on AppKit's
/// main thread only.
pub fn shared_controller() -> Result<Retained<WKWebExtensionController>, String> {
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        host.borrow()
            .as_ref()
            .map(|host| host.controller.clone())
            .ok_or_else(|| "Browser extension host is unavailable.".into())
    })
}

/// Registers one real Tauri child `WKWebView` as an extension tab.
///
/// This must be called on the AppKit main thread immediately after creating
/// the child view with [`child_webview_configuration`], and before treating
/// the extension as available for that tab. The UUID must remain stable until
/// `unregister_tauri_child_webview` is called. It deliberately retains the
/// view only while the tab is registered.
pub fn register_tauri_child_webview(
    tab_uuid: &str,
    web_view: Retained<WKWebView>,
) -> Result<(), String> {
    uuid::Uuid::parse_str(tab_uuid)
        .map_err(|_| "Native browser extension tabs require a UUID tab identifier.".to_string())?;
    let mtm = main_thread()?;
    ensure_host(mtm)?;

    let (controller, opened_window, window, tab) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        let host = host.as_mut().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        if host.tabs.contains_key(tab_uuid) {
            return Err(format!("Native browser extension tab {tab_uuid} is already registered."));
        }
        let tab = ExtensionTab::new(mtm, web_view, tab_uuid);
        let tab_for_webkit = as_extension_tab(tab.clone());
        let opened_window = !host.window_is_open;
        host.window_is_open = true;
        host.tab_order.push(tab_uuid.to_owned());
        host.tabs.insert(tab_uuid.to_owned(), tab);
        Ok::<_, String>((
            host.controller.clone(),
            opened_window,
            as_extension_window(host.extension_window.clone()),
            tab_for_webkit,
        ))
    })?;

    unsafe {
        if opened_window {
            controller.didOpenWindow(&window);
        }
        controller.didOpenTab(&tab);
    }
    Ok(())
}

/// Marks a previously registered Tauri child view as the tab actually visible
/// to the user. Call this only in the native selection/focus path, not merely
/// because a tab was created in application state.
pub fn focus_tauri_child_webview(tab_uuid: &str) -> Result<(), String> {
    let _mtm = main_thread()?;
    let (controller, window, activated, previous) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        let host = host.as_mut().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        let activated = host.tabs.get(tab_uuid).cloned()
            .ok_or_else(|| format!("Native browser extension tab {tab_uuid} is not registered."))?;
        let already_focused = host.focused_tab.as_deref() == Some(tab_uuid);
        let previous = (!already_focused).then(|| {
            host.focused_tab.as_ref()
                .and_then(|previous| host.tabs.get(previous).cloned())
        }).flatten();
        host.focused_tab = Some(tab_uuid.to_owned());
        Ok::<_, String>((
            host.controller.clone(),
            as_extension_window(host.extension_window.clone()),
            (!already_focused).then(|| as_extension_tab(activated)),
            previous.map(as_extension_tab),
        ))
    })?;
    unsafe {
        controller.didFocusWindow(Some(&window));
        if let Some(activated) = activated.as_deref() {
            controller.didActivateTab_previousActiveTab(activated, previous.as_deref());
        }
    }
    Ok(())
}

/// Removes a real Tauri child `WKWebView` from the extension tab registry.
///
/// This does not close the native child view; callers must remove the tab from
/// WebKit first, then call this while it is still on the AppKit main thread.
/// An unknown UUID is reported rather than pretending the extension saw a
/// close event.
pub fn unregister_tauri_child_webview(tab_uuid: &str) -> Result<(), String> {
    let _mtm = main_thread()?;
    let (controller, tab, closes_window, window) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        let host = host.as_mut().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        let tab = host.tabs.remove(tab_uuid)
            .ok_or_else(|| format!("Native browser extension tab {tab_uuid} is not registered."))?;
        host.tab_order.retain(|id| id != tab_uuid);
        if host.focused_tab.as_deref() == Some(tab_uuid) {
            host.focused_tab = None;
        }
        let closes_window = host.tabs.is_empty();
        if closes_window {
            host.window_is_open = false;
        }
        Ok::<_, String>((
            host.controller.clone(),
            as_extension_tab(tab),
            closes_window,
            as_extension_window(host.extension_window.clone()),
        ))
    })?;
    unsafe {
        controller.didCloseTab_windowIsClosing(&tab, closes_window);
        if closes_window {
            controller.didFocusWindow(None);
            controller.didCloseWindow(&window);
        }
    }
    Ok(())
}

/// Installs the only route by which WebKit may request popup presentation.
///
/// The callback receives WebKit's preloaded `NSPopover`; the native adapter
/// must anchor it to a real `NSView` and own its presentation lifecycle. With
/// no presenter, the delegate is detached, so a popup action is not advertised
/// as supported. This does not grant extension permissions or native messaging.
pub fn set_popup_presenter(
    presenter: Option<Box<dyn FnMut(Retained<NSPopover>)>>,
) -> Result<(), String> {
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    POPUP_PRESENTER.with(|slot| *slot.borrow_mut() = presenter);
    Ok(())
}

/// Reads the default extension action. Per-tab actions require the native
/// browser adapter to provide a real `WKWebExtensionTab` for its WKWebView;
/// Wry child views alone cannot be represented as an extension tab safely.
pub fn default_action(directory: impl AsRef<Path>) -> Result<Option<ExtensionActionState>, String> {
    let directory = fs::canonicalize(directory.as_ref())
        .map_err(|error| format!("Cannot resolve extension directory: {error}"))?;
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        let Some(context) = host.contexts.get(&directory) else {
            return Ok(None);
        };
        let action = unsafe { context.actionForTab(None) };
        Ok(action.map(|action| ExtensionActionState {
            label: unsafe { action.label() }.to_string(),
            enabled: unsafe { action.isEnabled() },
            presents_popup: unsafe { action.presentsPopup() },
        }))
    })
}

/// Returns WebKit's popup only for a loaded default action that declares one.
/// The caller must first install a popup presenter and must show the popover
/// from a native AppKit view; this helper does not synthesize an anchor.
pub fn default_action_popup(
    directory: impl AsRef<Path>,
) -> Result<Option<Retained<NSPopover>>, String> {
    let directory = fs::canonicalize(directory.as_ref())
        .map_err(|error| format!("Cannot resolve extension directory: {error}"))?;
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        let Some(context) = host.contexts.get(&directory) else {
            return Ok(None);
        };
        let Some(action) = (unsafe { context.actionForTab(None) }) else {
            return Ok(None);
        };
        if !unsafe { action.presentsPopup() } {
            return Ok(None);
        }
        Ok(unsafe { action.popupPopover() })
    })
}

/// Invokes a loaded extension's default action. Popup actions require a
/// presenter installed with `set_popup_presenter`; non-popup actions remain
/// extension-defined and are not treated as a successful login or autofill.
pub fn perform_default_action(directory: impl AsRef<Path>) -> Result<(), String> {
    let directory = fs::canonicalize(directory.as_ref())
        .map_err(|error| format!("Cannot resolve extension directory: {error}"))?;
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        let host = host.borrow();
        let host = host.as_ref().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        let context = host.contexts.get(&directory)
            .ok_or_else(|| "The unpacked browser extension is not loaded.".to_string())?;
        unsafe { context.performActionForTab(None) };
        Ok(())
    })
}

fn validate_unpacked_directory(directory: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(directory)
        .map_err(|error| format!("Cannot resolve extension directory: {error}"))?;
    if !canonical.is_dir() {
        return Err("Extension path must be an unpacked directory.".into());
    }
    let manifest = canonical.join("manifest.json");
    let metadata = fs::metadata(&manifest)
        .map_err(|_| "Unpacked extension directory must contain a regular manifest.json file.".to_string())?;
    if !metadata.is_file() {
        return Err("Unpacked extension directory must contain a regular manifest.json file.".into());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("Extension manifest.json exceeds the 1 MiB safety limit.".into());
    }
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifest).map_err(|error| format!("Cannot read extension manifest: {error}"))?,
    )
    .map_err(|error| format!("Extension manifest.json is not valid JSON: {error}"))?;
    if !value.is_object() {
        return Err("Extension manifest.json must contain a JSON object.".into());
    }
    Ok(canonical)
}

fn extension_identifier(directory: &Path) -> String {
    let digest = Sha256::digest(directory.to_string_lossy().as_bytes());
    format!("monitter-extension-{digest:x}")
}

fn webkit_error(error: *mut NSError) -> String {
    if error.is_null() {
        "WebKit rejected the extension without an error description.".into()
    } else {
        // Apple supplies this pointer only for the duration of the callback.
        unsafe { (&*error).localizedDescription().to_string() }
    }
}

/// Asynchronously validates and asks WebKit to load an unpacked extension.
///
/// This only accepts directories with a regular JSON `manifest.json`; ZIPs,
/// app bundles and Web Store installation are intentionally outside v1.
pub fn load_unpacked_extension(
    directory: impl AsRef<Path>,
    completion: impl FnOnce(Result<LoadedExtension, String>) + 'static,
) -> Result<(), String> {
    let directory = validate_unpacked_directory(directory.as_ref())?;
    let mtm = main_thread()?;
    ensure_host(mtm)?;
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let host = host.as_mut().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
        if !host.loaded_or_pending.insert(directory.clone()) {
            return Err::<(), String>(
                "This unpacked browser extension directory is already loaded or loading.".into(),
            );
        }
        Ok(())
    })?;
    let identifier = extension_identifier(&directory);
    let path = NSString::from_str(&directory.to_string_lossy());
    let resource_url = NSURL::fileURLWithPath_isDirectory(&path, true);
    let completion = RefCell::new(Some(completion));
    let callback = RcBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
        let result = (|| {
            let extension = std::ptr::NonNull::new(extension)
                .ok_or_else(|| format!("Cannot load browser extension: {}", webkit_error(error)))?;
            let mtm = main_thread()?;
            ensure_host(mtm)?;
            let context = unsafe { WKWebExtensionContext::contextForExtension(extension.as_ref()) };
            // WebKit's default context identifier is transient. This stable,
            // app-local value makes extension messaging/storage identity survive
            // process restart as long as the canonical unpacked path is stable.
            let identifier = NSString::from_str(&identifier);
            unsafe { context.setUniqueIdentifier(&identifier) };
            HOST.with(|host| {
                let mut host = host.borrow_mut();
                let host = host.as_mut().ok_or_else(|| "Browser extension host is unavailable.".to_string())?;
                unsafe { host.controller.loadExtensionContext_error(&context) }
                    .map_err(|error| format!("Cannot activate browser extension: {}", error.localizedDescription()))?;
                let display_name = unsafe { extension.as_ref().displayName() }.map(|name| name.to_string());
                host.contexts.insert(directory.clone(), context);
                Ok(LoadedExtension { directory: directory.clone(), display_name })
            })
        })();
        if result.is_err() {
            HOST.with(|host| {
                if let Some(host) = host.borrow_mut().as_mut() {
                    host.loaded_or_pending.remove(&directory);
                }
            });
        }
        if let Some(completion) = completion.borrow_mut().take() {
            completion(result);
        }
    });
    // WebKit copies the block for its asynchronous completion callback.
    unsafe {
        WKWebExtension::extensionWithResourceBaseURL_completionHandler(&resource_url, &callback, mtm);
    }
    Ok(())
}
