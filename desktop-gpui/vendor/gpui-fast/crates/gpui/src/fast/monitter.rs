//! Downstream Monitter clip compatibility for retained paint and scroll layers.
use crate::{ContentMask, Pixels, Point, ScaledPixels};

/// Extended viewport effects stay on the direct scene path. Their anchors
/// belong to the viewport, while scroll tiles belong to moving content.
pub(crate) fn rectangular(mask: ContentMask<Pixels>) -> bool {
    mask == ContentMask::new(mask.bounds)
}

pub(crate) fn translate_mask(mask: &mut ContentMask<ScaledPixels>, delta: Point<ScaledPixels>) {
    mask.bounds.origin += delta;
    #[cfg(target_os = "macos")]
    {
        mask.rounded_bounds.origin += delta;
        mask.fade_top += delta.y;
        mask.fade_bottom += delta.y;
        mask.fade_left += delta.x;
        mask.fade_right += delta.x;
    }
}

impl crate::Window {
    /// Observe an element's paint on this frame and whenever that paint is
    /// replayed from a retained subtree. Call this during the paint phase.
    ///
    /// The observer is owned by the painted frame range: it is discarded when
    /// that range is no longer drawn, including hidden or removed elements.
    /// Use this for paint-presence bookkeeping, not drawing or invalidation.
    pub fn observe_paint(&mut self, observer: impl Fn() + 'static) {
        self.invalidator.debug_assert_paint();
        observer();
        self.next_frame
            .fast_paint_observers
            .push(std::rc::Rc::new(observer));
    }
}

pub(crate) fn reuse_paint_observers(
    window: &mut crate::Window,
    range: &std::ops::Range<crate::window::PaintIndex>,
) {
    let observers = &window.rendered_frame.fast_paint_observers
        [range.start.fast.paint_observers..range.end.fast.paint_observers];
    for observer in observers {
        observer();
    }
    window
        .next_frame
        .fast_paint_observers
        .extend(observers.iter().cloned());
}
