//! Layout regression for the centered shared chat content column.
use super::transcript_column;
use crate::design;
use gpui::{
    AppContext as _, InteractiveElement as _, StatefulInteractiveElement as _, Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, ListState, canvas, div, px, size,
};
use gpui_kit::component::Root;
use std::{cell::RefCell, rc::Rc};

struct ColumnFixture {
    list: ListState,
    viewport: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
    content: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
}

impl Render for ColumnFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let viewport = self.viewport.clone();
        let content = self.content.clone();

        div()
            .size_full()
            .relative()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, _, _| {
                        *viewport.borrow_mut() = Some(bounds);
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                gpui::list(self.list.clone(), move |_, _, _| {
                    let content = content.clone();
                    transcript_column(
                        div().h(px(48.)).w_full().relative().child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, _, _| {
                                    *content.borrow_mut() = Some(bounds);
                                },
                            )
                            .absolute()
                            .size_full(),
                        ),
                    )
                })
                .w_full()
                .h_full(),
            )
    }
}

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

fn assert_column_centered(
    viewport: gpui::Bounds<gpui::Pixels>,
    content: gpui::Bounds<gpui::Pixels>,
    width: f32,
) {
    let viewport_width = viewport.size.width.as_f32();
    let content_width = content.size.width.as_f32();
    let viewport_center = viewport.origin.x.as_f32() + viewport_width / 2.0;
    let content_center = content.origin.x.as_f32() + content_width / 2.0;
    let expected_width = (width - design::ui_scaled_px(10.).as_f32())
        .min(design::ui_scaled_px(design::CHAT_MAX_WIDTH).as_f32());

    assert!(
        (content_width - expected_width).abs() <= 1.0,
        "content width {content_width:.1}px should fit the {expected_width:.1}px design cap"
    );
    assert!(
        (content_center - viewport_center).abs() <= 1.0,
        "content center {content_center:.1}px should match viewport center {viewport_center:.1}px"
    );
}

#[gpui_kit::test]
fn virtual_transcript_rows_use_a_centered_shared_width(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let viewport = Rc::new(RefCell::new(None));
    let content = Rc::new(RefCell::new(None));
    let viewport_slot = viewport.clone();
    let content_slot = content.clone();
    let (_, mut visual) = cx.add_window_view(move |window, cx| {
        let fixture = cx.new(|_| ColumnFixture {
            list: gpui::ListState::new(1, gpui::ListAlignment::Top, px(100.)),
            viewport: viewport_slot,
            content: content_slot,
        });
        Root::new(fixture, window, cx)
    });

    visual.simulate_resize(size(px(1200.), px(700.)));
    draw(&mut visual);
    assert_column_centered(
        viewport.borrow().expect("viewport canvas was laid out"),
        content.borrow().expect("virtual row marker was laid out"),
        1200.0,
    );

    visual.simulate_resize(size(px(500.), px(700.)));
    draw(&mut visual);
    assert_column_centered(
        viewport
            .borrow()
            .expect("resized viewport canvas was laid out"),
        content
            .borrow()
            .expect("resized virtual row marker was laid out"),
        500.0,
    );
}

#[::core::prelude::v1::test]
fn transcript_block_spacing_follows_density_and_doubles_normal() {
    use serde_json::json;
    assert_eq!(super::transcript_block_gap(&json!({"interfaceDensity":"tight"})), 10.);
    assert_eq!(super::transcript_block_gap(&json!({"interfaceDensity":"normal"})), 20.);
    assert_eq!(super::transcript_block_gap(&json!({"interfaceDensity":"spacious"})), 40.);
    assert_eq!(super::transcript_block_gap(&json!({})), 20.);
}

struct ComposerControlsFixture { hidden: gpui::Pixels }
impl Render for ComposerControlsFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().overflow_hidden().flex().flex_col()
            .child(div().flex_1().min_h_0())
            .child(super::composer_scroll_stack(self.hidden)
                .child(div().debug_selector(|| "scroll-fixture-composer".into()).h(px(80.)).w_full())
                .child(div().debug_selector(|| "scroll-fixture-companion-controls".into()).h(px(30.)).w_full()))
    }
}

