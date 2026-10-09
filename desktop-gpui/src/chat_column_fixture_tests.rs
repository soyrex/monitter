//! Layout regression for the centered shared chat content column.
use super::transcript_column;
use crate::design;
use gpui::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
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
    let expected_width = width.min(design::ui_scaled_px(design::CHAT_MAX_WIDTH).as_f32());

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
