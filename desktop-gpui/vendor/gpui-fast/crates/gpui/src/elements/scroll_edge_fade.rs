//! Overflow alpha cues. Scroll geometry and rectangular hit bounds remain unchanged.
use crate::*;

pub struct ScrollEdgeFade {
    child: AnyElement,
    scroll: ScrollHandle,
    axis: Axis,
    distance: Pixels,
}
impl ScrollEdgeFade {
    pub fn new(child: impl IntoElement, scroll: ScrollHandle, axis: Axis, distance: Pixels) -> Self {
        Self { child: child.into_any_element(), scroll, axis, distance }
    }
}
impl IntoElement for ScrollEdgeFade {
    type Element = Self;
    fn into_element(self) -> Self { self }
}
fn hidden_edges(offset: Pixels, maximum: Pixels) -> (bool, bool) {
    let maximum = maximum.max(Pixels::ZERO);
    let traveled = (-offset).max(Pixels::ZERO).min(maximum);
    (traveled > px(0.5), maximum - traveled > px(0.5))
}
impl Element for ScrollEdgeFade {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        #[cfg(target_os = "macos")]
        {
            // Layout/prepaint can update the handle's overflow and offset. Mark
            // both possible ramps before the child decides whether to use a
            // scroll layer; paint then selects the actual hidden edges using
            // that updated geometry. Alpha fields do not alter hit bounds.
            let mask = match self.axis {
                Axis::Horizontal => ContentMask::horizontal_fade_edges(bounds, self.distance, self.distance),
                Axis::Vertical => ContentMask::vertical_fade_edges(bounds, self.distance, self.distance),
            };
            window.with_content_mask(Some(mask), |window| self.child.prepaint(window, cx));
        }
        #[cfg(not(target_os = "macos"))]
        self.child.prepaint(window, cx);
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        #[cfg(target_os = "macos")]
        {
            let offset = self.scroll.offset();
            let maximum = self.scroll.max_offset();
            let (start, end) = match self.axis {
                Axis::Horizontal => hidden_edges(offset.x, maximum.x),
                Axis::Vertical => hidden_edges(offset.y, maximum.y),
            };
            let start = if start { self.distance } else { Pixels::ZERO };
            let end = if end { self.distance } else { Pixels::ZERO };
            let mask = match self.axis {
                Axis::Horizontal => ContentMask::horizontal_fade_edges(bounds, start, end),
                Axis::Vertical => ContentMask::vertical_fade_edges(bounds, start, end),
            };
            window.with_content_mask(Some(mask), |window| self.child.paint(window, cx));
        }
        #[cfg(not(target_os = "macos"))]
        self.child.paint(window, cx);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn edge_visibility_follows_overflow_position_and_resize() {
        assert_eq!(hidden_edges(px(0.), px(0.)), (false, false));
        assert_eq!(hidden_edges(px(0.), px(100.)), (false, true));
        assert_eq!(hidden_edges(px(-50.), px(100.)), (true, true));
        assert_eq!(hidden_edges(px(-100.), px(100.)), (true, false));
        assert_eq!(hidden_edges(px(-150.), px(100.)), (true, false));
        assert_eq!(hidden_edges(px(-50.), px(0.)), (false, false));
    }
    #[::core::prelude::v1::test]
    #[cfg(target_os = "macos")]
    fn horizontal_mask_preserves_alpha_edges_through_nested_clips_and_scale() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(40.)));
        let mask = ContentMask::horizontal_fade_edges(bounds, px(0.), px(20.));
        let child = ContentMask::new(Bounds::new(point(px(40.), px(20.)), size(px(50.), px(40.))));
        let clipped = child.intersect(&mask).scale(2.);
        assert_eq!(clipped.fade_left, ScaledPixels(20.));
        assert_eq!(clipped.fade_right, ScaledPixels(220.));
        assert_eq!(clipped.fade_left_height, ScaledPixels(0.));
        assert_eq!(clipped.fade_right_height, ScaledPixels(40.));
    }
}
