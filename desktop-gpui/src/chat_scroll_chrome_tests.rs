//! Native wheel input preserves editor ownership, prepared history and tail controls.
use super::ChatPane;
use crate::native_motion::{self, MotionPreference};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollDelta, ScrollWheelEvent, StyleRefinement, Styled as _, TestAppContext, VisualTestContext,
    Window, div, point, px, size,
};
use serde_json::json;
use std::rc::Rc;

struct ScrollHarness {
    pane: Entity<ChatPane>,
}
impl Render for ScrollHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.pane
                .clone()
                .cached(StyleRefinement::default().size_full()),
        )
    }
}
fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}
fn wheel(visual: &mut VisualTestContext, position: gpui_kit::Point<gpui_kit::Pixels>, delta: f32) {
    visual.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        ..Default::default()
    });
    draw(visual);
    draw(visual);
}

struct HoverScrollHarness {
    pane: Entity<ChatPane>,
    other: gpui_kit::FocusHandle,
}
impl Render for HoverScrollHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.pane.read(cx).focus.clone();
        let tracked = focus.clone();
        let other = self.other.clone();
        // Match NativePanel's focus-follows-mouse listener around its cached chat.
        div().size_full().flex()
            .child(div().flex_1().min_w_0().h_full().track_focus(&tracked)
                .on_mouse_move(move |event, window, cx| {
                    if event.pressed_button.is_none() && !focus.contains_focused(window, cx) {
                        focus.focus(window, cx);
                    }
                })
                .child(self.pane.clone().cached(StyleRefinement::default().size_full())))
            .child(div().id("other-pane").w(px(80.)).h_full().track_focus(&self.other)
                .on_mouse_move(move |_, window, cx| other.focus(window, cx)))
    }
}

