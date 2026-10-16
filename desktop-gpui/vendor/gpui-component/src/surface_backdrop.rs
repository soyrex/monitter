//! Shared, retained scene backdrop for transient popup surfaces.
//!
//! Captures are requested only when a popup lease opens. The image is shared
//! across simultaneously visible popups and released after the last lease.
use gpui::{
    canvas, fill, point, px, App, Bounds, Corners, ElementId, Pixels, RenderImage, Styled as _,
    Window,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
struct PopupGeometry {
    bounds: Option<Bounds<Pixels>>,
}
#[derive(Default)]
struct PopupGeometryRegistry(
    HashMap<gpui::WindowId, Vec<(gpui::GlobalElementId, gpui::WeakEntity<PopupGeometry>)>>,
);
impl gpui::Global for PopupGeometryRegistry {}

/// Actual positioned rectangles of live popovers and all menu levels. Keyed
/// element state survives cached rendering and is collected with its popup.
/// Query after deferred prepaint, before committing native child-view masks.
pub fn popup_bounds(window: &Window, cx: &App) -> Vec<Bounds<Pixels>> {
    cx.try_global::<PopupGeometryRegistry>()
        .and_then(|registry| registry.0.get(&window.window_handle().window_id()))
        .into_iter()
        .flatten()
        .filter(|(id, _)| window.is_element_state_current(id))
        .filter_map(|(_, lease)| {
            lease
                .upgrade()
                .and_then(|geometry| geometry.read(cx).bounds)
        })
        .collect()
}

fn register_popup_geometry(
    id: gpui::GlobalElementId,
    geometry: &gpui::Entity<PopupGeometry>,
    window: &Window,
    cx: &mut App,
) {
    if !cx.has_global::<PopupGeometryRegistry>() {
        cx.set_global(PopupGeometryRegistry::default());
    }
    let registry = cx.global_mut::<PopupGeometryRegistry>();
    registry.0.retain(|_, leases| {
        leases.retain(|(_, lease)| lease.upgrade().is_some());
        !leases.is_empty()
    });
    registry
        .0
        .entry(window.window_handle().window_id())
        .or_default()
        .push((id, geometry.downgrade()));
}

use crate::root::WindowState;
use crate::ActiveTheme as _;

pub type PopupBackdropFactory =
    std::rc::Rc<dyn Fn(&Window, &App) -> Option<gpui::Task<Option<Arc<RenderImage>>>> + 'static>;

pub fn set_active(owner: ElementId, active: bool, window: &mut Window, cx: &mut App) {
    let Some(state) = WindowState::entity(window, cx) else {
        return;
    };
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
    WindowState::entity(window, cx)?
        .read(cx)
        .popup_focus_origin
        .as_ref()?
        .upgrade()
}

/// Paint the captured workspace scene into the popup bounds, with a light
/// theme tint above it. The image is scaled against the full viewport, so this
/// paints the matching crop without allocating a per-popup crop texture.
pub fn paint_popup(bounds: Bounds<Pixels>, radius: Pixels, window: &mut Window, cx: &App) {
    let Some(state) = WindowState::entity(window, cx) else {
        return;
    };
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
    PopupBackdropLayer {
        radius,
        backdrop: true,
    }
}

/// Geometry-only registration for tooltips and other opaque floating surfaces.
pub fn popup_geometry_layer() -> impl gpui::IntoElement {
    PopupBackdropLayer {
        radius: px(0.),
        backdrop: false,
    }
}

#[derive(gpui::IntoElement)]
struct PopupBackdropLayer {
    radius: Pixels,
    backdrop: bool,
}
impl gpui::RenderOnce for PopupBackdropLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl gpui::IntoElement {
        let geometry =
            window.use_keyed_state("popup-native-geometry", cx, |_, _| PopupGeometry::default());
        if geometry.read(cx).bounds.is_none() {
            let id = window.with_global_id("popup-native-geometry".into(), |id, _| id.clone());
            register_popup_geometry(id, &geometry, window, cx);
        }
        canvas(
            move |bounds, _, cx| {
                geometry.update(cx, |geometry, _| geometry.bounds = Some(bounds));
            },
            move |bounds, _, window, cx| {
                if self.backdrop {
                    paint_popup(bounds, self.radius, window, cx);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full()
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::{popup_backdrop_layer, popup_bounds};
    use gpui::{
        div, px, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window,
    };

    struct GeometryFixture {
        visible: bool,
    }
    impl Render for GeometryFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let mut root = div().size(px(200.));
            if self.visible {
                root = root.child(
                    div()
                        .relative()
                        .w(px(80.))
                        .h(px(40.))
                        .child(popup_backdrop_layer(px(4.))),
                );
            }
            root
        }
    }

    #[gpui::test]
    fn popup_geometry_tracks_actual_bounds_and_disappears_with_its_element(
        cx: &mut TestAppContext,
    ) {
        let (view, visual) = cx.add_window_view(|_, _| GeometryFixture { visible: true });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            let bounds = popup_bounds(window, cx);
            assert_eq!(bounds.len(), 1);
            assert_eq!(bounds[0].size, gpui::size(px(80.), px(40.)));
            window.draw(cx).clear(cx);
            assert_eq!(popup_bounds(window, cx).len(), 1);
        });
        view.update(visual, |view, cx| {
            view.visible = false;
            cx.notify();
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(popup_bounds(window, cx).is_empty(), "closed popup has no stale native mask in its first closing frame");
        });
    }
}
