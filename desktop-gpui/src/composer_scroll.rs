//! Geometry shared by the retained composer and transcript tail controls.
use gpui::{Pixels, px};

pub(crate) fn composer_slide(distance: Pixels, height: Pixels, reduced: bool) -> Pixels {
    let height = height.max(px(0.));
    if reduced && distance > px(0.) {
        height
    } else {
        distance.max(px(0.)).min(height)
    }
}

pub(crate) fn remaining_tail_distance(
    max: Pixels,
    current_negative_offset: Pixels,
    hidden: Pixels,
) -> Pixels {
    (max + current_negative_offset + hidden).max(px(0.))
}

/// Frame ratios keep the jump smooth even if new rows are measured or the
/// viewport changes while the composer returns. No timer runs when idle.
pub(crate) struct BottomJump {
    started: std::time::Instant,
    remaining: f32,
}
impl BottomJump {
    pub(crate) fn new(now: std::time::Instant) -> Self {
        Self { started: now, remaining: 1. }
    }
    pub(crate) fn step(&mut self, now: std::time::Instant) -> (f32, bool) {
        let progress = (now.saturating_duration_since(self.started).as_secs_f32() / 0.28).clamp(0., 1.);
        let next = (1. - progress).powi(3);
        let ratio = if self.remaining > 0. { next / self.remaining } else { 0. };
        self.remaining = next;
        (ratio.clamp(0., 1.), progress >= 1.)
    }
}