#[gpui_kit::test]
fn scrolled_composer_stays_hidden_through_pane_hover_and_draft(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        native_motion::initialize_for_ui_fixture(cx);
    });
    let source = Rc::new(json!({
        "settings":{"theme":"dark","interfaceFontSize":14},
        "tasks":[{"id":"hover-scroll","agentId":"agent","title":"Hover scroll",
            "provider":"codex","status":"idle","queue":[]}],
        "agents":[{"id":"agent","provider":"codex","name":"Codex"}],
        "messages":(0..80).map(|index| json!({"id":format!("row-{index}"),
            "taskId":"hover-scroll","role":"user","text":format!("History row {index}. Keep the reading position."),
            "createdAt":1_700_000_000_000_i64 + index})).collect::<Vec<_>>(),
        "events":[],"approvalRequests":[],"hosts":[],"channels":[]
    }));
    let mut retained = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let pane = cx.new(|cx| ChatPane::performance_fixture("hover-scroll".into(), source, window, cx));
        pane.read(cx).transcripts["hover-scroll"].list.clone().measure_all();
        retained = Some(pane.clone());
        let harness = cx.new(|cx| HoverScrollHarness { pane, other: cx.focus_handle() });
        gpui_kit::component::Root::new(harness, window, cx)
    });
    visual.simulate_resize(size(px(780.), px(800.)));
    let pane = retained.unwrap();
    visual.update(|window, _| window.activate_window());
    for draft in ["", "Keep this unsent draft"] {
        visual.update(|window, cx| {
            pane.update(cx, |pane, cx| pane.finish_bottom_jump(cx));
            pane.read(cx).composer.read(cx).focus_handle(cx).focus(window, cx);
        });
        draw(visual);
        if !draft.is_empty() { visual.simulate_input(draft); }
        draw(visual);
        let position = point(px(300.), px(250.));
        for _ in 0..3 { wheel(visual, position, 600.); }
        let (offset, distance) = pane.read_with(visual, |pane, _| {
            (pane.transcripts["hover-scroll"].list.logical_scroll_top(), pane.composer_scroll_distance)
        });
        for hovered in [false, true, false, true] {
            visual.simulate_mouse_move(if hovered { position } else { point(px(740.), px(250.)) }, None, Default::default());
            for _ in 0..24 {
                visual.executor().advance_clock(std::time::Duration::from_millis(25));
                visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
            }
            let composer = visual.debug_bounds("normal-chat-composer").expect("retained composer");
            let bounds = pane.read_with(visual, |pane, _| pane.decoration_bounds.pane.get().unwrap());
            assert!(composer.top() >= bounds.bottom() - px(1.),
                "scrolled composer must remain outside pane on hover, hovered={hovered}, draft={draft:?}: {composer:?}, pane={bounds:?}");
            pane.read_with(visual, |pane, cx| {
                assert!(!pane.transcripts["hover-scroll"].list.is_following_tail());
                let after = pane.transcripts["hover-scroll"].list.logical_scroll_top();
                assert_eq!((after.item_ix, after.offset_in_item), (offset.item_ix, offset.offset_in_item),
                    "pane hover must preserve the reading position");
                assert_eq!(pane.composer_scroll_distance, distance);
                assert!(distance > px(0.));
                assert_eq!(pane.composer.read(cx).value().as_ref(), draft);
            });
        }
        pane.update(visual, |pane, cx| pane.finish_bottom_jump(cx));
        draw(visual);
        draw(visual);
        let composer = visual.debug_bounds("normal-chat-composer").unwrap();
        assert!(composer.top() < px(800.), "return to bottom restores composer");
    }
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
        pane.read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    });
    draw(visual);
    draw(visual);
    visual.simulate_keystrokes("right");
    visual.simulate_input("Keep this draft and its cursor");
    draw(visual);
    let (position, cursor, sidebar) = pane.read_with(visual, |pane, cx| {
        (
            pane.decoration_bounds.chat_column.get().unwrap().origin + point(px(50.), px(50.)),
            pane.composer.read(cx).cursor(),
            pane.secondary_sidebar_state(),
        )
    });
    // Prime pointer modality and fully hide the composer before measuring.
    for _ in 0..3 {
        wheel(visual, position, 600.);
    }
    let (baseline, offset, distance, renderer) = pane.read_with(visual, |pane, cx| {
        let height = pane
            .decoration_bounds
            .working_anchor
            .get()
            .unwrap()
            .size
            .height;
        assert!(pane.composer_scroll_distance > height);
        assert!(!pane.transcripts["scroll-task"].list.is_following_tail());
        (
            pane.chrome_render_count,
            pane.transcripts["scroll-task"].list.logical_scroll_top(),
            pane.composer_scroll_distance,
            pane.detail_transcript_cache
                .as_ref()
                .unwrap()
                .read(cx)
                .rows
                .clone(),
        )
    });
    wheel(visual, position, 80.);
    pane.read_with(visual, |pane, cx| {
        let after = pane.transcripts["scroll-task"].list.logical_scroll_top();
        assert_ne!(
            (after.item_ix, after.offset_in_item),
            (offset.item_ix, offset.offset_in_item),
            "native wheel input must still move the retained virtual list"
        );
        assert_ne!(
            pane.composer_scroll_distance, distance,
            "retain the full jump distance"
        );
        assert!(
            Rc::ptr_eq(
                &renderer,
                &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows
            ),
            "native scrolling must reuse prepared transcript wrappers"
        );
        assert_eq!(
            pane.composer.read(cx).value().as_ref(),
            "Keep this draft and its cursor"
        );
        assert_eq!(pane.composer.read(cx).cursor(), cursor);
        assert_eq!(pane.secondary_sidebar_state(), sidebar);
    });
    visual.update(|window, cx| {
        assert!(
            pane.read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });

    for _ in 0..4 {
        wheel(visual, position, -100_000.);
    }
    pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["scroll-task"];
        assert!(
            state.list.is_following_tail(),
            "native wheel return to bottom resumes tail follow"
        );
        assert_eq!(pane.composer_scroll_distance, px(0.));
        assert!(!state.history_top_reached);
        assert!(
            pane.chrome_render_count > baseline,
            "tail state still invalidates chrome"
        );
    });

    // Reaching the history boundary still rebuilds and mounts its pager chrome.
    pane.update(visual, |pane, cx| {
        pane.transcripts["scroll-task"]
            .list
            .scroll_to(gpui_kit::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        pane.invalidate_transcript_list(cx);
    });
    draw(visual);
    wheel(visual, position, 20.);
    wheel(visual, position, 20.);
    pane.read_with(visual, |pane, _| {
        assert!(pane.transcripts["scroll-task"].history_top_reached)
    });
    assert!(
        visual
            .debug_bounds("chat-history-boundary-controls")
            .is_some(),
        "history boundary controls remain visible"
    );

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
        assert!(
            pane.chrome_render_count > baseline,
            "tail and pager state still invalidate chrome"
        );
        assert_eq!(pane.secondary_sidebar_state(), sidebar);
    });
}

#[gpui_kit::test]
fn native_scroll_preserves_prepared_history_draft_focus_tail_pager_and_sidebar(
    cx: &mut TestAppContext,
) {
    assert_native_scroll_preserves_input(cx, MotionPreference::On);
}

#[gpui_kit::test]
fn reduced_motion_scroll_preserves_prepared_history_draft_focus_tail_pager_and_sidebar(
    cx: &mut TestAppContext,
) {
    assert_native_scroll_preserves_input(cx, MotionPreference::Off);
}

