//! Disclosure clicks must survive a chat redraw between mouse down and up.
use super::{StableRowRenderer, StableTranscriptList, retain_transcript_list};
use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ListAlignment, ListState, Modifiers,
    MouseButton, ParentElement as _, Render, StyleRefinement, Styled as _, TestAppContext, Window,
    div, point,
};
use gpui_kit::component::button::Button;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct TreeClickHarness {
    retained: Option<Entity<StableTranscriptList>>,
    state: ListState,
    clicks: Rc<Cell<usize>>,
}
impl Render for TreeClickHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        let rows: StableRowRenderer = Rc::new(RefCell::new(
            move |_, _: &mut Window, _: &mut App, _: f32| {
                let clicks = clicks.clone();
                Button::new("completed-turn")
                    .label("Show completed turn")
                    .w_full()
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                    .into_any_element()
            },
        ));
        retain_transcript_list(
            &mut self.retained,
            self.state.clone(),
            rows,
            Rc::new(Vec::new()),
            cx,
        );
        div().size_full().child(
            self.retained
                .as_ref()
                .unwrap()
                .clone()
                .cached(StyleRefinement::default().size_full()),
        )
    }
}

#[gpui::test]
fn completed_turn_click_survives_chat_redraw_between_press_and_release(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    let (host, visual) = cx.add_window_view(move |_, _| TreeClickHarness {
        retained: None,
        state: ListState::new(1, ListAlignment::Top, gpui_kit::px(0.)),
        clicks: count,
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let original = host.read_with(visual, |host, _| {
        host.retained.as_ref().unwrap().entity_id()
    });
    let position = point(gpui_kit::px(80.), gpui_kit::px(12.));
    visual.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    host.update(visual, |_, cx| cx.notify());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        original,
        host.read_with(visual, |host, _| host
            .retained
            .as_ref()
            .unwrap()
            .entity_id())
    );
    visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        clicks.get(),
        1,
        "a redraw must not swallow the first disclosure click"
    );
}
