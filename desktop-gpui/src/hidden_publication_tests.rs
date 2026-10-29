//! Detached native fixtures for latest-only hidden publication and reveal.
use super::{ChatPane, transcript_preparation_counts};
use crate::{streaming_display, transcript::HistoryBoundary};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, StyleRefinement,
    Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use serde_json::{Value, json};
use std::{collections::HashMap, rc::Rc, sync::Arc};

fn snapshot(text: &str, status: &str) -> Value {
    json!({
        "settings":{"theme":"dark", "accent":"#6594db"},
        "tasks":[{"id":"hidden-fixture", "agentId":"fixture-agent", "title":"Synthetic hidden publication",
            "provider":"codex", "status":"running", "queue":[], "hostId":"fixture-host", "cwd":""}],
        "agents":[{"id":"fixture-agent", "name":"Original synthetic agent", "provider":"codex"}],
        "messages":[{"id":"fixture-user", "taskId":"hidden-fixture", "role":"user", "text":"Synthetic prompt", "createdAt":1},
            {"id":"fixture-body", "taskId":"hidden-fixture", "role":"assistant", "text":text, "streamStatus":status, "createdAt":2}],
        "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
    })
}
fn mount(cx: &mut TestAppContext, source: Value) -> (Entity<ChatPane>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
        // Direct global installation has no streaming preference file IO.
        cx.set_global(streaming_display::Selection {
            options: streaming_display::Options {
                mode: streaming_display::Mode::Typing,
                characters_per_second: 600,
            },
            error: None,
        });
    });
    cx.add_window_view(move |window, cx| {
        ChatPane::performance_fixture("hidden-fixture".into(), Rc::new(source), window, cx)
    })
}
fn mount_cached(
    cx: &mut TestAppContext,
    source: Value,
) -> (
    Entity<ChatPane>,
    Entity<CachedVisibilityHarness>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
        cx.set_global(streaming_display::Selection {
            options: streaming_display::Options {
                mode: streaming_display::Mode::Typing,
                characters_per_second: 600,
            },
            error: None,
        });
    });
    let (host, visual) = cx.add_window_view(move |window, cx| {
        let pane = cx.new(|cx| {
            ChatPane::performance_fixture("hidden-fixture".into(), Rc::new(source), window, cx)
        });
        CachedVisibilityHarness { pane, shown: true }
    });
    let pane = host.read_with(visual, |host, _| host.pane.clone());
    (pane, host, visual)
}
fn hide_cached(
    pane: &Entity<ChatPane>,
    host: &Entity<CachedVisibilityHarness>,
    visual: &mut VisualTestContext,
) {
    pane.update(visual, |pane, cx| pane.set_dock_visible(false, cx));
    draw(visual);
    host.update(visual, |host, cx| {
        host.shown = false;
        cx.notify();
    });
    draw(visual);
}
fn show_cached(host: &Entity<CachedVisibilityHarness>, visual: &mut VisualTestContext) {
    host.update(visual, |host, cx| {
        host.shown = true;
        cx.notify();
    });
}

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}
fn publish(
    pane: &Entity<ChatPane>,
    visual: &mut VisualTestContext,
    source: &Value,
    revision: &str,
    context_unchanged: bool,
    history: bool,
    reset: bool,
    boundary: Option<Option<String>>,
) {
    pane.update(visual, |pane, cx| {
        pane.publish_prepared(
            Rc::new(source.clone()),
            Some(revision.into()),
            Arc::new(vec![]),
            Arc::new(HashMap::new()),
            context_unchanged,
            history.then(|| source["messages"].as_array().unwrap().clone()),
            reset,
            boundary,
            cx,
        );
    });
}

#[gpui::test]
fn hidden_metadata_keeps_retained_viewport_prepared(cx: &mut TestAppContext) {
    let initial = snapshot("Stable synthetic output", "complete");
    let (pane, host, visual) = mount_cached(cx, initial.clone());
    draw(visual);
    let prepared = visual.update(|window, cx| {
        pane.update(cx, |pane, cx| pane.prepare_visible_transcript(window, cx))
    });
    assert!(prepared, "fixture must begin with an accepted viewport");
    hide_cached(&pane, &host, visual);

    let mut metadata = initial.clone();
    metadata["tasks"][0]["updatedAt"] = json!(3);
    publish(&pane, visual, &metadata, "metadata-only", true, false, false, None);
    assert!(
        visual.update(|window, cx| pane.read(cx).transcript_viewport_ready(window, cx)),
        "an unchanged hidden transcript should stay prepared while controls await reveal"
    );
    assert!(pane.read_with(visual, |pane, _| pane.deferred_publication.is_some()));

    publish(&pane, visual, &metadata, "history-dirty", true, true, false, None);
    assert!(
        !visual.update(|window, cx| pane.read(cx).transcript_viewport_ready(window, cx)),
        "pending history must still block the prepared path"
    );
    pane.update(visual, |pane, cx| pane.set_dock_visible(true, cx));
    show_cached(&host, visual);
    draw(visual);
    assert!(pane.read_with(visual, |pane, _| pane.deferred_publication.is_none()));
}

