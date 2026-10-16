//! Short, retained paint transitions. These entities own their frame requests;
//! their containing workspace/chat does not observe animation notifications.
use gpui_kit::{assets::IconName, component::{Icon, Sizable}, prelude::*, *};
use std::time::{Duration, Instant};
use crate::chat::feedback::{self, Clock, FeedbackClock};

const DURATION: Duration = Duration::from_millis(140);

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    target: f32,
    started: Option<Instant>,
}
impl Tween {
    fn new(target: f32) -> Self { Self { from: target, target, started: None } }
    fn value(&self, now: Instant) -> (f32, bool) {
        let Some(started) = self.started else { return (self.target, false); };
        let progress = now.saturating_duration_since(started).as_secs_f32() / DURATION.as_secs_f32();
        if progress >= 1. { return (self.target, false); }
        let eased = 1. - (1. - progress).powi(3);
        (self.from + (self.target - self.from) * eased, true)
    }
    fn retarget(&mut self, target: f32, reduced: bool, now: Instant) -> bool {
        if reduced {
            let changed = self.target != target || self.started.is_some();
            *self = Self::new(target);
            return changed;
        }
        if self.target == target { return false; }
        self.from = self.value(now).0;
        self.target = target;
        self.started = Some(now);
        true
    }
    fn sample(&mut self, reduced: bool) -> (f32, bool) {
        if reduced { *self = Self::new(self.target); }
        let (value, running) = self.value(Instant::now());
        if !running { self.started = None; }
        (value, running)
    }
}

pub(crate) struct SelectionChrome { tween: Tween, color: Rgba, radius: Pixels, clock: Clock }
impl Render for SelectionChrome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settled = self.clock.borrow_mut().progress(cx.reduce_motion()) >= 1.;
        let (value, _) = self.tween.sample(settled);
        feedback::painted(div().size_full().rounded(self.radius).border_1()
            .border_color(self.color.opacity(0.65 * value))
            .bg(self.color.opacity(0.14 * value)), self.clock.clone())
    }
}

pub(crate) fn selection_chrome(
    id: SharedString, active: bool, color: Rgba, radius: Pixels,
    window: &mut Window, cx: &mut App,
) -> Entity<SelectionChrome> {
    // The keyed holder is never notified. Observing the animated child itself
    // here would propagate every frame back into Desktop's render path.
    let holder = window.use_keyed_state(id, cx, |_, cx| cx.new(|_| SelectionChrome {
        tween: Tween::new(if active { 1. } else { 0. }), color, radius, clock: FeedbackClock::new(),
    }));
    let entity = holder.read(cx).clone();
    entity.update(cx, |view, cx| {
        let reduced = cx.reduce_motion() || !view.clock.borrow().painted;
        let changed = view.tween.retarget(if active { 1. } else { 0. }, reduced, Instant::now());
        if changed {
            if reduced { view.clock.borrow_mut().finish(); }
            else { view.clock.borrow_mut().start(DURATION); }
        }
        if changed || view.color != color || view.radius != radius {
            view.color = color;
            view.radius = radius;
            cx.notify();
        }
    });
    entity
}

pub(crate) struct DisclosureChevron { tween: Tween, clock: Clock }
impl Render for DisclosureChevron {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settled = self.clock.borrow_mut().progress(cx.reduce_motion()) >= 1.;
        let (value, _) = self.tween.sample(settled);
        feedback::painted(Icon::new(IconName::ChevronRight).with_size(crate::design::ui_scaled_px(14.))
            .rotate(radians(value * std::f32::consts::FRAC_PI_2)), self.clock.clone())
    }
}

