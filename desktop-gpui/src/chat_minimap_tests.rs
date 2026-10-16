//! Native event/layout regressions for the retained transcript overview.
use super::{TranscriptViewport, TranscriptGeometry};
use super::super::{StableTranscriptList, StableRowRenderer, minimap};
use crate::{design, transcript};
use gpui::{App, AppContext as _, Context, Entity, IntoElement, ListAlignment, ListState,
    Modifiers, MouseButton, ParentElement as _, InteractiveElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowAppearance, ScrollDelta, ScrollWheelEvent, div, point};
use serde_json::{Value, json};
use std::{rc::Rc, cell::RefCell, time::Duration};

struct MinimapHarness {
    viewport: Entity<TranscriptViewport>,
    state: ListState,
    height: gpui::Pixels,
}

#[gpui::test]
fn native_pointer_proximity_brightens_minimap_without_moving_transcript(cx: &mut TestAppContext) {
    let (host, visual) = fixture(cx);
    reveal(visual);
    let rail = visual.debug_bounds("chat-minimap").unwrap();
    let viewport = host.read_with(visual, |host, _| host.viewport.clone());
    let before = host.read_with(visual, |host, _| host.state.logical_scroll_top());
    visual.simulate_mouse_move(point(rail.left() - gpui_kit::px(32.), rail.center().y), None, Modifiers::default());
    draw(visual);
    assert_eq!(viewport.read_with(visual, |view, _| view.minimap_emphasis.target), 0.5);
    visual.simulate_mouse_move(rail.center(), None, Modifiers::default());
    draw(visual);
    assert_eq!(viewport.read_with(visual, |view, _| view.minimap_emphasis.target), 1.);
    visual.simulate_mouse_move(point(rail.left() - gpui_kit::px(80.), rail.center().y), None, Modifiers::default());
    draw(visual);
    assert_eq!(viewport.read_with(visual, |view, _| view.minimap_emphasis.target), 0.);
    let after = host.read_with(visual, |host, _| host.state.logical_scroll_top());
    assert_eq!((after.item_ix, after.offset_in_item), (before.item_ix, before.offset_in_item));
}

