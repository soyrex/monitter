//! Native wheel input preserves editor ownership, prepared history and tail controls.
use super::ChatPane;
use crate::native_motion::{self, MotionPreference};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Render, StyleRefinement, Styled as _, TestAppContext, VisualTestContext, Window,
    ScrollDelta, ScrollWheelEvent, div, point, px, size,
};
use serde_json::json;
use std::rc::Rc;

struct ScrollHarness {
    pane: Entity<ChatPane>,
}
impl Render for ScrollHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone().cached(StyleRefinement::default().size_full()))
    }
}
fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}
fn wheel(visual: &mut VisualTestContext, position: gpui_kit::Point<gpui_kit::Pixels>, delta: f32) {
    visual.simulate_event(ScrollWheelEvent {
        position, delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        ..Default::default()
    });
    draw(visual);
    draw(visual);
}

fn assert_native_scroll_preserves_input(cx: &mut TestAppContext, motion: MotionPreference) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        native_motion::initialize_for_ui_fixture(cx);
        native_motion::set_preference_for_ui_fixture(motion, cx);
    });
    let source = Rc::new(json!({
        "settings":{"theme":"dark", "interfaceScale":100, "interfaceFontSize":14},
        "tasks":[{"id":"scroll-task", "agentId":"scroll-agent", "title":"Scroll fixture",
            "provider":"codex", "status":"idle", "queue":[], "hostId":"fixture", "cwd":""}],
        "agents":[{"id":"scroll-agent", "name":"Fixture agent", "provider":"codex"}],
        "messages":(0..80).map(|index| json!({
            "id":format!("scroll-{index}"), "taskId":"scroll-task", "role":"user",
            "text":format!("Native scroll row {index}. Keep history and composer independent."),
            "createdAt":1_700_000_000_000_i64 + index,
        })).collect::<Vec<_>>(),
        "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
    }));
    let (host, visual) = cx.add_window_view(move |window, cx| {
        let pane = cx.new(|cx| {
            let pane = ChatPane::performance_fixture("scroll-task".into(), source, window, cx);
            // Pager and native end-of-list assertions need a known finite
            // extent; production intentionally reports None while rows remain
            // unmeasured. Keep real row rendering and wheel dispatch intact.
            pane.transcripts["scroll-task"].list.clone().measure_all();
            pane
        });
        ScrollHarness { pane }
    });
    visual.simulate_resize(size(px(1_100.), px(800.)));
    let pane = host.read_with(visual, |host, _| host.pane.clone());
    visual.update(|window, cx| {
        pane.read(cx).composer.read(cx).focus_handle(cx).focus(window, cx);
    });
    draw(visual);
    draw(visual);
    visual.simulate_keystrokes("right");
    visual.simulate_input("Keep this draft and its cursor");
    draw(visual);
    let (position, cursor, sidebar) = pane.read_with(visual, |pane, cx| {
        (pane.decoration_bounds.chat_column.get().unwrap().origin + point(px(50.), px(50.)),
            pane.composer.read(cx).cursor(), pane.secondary_sidebar_state())
    });
    // Prime pointer modality and fully hide the composer before measuring.
    for _ in 0..3 { wheel(visual, position, 600.); }
    let (baseline, offset, distance, renderer) = pane.read_with(visual, |pane, cx| {
        let height = pane.decoration_bounds.working_anchor.get().unwrap().size.height;
        assert!(pane.composer_scroll_distance > height);
        assert!(!pane.transcripts["scroll-task"].list.is_following_tail());
        (pane.chrome_render_count, pane.transcripts["scroll-task"].list.logical_scroll_top(),
            pane.composer_scroll_distance, pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows.clone())
    });
    wheel(visual, position, 80.);
    pane.read_with(visual, |pane, cx| {
        let after = pane.transcripts["scroll-task"].list.logical_scroll_top();
        assert_ne!((after.item_ix, after.offset_in_item), (offset.item_ix, offset.offset_in_item),
            "native wheel input must still move the retained virtual list");
        assert_ne!(pane.composer_scroll_distance, distance, "retain the full jump distance");
        assert!(Rc::ptr_eq(&renderer, &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows),
            "native scrolling must reuse prepared transcript wrappers");
        assert_eq!(pane.composer.read(cx).value().as_ref(), "Keep this draft and its cursor");
        assert_eq!(pane.composer.read(cx).cursor(), cursor);
        assert_eq!(pane.secondary_sidebar_state(), sidebar);
    });
    visual.update(|window, cx| assert!(pane.read(cx).composer.read(cx).focus_handle(cx).is_focused(window)));

    for _ in 0..4 { wheel(visual, position, -100_000.); }
    pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["scroll-task"];
        assert!(state.list.is_following_tail(), "native wheel return to bottom resumes tail follow");
        assert_eq!(pane.composer_scroll_distance, px(0.));
        assert!(!state.history_top_reached);
        assert!(pane.chrome_render_count > baseline, "tail state still invalidates chrome");
    });

    // Reaching the history boundary still rebuilds and mounts its pager chrome.
    pane.update(visual, |pane, cx| {
        pane.transcripts["scroll-task"].list.scroll_to(gpui_kit::ListOffset {
            item_ix: 0, offset_in_item: px(0.),
        });
        pane.invalidate_transcript_list(cx);
    });
    draw(visual);
    wheel(visual, position, 20.);
    wheel(visual, position, 20.);
    pane.read_with(visual, |pane, _| assert!(pane.transcripts["scroll-task"].history_top_reached));
    assert!(visual.debug_bounds("chat-history-boundary-controls").is_some(), "history boundary controls remain visible");

    // The explicit jump action must also remount its composer and clear the
    // boundary controls, without requiring a new backend publication.
    pane.update(visual, |pane, cx| pane.finish_bottom_jump(cx));
    draw(visual);
    draw(visual);
    pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["scroll-task"];
        assert!(state.list.is_following_tail());
        assert_eq!(pane.composer_scroll_distance, px(0.));
        assert!(!state.history_top_reached);
        assert!(pane.chrome_render_count > baseline, "tail and pager state still invalidate chrome");
        assert_eq!(pane.secondary_sidebar_state(), sidebar);
    });
}

#[gpui_kit::test]
fn native_scroll_preserves_prepared_history_draft_focus_tail_pager_and_sidebar(cx: &mut TestAppContext) {
    assert_native_scroll_preserves_input(cx, MotionPreference::On);
}

#[gpui_kit::test]
fn reduced_motion_scroll_preserves_prepared_history_draft_focus_tail_pager_and_sidebar(cx: &mut TestAppContext) {
    assert_native_scroll_preserves_input(cx, MotionPreference::Off);
}
