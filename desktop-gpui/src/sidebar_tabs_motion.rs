//! Retained, body-only sidebar tab motion and deliberate horizontal wheel gestures.
use gpui_kit::{prelude::*, *};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SidebarTabsMotion {
    index: Option<usize>,
    generation: u64,
    animate: bool,
    direction: f32,
    last_wheel: Option<Instant>,
    horizontal: f32,
    latched: bool,
}
impl SidebarTabsMotion {
    pub fn observe(&mut self, index: usize) {
        if let Some(previous) = self.index {
            if previous != index {
                self.direction = if index > previous { 1. } else { -1. };
                self.generation += 1;
                self.animate = true;
            }
        }
        self.index = Some(index);
    }
    /// Restore the selected body without treating it as a user tab gesture.
    /// Preserve generation so the next gesture gets a fresh animation identity.
    pub fn settle(&mut self, index: usize) {
        self.index = Some(index);
        self.animate = false;
        self.horizontal = 0.;
        self.last_wheel = None;
        self.latched = false;
    }
    /// One change per wheel sequence, including momentum. Vertical scrolling
    /// never contributes to the threshold and is never consumed by this state.
    pub fn wheel(&mut self, x: f32, y: f32, count: usize, now: Instant) -> Option<usize> {
        if self
            .last_wheel
            .is_none_or(|last| now.duration_since(last) > Duration::from_millis(350))
        {
            self.horizontal = 0.;
            self.latched = false;
        }
        self.last_wheel = Some(now);
        if x.abs() <= y.abs() * 1.5 || x.abs() < 0.5 {
            return None;
        }
        if self.latched {
            return None;
        }
        self.horizontal += x;
        if self.horizontal.abs() < 40. {
            return None;
        }
        self.latched = true;
        let index = self.index?;
        let next = if self.horizontal < 0. {
            (index + 1).min(count.saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        (next != index).then_some(next)
    }
    pub fn body(&self, id: &str, body: AnyElement, reduced: bool, fill: bool) -> AnyElement {
        let body = div()
            .id(SharedString::from(format!("{id}-body-motion")))
            .relative()
            .when(fill, |body| body.flex_1().min_h_0().flex().flex_col().overflow_hidden())
            .min_w_0()
            .w_full()
            .child(body);
        if reduced || !self.animate {
            return body.into_any_element();
        }
        let direction = self.direction;
        body.with_animation(
            SharedString::from(format!("sidebar-tab-{}", self.generation)),
            Animation::new(Duration::from_millis(160)),
            move |body, progress| body.left(px(direction * 24. * (1. - progress).powi(2))),
        )
        .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn wheel_requires_horizontal_intent_and_latches_momentum() {
        let now = Instant::now();
        let mut state = SidebarTabsMotion::default();
        state.observe(1);
        assert_eq!(state.wheel(-80., 100., 3, now), None);
        assert_eq!(state.wheel(-20., 1., 3, now), None);
        assert_eq!(state.wheel(-25., 1., 3, now), Some(2));
        state.observe(2);
        assert_eq!(state.wheel(100., 0., 3, now), None);
        assert_eq!(
            state.wheel(45., 0., 3, now + Duration::from_millis(351)),
            Some(1)
        );
    }
    #[::core::prelude::v1::test]
    fn direction_tracks_tab_order_and_stops_at_edges() {
        let mut state = SidebarTabsMotion::default();
        state.observe(0);
        assert_eq!(state.generation, 0);
        assert_eq!(state.wheel(50., 0., 3, Instant::now()), None);
        state.observe(2);
        assert_eq!(state.direction, 1.);
        state.observe(1);
        assert_eq!(state.direction, -1.);
        let generation = state.generation;
        state.observe(1);
        assert_eq!(state.generation, generation);
    }
    #[::core::prelude::v1::test]
    fn restoring_tab_cancels_motion_and_next_gesture_uses_a_new_identity() {
        let mut state = SidebarTabsMotion::default();
        state.observe(0);
        state.observe(1);
        assert!(state.animate);
        let generation = state.generation;
        state.settle(3);
        state.observe(3);
        assert!(!state.animate);
        assert_eq!(state.generation, generation);
        state.observe(2);
        assert!(state.animate);
        assert_eq!(state.generation, generation + 1);
        assert_eq!(state.direction, -1.);
    }
    #[::core::prelude::v1::test]
    fn restoring_tab_discards_outgoing_wheel_momentum() {
        let mut state = SidebarTabsMotion::default();
        let now = Instant::now();
        state.observe(0);
        assert_eq!(state.wheel(-50., 0., 4, now), Some(1));
        state.settle(2);
        assert_eq!(state.wheel(-20., 0., 4, now), None);
        assert_eq!(state.wheel(-25., 0., 4, now), Some(3));
    }
}