#[gpui_kit::test]
fn companion_controls_share_composer_scroll_offset_and_leave_viewport(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut retained = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|_| ComposerControlsFixture { hidden: px(0.) });
        retained = Some(fixture.clone());
        Root::new(fixture, window, cx)
    });
    let fixture = retained.unwrap();
    visual.simulate_resize(size(px(600.), px(500.)));
    draw(visual);
    let composer = visual.debug_bounds("scroll-fixture-composer").unwrap();
    let controls = visual.debug_bounds("scroll-fixture-companion-controls").unwrap();
    assert!((composer.bottom() - controls.top()).abs() <= px(1.));
    visual.update(|_, cx| fixture.update(cx, |fixture, cx| { fixture.hidden = px(45.); cx.notify(); }));
    draw(visual);
    let shifted_composer = visual.debug_bounds("scroll-fixture-composer").unwrap();
    let shifted_controls = visual.debug_bounds("scroll-fixture-companion-controls").unwrap();
    assert!((shifted_composer.top() - composer.top() - px(45.)).abs() <= px(1.));
    assert!((shifted_controls.top() - controls.top() - px(45.)).abs() <= px(1.));
    visual.update(|_, cx| fixture.update(cx, |fixture, cx| { fixture.hidden = px(110.); cx.notify(); }));
    draw(visual);
    for selector in ["scroll-fixture-composer", "scroll-fixture-companion-controls"] {
        assert!(visual.debug_bounds(selector).is_none_or(|bounds| bounds.top() >= px(500.)),
            "{selector} must leave the visible pane");
    }
}
use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _};

struct BackToBottomClickFixture {
    disabled: bool,
    arrow_clicks: Rc<std::cell::Cell<usize>>,
    underlay_clicks: Rc<std::cell::Cell<usize>>,
    ancestor_clicks: Rc<std::cell::Cell<usize>>,
    ancestor_presses: Rc<std::cell::Cell<usize>>,
    composer: crate::activity_sparkles::PaintBounds,
    column: crate::activity_sparkles::PaintBounds,
}
impl Render for BackToBottomClickFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{Disableable as _, Sizable as _, button::Button};
        use gpui::InteractiveElement as _;
        let arrow = self.arrow_clicks.clone();
        let underlay = self.underlay_clicks.clone();
        let ancestor = self.ancestor_clicks.clone();
        let presses = self.ancestor_presses.clone();
        let releases = self.ancestor_presses.clone();
        let jump = super::back_to_bottom_surface().size(px(36.)).rounded_full()
            .child(Button::new("click-fixture-back-to-bottom").compact().size_full().p_0()
                .disabled(self.disabled).label("↓")
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    arrow.set(arrow.get() + 1);
                })).into_any_element();
        div().id("click-fixture-ancestor").size_full().relative()
            .on_click(move |_, _, _| ancestor.set(ancestor.get() + 1))
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, _| presses.set(presses.get() + 1))
            .on_mouse_up(gpui::MouseButton::Left, move |_, _, _| releases.set(releases.get() + 1))
            .child(Button::new("click-fixture-underlay").absolute().inset_0()
                .label("Underlying composer control")
                .on_click(move |_, _, _| underlay.set(underlay.get() + 1)))
            .child(super::task_activity::working_bubble::ComposerAnchor::new(
                self.composer.clone(), self.column.clone(), jump, px(14.),
            ))
    }
}

#[gpui_kit::test]
fn back_to_bottom_anchor_swallows_native_clicks_even_when_exit_disables_button(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let arrow = Rc::new(std::cell::Cell::new(0));
    let underlay = Rc::new(std::cell::Cell::new(0));
    let ancestor = Rc::new(std::cell::Cell::new(0));
    let presses = Rc::new(std::cell::Cell::new(0));
    let mut retained = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|_| BackToBottomClickFixture {
            disabled: false, arrow_clicks: arrow.clone(), underlay_clicks: underlay.clone(),
            ancestor_clicks: ancestor.clone(), ancestor_presses: presses.clone(),
            composer: Rc::new(std::cell::Cell::new(Some(gpui::Bounds::new(
                gpui::point(px(0.), px(300.)), size(px(600.), px(100.)))))),
            column: Rc::new(std::cell::Cell::new(Some(gpui::Bounds::new(
                gpui::point(px(0.), px(0.)), size(px(600.), px(500.)))))),
        });
        retained = Some(fixture.clone());
        Root::new(fixture, window, cx)
    });
    let fixture = retained.unwrap();
    visual.simulate_resize(size(px(600.), px(500.)));
    draw(visual);
    let target = visual.debug_bounds("chat-back-to-bottom-hit-surface").unwrap().center();
    visual.simulate_mouse_down(target, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(arrow.get(), 1);
    assert_eq!((underlay.get(), ancestor.get(), presses.get()), (0, 0, 0));
    fixture.update(visual, |fixture, cx| { fixture.disabled = true; cx.notify(); });
    draw(visual);
    visual.simulate_mouse_down(target, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(arrow.get(), 1, "disabled exiting arrow must not invoke its action");
    assert_eq!((underlay.get(), ancestor.get(), presses.get()), (0, 0, 0), "paint-anchored surface must occlude and stop both pointer events during exit");
}
