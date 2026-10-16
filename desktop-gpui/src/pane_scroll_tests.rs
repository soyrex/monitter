//! Scroll-anchor regression for two retained transcript panes.
use super::{DockWorkspace, PaneFactory, PaneSpec};
use crate::transcript::{ConversationItem, TranscriptState};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, div,
};
use gpui_kit::{
    ListAlignment, ScrollDelta, ScrollWheelEvent,
    component::{
        Placement,
        button::Button,
        dock::{DockPlacement, InsertTarget},
    },
    test::TestWindowExt as _,
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

struct TranscriptFixture {
    state: TranscriptState,
}

fn transcript_fixture(prefix: &str) -> TranscriptFixture {
    let messages: Vec<Value> = (0..100)
        .map(|index| {
            let text = if index % 4 == 0 {
                format!(
                    "## {prefix} response {index}\n\n{}",
                    "Variable-height markdown content. ".repeat(35)
                )
            } else {
                format!("{prefix} response {index} with **markdown** content.")
            };
            json!({
                "id": format!("{prefix}-{index}"),
                "role": "user",
                "text": text,
                "createdAt": index,
            })
        })
        .collect();
    let mut state = TranscriptState::default();
    state.merge_page(&json!({"messages": messages}));
    TranscriptFixture { state }
}

struct TranscriptColumn {
    name: &'static str,
    transcript: Entity<TranscriptFixture>,
    bounds: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
}

impl Render for TranscriptColumn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ChatPane calls sync_projection while rendering. Keep that behavior in
        // this fixture so an unrelated pane redraw can expose an anchor reset.
        let (list_state, projection) = self.transcript.update(cx, |fixture, _| {
            fixture.state.sync_projection(&[]);
            (fixture.state.list.clone(), fixture.state.projection.clone())
        });
        let name = self.name;
        let bounds_slot = self.bounds.clone();
        div().size_full().flex().flex_col().min_h_0().child(
            div()
                .id(format!("transcript-{name}"))
                .relative()
                .px_5()
                .py_3()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap_4()
                .on_scroll_wheel(|_: &ScrollWheelEvent, _, _| {})
                .child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |layout_bounds, _, _, _| {
                            *bounds_slot.borrow_mut() = Some(layout_bounds)
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    gpui_kit::list(list_state, move |index, _, _| {
                        let text = projection
                            .get(index)
                            .and_then(|item| match item {
                                ConversationItem::Message(message) => {
                                    message["text"].as_str().map(str::to_owned)
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| "Transcript activity".to_owned());
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                gpui_kit::component::text::TextView::markdown(
                                    format!("{name}-message-{index}"),
                                    text,
                                )
                                .selectable(true)
                                .scrollable(false)
                                .w_full(),
                            )
                            .child(
                                Button::new(format!("{name}-row-{index}")).label("Message action"),
                            )
                            .into_any_element()
                    })
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .h_full(),
                ),
        )
    }
}

struct TwoTranscriptDock {
    workspace: Entity<DockWorkspace>,
}

impl Render for TwoTranscriptDock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.workspace.clone())
    }
}