pub(crate) fn disclosure_chevron(
    id: SharedString, expanded: bool, window: &mut Window, cx: &mut App,
) -> (Entity<DisclosureChevron>, bool) {
    let holder = window.use_keyed_state(id, cx, |_, cx| cx.new(|_| DisclosureChevron {
        tween: Tween::new(if expanded { 1. } else { 0. }),
        clock: FeedbackClock::new(),
    }));
    let entity = holder.read(cx).clone();
    entity.update(cx, |view, cx| {
        let reduced = cx.reduce_motion() || !view.clock.borrow().painted;
        if view.tween.retarget(if expanded { 1. } else { 0. }, reduced, Instant::now()) {
            if reduced { view.clock.borrow_mut().finish(); }
            else { view.clock.borrow_mut().start(DURATION); }
            cx.notify();
        }
    });
    let entering = expanded && entity.read(cx).tween.value(Instant::now()).1;
    (entity, entering)
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;
    use super::*;
    #[::core::prelude::v1::test]
    fn first_paint_and_repeated_state_do_not_animate() {
        let now = Instant::now();
        let mut tween = Tween::new(1.);
        assert_eq!(tween.value(now), (1., false));
        assert!(!tween.retarget(1., false, now));
    }
    #[::core::prelude::v1::test]
    fn reversal_starts_at_displayed_value_and_finishes_exactly() {
        let now = Instant::now();
        let mut tween = Tween::new(0.);
        tween.retarget(1., false, now);
        let middle = now + DURATION / 2;
        let displayed = tween.value(middle).0;
        tween.retarget(0., false, middle);
        assert_eq!(tween.value(middle).0, displayed);
        assert_eq!(tween.value(middle + DURATION), (0., false));
    }
    #[::core::prelude::v1::test]
    fn reduce_motion_settles_and_does_not_replay() {
        let now = Instant::now();
        let mut tween = Tween::new(0.);
        tween.retarget(1., false, now);
        tween.retarget(1., true, now + DURATION / 3);
        assert_eq!(tween.value(now), (1., false));
        assert!(!tween.retarget(1., false, now));
    }

    #[cfg(feature = "performance-profiling")]
    struct SelectionHost {
        active: bool,
        shown: bool,
        renders: std::rc::Rc<std::cell::Cell<usize>>,
        child: Option<Entity<SelectionChrome>>,
    }
    #[cfg(feature = "performance-profiling")]
    impl Render for SelectionHost {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            if !self.shown {
                self.child = None;
                return div().size(px(100.));
            }
            let child = selection_chrome("retained-selection-test".into(), self.active,
                rgb(0x6594db), px(5.), window, cx);
            self.child = Some(child.clone());
            div().size(px(100.)).child(child)
        }
    }

    #[cfg(feature = "performance-profiling")]
    #[gpui_kit::test]
    fn retained_selection_frames_use_local_notifications_and_reduction_drains_callbacks(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| crate::native_motion::initialize_for_ui_fixture(cx));
        let renders = std::rc::Rc::new(std::cell::Cell::new(0));
        let (host, visual) = cx.add_window_view({
            let renders = renders.clone();
            move |_, _| SelectionHost { active: false, shown: true, renders, child: None }
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let child = visual.update(|_, cx| host.read(cx).child.clone().unwrap());
        let host_notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        visual.update(|_, cx| {
            let count = host_notifications.clone();
            cx.observe(&host, move |_, _| count.set(count.get() + 1)).detach();
        });
        assert!(visual.update(|_, cx| child.read(cx).clock.borrow().painted));
        visual.update(|_, cx| host.update(cx, |host, cx| { host.active = true; cx.notify(); }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(visual.update(|_, cx| host.read(cx).child.as_ref().unwrap().entity_id()), child.entity_id());
        let parent_notifications = host_notifications.get();
        let delivered = visual.update(|window, cx| window.simulate_next_frame(cx));
        assert!(delivered > 0, "a painted selection should request a local frame");
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        // Uncached GPUI ancestors still render during a window draw. The local
        // notification guarantee is narrower: no explicit host observer event
        // or parent-owned semantic update is generated by the animated child.
        assert_eq!(host_notifications.get(), parent_notifications,
            "animated child must not explicitly notify the host");

        // Reduction settles an already active tween. It must never resume when
        // motion is enabled again, and its queued callback must drain once.
        visual.update(|_, cx| cx.set_reduce_motion(true));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|window, cx| { window.simulate_next_frame(cx); });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(visual.update(|window, cx| window.simulate_next_frame(cx)), 0);
        assert!(visual.update(|_, cx| child.read(cx).tween.started.is_none()));
        visual.update(|_, cx| cx.set_reduce_motion(false));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(visual.update(|window, cx| window.simulate_next_frame(cx)), 0);
    }


    #[cfg(feature = "performance-profiling")]
    #[gpui_kit::test]
    fn hiding_selection_drains_its_pending_frame_and_reveal_does_not_replay(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| crate::native_motion::initialize_for_ui_fixture(cx));
        let (host, visual) = cx.add_window_view(|_, _| SelectionHost {
            active: false, shown: true,
            renders: std::rc::Rc::new(std::cell::Cell::new(0)), child: None,
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|_, cx| host.update(cx, |host, cx| { host.active = true; cx.notify(); }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|_, cx| host.update(cx, |host, cx| { host.shown = false; cx.notify(); }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|window, cx| { window.simulate_next_frame(cx); });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(visual.update(|window, cx| window.simulate_next_frame(cx)), 0);
        visual.update(|_, cx| host.update(cx, |host, cx| { host.shown = true; cx.notify(); }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let child = visual.update(|_, cx| host.read(cx).child.clone().unwrap());
        assert!(visual.update(|_, cx| child.read(cx).tween.started.is_none()), "reveal renders current selection immediately");
        assert_eq!(visual.update(|window, cx| window.simulate_next_frame(cx)), 0);
    }


    #[cfg(feature = "performance-profiling")]
    struct PreparedChatHost {
        chrome: Entity<SelectionHost>,
        chat: Entity<crate::chat::ChatPane>,
    }
    #[cfg(feature = "performance-profiling")]
    impl Render for PreparedChatHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().flex().flex_col()
                .child(div().h(px(100.)).flex_shrink_0().child(self.chrome.clone()))
                .child(div().flex_1().min_h_0().child(self.chat.clone()))
        }
    }
    #[cfg(feature = "performance-profiling")]
    #[gpui_kit::test]
    fn selection_feedback_frames_preserve_real_chat_transcript_preparation(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let snapshot = std::rc::Rc::new(serde_json::json!({
            "settings":{"theme":"dark", "accent":"#6594db"},
            "tasks":[{"id":"motion-chat", "agentId":"motion-agent", "title":"Detached motion fixture",
                "provider":"codex", "status":"idle", "queue":[], "hostId":"fixture-host", "cwd":""}],
            "agents":[{"id":"motion-agent", "name":"Synthetic motion agent", "provider":"codex"}],
            "messages":(0..1_000).map(|index| serde_json::json!({
                "id":format!("motion-message-{index}"), "taskId":"motion-chat", "role":"assistant",
                "text":format!("Synthetic retained history row {index}"), "createdAt":index + 1,
            })).collect::<Vec<_>>(),
            "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
        }));
        let (host, visual) = cx.add_window_view(move |window, cx| {
            let chat = cx.new(|cx| crate::chat::ChatPane::performance_fixture(
                "motion-chat".into(), snapshot, window, cx));
            let chrome = cx.new(|_| SelectionHost { active: false, shown: true,
                renders: std::rc::Rc::new(std::cell::Cell::new(0)), child: None });
            PreparedChatHost { chrome, chat }
        });
        visual.simulate_resize(size(px(1200.), px(820.)));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let chrome = visual.update(|_, cx| host.read(cx).chrome.clone());
        let baseline = crate::chat::transcript_preparation_counts();
        let reasons = crate::chat::transcript_preparation_reason_counts();
        visual.update(|_, cx| chrome.update(cx, |chrome, cx| { chrome.active = true; cx.notify(); }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        for _ in 0..5 {
            visual.update(|window, cx| { window.simulate_next_frame(cx); });
            visual.update(|window, cx| { window.draw(cx).clear(cx); });
            assert_eq!(crate::chat::transcript_preparation_counts(), baseline,
                "feedback redraws must reuse prepared history even when uncached ancestors render");
            assert_eq!(crate::chat::transcript_preparation_reason_counts(), reasons);
        }
    }

}
