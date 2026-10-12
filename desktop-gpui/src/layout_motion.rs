//! Finite, interruptible geometry interpolation. Owners schedule only live frames.
use std::time::{Duration, Instant};

pub const LAYOUT_DURATION: Duration = Duration::from_millis(200);

#[derive(Default)]
pub(crate) struct WidthMotion {
    transition: Option<WidthTransition>,
}
struct WidthTransition {
    from: f32,
    target: f32,
    started: Instant,
}
impl WidthMotion {
    pub fn active(&self) -> bool {
        self.transition.is_some()
    }
    pub fn target(&self) -> Option<f32> {
        self.transition.as_ref().map(|t| t.target)
    }
    /// Sample before retargeting so reversal never jumps back to an endpoint.
    pub fn start(&mut self, displayed: f32, target: f32, now: Instant, allowed: bool) {
        let from = displayed;
        self.transition = (allowed && (target - from).abs() >= 0.5).then_some(WidthTransition {
            from,
            target,
            started: now,
        });
    }
    pub fn sample(&mut self, now: Instant) -> Option<f32> {
        let t = self.transition.as_ref()?;
        let progress = layout_progress(now.saturating_duration_since(t.started));
        let value = t.from + (t.target - t.from) * progress;
        if progress >= 1. {
            self.transition = None;
        }
        Some(value)
    }
    pub fn cancel(&mut self) {
        self.transition = None;
    }
}
pub(crate) fn layout_progress(elapsed: Duration) -> f32 {
    let linear = (elapsed.as_secs_f32() / LAYOUT_DURATION.as_secs_f32()).clamp(0., 1.);
    1. - (1. - linear).powi(3)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn reversal_starts_at_displayed_width_and_completes_once() {
        let now = Instant::now();
        let mut motion = WidthMotion::default();
        motion.start(252., 48., now, true);
        let middle = motion.sample(now + Duration::from_millis(80)).unwrap();
        motion.start(middle, 252., now + Duration::from_millis(80), true);
        assert_eq!(motion.sample(now + Duration::from_millis(80)), Some(middle));
        assert_eq!(motion.sample(now + Duration::from_millis(280)), Some(252.));
        assert!(!motion.active());
        assert_eq!(motion.sample(now + Duration::from_millis(300)), None);
    }
    #[::core::prelude::v1::test]
    fn off_noop_and_drag_cancel_have_no_live_frames() {
        let mut motion = WidthMotion::default();
        let now = Instant::now();
        motion.start(252., 0., now, false);
        assert!(!motion.active());
        motion.start(48., 48., now, true);
        assert!(!motion.active());
        motion.start(252., 0., now, true);
        motion.cancel();
        assert_eq!(motion.sample(now), None);
    }
    #[::core::prelude::v1::test]
    fn easing_is_monotonic_and_bounded() {
        let mut previous = 0.;
        for millis in 0..250 {
            let value = layout_progress(Duration::from_millis(millis));
            assert!((previous..=1.).contains(&value));
            previous = value;
        }
        assert_eq!(previous, 1.);
    }
}