#[gpui::test]
fn hidden_publication_many_chunks_reveal_latest_completion_without_catchup(
    cx: &mut TestAppContext,
) {
    let initial = snapshot("Initial synthetic output", "streaming");
    let (pane, host, visual) = mount_cached(cx, initial.clone());
    draw(visual);
    let (canonical, body, activity_refreshes) = pane.read_with(visual, |pane, _| {
        (
            pane.transcripts["hidden-fixture"].messages.clone(),
            pane.streaming_bodies["fixture-body"].clone(),
            pane.activity_context_refreshes,
        )
    });
    hide_cached(&pane, &host, visual);
    let baseline = transcript_preparation_counts();
    for version in 1..=256 {
        let latest = snapshot(
            &format!("Synthetic hidden chunk {version}"),
            if version == 256 {
                "complete"
            } else {
                "streaming"
            },
        );
        publish(
            &pane,
            visual,
            &latest,
            &format!("synthetic-{version}"),
            true,
            true,
            false,
            None,
        );
    }
    pane.read_with(visual, |pane, cx| {
        assert!(
            Rc::ptr_eq(&canonical, &pane.transcripts["hidden-fixture"].messages),
            "hidden chunks must not reconcile native history"
        );
        assert_eq!(
            pane.activity_context_refreshes, activity_refreshes,
            "hidden chunks must not refresh native activity"
        );
        assert_eq!(
            body.read(cx).source()["text"],
            initial["messages"][1]["text"]
        );
        let pending = pane.deferred_publication.as_ref().unwrap();
        assert_eq!(pending.revision.as_deref(), Some("synthetic-256"));
        assert_eq!(
            pending.history.as_ref().unwrap()[1]["text"],
            "Synthetic hidden chunk 256"
        );
        assert_eq!(
            pending.history.as_ref().unwrap()[1]["streamStatus"],
            "complete"
        );
        assert!(!body.read(cx).has_pending_reveal());
    });
    assert_eq!(transcript_preparation_counts(), baseline);
    pane.update(visual, |pane, cx| {
        pane.set_dock_visible(true, cx);
        assert!(pane.deferred_publication.is_none());
        assert_eq!(
            body.read(cx).source()["text"],
            "Synthetic hidden chunk 256",
            "canonical source is installed before visibility enables reveal"
        );
        assert_eq!(body.read(cx).source()["streamStatus"], "complete");
        assert!(
            !body.read(cx).has_pending_reveal(),
            "completion received while hidden must never replay typing backlog"
        );
        pane.copy_message_text("hidden-fixture", "fixture-body", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "Synthetic hidden chunk 256"
        );
    });
    show_cached(&host, visual);
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        assert_eq!(pane.revision.as_deref(), Some("synthetic-256"));
        assert!(!body.read(cx).has_pending_reveal());
    });
}

#[gpui::test]
fn hidden_publication_metadata_guard_stays_invalidated_and_new_bodies_seed_latest(
    cx: &mut TestAppContext,
) {
    let (pane, host, visual) = mount_cached(cx, snapshot("Initial synthetic output", "streaming"));
    draw(visual);
    let baseline = transcript_preparation_counts();
    hide_cached(&pane, &host, visual);
    let mut metadata = snapshot("Initial synthetic output", "streaming");
    metadata["agents"][0]["name"] = json!("New synthetic agent label");
    metadata["approvalRequests"] = json!([{"id":"fixture-approval", "taskId":"hidden-fixture", "provider":"codex",
        "status":"pending", "input":null, "createdAt":3, "tool":"Synthetic approval", "summary":"Synthetic request", "risk":"low"}]);
    publish(
        &pane,
        visual,
        &metadata,
        "metadata-2",
        false,
        false,
        false,
        None,
    );
    let mut latest = metadata;
    latest["messages"][1]["text"] = json!("Latest output after hidden metadata change");
    latest["messages"].as_array_mut().unwrap().push(json!({"id":"new-hidden-body", "taskId":"hidden-fixture", "role":"assistant",
        "text":"Entire synthetic body created while hidden", "streamStatus":"streaming", "createdAt":4}));
    publish(&pane, visual, &latest, "body-3", true, true, false, None);
    pane.read_with(visual, |pane, _| {
        assert!(
            !pane
                .deferred_publication
                .as_ref()
                .unwrap()
                .context_unchanged,
            "a later body-only flag cannot erase an earlier deferred metadata invalidation"
        );
    });
    pane.update(visual, |pane, cx| pane.set_dock_visible(true, cx));
    show_cached(&host, visual);
    draw(visual);
    assert!(
        transcript_preparation_counts().0 > baseline.0,
        "the changed label/approval metadata requires fresh row preparation"
    );
    pane.read_with(visual, |pane, cx| {
        assert_eq!(
            pane.snapshot["agents"][0]["name"],
            "New synthetic agent label"
        );
        assert_eq!(
            pane.pending_tool_approval().unwrap()["id"],
            "fixture-approval",
            "approval UI must use the latest source on reveal"
        );
        let new_body = &pane.streaming_bodies["new-hidden-body"];
        assert_eq!(
            new_body.read(cx).source()["text"],
            "Entire synthetic body created while hidden"
        );
        assert!(
            !new_body.read(cx).has_pending_reveal(),
            "new bodies must seed their full hidden output on first visible render"
        );
    });
}

