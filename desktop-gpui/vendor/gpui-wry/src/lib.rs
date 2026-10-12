use std::{ops::Deref, rc::Rc};
#[cfg(feature = "performance-profiling")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "performance-profiling")]
use std::sync::OnceLock;

use wry::{
    Rect,
    dpi::{self, LogicalSize},
};

use gpui::{
    App, Bounds, ContentMask, DismissEvent, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hitbox, InteractiveElement, IntoElement, LayoutId, MouseDownEvent,
    ParentElement as _, Pixels, Render, Size, Style, Styled as _, Window, canvas, div,
};

#[cfg(feature = "performance-profiling")]
static BOUNDS_WRITES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static BOUNDS_TRACE_WRITES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static BOUNDS_TRACE_ENABLED: OnceLock<bool> = OnceLock::new();
#[cfg(feature = "performance-profiling")]
static SHOW_TRANSITIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "performance-profiling")]
static HIDE_TRANSITIONS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "performance-profiling")]
#[derive(Clone, Copy, Debug, Default)]
pub struct PerformanceCounts {
    pub bounds_writes: u64,
    pub show_transitions: u64,
    pub hide_transitions: u64,
}

#[cfg(feature = "performance-profiling")]
pub fn performance_counts() -> PerformanceCounts {
    PerformanceCounts {
        bounds_writes: BOUNDS_WRITES.load(Ordering::Relaxed),
        show_transitions: SHOW_TRANSITIONS.load(Ordering::Relaxed),
        hide_transitions: HIDE_TRANSITIONS.load(Ordering::Relaxed),
    }
}

#[cfg(feature = "performance-profiling")]
fn trace_bounds_enabled() -> bool {
    *BOUNDS_TRACE_ENABLED.get_or_init(|| {
        std::env::var_os("MONITTER_GPUI_PERF_TRACE_BOUNDS").as_deref()
            == Some(std::ffi::OsStr::new("1"))
    })
}

#[cfg(feature = "performance-profiling")]
fn numeric_bounds(bounds: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (
        bounds.origin.x.into(),
        bounds.origin.y.into(),
        bounds.size.width.into(),
        bounds.size.height.into(),
    )
}

/// An owned, UI-thread-local handle to the raw wry webview.
///
/// Cloning this handle prolongs the native webview's lifetime. Dropping the owning [`WebView`]
/// entity hides the child view, but final native destruction waits until all handle and frame
/// clones are dropped. All handles must be dropped before the parent window is destroyed.
#[derive(Clone)]
pub struct WebViewHandle(Rc<wry::WebView>);

impl WebViewHandle {
    /// Get the raw wry webview.
    pub fn raw(&self) -> &wry::WebView {
        &self.0
    }
}

/// A webview based on wry WebView.
///
/// [experimental]
pub struct WebView {
    focus_handle: FocusHandle,
    webview: Rc<wry::WebView>,
    visible: bool,
    bounds: Bounds<Pixels>,
    applied_native_bounds: Option<Bounds<Pixels>>,
}

impl Drop for WebView {
    fn drop(&mut self) {
        self.hide();
    }
}

impl WebView {
    /// Create a new WebView from a wry WebView.
    pub fn new(webview: wry::WebView, _: &mut Window, cx: &mut App) -> Self {
        let _ = webview.set_bounds(Rect::default());

        Self {
            focus_handle: cx.focus_handle(),
            visible: true,
            bounds: Bounds::default(),
            applied_native_bounds: None,
            webview: Rc::new(webview),
        }
    }

    /// Show the webview.
    pub fn show(&mut self) {
        if self.visible {
            return;
        }
        let _ = self.webview.set_visible(true);
        #[cfg(feature = "performance-profiling")]
        SHOW_TRANSITIONS.fetch_add(1, Ordering::Relaxed);
        self.visible = true;
    }