#[gpui_kit::test]
fn project_status_bar_stays_within_chat_pane(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        native_motion::initialize_for_ui_fixture(cx);
    });
    cx.update(|cx| native_motion::set_preference_for_ui_fixture(MotionPreference::On, cx));
    let snapshot = Rc::new(json!({
        "settings":{"theme":"dark","interfaceScale":100,"interfaceFontSize":14},
        "tasks":[{"id":"project-chat","projectId":"project","agentId":"agent",
            "title":"Project chat","provider":"codex","status":"idle","queue":[]}],
        "projects":[{"id":"project","name":"Project","buildManagementEnabled":true}],
        "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
        "messages":[],"events":[],"approvalRequests":[],"buildRequests":[],
        "buildRuns":[],"hosts":[],"channels":[],
        "collaborations":[{"id":"child-work","kind":"delegation","fromTaskId":"project-chat",
            "toTaskId":"child-task","toAgentId":"agent","text":"Child work","status":"running"}],
        "subagentSessions":[]
    }));
    let mut retained = None;
    let (_, mut visual) = cx.add_window_view(|window, cx| {
        let pane = cx.new(|cx| ChatPane::performance_fixture(
            "project-chat".into(), snapshot.clone(), window, cx,
        ));
        retained = Some(pane.clone());
        let harness = cx.new(|_| ScrollHarness { pane });
        gpui_kit::component::Root::new(harness, window, cx)
    });
    visual.simulate_resize(size(px(700.), px(600.)));
    draw(&mut visual);
    draw(&mut visual);
    let pane = retained.expect("chat pane mounted");
    let viewport = visual.update(|_, cx| {
        pane.read(cx).decoration_bounds.pane.get().expect("pane measured")
    });
    let header = visual.debug_bounds("chat-pane-header").expect("chat header mounted");
    let bar = visual.debug_bounds("pane-drawer-toolbar").expect("project bar mounted");
    let composer = visual.debug_bounds("normal-chat-composer");
    let transcript = visual.debug_bounds("retained-transcript-viewport");
    assert!(bar.top() >= header.bottom(), "project bar follows chat header: {bar:?}, {header:?}");
    assert!(bar.bottom() <= viewport.bottom() + px(1.),
        "project bar must remain inside the visible chat pane: {bar:?}, {viewport:?}, header={header:?}, composer={composer:?}, transcript={transcript:?}");
    assert!(bar.bottom() >= viewport.bottom() - px(100.),
        "project bar must stay near the bottom of the chat pane: {bar:?}, {viewport:?}");
    let before_open = visual.update(|_, cx| pane.read(cx).chrome_render_count);
    let position = bar.origin + point(bar.size.width - px(25.), bar.size.height / 2.);
    visual.simulate_mouse_down(position, gpui_kit::MouseButton::Left, Default::default());
    draw(&mut visual);
    visual.simulate_mouse_up(position, gpui_kit::MouseButton::Left, Default::default());
    for _ in 0..4 {
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    }
    assert!(visual.update(|_, cx| pane.read(cx).companion.read(cx).visor_open()),
        "Build Queue button opens its drawer");
    assert!(visual.update(|_, cx| pane.read(cx).chrome_render_count) > before_open,
        "drawer animation must invalidate its cached parent layout");
    let drawer = visual.debug_bounds("build-queue-drawer").expect("build queue drawer rendered");
    assert!(drawer.size.height > px(50.) && drawer.bottom() <= viewport.bottom() + px(1.),
        "opened drawer must be visible inside the pane: {drawer:?}, {viewport:?}");
    let notifications = Rc::new(std::cell::Cell::new(0));
    let observed = notifications.clone();
    let _parent_changed = visual.update(|_, cx| cx.observe(&pane, move |_, cx| {
        observed.set(observed.get() + 1);
        let _ = cx;
    }));
    visual.update(|_, cx| {
        let child = pane.read(cx).companion.clone();
        child.update(cx, |_, cx| cx.notify());
    });
    visual.run_until_parked();
    assert!(notifications.get() > 0, "open drawer invalidation must reach the cached parent");
    let bar = visual.debug_bounds("pane-drawer-toolbar").unwrap();
    let position = bar.origin + point(px(25.), bar.size.height / 2.);
    visual.simulate_mouse_down(position, gpui_kit::MouseButton::Left, Default::default());
    draw(&mut visual);
    visual.simulate_mouse_up(position, gpui_kit::MouseButton::Left, Default::default());
    for _ in 0..4 {
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.update(|window, cx| { window.simulate_next_frame(cx); window.draw(cx).clear(cx); });
    }
    let drawer = visual.debug_bounds("subagent-drawer-list").expect("Subagents button switches drawer");
    assert!(drawer.size.height > px(50.) && drawer.bottom() <= viewport.bottom() + px(1.),
        "subagent drawer must be visible inside the pane: {drawer:?}, {viewport:?}");
    let bar = visual.debug_bounds("pane-drawer-toolbar").unwrap();
    let position = bar.origin + point(px(25.), bar.size.height / 2.);
    visual.simulate_mouse_down(position, gpui_kit::MouseButton::Left, Default::default());
    draw(&mut visual);
    visual.simulate_mouse_up(position, gpui_kit::MouseButton::Left, Default::default());
    draw(&mut visual);
    draw(&mut visual);
    assert!(!visual.update(|_, cx| pane.read(cx).companion.read(cx).visor_open()),
        "second Subagents click closes the drawer");
}
