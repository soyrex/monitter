//! AppKit adapter for GPUI Fast's managed native composition surface.
//!
//! Wry still owns the `WKWebView` and its browser state. GPUI Fast owns the
//! intermediate AppKit surface so deferred GPUI draws can be stacked above it.

use std::{cell::Cell, rc::Rc};

use anyhow::{Context as _, Result};
use gpui::{AnyWindowHandle, AsyncApp, Bounds, Window, WindowCompositionSurface};
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use wry::WebViewExtMacOS as _;

pub(super) struct NativeWebViewSurface {
    surface: WindowCompositionSurface,
    original_parent: Retained<NSView>,
    scale_factor: Cell<f32>,
    window: AnyWindowHandle,
    cx: AsyncApp,
}

struct PreparedSurface {
    surface: WindowCompositionSurface,
    composition_parent: usize,
    destination: Retained<NSWindow>,
    original_parent: Retained<NSView>,
    scale_factor: f32,
    window: AnyWindowHandle,
    cx: AsyncApp,
}

impl PreparedSurface {
    fn new(window: &Window, cx: &gpui::App) -> Result<Self> {
        let destination = destination_window(window)?;
        // SAFETY: the raw window handle names the live AppKit window backing
        // `window`, and this code runs on GPUI's UI thread.
        let original_parent = destination
            .contentView()
            .context("destination window has no content view")?;
        let composition = window.enable_window_composition()?;
        let surface = composition.create_native_surface()?;
        let prepared = (|| {
            surface.platform_surface()?.set_visible(false)?;
            let handle = surface.platform_surface()?.platform_handle()?;
            let composition_parent = *handle
                .downcast::<usize>()
                .map_err(|_| anyhow::anyhow!("native surface is not an AppKit view"))?;
            anyhow::ensure!(composition_parent != 0, "native surface has no AppKit view");
            Ok(Self {
                surface: surface.clone(),
                composition_parent,
                destination,
                original_parent,
                scale_factor: window.scale_factor(),
                window: window.window_handle(),
                cx: cx.to_async(),
            })
        })();
        if prepared.is_err() {
            composition.remove_surface(surface.id())?;
        }
        prepared
    }

    fn attach(self, webview: &wry::WebView) -> NativeWebViewSurface {
        let view = webview.webview();
        // SAFETY: the composition surface owns this AppKit view for at least
        // as long as `surface`; `addSubview` transfers the WKWebView from its
        // current parent without changing Wry's ownership.
        unsafe {
            (&*(self.composition_parent as *const NSView)).addSubview(&view);
        }
        NativeWebViewSurface {
            surface: self.surface,
            original_parent: self.original_parent,
            scale_factor: Cell::new(self.scale_factor),
            window: self.window,
            cx: self.cx,
        }
    }

    fn discard(self) {
        remove_surface(self.surface, self.window, self.cx);
    }
}

impl NativeWebViewSurface {
    pub(super) fn new(webview: &wry::WebView, window: &Window, cx: &gpui::App) -> Result<Self> {
        Ok(PreparedSurface::new(window, cx)?.attach(webview))
    }

    /// Move Wry's retained page to another GPUI window and attach it to that
    /// window's composition tree. The destination surface is prepared first,
    /// so a composition failure leaves the current page untouched.
    pub(super) fn reparent(
        webview: &wry::WebView,
        window: &Window,
        cx: &gpui::App,
    ) -> Result<Self> {
        let prepared = PreparedSurface::new(window, cx)?;
        if let Err(error) = webview.reparent(Retained::as_ptr(&prepared.destination).cast_mut()) {
            prepared.discard();
            return Err(error.into());
        }
        Ok(prepared.attach(webview))
    }

    pub(super) fn set_scale_factor(&self, scale_factor: f32) {
        self.scale_factor.set(scale_factor);
    }

    /// Keep the full DOM viewport while the managed surface clips to GPUI's
    /// inherited content mask.
    pub(super) fn set_geometry(
        &self,
        webview: &wry::WebView,
        document: Bounds<gpui::Pixels>,
        visible: Bounds<gpui::Pixels>,
    ) -> Result<()> {
        let scale = self.scale_factor.get();
        self.surface
            .platform_surface()?
            .set_bounds(visible.to_device_pixels(scale))?;
        let offset = document.origin - visible.origin;
        webview.webview().setFrame(NSRect::new(
            NSPoint::new(
                f64::from(offset.x),
                f64::from(visible.size.height - offset.y - document.size.height),
            ),
            NSSize::new(
                f64::from(document.size.width),
                f64::from(document.size.height),
            ),
        ));
        Ok(())
    }

    pub(super) fn set_visible(&self, visible: bool) -> Result<()> {
        self.surface.platform_surface()?.set_visible(visible)
    }

    /// Remove a surface after its page has already moved to another window.
    pub(super) fn remove(self) {
        remove_surface(self.surface, self.window, self.cx);
    }

    /// Restore Wry's child to the current window before removing its managed
    /// surface. This keeps native teardown on a normal AppKit parent.
    pub(super) fn release(self, webview: Rc<wry::WebView>) {
        self.cx
            .clone()
            .spawn(async move |cx| {
                self.original_parent.addSubview(&webview.webview());
                remove_surface_now(self.surface, self.window, cx);
            })
            .detach();
    }
}

fn destination_window(window: &Window) -> Result<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("failed to get AppKit window handle: {error:?}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("WebView composition requires an AppKit window");
    };
    // SAFETY: GPUI's AppKit handle is live for the duration of this call.
    unsafe {
        let view = &*handle.ns_view.as_ptr().cast::<NSView>();
        view.window().context("GPUI view has no AppKit window")
    }
}

fn remove_surface(surface: WindowCompositionSurface, window: AnyWindowHandle, cx: AsyncApp) {
    cx.clone()
        .spawn(async move |cx| remove_surface_now(surface, window, cx))
        .detach();
}

fn remove_surface_now(
    surface: WindowCompositionSurface,
    window: AnyWindowHandle,
    cx: &mut gpui::AsyncApp,
) {
    let result = window.update(cx, |_, window, _| {
        window
            .enable_window_composition()?
            .remove_surface(surface.id())
    });
    if let Ok(Err(error)) = result {
        log::warn!("cannot release WebView composition surface: {error:#}");
    }
}

#[cfg(test)]
fn logical_bounds(
    rect: wry::Rect,
    scale: f32,
) -> (wry::dpi::LogicalPosition<f64>, wry::dpi::LogicalSize<f64>) {
    (
        rect.position.to_logical(f64::from(scale)),
        rect.size.to_logical(f64::from(scale)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wry::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

    #[test]
    fn retina_physical_bounds_keep_the_child_inside_its_container() {
        let (position, extent) = logical_bounds(
            wry::Rect {
                position: PhysicalPosition::new(400, 160).into(),
                size: PhysicalSize::new(1280, 960).into(),
            },
            2.,
        );
        assert_eq!(position, LogicalPosition::new(200., 80.));
        assert_eq!(extent, LogicalSize::new(640., 480.));
    }

    #[test]
    fn offscreen_logical_loading_bounds_do_not_shift_on_retina() {
        let rect = wry::Rect {
            position: LogicalPosition::new(-20000., -20000.).into(),
            size: LogicalSize::new(640., 480.).into(),
        };
        for scale in [1., 2.] {
            let (position, extent) = logical_bounds(rect, scale);
            assert_eq!(position, LogicalPosition::new(-20000., -20000.));
            assert_eq!(extent, LogicalSize::new(640., 480.));
        }
    }
}
