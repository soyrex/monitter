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
        ActiveTheme as _, Disableable, Icon, Sizable,
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
use std::{collections::HashSet, sync::mpsc, time::Duration};
use url::Url;

#[cfg(target_os = "macos")]
use wry::{WebViewBuilderExtDarwin, WebViewExtMacOS};

gpui_kit::actions!(
    native_browser,
    [
        FocusAddress,
        ReloadTab,
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
    NextTab,
    PreviousTab,
}

#[derive(Clone, Copy, Debug)]
enum BrowserTabEvent {
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
    Location(String),
    Title(String),
    Error(String),
}

fn validated_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text.trim())
        .map_err(|_| "Enter an absolute HTTP(S) URL or about:blank.".to_owned())?;
    if url.as_str() == "about:blank" {
        return Ok(url);
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Only HTTP(S) URLs without embedded credentials and about:blank are allowed.".into(),
        );
    }
    Ok(url)
}

fn create_native(
    window: &Window,
    events: mpsc::Sender<BrowserEvent>,
) -> Result<wry::WebView, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, events);
        return Err("This experimental browser currently supports macOS only.".into());
    }
    #[cfg(target_os = "macos")]
    {
        // WKWebView ignores filesystem data_directory; its isolated persistent
        // data-store identifier requires macOS 14. Never silently use default.
        let supported = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|version| version.trim().split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 14);
        if !supported {
            return Err("The isolated browser requires macOS 14 or later.".into());
        }
        let navigation = events.clone();
        let title = events.clone();
        wry::WebViewBuilder::new()
            .with_url("about:blank")
            .with_visible(false)
            .with_focused(false)
            .with_data_store_identifier(STORE_ID)
            .with_navigation_handler(move |location| match validated_url(&location) {
                Ok(_) => true,
                Err(error) => {
                    let _ = navigation.send(BrowserEvent::Error(error));
                    false
                }
            })
            .with_on_page_load_handler(move |_, location| {
                let _ = events.send(BrowserEvent::Location(location));
            })
            .with_document_title_changed_handler(move |value| {
                let _ = title.send(BrowserEvent::Title(value));
            })
            // Popup-to-tab routing is not wired yet; never open an untracked
            // native view outside the retained tab manager.
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .build_as_child(window)
            .map_err(|error| format!("Cannot create native browser: {error}"))
    }
}

pub(crate) struct BrowserTab {
    focus: FocusHandle,
    address: Entity<InputState>,
    webview: Option<Entity<WebView>>,
    #[cfg(target_os = "macos")]
    applied_corner_radius: Option<f64>,
    location: String,
    requested_url: String,
    actual_url: Option<String>,
    title: String,
    error: Option<String>,
    visible: bool,
    pending_address: Option<String>,
    dirty_address: bool,
    unloaded: bool,
    docked_mode: bool,
    dock_group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
    palette: Option<crate::design::Palette>,
    interface_font_ratio: f32,
    restoring_metadata: bool,
    ignore_initial_blank_event: bool,
    _enter: Subscription,
    _release: Subscription,
    _quit: Subscription,
    _events: Task<()>,
}

impl EventEmitter<BrowserTabEvent> for BrowserTab {}

