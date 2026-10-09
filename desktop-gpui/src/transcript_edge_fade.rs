//! Paint-only alpha mask for the virtual transcript. Layout, selection and
//! scrolling keep their native geometry; no capture or extra frame is needed.
use gpui_kit::*;

pub(crate) struct TranscriptEdgeFade {
    child: AnyElement,
    top_height: Pixels,
    bottom_height: Pixels,
    list: Option<ListState>,
}
impl TranscriptEdgeFade {
    pub(crate) fn new(child: impl IntoElement, top_height: Pixels, bottom_height: Pixels) -> Self {
        Self {
            child: child.into_any_element(),
            top_height,
            bottom_height,
            list: None,
        }
    }
    /// Virtual lists expose their measured offset after prepaint, so fades
    /// follow scrolling without another parent render or layout pass.
    pub(crate) fn for_list(child: impl IntoElement, list: ListState, distance: Pixels) -> Self {
        Self { child: child.into_any_element(), top_height: distance, bottom_height: distance, list: Some(list) }
    }

}
impl IntoElement for TranscriptEdgeFade {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TranscriptEdgeFade {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
            self.child.prepaint(window, cx)
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(target_os = "macos")]
        let (top_height, bottom_height) = if let Some(list) = &self.list {
            let maximum = list.max_offset_for_scrollbar().y.max(Pixels::ZERO);
            let traveled = (-list.scroll_px_offset_for_scrollbar().y).max(Pixels::ZERO).min(maximum);
            (if traveled > px(0.5) { self.top_height } else { Pixels::ZERO },
             if maximum - traveled > px(0.5) { self.bottom_height } else { Pixels::ZERO })
        } else { (self.top_height, self.bottom_height) };
        #[cfg(target_os = "macos")]
        window.with_content_mask(
            Some(ContentMask::vertical_fade_edges(
                bounds,
                top_height,
                bottom_height,
            )),
            |window| self.child.paint(window, cx),
        );
        #[cfg(not(target_os = "macos"))]
        self.child.paint(window, cx);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn independent_edge_heights_survive_nested_clips_and_scale() {
        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(500.)));
        let mask = ContentMask::vertical_fade_edges(viewport, px(80.), px(24.));
        let row = ContentMask::new(Bounds::new(
            point(px(0.), px(40.)),
            size(px(400.), px(100.)),
        ));
        let scaled = row.intersect(&mask).scale(2.);
        assert_eq!(scaled.fade_height, ScaledPixels(160.));
        assert_eq!(scaled.fade_bottom_height, ScaledPixels(48.));
        assert_eq!(scaled.fade_top, ScaledPixels(0.));
        assert_eq!(scaled.fade_bottom, ScaledPixels(1000.));
        let small = ContentMask::vertical_fade_edges(viewport, px(1000.), px(24.));
        assert_eq!(small.fade_height, px(250.));
        assert_eq!(small.fade_bottom_height, px(24.));
    }

    #[::core::prelude::v1::test]
    fn content_mask_fade_survives_scale_and_nested_clips() {
        let viewport = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(200.)));
        let fade = ContentMask::vertical_fade(viewport, px(24.));
        let row = ContentMask::new(Bounds::new(point(px(12.), px(28.)), size(px(90.), px(40.))));
        let clipped = row.intersect(&fade);
        assert_eq!(clipped.bounds, row.bounds);
        assert_eq!(clipped.fade_top, px(20.));
        assert_eq!(clipped.fade_bottom, px(220.));
        assert_eq!(clipped.fade_height, px(24.));
        let scaled = clipped.scale(2.);
        assert_eq!(scaled.bounds, row.bounds.scale(2.));
        assert_eq!(scaled.fade_top, ScaledPixels(40.));
        assert_eq!(scaled.fade_bottom, ScaledPixels(440.));
        assert_eq!(scaled.fade_height, ScaledPixels(48.));
        // A deeper fade cannot reset the enclosing viewport's ramp.
        assert_eq!(
            ContentMask::vertical_fade(row.bounds, px(8.)).intersect(&fade),
            clipped
        );
    }

    #[::core::prelude::v1::test]
    #[cfg(target_os = "macos")]
    fn content_mask_fade_disabled_and_height_limits() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(40.), px(100.)));
        let mask = ContentMask::new(bounds);
        assert_eq!(mask.fade_height, Pixels::ZERO);
        assert_eq!(mask.scale(2.).fade_height, ScaledPixels(0.));
        for height in [px(0.), px(-1.), px(f32::NAN), px(f32::INFINITY)] {
            assert_eq!(
                ContentMask::vertical_fade(bounds, height).fade_height,
                Pixels::ZERO
            );
        }
        assert_eq!(
            ContentMask::vertical_fade(bounds, px(1000.)).fade_height,
            px(50.)
        );
        assert_eq!(mask.intersect(&ContentMask::new(bounds)), mask);
    }
}