#[gpui::test]
fn hidden_publication_reset_clears_old_cursor_and_preserves_latest_history_through_metadata(
    cx: &mut TestAppContext,
) {
    let (pane, host, visual) = mount_cached(cx, snapshot("Initial synthetic output", "streaming"));
    draw(visual);
    hide_cached(&pane, &host, visual);
    let initial = snapshot("Before reset", "streaming");
    publish(
        &pane,
        visual,
        &initial,
        "before-reset",
        true,
        true,
        false,
        Some(Some("fixture-user".into())),
    );
    let mut reset = snapshot("Synthetic replacement journal", "complete");
    reset["messages"][0]["id"] = json!("reset-user");
    reset["messages"][1]["id"] = json!("reset-body");
    publish(&pane, visual, &reset, "reset-1", false, true, true, None);
    reset["agents"][0]["name"] = json!("Metadata after reset");
    publish(&pane, visual, &reset, "reset-2", false, false, false, None);
    pane.read_with(visual, |pane, _| {
        let pending = pane.deferred_publication.as_ref().unwrap();
        assert!(pending.reset);
        assert!(
            pending.boundary.is_none(),
            "a newer reset cannot retain an earlier page cursor"
        );
        assert_eq!(
            pending.history.as_ref().unwrap()[0]["id"],
            "reset-user",
            "a metadata-only publication cannot erase the latest authoritative history"
        );
    });
    // This is the explicit-page completion ordering used by the desktop:
    // install deferred journal state before merging the accepted older page.
    pane.update(visual, |pane, cx| {
        pane.apply_deferred_publication(cx);
        assert!(matches!(pane.transcripts["hidden-fixture"].boundary, HistoryBoundary::Unknown));
        pane.accept_history_page(Ok(json!({"messages":[{"id":"accepted-page", "taskId":"hidden-fixture", "role":"user",
            "text":"Synthetic accepted older page", "createdAt":0}], "nextBeforeId":"accepted-page"})), cx);
        assert_eq!(pane.transcripts["hidden-fixture"].before_id().as_deref(), Some("accepted-page"));
        pane.set_dock_visible(true, cx);
        assert_eq!(pane.transcripts["hidden-fixture"].messages[0]["id"], "accepted-page",
            "revealing cannot reinstall the queued pre-page history over the accepted page");
        assert!(pane.deferred_publication.is_none());
    });
    show_cached(&host, visual);
    draw(visual);
    pane.read_with(visual, |pane, _| {
        assert_eq!(pane.transcripts["hidden-fixture"].messages.len(), 3);
        assert_eq!(
            pane.transcripts["hidden-fixture"].before_id().as_deref(),
            Some("accepted-page")
        );
    });
}