impl Focusable for BrowserTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BrowserTab {
    pub(crate) fn set_dock_group(
        &mut self,
        group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
        cx: &mut Context<Self>,
    ) {
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
            let flipped: bool = msg_send![&*native, isFlipped];
            // CACornerMask: minX/minY = 1, maxX/minY = 2,
            // minX/maxY = 4, maxX/maxY = 8. Only the visual bottom is rounded:
            // the browser toolbar above this view owns the tile's top corners.
            let bottom_corners: usize = if flipped { 4 | 8 } else { 1 | 2 };
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            let _: () = msg_send![layer, setMaskedCorners: bottom_corners];
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: radius > 0.];
            let _: () = msg_send![class!(CATransaction), commit];
        }
        self.applied_corner_radius = Some(radius);
    }

    fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let focus = self.address.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = cx.new(|cx| InputState::new(window, cx).placeholder("https://example.com"));
        let focus = cx.focus_handle();
        let (events, receiver) = mpsc::channel();
        let (webview, error) = match create_native(window, events) {
            Ok(raw) => (Some(cx.new(|cx| WebView::new(raw, window, cx))), None),
            Err(error) => (None, Some(error)),
        };
        let enter =
            cx.subscribe_in(
                &address,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.navigate(cx),
                    InputEvent::Change if !this.restoring_metadata => {
                        this.dirty_address = true;
                        cx.emit(BrowserTabEvent::AddressEdited);
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
        let event_task = cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let pending: Vec<_> = receiver.try_iter().collect();
                if pane
                    .update(cx, |this, cx| {
                        if !pending.is_empty() {
                            for event in pending {
                                match event {
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
                                    BrowserEvent::Error(value) => this.error = Some(value),
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
        });
        Self {
            focus,
            address,
            webview,
            #[cfg(target_os = "macos")]
            applied_corner_radius: None,
            location: "about:blank".into(),
            requested_url: "about:blank".into(),
            actual_url: None,
            title: String::new(),
            error,
            visible: false,
            pending_address: None,
            dirty_address: false,
            unloaded: true,
            docked_mode: false,
            dock_group: None,
            palette: None,
            interface_font_ratio: 1.,
            restoring_metadata: false,
            ignore_initial_blank_event: true,
            _enter: enter,
            _release: release,
            _quit: quit,
            _events: event_task,
        }
    }

    fn navigate(&mut self, cx: &mut Context<Self>) {
        let result = validated_url(&self.address.read(cx).value()).and_then(|url| {
            if url.as_str().len() > MAX_BROWSER_URL_BYTES {
                return Err("Browser URL exceeds the 8192 byte limit.".into());
            }
            let view = self
                .webview
                .as_ref()
                .ok_or("Native browser is unavailable.")?;
            view.read(cx)
                .raw()
                .load_url(url.as_str())
                .map(|()| url.to_string())
                .map_err(|error| format!("Navigation failed: {error}"))
        });
        match result {
            Ok(requested_url) => {
                self.requested_url = requested_url.clone();
                self.actual_url = None;
                self.location = requested_url;
                self.unloaded = false;
                self.dirty_address = false;
                self.pending_address = None;
                self.error = None;
                if self.visible && !crate::session_lock::locked(cx) {
                    if let Some(view) = self.webview.clone() {
                        match view.update(cx, |view, _| {
                            view.raw()
                                .set_visible(true)
                                .map_err(|error| error.to_string())?;
                            view.show();
                            Ok::<(), String>(())
                        }) {
                            Ok(()) => {}
                            Err(error) => {
                                self.error = Some(format!("Cannot show native browser: {error}"));
                            }
                        }
                    }
                }
                // Loading the URL is accepted independently of whether the
                // native host could make the view visible afterward.
                cx.emit(BrowserTabEvent::MetadataChanged);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.unloaded {
            self.navigate(cx);
            return;
        }
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

    fn history_state(&self, cx: &App) -> (bool, bool) {
        #[cfg(target_os = "macos")]
        if let Some(view) = &self.webview {
            let native = view.read(cx).raw().webview();
            // GPUI entity access runs on the AppKit main thread. The retained
            // Wry handle owns this WKWebView throughout these native calls.
            return unsafe { (native.canGoBack(), native.canGoForward()) };
        }
        (false, false)
    }
    fn history_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if let Some(view) = &self.webview {
            let native = view.read(cx).raw().webview();
            // Use WKWebView history directly, never page-controlled JavaScript.
            unsafe {
                if forward && native.canGoForward() {
                    let _ = native.goForward();
                } else if !forward && native.canGoBack() {
                    let _ = native.goBack();
                }
            }
        }
        cx.notify();
    }

    /// Call before removing the pane from the render tree; preserves session.
    pub fn hide(&mut self, cx: &mut App) {
        self.visible = false;
        if let Some(view) = &self.webview {
            view.update(cx, |view, _| {
                let focus_error = view.raw().focus_parent().err();
                let visibility_error = view.raw().set_visible(false).err();
                view.hide();
                if let Some(error) = visibility_error.or(focus_error) {
                    self.error = Some(format!("Cannot hide native browser: {error}"));
                }
            });
        }
    }

    /// Restore a retained pane when returning to its native browser surface.
    pub fn show(&mut self, cx: &mut App) {
        if crate::session_lock::locked(cx) {self.hide(cx);return;}
        self.visible = true;
        if let Some(view) = &self.webview {
            if self.unloaded {
                view.update(cx, |view, _| match view.raw().set_visible(false) {
                    Ok(()) => view.hide(),
                    Err(error) => {
                        self.error = Some(format!("Cannot hide unloaded browser view: {error}"));
                    }
                });
            } else {
                view.update(cx, |view, _| match view.raw().set_visible(true) {
                    Ok(()) => view.show(),
                    Err(error) => self.error = Some(format!("Cannot show native browser: {error}")),
                });
            }
        }
    }

    /// Drop native ownership before window destruction; see module contract.
    pub fn close(&mut self, cx: &mut App) {
        self.hide(cx);
        self.webview.take();
    }

    fn restore_metadata(
        &mut self,
        metadata: &RestoredTabMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        #[cfg(target_os = "macos")]
        self.apply_native_tile_corners(cx);
        if !self.dirty_address && !self.address.read(cx).focus_handle(cx).is_focused(window) {
            if let Some(location) = self.pending_address.take() {
                self.address
                    .update(cx, |address, cx| address.set_value(location, window, cx));
                self.dirty_address = false;
            }
        }
        let (can_back, can_forward) = self.history_state(cx);
        let available = self.webview.is_some();
        let palette = self.palette.unwrap_or_else(|| {
            crate::design::palette(&serde_json::Value::Null, window.appearance())
        });
        let font_ratio = self.interface_font_ratio;
        let browser_mark = div().flex_none().h(px(29.)).flex().items_center()
            .child(Icon::new(IconName::Globe).with_size(px(16.)).flex_none());
        let browser_mark = if crate::design::tile_theme()
            == monitter_lib::experimental_desktop::NativeTileTheme::Tern
        {
            if let Some(group) = self.dock_group.as_ref().and_then(|group| group.upgrade()) {
                gpui_kit::component::dock::pane_drag_region(browser_mark, &group.read(cx).context(cx), cx)
                    .into_any_element()
            } else { browser_mark.into_any_element() }
        } else { browser_mark.into_any_element() };
        let address = Input::new(&self.address)
            .appearance(false)
            .bordered(false)
            .small()
            .aria_label("Browser address")
            .text_size(px(12. * font_ratio))
            .text_color(rgb(palette.ink))
            .w_full()
            .h(px(29.));
        let mut content = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .py(px(7.))
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .border_blend_mode(crate::surface_blend::separator_mode())
                    .when(crate::design::tile_theme()
                        == monitter_lib::experimental_desktop::NativeTileTheme::Tern
                        && crate::design::border_opacity() == 0, |this| this.border_b_0())
                    .bg(*cx.theme().tokens.tab_active)
                    .text_color(rgb(palette.muted))
                    .child(browser_mark)
                    .child(
                        Button::new("browser-back")
                            .text()
                            .compact()
                            .w(px(29.))
                            .h(px(29.))
                            .p_0()
                            .min_w(px(0.))
                            .flex_none()
                            .flex_shrink_0()
                            .accessibility_label("Back")
                            .disabled(!available || !can_back)
                            .child(
                                Icon::new(IconName::ChevronLeft)
                                    .with_size(px(16.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.history_step(false, cx))),
                    )
                    .child(
                        Button::new("browser-forward")
                            .text()
                            .compact()
                            .w(px(29.))
                            .h(px(29.))
                            .p_0()
                            .min_w(px(0.))
                            .flex_none()
                            .flex_shrink_0()
                            .accessibility_label("Forward")
                            .disabled(!available || !can_forward)
                            .child(
                                Icon::new(IconName::ChevronRight)
                                    .with_size(px(16.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.history_step(true, cx))),
                    )
                    .child(
                        Button::new("browser-reload")
                            .text()
                            .compact()
                            .w(px(29.))
                            .h(px(29.))
                            .p_0()
                            .min_w(px(0.))
                            .flex_none()
                            .flex_shrink_0()
                            .accessibility_label(if self.unloaded {
                                if self.requested_url == "about:blank" {
                                    "Reload browser tab"
                                } else {
                                    "Load saved page"
                                }
                            } else {
                                "Reload browser tab"
                            })
                            .disabled(
                                !available
                                    || (self.unloaded && self.requested_url == "about:blank")
                                    || (!self.unloaded && self.requested_url.is_empty()),
                            )
                            .child(
                                Icon::new(IconName::RefreshCw)
                                    .with_size(px(15.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(29.))
                            .flex()
                            .items_center()
                            .px(px(8.))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .rounded(px(5.))
                            .bg(crate::design::surface_color(palette.paper))
                            .child(address),
                    )
                    .child(
                        Button::new("browser-go")
                            .text()
                            .compact()
                            .w(px(29.))
                            .h(px(29.))
                            .p_0()
                            .min_w(px(0.))
                            .flex_none()
                            .flex_shrink_0()
                            .accessibility_label(if self.unloaded {
                                "Navigate and load saved page"
                            } else {
                                "Navigate"
                            })
                            .disabled(!available)
                            .child(
                                Icon::new(IconName::ArrowRight)
                                    .with_size(px(15.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.navigate(cx))),
                    )
                    .when(self.docked_mode, |bar| {
                        bar.child(
                            Button::new("browser-new-tab")
                                .text()
                                .compact()
                                .w(px(30.))
                                .h(px(29.))
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
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.emit(BrowserTabEvent::WorkspaceAction(
                                        BrowserWorkspaceAction::NewTab,
                                    ));
                                })),
                        )
                    }),
            );
        if self.docked_mode {
            content =
                content
                    .key_context("MonitterNativeBrowser")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &FocusAddress, window, cx| {
                        this.focus_address(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ReloadTab, _, cx| {
                        if this.visible {
                            this.reload(cx);
                        }
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
        if let Some(error) = &self.error {
            content = content.child(
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
                "Type a web address above, then choose Go to load a page."
            } else {
                "Enter an address or choose Load saved page to attach the native browser view."
            };
            content = content.child(
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
            content = content.child(
                div().flex_1().min_h(px(0.)).p(gpui::px(4.)).child(view.clone()),
            );
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
            KeyBinding::new("cmd-r", ReloadTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-t", NewTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("cmd-w", CloseTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("ctrl-tab", NextTab, Some("MonitterNativeBrowser")),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("MonitterNativeBrowser")),
        ]);
        let tab = cx.new(|cx| BrowserTab::new(window, cx));
        let observation = cx.observe(&tab, |_, _, cx| cx.notify());
        let metadata_subscription =
            cx.subscribe(&tab, |pane, _, event: &BrowserTabEvent, cx| match event {
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
                let mut tab = BrowserTab::new(window, cx);
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
        // Every restored WebView was created on about:blank; preserve the
        // pane's existing visibility without loading any saved external URL.
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
    fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if let Some(tab) = self.active_tab() {
            let address = tab.read(cx).address.clone();
            let focus = address.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
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
            .on_action(cx.listener(|this, _: &ReloadTab, _, cx| {
                if this.visible {
                    if let Some(tab) = this.active_tab() {
                        tab.update(cx, |tab, cx| tab.reload(cx));
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::{
        BrowserMetadataSnapshot, BrowserTabMetadata, MAX_BROWSER_TABS, bounded_title,
        is_stale_initial_blank, is_stale_initial_title, restore_plan, restore_revision_is_current,
        validated_url,
    };

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