#[gpui_kit::test]
fn split_transcripts_keep_wheel_position_across_projection_refresh(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let left_capture = Rc::new(RefCell::new(None::<Entity<TranscriptFixture>>));
    let right_capture = Rc::new(RefCell::new(None::<Entity<TranscriptFixture>>));
    let left_bounds_capture = Rc::new(RefCell::new(None::<gpui::Bounds<gpui::Pixels>>));
    let right_bounds_capture = Rc::new(RefCell::new(None::<gpui::Bounds<gpui::Pixels>>));
    let left_slot = left_capture.clone();
    let right_slot = right_capture.clone();
    let left_bounds_slot = left_bounds_capture.clone();
    let right_bounds_slot = right_bounds_capture.clone();

    let (harness, cx) = cx.add_window_view(move |window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| DockWorkspace::new("scroll-regression", factory, window, cx));

        let left_transcript = cx.new(|_| transcript_fixture("left"));
        *left_slot.borrow_mut() = Some(left_transcript.clone());
        let left_content = cx.new(|_| TranscriptColumn {
            name: "left",
            transcript: left_transcript,
            bounds: left_bounds_slot,
        });
        let left_focus = cx.focus_handle();
        workspace.update(cx, |workspace, cx| {
            workspace
                .add(
                    PaneSpec {
                        id: "chat:left-scroll".into(),
                        title: "Left chat".into(),
                        content: left_content.into(),
                        focus: left_focus,
                        lifecycle: None,
                        closable: true,
                    },
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
        });

        let right_transcript = cx.new(|_| transcript_fixture("right"));
        *right_slot.borrow_mut() = Some(right_transcript.clone());
        let right_content = cx.new(|_| TranscriptColumn {
            name: "right",
            transcript: right_transcript,
            bounds: right_bounds_slot,
        });
        let right_focus = cx.focus_handle();
        workspace.update(cx, |workspace, cx| {
            workspace
                .add(
                    PaneSpec {
                        id: "chat:right-scroll".into(),
                        title: "Right chat".into(),
                        content: right_content.into(),
                        focus: right_focus,
                        lifecycle: None,
                        closable: true,
                    },
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            // Build two actual center-workspace columns. DockPlacement::Right
            // creates a collapsed edge dock, so it is not a second visible
            // transcript for a wheel regression.
            workspace.focus("chat:left-scroll", window, cx);
            let group = workspace.focused_group_node(window, cx).unwrap();
            assert!(workspace.move_pane(
                "chat:right-scroll",
                InsertTarget::Split {
                    node: group,
                    placement: Placement::Right,
                    size: None,
                },
                window,
                cx,
            ));
        });
        let content = cx.new(|_| TwoTranscriptDock { workspace });
        gpui_kit::component::Root::new(content, window, cx)
    });

    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    harness.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let left = left_capture.borrow().as_ref().unwrap().clone();
    let right = right_capture.borrow().as_ref().unwrap().clone();
    let left_bounds = left_bounds_capture
        .borrow()
        .expect("left transcript bounds");
    let right_bounds = right_bounds_capture
        .borrow()
        .expect("right transcript bounds");
    let left_center = left_bounds.center();
    let right_center = right_bounds.center();
    assert!(
        left_bounds.size.width > gpui::px(0.)
            && left_bounds.size.height > gpui::px(0.)
            && right_bounds.size.width > gpui::px(0.)
            && right_bounds.size.height > gpui::px(0.),
        "the rendered transcript columns must have nonempty layout bounds: left={left_bounds:?}, right={right_bounds:?}"
    );
    for (column, position, target, other) in [
        ("left", left_center, &left, &right),
        ("right", right_center, &right, &left),
    ] {
        let target_list = target.read_with(cx, |fixture, _| fixture.state.list.clone());
        let other_list = other.read_with(cx, |fixture, _| fixture.state.list.clone());
        assert!(
            target_list.is_scrolled_to_end().is_none(),
            "variable-height unmeasured rows reproduce GPUI's unknown-height state"
        );
        let before = target_list.logical_scroll_top();
        let other_before = other_list.logical_scroll_top();
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(120.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after_wheel = target_list.logical_scroll_top();
        assert!(
            (after_wheel.item_ix, after_wheel.offset_in_item)
                != (before.item_ix, before.offset_in_item),
            "the hovered transcript should respond to its wheel event: column={column}, bounds=({left_bounds:?}, {right_bounds:?}), before={before:?}, after={after_wheel:?}, end={:?}",
            target_list.is_scrolled_to_end()
        );
        assert!(!target_list.is_following_tail());
        assert_eq!(
            (
                other_list.logical_scroll_top().item_ix,
                other_list.logical_scroll_top().offset_in_item
            ),
            (other_before.item_ix, other_before.offset_in_item),
            "wheel events should not move the other transcript column"
        );

        // The ordinary transcript refresh invokes sync_projection again with
        // an unchanged projection while ListState still has unknown heights.
        target.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after_refresh = target_list.logical_scroll_top();
        assert_eq!(
            (after_refresh.item_ix, after_refresh.offset_in_item),
            (after_wheel.item_ix, after_wheel.offset_in_item),
            "unchanged projection refresh must not snap an unknown-height reader to tail"
        );
        assert!(!target_list.is_following_tail());

        // A genuine projection append must also preserve the user's stable
        // anchor while GPUI still cannot calculate total height.
        target.update(cx, |fixture, cx| {
            let mut messages = fixture.state.messages.iter().cloned().collect::<Vec<_>>();
            messages.push(json!({
                "id": format!("{column}-new"),
                "role": "user",
                "text": "A new live row after the reader moved away from the tail.",
                "createdAt": 101,
            }).into());
            fixture.state.merge_page(&json!({"messages": messages}));
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after_append = target_list.logical_scroll_top();
        assert_eq!(
            (after_append.item_ix, after_append.offset_in_item),
            (after_refresh.item_ix, after_refresh.offset_in_item),
            "changed projection should keep the reader anchored when tail-follow is paused"
        );
        assert!(!target_list.is_following_tail());
    }
}