    /// Hide the webview.
    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        _ = self.webview.focus_parent();
        _ = self.webview.set_visible(false);
        #[cfg(feature = "performance-profiling")]
        HIDE_TRANSITIONS.fetch_add(1, Ordering::Relaxed);
        self.visible = false;
    }

    /// Get whether the webview is visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Get the current bounds of the webview.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    /// Go back in the webview history.
    pub fn back(&mut self) -> anyhow::Result<()> {
        Ok(self.webview.evaluate_script("history.back();")?)
    }

    /// Load a URL in the webview.
    pub fn load_url(&mut self, url: &str) {
        let _ = self.webview.load_url(url);
    }

    /// Get the raw wry webview.
    pub fn raw(&self) -> &wry::WebView {
        &self.webview
    }

    /// Get an owned, UI-thread-local handle to the raw wry webview.
    pub fn handle(&self) -> WebViewHandle {
        WebViewHandle(self.webview.clone())
    }
}

impl Deref for WebView {
    type Target = wry::WebView;

    fn deref(&self) -> &Self::Target {
        &self.webview
    }
}

impl Focusable for WebView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for WebView {}

impl Render for WebView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child({
                let view = cx.entity().clone();
                canvas(
                    move |bounds, _, cx| view.update(cx, |r, _| r.bounds = bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            })
            .child(WebViewElement::new(self.webview.clone(), view, window, cx))
    }
}

/// A webview element can display a wry webview.
pub struct WebViewElement {
    parent: Entity<WebView>,
    view: Rc<wry::WebView>,
}

impl WebViewElement {
    /// Create a new webview element from a wry WebView.
    pub fn new(
        view: Rc<wry::WebView>,
        parent: Entity<WebView>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self {
        Self { view, parent }
    }
}

impl IntoElement for WebViewElement {
    type Element = WebViewElement;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WebViewElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: Size::full(),
            flex_shrink: 1.,
            ..Default::default()
        };

        // If the parent view is no longer visible, we don't need to layout the webview
        let id = window.request_layout(style, [], cx);
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if !self.parent.read(cx).visible() {
            return None;
        }

        // Native frame writes can invalidate WKWebView even when GPUI merely
        // repaints another pane. Keep the retained native surface unchanged
        // until layout actually moves or resizes it. Cache only successful
        // writes, allowing a failed platform operation to be retried.
        if self.parent.read(cx).applied_native_bounds != Some(bounds) {
            #[cfg(feature = "performance-profiling")]
            BOUNDS_WRITES.fetch_add(1, Ordering::Relaxed);
            #[cfg(feature = "performance-profiling")]
            if trace_bounds_enabled() {
                let trace_index = BOUNDS_TRACE_WRITES.fetch_add(1, Ordering::Relaxed);
                if trace_index < 100 {
                    let old = self.parent.read(cx).applied_native_bounds.map(numeric_bounds);
                    eprintln!(
                        "GPUI PERF native_bounds_change #{} old={:?} new={:?}",
                        trace_index + 1,
                        old,
                        numeric_bounds(bounds),
                    );
                }
            }
            let result = self.view.set_bounds(Rect {
                size: dpi::Size::Logical(LogicalSize {
                    width: bounds.size.width.into(),
                    height: bounds.size.height.into(),
                }),
                position: dpi::Position::Logical(dpi::LogicalPosition::new(
                    bounds.origin.x.into(),
                    bounds.origin.y.into(),
                )),
            });

            if result.is_ok() {
                self.parent
                    .update(cx, |view, _| view.applied_native_bounds = Some(bounds));
            }
        }

        // Create a hitbox to handle mouse event
        Some(window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let bounds = hitbox.clone().map(|h| h.bounds).unwrap_or(bounds);
        window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
            let webview = self.view.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, _, _, _| {
                if !bounds.contains(&event.position) {
                    // Click white space to blur the input focus
                    let _ = webview.focus_parent();
                }
            });
        });
    }
}