/// Kept independently of scroll completion so an early tail does not cut off exit.
pub(crate) struct BottomButtonExit { started: std::time::Instant }
impl BottomButtonExit {
    pub(crate) fn new(now: std::time::Instant) -> Self { Self { started: now } }
    pub(crate) fn sample(&self, now: std::time::Instant) -> (f32, bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f32() / 0.24).clamp(0., 1.);
        (t * t, t >= 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::{BottomButtonExit, BottomJump, composer_slide, remaining_tail_distance};
    #[test]
    fn bottom_button_exit_remains_mounted_until_it_flies_out() {
        let now = std::time::Instant::now();
        let exit = BottomButtonExit::new(now);
        let mut previous = 0.;
        for ms in [0, 16, 80, 120, 200, 239] {
            let (progress, finished) = exit.sample(now + std::time::Duration::from_millis(ms));
            assert!(!finished);
            assert!(progress >= previous && progress < 1.);
            previous = progress;
        }
        assert_eq!(exit.sample(now + std::time::Duration::from_millis(240)), (1., true));
        assert_eq!(exit.sample(now + std::time::Duration::from_secs(1)), (1., true));
    }
    use gpui::px;

    #[test]
    fn composer_scroll_jump_eases_monotonically_and_finishes() {
        let now = std::time::Instant::now();
        let mut jump = BottomJump::new(now);
        let mut distance = 1000.;
        for ms in [0, 16, 32, 80, 140, 220, 280] {
            let previous = distance;
            let (ratio, done) = jump.step(now + std::time::Duration::from_millis(ms));
            distance *= ratio;
            assert!(distance <= previous && distance >= 0.);
            assert_eq!(done, ms >= 280);
        }
        assert_eq!(distance, 0.);
    }
    #[test]
    fn composer_scroll_slide_clamps_and_respects_reduced_motion() {
        assert_eq!(composer_slide(px(-1.), px(120.), false), px(0.));
        assert_eq!(composer_slide(px(40.), px(120.), false), px(40.));
        assert_eq!(composer_slide(px(200.), px(120.), false), px(120.));
        assert_eq!(composer_slide(px(0.), px(120.), true), px(0.));
        assert_eq!(composer_slide(px(1.), px(120.), true), px(120.));
        assert_eq!(composer_slide(px(10.), px(-1.), false), px(0.));
    }

    #[test]
    fn composer_scroll_tail_distance_includes_hidden_footer() {
        assert_eq!(remaining_tail_distance(px(300.), px(-300.), px(0.)), px(0.));
        assert_eq!(
            remaining_tail_distance(px(180.), px(-300.), px(120.)),
            px(0.)
        );
        assert_eq!(
            remaining_tail_distance(px(180.), px(-250.), px(120.)),
            px(50.)
        );
        assert_eq!(
            remaining_tail_distance(px(180.), px(-500.), px(120.)),
            px(0.)
        );
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod layout_tests {
    use gpui::{
        AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _,
        IntoElement, ParentElement as _, Pixels, Render, Styled as _, TestAppContext,
        VisualTestContext, Window, canvas, div, px, size,
    };
    use gpui_kit::component::Root;
    use std::{cell::Cell, rc::Rc};

    struct ComposerFixture {
        hidden: Pixels,
        focus: FocusHandle,
        viewport: Rc<Cell<Option<Bounds<Pixels>>>>,
        composer: Rc<Cell<Option<Bounds<Pixels>>>>,
    }

    impl Render for ComposerFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let viewport = self.viewport.clone();
            let composer = self.composer.clone();
            div()
                .h(px(400.))
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div().flex_1().min_h_0().relative().child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, _, _| viewport.set(Some(bounds)),
                        )
                        .absolute()
                        .inset_0(),
                    ),
                )
                .child(
                    div()
                        .h(px(120.))
                        .flex_shrink_0()
                        .mb(-self.hidden)
                        .relative()
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, _, _| composer.set(Some(bounds)),
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .child(
                            div()
                                .id("retained-composer-input")
                                .track_focus(&self.focus)
                                .h(px(30.))
                                .child("Retained draft"),
                        ),
                )
        }
    }

    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }

    struct TailFixture {
        list: gpui::ListState,
        hidden: Pixels,
    }

    impl Render for TailFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let owner = cx.weak_entity();
            self.list.set_scroll_handler(move |event, _, cx| {
                let at_bottom = !event.is_scrolled;
                let owner = owner.clone();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |fixture, cx| {
                        if at_bottom {
                            fixture.hidden = px(0.);
                            fixture.list.set_follow_mode(gpui::FollowMode::Tail);
                            fixture.list.scroll_to_end();
                        } else {
                            fixture.list.set_follow_mode(gpui::FollowMode::Normal);
                            fixture.hidden = super::composer_slide(
                                super::remaining_tail_distance(
                                    fixture.list.max_offset_for_scrollbar().y,
                                    fixture.list.scroll_px_offset_for_scrollbar().y,
                                    fixture.hidden), px(120.), false);
                        }
                        cx.notify();
                    });
                });
            });
            div()
                .h(px(400.))
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div().flex_1().min_h_0().child(
                        gpui::list(self.list.clone(), |_, _, _| {
                            div().h(px(50.)).w_full().into_any_element()
                        })
                        .size_full(),
                    ),
                )
                .child(div().h(px(120.)).flex_shrink_0().mb(-self.hidden))
        }
    }

    #[gpui_kit::test]
    fn composer_scroll_small_upward_gesture_stays_detached_until_jump(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let list = gpui::ListState::new(20, gpui::ListAlignment::Bottom, px(1000.));
        list.set_follow_mode(gpui::FollowMode::Tail);
        let retained: Rc<Cell<Option<Entity<TailFixture>>>> = Rc::new(Cell::new(None));
        let fixture_slot = retained.clone();
        let fixture_list = list.clone();
        let (_, mut visual) = cx.add_window_view(move |window, cx| {
            let fixture = cx.new(|_| TailFixture {
                list: fixture_list,
                hidden: px(0.),
            });
            fixture_slot.set(Some(fixture.clone()));
            Root::new(fixture, window, cx)
        });
        visual.simulate_resize(size(px(600.), px(400.)));
        draw(&mut visual);
        assert!(list.is_following_tail());
        let initial_height = list.viewport_bounds().size.height;
        let fixture = retained.take().expect("retained tail fixture");
        visual.update(|window, cx| {
            window.dispatch_event(gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                position: gpui::point(px(300.), px(200.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(20.))),
                modifiers: Default::default(), touch_phase: gpui::TouchPhase::Moved,
            }), cx);
        });
        draw(&mut visual);
        assert_eq!(list.viewport_bounds().size.height - initial_height, px(20.));
        assert!(!list.is_following_tail());
        visual.update(|window, cx| {
            window.dispatch_event(gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                position: gpui::point(px(300.), px(200.)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(40.))),
                modifiers: Default::default(), touch_phase: gpui::TouchPhase::Moved,
            }), cx);
        });
        draw(&mut visual);
        assert_eq!(list.viewport_bounds().size.height - initial_height, px(60.));
        assert!(!list.is_following_tail());
        // A later render/measurement must not reinterpret the larger viewport
        // as an intentional return to the latest message.
        list.remeasure();
        visual.update(|_, cx| fixture.update(cx, |_, cx| cx.notify()));
        draw(&mut visual);
        assert!(!list.is_following_tail());
        visual.update(|_, cx| {
            fixture.update(cx, |fixture, cx| {
                fixture.hidden = px(0.);
                fixture.list.set_follow_mode(gpui::FollowMode::Tail);
                fixture.list.scroll_to_end();
                cx.notify();
            })
        });
        draw(&mut visual);
        assert!(list.is_following_tail());
        assert_eq!(list.is_scrolled_to_end(), Some(true));
        assert_eq!(list.viewport_bounds().size.height, initial_height);
    }

    #[gpui_kit::test]
    fn composer_scroll_negative_margin_reclaims_viewport_and_retains_focus(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| gpui_kit::init(cx));
        let viewport = Rc::new(Cell::new(None));
        let composer = Rc::new(Cell::new(None));
        let retained: Rc<Cell<Option<Entity<ComposerFixture>>>> = Rc::new(Cell::new(None));
        let fixture_slot = retained.clone();
        let viewport_slot = viewport.clone();
        let composer_slot = composer.clone();
        let (_, mut visual) = cx.add_window_view(move |window, cx| {
            let fixture = cx.new(|cx| ComposerFixture {
                hidden: px(0.),
                focus: cx.focus_handle(),
                viewport: viewport_slot,
                composer: composer_slot,
            });
            let focus = fixture.read(cx).focus.clone();
            focus.focus(window, cx);
            fixture_slot.set(Some(fixture.clone()));
            Root::new(fixture, window, cx)
        });
        visual.simulate_resize(size(px(600.), px(400.)));
        draw(&mut visual);
        let before_viewport = viewport.get().expect("transcript laid out");
        let before_composer = composer.get().expect("composer laid out");
        let fixture = retained.take().expect("retained fixture");
        visual.update(|_, cx| {
            fixture.update(cx, |fixture, cx| {
                fixture.hidden = px(120.);
                cx.notify();
            })
        });
        draw(&mut visual);
        let after_viewport = viewport.get().expect("expanded transcript laid out");
        let after_composer = composer.get().expect("hidden composer remains laid out");
        assert_eq!(after_composer.origin.y - before_composer.origin.y, px(120.));
        assert_eq!(
            after_viewport.size.height - before_viewport.size.height,
            px(120.)
        );
        assert_eq!(after_composer.size.height, px(120.));
        visual.update(|window, cx| assert!(fixture.read(cx).focus.is_focused(window)));
    }
}