#[gpui::test]
fn composer_hiding_keeps_minimap_rail_and_markers_in_place(cx: &mut TestAppContext) {
    let (host, visual) = fixture(cx);
    reveal(visual);
    let viewport = host.read_with(visual, |host, _| host.viewport.clone());
    let rail = visual.debug_bounds("chat-minimap").unwrap();
    let bubble = visual.debug_bounds("chat-minimap-marker-20").unwrap();
    let notch = visual.debug_bounds("chat-minimap-marker-19").unwrap();
    for hidden in [80., 160., 0.] {
        host.update(visual, |host, cx| { host.height = gpui_kit::px(320. + hidden); cx.notify(); });
        viewport.update(visual, |view, cx| view.set_composer_hidden(gpui_kit::px(hidden), cx));
        draw(visual);
        let next_rail = visual.debug_bounds("chat-minimap").unwrap();
        assert_eq!((next_rail.origin, next_rail.size), (rail.origin, rail.size));
        for (selector, before) in [("chat-minimap-marker-20", bubble), ("chat-minimap-marker-19", notch)] {
            let after = visual.debug_bounds(selector).unwrap();
            assert_eq!((after.origin, after.size), (before.origin, before.size));
        }
    }
    // A genuine pane resize still changes the available overview height.
    host.update(visual, |host, cx| { host.height = gpui_kit::px(380.); cx.notify(); });
    draw(visual);
    assert_eq!(visual.debug_bounds("chat-minimap").unwrap().size.height, rail.size.height + gpui_kit::px(60.));
}
impl Render for MinimapHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(gpui_kit::px(480.)).h(self.height).flex().flex_col().child(self.viewport.clone().cached(gpui::StyleRefinement::default().size_full()))
    }
}
fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}
fn fixture(cx: &mut TestAppContext) -> (Entity<MinimapHarness>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (host, visual) = cx.add_window_view(|_, cx| {
        let palette = design::palette(&Value::Null, WindowAppearance::Dark);
        let rows = (0..80).map(|row| transcript::ConversationItem::Message(Rc::new(json!({
            "id":format!("message-{row}"), "role":if row % 2 == 0 { "user" } else { "assistant" },
            "phase":if row % 3 == 0 { "final_answer" } else { "commentary" },
            "text":format!("Native minimap message {row}. {}", "Long message text. ".repeat(row % 7))
        })))).collect::<Vec<_>>();
        let state = ListState::new(rows.len(), ListAlignment::Bottom, gpui_kit::px(300.));
        state.set_follow_mode(gpui::FollowMode::Tail);
        let renderer: StableRowRenderer = Rc::new(RefCell::new(|row, _: &mut Window, _: &mut App, scale: f32| {
            div().debug_selector(move || format!("fixture-message-{row}-{}", if scale == 1. { "transcript" } else { "preview" }))
                .h(gpui_kit::px(40. * scale)).w_full().child(format!("Message {row}")).into_any_element()
        }));
        let list = cx.new(|_| StableTranscriptList { state: state.clone(), rows: renderer, minimap: Rc::new(minimap::prepare(&rows, palette)) });
        let viewport = cx.new(|_| TranscriptViewport {
            selection_group: gpui_kit::base::TextSelectionGroupId::new(),
            task_id: "fixture".into(), owner: None,
            load_earlier: Rc::new(|_, _| ()), load_unmeasured_history: Rc::new(|_, _| ()),
            render_count: 0, list: Some(list), error: None, palette,
            geometry: Rc::new(std::cell::Cell::new(TranscriptGeometry::default())),
            minimap_visible: false, minimap_exit: None, minimap_jump: None,
            minimap_emphasis: minimap::Emphasis::default(),
            minimap_bottom_inset: gpui_kit::px(0.),
        });
        let scroll_viewport = viewport.downgrade();
        state.set_scroll_handler(move |_, _, cx| {
            let viewport = scroll_viewport.clone();
            cx.defer(move |cx| { let _ = viewport.update(cx, |view, cx| { view.cancel_minimap_jump(); view.repaint_scroll(cx); }); });
        });
        MinimapHarness { viewport, state, height: gpui_kit::px(320.) }
    });
    visual.update(|window, _| window.activate_window());
    draw(visual);
    (host, visual)
}
fn reveal(visual: &mut VisualTestContext) {
    visual.simulate_event(ScrollWheelEvent {
        position: point(gpui_kit::px(200.), gpui_kit::px(120.)),
        delta: ScrollDelta::Pixels(point(gpui_kit::px(0.), gpui_kit::px(180.))),
        ..Default::default()
    });
    draw(visual);
    // GPUI's one-shot AnimationElement samples Instant::elapsed, while the
    // test executor clock controls timer futures. Settle both clocks.
    std::thread::sleep(Duration::from_millis(180));
    visual.executor().advance_clock(Duration::from_millis(180));
    visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    draw(visual);
}
#[gpui::test]
fn native_wheel_reveals_right_aligned_markers_and_click_navigates(cx: &mut TestAppContext) {
    let (host, visual) = fixture(cx);
    assert!(visual.debug_bounds("chat-minimap").is_none(), "tail starts without the overview");
    reveal(visual);
    let diagnostics = host.read_with(visual, |host, _| format!("end={:?} offset={:?} bounds={:?}", host.state.is_scrolled_to_end(), host.state.logical_scroll_top(), host.state.viewport_bounds()));
    let rail = visual.debug_bounds("chat-minimap").unwrap_or_else(|| panic!("upward wheel reveals rail: {diagnostics}"));
    assert_eq!(rail.size.width, gpui_kit::px(16.));
    let marker = visual.debug_bounds("chat-minimap-marker-20").expect("prepared message marker");
    assert_eq!(marker.size.width, gpui_kit::px(16.));
    assert_eq!(marker.right(), rail.right());
    let bar = visual.debug_bounds("chat-minimap-marker-19").expect("non-bubble marker");
    assert_eq!(bar.size.width, gpui_kit::px(8.));
    assert_eq!(bar.size.height, gpui_kit::px(1.));
    assert_eq!(bar.right(), rail.right());
    assert!(marker.size.height >= gpui_kit::px(3.));
    let position = marker.center();
    visual.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    draw(visual);
    if crate::native_motion::motion_allowed() {
        assert!(host.read_with(visual, |host, _| host.state.logical_scroll_top().item_ix) > 20, "marker click starts an animation");
        std::thread::sleep(Duration::from_millis(100));
        visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
        draw(visual);
        let intermediate = host.read_with(visual, |host, _| host.state.logical_scroll_top().item_ix);
        assert!(intermediate > 20 && intermediate < 67, "animated navigation passes through intermediate history");
        std::thread::sleep(Duration::from_millis(200));
        visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
        draw(visual);
    }
    assert_eq!(host.read_with(visual, |host, _| host.state.logical_scroll_top().item_ix), 20);
    if crate::native_motion::motion_allowed() {
        let second = visual.debug_bounds("chat-minimap-marker-40").expect("second bubble marker").center();
        visual.simulate_mouse_down(second, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(second, MouseButton::Left, Modifiers::default());
        draw(visual);
        assert!(host.read_with(visual, |host, cx| host.viewport.read(cx).minimap_jump.is_some()));
        visual.simulate_event(ScrollWheelEvent {
            position: point(gpui_kit::px(200.), gpui_kit::px(120.)),
            delta: ScrollDelta::Pixels(point(gpui_kit::px(0.), gpui_kit::px(30.))),
            ..Default::default()
        });
        draw(visual);
        assert!(host.read_with(visual, |host, cx| host.viewport.read(cx).minimap_jump.is_none()), "native wheel cancels marker animation");
    }

    host.update(visual, |host, cx| {
        host.state.set_follow_mode(gpui::FollowMode::Tail);
        host.state.scroll_to_end();
        host.viewport.update(cx, |view, cx| { view.cancel_minimap_jump(); view.repaint_scroll(cx); });
    });
    draw(visual);
    std::thread::sleep(Duration::from_millis(180));
    visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap").is_none(), "returning to tail finishes slide-out");
}
#[gpui::test]
fn bubble_hover_opens_interactive_full_text_preview(cx: &mut TestAppContext) {
    let (_, visual) = fixture(cx);
    reveal(visual);
    let marker = visual.debug_bounds("chat-minimap-marker-20").expect("bubble marker");
    visual.simulate_mouse_move(marker.center(), None, Modifiers::default());
    draw(visual);
    draw(visual);
    let grown = visual.debug_bounds("chat-minimap-marker-20").expect("hovered bubble marker");
    assert_eq!(grown.size.width, gpui_kit::px(24.), "bubble expands toward the left");
    assert_eq!(grown.size.height, marker.size.height + gpui_kit::px(4.));
    assert_eq!(grown.right(), marker.right());
    assert_eq!(grown.center().y, marker.center().y);

    visual.executor().advance_clock(Duration::from_millis(499));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_none(), "popup waits the full 500ms hover delay");
    visual.executor().advance_clock(Duration::from_millis(2));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_some(), "popup opens after 501ms");
    std::thread::sleep(Duration::from_millis(180));
    visual.executor().advance_clock(Duration::from_millis(180));
    visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    draw(visual);
    let popup = visual.debug_bounds("chat-minimap-preview").expect("settled popup");
    let rail = visual.debug_bounds("chat-minimap").expect("visible minimap");
    assert!(popup.size.width > gpui_kit::px(20.));
    let preview_message = visual.debug_bounds("fixture-message-20-preview").expect("same row renderer supplies preview");
    assert_eq!(preview_message.size.height, gpui_kit::px(32.), "preview scales the actual row to 80 percent");
    assert!(visual.debug_bounds("fixture-message-19-preview").is_none(), "preview represents the selected bubble");
    assert!(popup.right() < rail.left(), "settled popup finishes entirely left of the minimap");
    visual.simulate_mouse_move(popup.center(), None, Modifiers::default());
    draw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_some(), "preview stays open while pointer enters it");
    let restored = visual.debug_bounds("chat-minimap-marker-20").expect("restored bubble marker");
    assert_eq!(restored.size.width, gpui_kit::px(16.));
    assert_eq!(restored.size.height, marker.size.height);
    visual.simulate_mouse_move(point(gpui_kit::px(200.), gpui_kit::px(280.)), None, Modifiers::default());
    draw(visual);
    visual.executor().advance_clock(Duration::from_millis(300));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_none(), "leaving both marker and popup closes it");
    visual.simulate_mouse_move(restored.center(), None, Modifiers::default());
    draw(visual);
    visual.executor().advance_clock(Duration::from_millis(499));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_none(), "reopening preserves the 500ms delay");
    visual.executor().advance_clock(Duration::from_millis(2));
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").is_some(), "popup can reopen after the delay");
    std::thread::sleep(Duration::from_millis(180));
    visual.executor().advance_clock(Duration::from_millis(180));
    visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    draw(visual);
    assert!(visual.debug_bounds("chat-minimap-preview").unwrap().right() < rail.left());
}

