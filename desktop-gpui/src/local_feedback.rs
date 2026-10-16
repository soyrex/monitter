//! Paint-driven, retained one-shot feedback. Only the painted control is invalidated.
use gpui_kit::{prelude::*, *};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct FeedbackClock {
    pub(crate) painted: bool,
    allowed: bool,
    generation: u64,
    transition: Option<(Instant, Duration)>,
    pending: Option<u64>,
}
pub(crate) type Clock = Rc<RefCell<FeedbackClock>>;
impl FeedbackClock {
    pub(crate) fn new() -> Clock {
        Rc::new(RefCell::new(Self {
            allowed: true,
            ..Self::default()
        }))
    }
    pub(crate) fn start(&mut self, duration: Duration) {
        self.generation = self.generation.wrapping_add(1);
        self.transition =
            crate::native_motion::motion_allowed().then_some((Instant::now(), duration));
    }
    pub(crate) fn set_visible(&mut self, allowed: bool) {
        self.allowed = allowed;
        if !allowed {
            self.painted = false;
            self.finish();
        }
    }
    pub(crate) fn finish(&mut self) {
        self.transition = None;
        self.generation = self.generation.wrapping_add(1);
    }
    fn deliver_frame(&mut self, generation: u64, motion: bool) -> bool {
        if self.pending == Some(generation) {
            self.pending = None;
        }
        let active = self.generation == generation
            && self.painted
            && self.allowed
            && self.transition.is_some();
        // A policy change still needs one final local paint, then no callbacks.
        if active && !motion {
            self.finish();
        }
        active
    }
    pub(crate) fn progress(&mut self, reduced: bool) -> f32 {
        if reduced || !self.allowed {
            self.finish();
            return 1.;
        }
        let Some((start, duration)) = self.transition else {
            return 1.;
        };
        let t = (start.elapsed().as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.);
        if t >= 1. {
            self.transition = None;
        }
        1. - (1. - t).powi(2)
    }
}

pub(crate) fn painted(child: impl IntoElement, clock: Clock) -> AnyElement {
    FeedbackPaint {
        child: child.into_any_element(),
        clock,
    }
    .into_any_element()
}
struct FeedbackPaint {
    child: AnyElement,
    clock: Clock,
}
impl IntoElement for FeedbackPaint {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for FeedbackPaint {
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
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
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
        self.child.paint(window, cx);
        let mut clock = self.clock.borrow_mut();
        clock.painted = clock.allowed && bounds.intersects(&window.content_mask().bounds);
        if !clock.painted || cx.reduce_motion() || !crate::native_motion::motion_allowed() {
            let settle = clock.transition.is_some();
            clock.finish();
            // Clear a cached intermediate pose even when scrolling clips the surface.
            if settle {
                cx.notify(window.current_view());
            }
            return;
        }
        if clock.transition.is_none() || clock.pending == Some(clock.generation) {
            return;
        }
        let generation = clock.generation;
        clock.pending = Some(generation);
        let clock = self.clock.clone();
        let owner = window.current_view();
        window.on_next_frame(move |_, cx| {
            let mut clock = clock.borrow_mut();
            if clock.deliver_frame(
                generation,
                !cx.reduce_motion() && crate::native_motion::motion_allowed(),
            ) {
                cx.notify(owner);
            }
        });
    }
}

#[derive(Default)]
pub(crate) struct Confirmation {
    generation: u64,
    confirmed: bool,
}
impl Confirmation {
    pub(crate) fn renew(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.confirmed = true;
        self.generation
    }
    pub(crate) fn expire(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.confirmed {
            return false;
        }
        self.confirmed = false;
        true
    }
    pub(crate) fn confirmed(&self) -> bool {
        self.confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn retarget_fences_callbacks_without_losing_the_new_frame() {
        let mut clock = FeedbackClock {
            allowed: true,
            painted: true,
            generation: 2,
            transition: Some((Instant::now(), Duration::from_secs(1))),
            pending: Some(2),
        };
        assert!(!clock.deliver_frame(1, true));
        assert_eq!(clock.pending, Some(2));
        assert!(clock.deliver_frame(2, true));
        assert_eq!(clock.pending, None);
        clock.pending = Some(2);
        clock.set_visible(false);
        assert!(!clock.deliver_frame(2, true));
        assert!(clock.transition.is_none());
        assert!(!clock.deliver_frame(clock.generation, false));
    }
    #[::core::prelude::v1::test]
    fn reduction_delivers_one_final_paint_then_stops() {
        let mut clock = FeedbackClock {
            allowed: true,
            painted: true,
            generation: 1,
            transition: Some((Instant::now(), Duration::from_secs(1))),
            pending: Some(1),
        };
        assert!(clock.deliver_frame(1, false));
        assert!(clock.transition.is_none());
        assert!(!clock.deliver_frame(1, false));
        assert_eq!(clock.progress(false), 1.);
    }
    #[::core::prelude::v1::test]
    fn expired_entrance_stays_final_after_rerender() {
        let mut clock = FeedbackClock {
            allowed: true,
            transition: Some((
                Instant::now() - Duration::from_secs(1),
                Duration::from_millis(120),
            )),
            ..FeedbackClock::default()
        };
        assert_eq!(clock.progress(false), 1.);
        assert!(clock.transition.is_none());
        assert_eq!(clock.progress(false), 1.);
    }
    #[::core::prelude::v1::test]
    fn newer_confirmation_fences_stale_expiry() {
        let mut state = Confirmation::default();
        let first = state.renew();
        let second = state.renew();
        assert!(!state.expire(first));
        assert!(state.confirmed());
        assert!(state.expire(second));
        assert!(!state.expire(second));
    }
    #[::core::prelude::v1::test]
    fn hidden_and_reduced_feedback_never_replays() {
        let mut clock = FeedbackClock::default();
        clock.allowed = true;
        clock.transition = Some((Instant::now(), Duration::from_secs(1)));
        clock.set_visible(false);
        clock.set_visible(true);
        assert_eq!(clock.progress(false), 1.);
        clock.transition = Some((Instant::now(), Duration::from_secs(1)));
        assert_eq!(clock.progress(true), 1.);
        assert_eq!(clock.progress(false), 1.);
    }
}
