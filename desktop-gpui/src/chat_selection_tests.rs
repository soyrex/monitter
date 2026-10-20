//! Pane boundaries must survive cached virtual-list/message descendants.
use super::super::{StableRowRenderer, StableTranscriptList, minimap};
use super::{TranscriptGeometry, TranscriptViewport};
use crate::design;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ListAlignment,
    ListState, Modifiers, MouseButton, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowAppearance, div, point, px, size,
};
use gpui_kit::base::{TextSelection, TextSelectionGroupId};
use gpui_kit::component::{
    Root, WindowExt as _,
    text::{TextView, TextViewState},
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Panes {
    panes: Vec<Entity<TranscriptViewport>>,
    texts: Vec<Entity<TextViewState>>,
}
impl Render for Panes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().children(
            self.panes
                .iter()
                .map(|pane| div().w(px(250.)).h_full().child(pane.clone())),
        )
    }
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}
fn selected(cx: &mut VisualTestContext) -> String {
    cx.update(TextSelection::selected_text)
}
fn drag(
    cx: &mut VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn pane_selection_stays_in_cached_transcript_and_changes_owner_on_new_gesture(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let (root, visual) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|cx| {
            let mut panes = Vec::new();
            let mut texts = Vec::new();
            for side in ["left", "right"] {
                let first =
                    cx.new(|cx| TextViewState::markdown(&format!("{side} alpha first"), cx));
                let second =
                    cx.new(|cx| TextViewState::markdown(&format!("{side} beta second"), cx));
                texts.extend([first.clone(), second.clone()]);
                let states = [first, second];
                let rows: StableRowRenderer = Rc::new(RefCell::new(
                    move |row, _: &mut Window, _: &mut gpui::App, _: f32| {
                        div()
                            .debug_selector(move || format!("{side}-selection-row-{row}"))
                            .w_full()
                            .h(px(48.))
                            .child(TextView::new(&states[row]).selectable(true))
                            .into_any_element()
                    },
                ));
                let list = cx.new(|_| StableTranscriptList {
                    state: ListState::new(2, ListAlignment::Top, px(100.)),
                    rows,
                    minimap: Rc::new(Vec::new()),
                });
                panes.push(cx.new(|_| TranscriptViewport {
                    task_id: side.into(),
                    selection_group: TextSelectionGroupId::new(),
                    owner: None,
                    load_earlier: Rc::new(|_, _| ()),
                    load_unmeasured_history: Rc::new(|_, _| ()),
                    render_count: 0,
                    list: Some(list),
                    error: None,
                    palette: design::palette(&Value::Null, WindowAppearance::Dark),
                    geometry: Rc::new(Cell::new(TranscriptGeometry::default())),
                    minimap_visible: false,
                    minimap_exit: None,
                    minimap_jump: None,
                    minimap_emphasis: minimap::Emphasis::default(),
                    minimap_bottom_inset: px(0.),
                }));
            }
            Panes { panes, texts }
        });
        Root::new(fixture, window, cx)
    });
    visual.simulate_resize(size(px(500.), px(240.)));
    visual.update(|window, _| window.activate_window());
    draw(visual);
    let left_first = visual.debug_bounds("left-selection-row-0").unwrap();
    let left_second = visual.debug_bounds("left-selection-row-1").unwrap();
    let right_second = visual.debug_bounds("right-selection-row-1").unwrap();
    let from = point(left_first.left() + px(1.), left_first.top() + px(8.));
    drag(
        visual,
        from,
        point(right_second.right() - px(2.), right_second.top() + px(8.)),
    );
    let text = selected(visual);
    assert!(
        text.contains("alpha") && text.contains("beta"),
        "same-pane messages remain selectable: {text:?}"
    );
    assert!(
        !text.contains("right"),
        "selection crossed the pane boundary: {text:?}"
    );
    // A redraw of retained cached descendants must preserve that boundary.
    draw(visual);
    assert_eq!(selected(visual), text);
    let right_first = visual.debug_bounds("right-selection-row-0").unwrap();
    drag(
        visual,
        point(right_first.left() + px(1.), right_first.top() + px(8.)),
        point(right_second.right() - px(2.), right_second.top() + px(8.)),
    );
    let text = selected(visual);
    assert!(
        text.contains("right") && !text.contains("left"),
        "new gesture must replace the old pane's selection: {text:?}"
    );
    // Window select-all is restricted to the currently selected document.
    let fixture = root.read_with(visual, |root, _| {
        root.view().clone().downcast::<Panes>().unwrap()
    });
    let right_text = fixture.read_with(visual, |fixture, _| fixture.texts[3].clone());
    visual.update(|_, cx| right_text.update(cx, |text, cx| text.select_all(cx)));
    draw(visual);
    assert_eq!(
        selected(visual).trim_end_matches('\n'),
        "right beta second",
        "local select-all must replace selections in other messages and panes"
    );
    // Shift-clicking another pane starts a fresh anchor in that pane.
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for target in [left_second, left_first] {
        let point = point(target.left() + px(8.), target.top() + px(8.));
        visual.simulate_mouse_down(point, MouseButton::Left, shift);
        visual.simulate_mouse_up(point, MouseButton::Left, shift);
    }
    draw(visual);
    let text = selected(visual);
    assert!(
        !text.is_empty() && !text.contains("right"),
        "Shift-click joined different panes: {text:?}"
    );
    // Modal scope remains independent of the pane group.
    let modal_text = visual.update(|window, cx| {
        let text = cx.new(|cx| TextViewState::markdown("Modal selection text", cx));
        let content = text.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.width(px(320.)).child(
                div()
                    .debug_selector(|| "selection-modal-text".into())
                    .h(px(80.))
                    .child(TextView::new(&content).selectable(true)),
            )
        });
        text
    });
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(500));
    draw(visual);
    draw(visual);
    assert!(
        selected(visual).is_empty(),
        "opening a modal must clear the underlying selection"
    );
    let modal = visual.debug_bounds("selection-modal-text").unwrap();
    let point = point(modal.left() + px(8.), modal.top() + px(8.));
    visual.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    visual.update(|_, cx| modal_text.update(cx, |text, cx| text.select_all(cx)));
    draw(visual);
    assert_eq!(
        selected(visual).trim_end_matches('\n'),
        "Modal selection text"
    );
    visual.update(|window, cx| window.close_dialog(cx));
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(500));
    draw(visual);
    draw(visual);
    assert!(selected(visual).is_empty());
    // Retain a reference to the actual app viewport rather than a surrogate wrapper.
    let _ = (root, left_second);
}