#[gpui::test]
fn dragging_continues_outside_rail_and_releases(cx: &mut TestAppContext) {
    let (host, visual) = fixture(cx);
    reveal(visual);
    let rail = visual.debug_bounds("chat-minimap").expect("revealed rail");
    let x = rail.center().x;
    let marker_bounds = (0..80).filter_map(|index| {
        let selector: &'static str = Box::leak(format!("chat-minimap-marker-{index}").into_boxed_str());
        visual.debug_bounds(selector)
    }).collect::<Vec<_>>();
    let gap = (1..f32::from(rail.size.height) as usize).map(|y| point(x, rail.top() + gpui_kit::px(y as f32)))
        .find(|position| marker_bounds.iter().all(|bounds| !bounds.contains(position))).expect("sparse history has a draggable rail gap");
    visual.simulate_mouse_down(gap, MouseButton::Left, Modifiers::default());
    draw(visual);
    assert!(host.read_with(visual, |host, _| host.state.is_scrollbar_dragging()));
    let outside = point(rail.left() - gpui_kit::px(40.), rail.top() + rail.size.height * 0.2);
    visual.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    draw(visual);
    assert_eq!(host.read_with(visual, |host, _| host.state.logical_scroll_top().item_ix), 16);
    visual.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    draw(visual);
    assert!(!host.read_with(visual, |host, _| host.state.is_scrollbar_dragging()), "outside release clears native drag extent");
}
