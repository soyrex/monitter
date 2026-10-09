//! Shared, retained scene backdrop for transient popup surfaces.
//!
//! Captures are requested only when a popup lease opens. The image is shared
//! across simultaneously visible popups and released after the last lease.
use gpui::{
    App, Bounds, Corners, ElementId, Pixels, RenderImage, Styled as _, Window, canvas, fill, point, px,
};
use std::sync::Arc;

use crate::root::WindowState;
use crate::ActiveTheme as _;

pub type PopupBackdropFactory =
    std::rc::Rc<dyn Fn(&Window, &App) -> Option<gpui::Task<Option<Arc<RenderImage>>>> + 'static>;

pub fn set_active(owner: ElementId, active: bool, window: &mut Window, cx: &mut App) {
    let Some(state) = WindowState::entity(window, cx) else { return; };
    state.update(cx, |state, cx| {
        if active {
            state.acquire_popup_backdrop(owner, window, cx);
        } else {
            state.release_popup_backdrop(&owner, window, cx);
        }
    });
}

/// The original pane keeps its visual activity while a popup owns keyboard
/// focus. Nested popups share the first owner's focus until the last closes.
pub fn popup_focus_origin(window: &Window, cx: &App) -> Option<gpui::FocusHandle> {
    WindowState::entity(window, cx)?.read(cx).popup_focus_origin.as_ref()?.upgrade()
}

/// Paint the captured workspace scene into the popup bounds, with a light
/// theme tint above it. The image is scaled against the full viewport, so this
/// paints the matching crop without allocating a per-popup crop texture.
pub fn paint_popup(bounds: Bounds<Pixels>, radius: Pixels, window: &mut Window, cx: &App) {
    let Some(state) = WindowState::entity(window, cx) else { return; };
    let Some(image) = state.read(cx).popup_backdrop_image.clone() else {
        return;
    };
    let viewport = window.viewport_size();
    let image_bounds = Bounds::new(point(px(0.), px(0.)), viewport);
    let corners = Corners::all(radius);
    let _ = window.paint_image(bounds, image_bounds, corners.clone(), image, 0, false);
    window.paint_quad(fill(bounds, cx.theme().popover).corner_radii(corners));
}

pub fn popup_backdrop_layer(radius: Pixels) -> impl gpui::IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            paint_popup(bounds, radius, window, cx);
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