#[gpui::test]
fn hidden_publication_reveal_preserves_offtail_reader_and_held_latest_output(
    cx: &mut TestAppContext,
) {
    let mut initial = snapshot("Frozen synthetic body", "streaming");
    let mut history: Vec<_> = (0..100)
        .map(|index| {
            json!({"id":format!("older-{index}"), "taskId":"hidden-fixture",
        "role":"user", "text":"Older synthetic scrollback", "createdAt":index - 200})
        })
        .collect();
    history.extend(initial["messages"].as_array().unwrap().clone());
    initial["messages"] = Value::Array(history);
    let (pane, visual) = mount(cx, initial.clone());
    draw(visual);
    let position = pane.read_with(visual, |pane, _| {
        pane.decoration_bounds.chat_column.get().unwrap().origin
            + gpui::point(gpui::px(50.), gpui::px(50.))
    });
    // A two-row transcript fits entirely in the window, so layout legitimately
    // clamps a fabricated scroll offset to its tail. Use actual scrollable
    // history and a native wheel gesture to establish the held reader instead.
    visual.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(1_000.))),
        ..Default::default()
    });
    draw(visual);
    let (canonical, anchor, body) = pane.update(visual, |pane, cx| {
        let state = &pane.transcripts["hidden-fixture"];
        assert!(!state.list.is_following_tail());
        let held = (
            state.messages.clone(),
            state.list.logical_scroll_top(),
            pane.streaming_bodies["fixture-body"].clone(),
        );
        pane.set_dock_visible(false, cx);
        held
    });
    let mut latest = initial;
    latest["messages"][101]["text"] = json!("Latest held hidden completion");
    latest["messages"][101]["streamStatus"] = json!("complete");
    publish(&pane, visual, &latest, "held-2", true, true, false, None);
    pane.update(visual, |pane, cx| {
        pane.set_dock_visible(true, cx);
        let state = &pane.transcripts["hidden-fixture"];
        assert!(Rc::ptr_eq(&canonical, &state.messages));
        assert!(state.has_pending_live());
        assert!(!state.list.is_following_tail());
        assert_eq!(state.list.logical_scroll_top().item_ix, anchor.item_ix);
        assert_eq!(
            state.list.logical_scroll_top().offset_in_item,
            anchor.offset_in_item
        );
        assert_eq!(
            body.read(cx).source()["text"],
            "Frozen synthetic body",
            "reveal cannot replace a reader's held transcript"
        );
        pane.finish_bottom_jump(cx);
    });
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        let state = &pane.transcripts["hidden-fixture"];
        assert!(!state.has_pending_live());
        assert!(state.list.is_following_tail());
        assert_eq!(state.messages[101]["text"], "Latest held hidden completion");
        assert_eq!(body.read(cx).source()["streamStatus"], "complete");
    });
}

struct CachedVisibilityHarness {
    pane: Entity<ChatPane>,
    shown: bool,
}
impl Render for CachedVisibilityHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().when(self.shown, |frame| {
            // Match NativePanel's actual retained chat cache boundary.
            frame.child(
                self.pane
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        })
    }
}

#[gpui::test]
fn hidden_publication_cached_first_reveal_paints_latest_without_visibility_callback(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
        cx.set_global(streaming_display::Selection {
            options: streaming_display::Options {
                mode: streaming_display::Mode::Typing,
                characters_per_second: 600,
            },
            error: None,
        });
    });
    let (host, visual) = cx.add_window_view(|window, cx| {
        let pane = cx.new(|cx| {
            ChatPane::performance_fixture(
                "hidden-fixture".into(),
                Rc::new(snapshot("Initial cached synthetic body", "streaming")),
                window,
                cx,
            )
        });
        CachedVisibilityHarness { pane, shown: true }
    });
    draw(visual);
    let pane = host.read_with(visual, |host, _| host.pane.clone());
    let body = pane.read_with(visual, |pane, _| {
        pane.streaming_bodies["fixture-body"].clone()
    });
    // First settle the hide notification while the chat is still mounted.
    // Removing it afterwards preserves a clean cached scene with unchanged
    // geometry/focus; a reveal cannot rely on those incidental cache misses.
    pane.update(visual, |pane, cx| pane.set_dock_visible(false, cx));
    draw(visual);
    host.update(visual, |host, cx| {
        host.shown = false;
        cx.notify();
    });
    draw(visual);
    let (canonical, baseline_renders) = pane.read_with(visual, |pane, cx| {
        (
            pane.transcripts["hidden-fixture"].messages.clone(),
            body.read(cx).render_count,
        )
    });
    for version in 1..=32 {
        publish(
            &pane,
            visual,
            &snapshot(
                &format!("Latest cached hidden version {version}"),
                if version == 32 {
                    "complete"
                } else {
                    "streaming"
                },
            ),
            "cached-latest",
            true,
            true,
            false,
            None,
        );
        draw(visual);
        pane.read_with(visual, |pane, cx| {
            assert!(Rc::ptr_eq(
                &canonical,
                &pane.transcripts["hidden-fixture"].messages
            ));
            assert_eq!(
                body.read(cx).render_count,
                baseline_renders,
                "hidden chunks cannot render/reconcile the cached native body"
            );
        });
    }
    host.update(visual, |host, cx| {
        host.shown = true;
        cx.notify();
    });
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.dock_visible, "the post-paint dock visibility callback has not fired in this fixture");
        assert!(pane.deferred_publication.is_none(), "first-slot invalidation must reach ChatPane::render before cache replay can paint stale output");
        assert_eq!(body.read(cx).source()["text"], "Latest cached hidden version 32");
        assert_eq!(body.read(cx).source()["streamStatus"], "complete");
        assert!(body.read(cx).render_count > baseline_renders, "the first revealed frame actually rendered the current native body");
        assert!(!body.read(cx).has_pending_reveal());
    });
    pane.update(visual, |pane, cx| pane.set_dock_visible(true, cx));
    assert!(!body.read_with(visual, |body, _| body.has_pending_reveal()));
}
