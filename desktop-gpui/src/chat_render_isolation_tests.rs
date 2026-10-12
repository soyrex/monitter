//! Exercise real retained GPUI siblings and native input, with history-sized
//! preparation work counted separately from the virtual list's visible rows.
use super::super::{StableRowRenderer, StableTranscriptList, retain_transcript_list};
use super::{TranscriptPreparationKey, TranscriptViewport, defer_for_task};
use crate::{design, transcript::TimelineRow};
use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ListAlignment, ListState,
    ParentElement as _, Render, StyleRefinement, Styled as _, Subscription, TestAppContext,
    VisualTestContext, Window, WindowAppearance, div,
};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct AnimationSibling {
    ticks: usize,
}
impl Render for AnimationSibling {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(gpui_kit::px(20.))
            .child(format!("frame {}", self.ticks))
    }
}
struct IsolationHarness {
    snapshot: Rc<Value>,
    history: Rc<Vec<TimelineRow>>,
    key: Option<TranscriptPreparationKey>,
    list: Option<Entity<StableTranscriptList>>,
    viewport: Entity<TranscriptViewport>,
    composer: Entity<TextareaState>,
    fixed_composer_height: bool,
    animation: Entity<AnimationSibling>,
    preparation_visits: Rc<Cell<usize>>,
    row_renders: Rc<Cell<usize>>,
    input_changes: Rc<Cell<usize>>,
    _input_change: Subscription,
}
fn fixture_key(snapshot: Rc<Value>, rows: Rc<Vec<TimelineRow>>) -> TranscriptPreparationKey {
    TranscriptPreparationKey {
        snapshot: Rc::downgrade(&snapshot),
        rows: Some(rows),
        selected: Some("task".into()),
        palette: design::palette(&Value::Null, WindowAppearance::Dark),
        interface_family: ".SystemUIFont".into(),
        appearance: WindowAppearance::Dark,
        typography: None,
        scale: 100,
        width: 800.,
        companion_width: 0.,
        companion_visible: false,
        compact_detail: false,
        history_error: None,
        loading: false,
        complete: true,
        history_top_reached: false,
        refreshing: false,
        disclosure_generation: 0,
        date: chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
    }
}
impl Render for IsolationHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = fixture_key(self.snapshot.clone(), self.history.clone());
        if self.key.as_ref() != Some(&key) {
            // This represents date/avatar/entity preparation in ChatPane. It
            // deliberately traverses history, so the test fails if a keystroke
            // accidentally invalidates its production preparation key.
            for _ in self.history.iter() {
                self.preparation_visits
                    .set(self.preparation_visits.get() + 1);
            }
            let history = self.history.clone();
            let rendered = self.row_renders.clone();
            let rows: StableRowRenderer = Rc::new(RefCell::new(
                move |index: usize, _: &mut Window, _: &mut App| {
                    rendered.set(rendered.get() + 1);
                    let Some(row) = history.get(index) else {
                        return div().into_any_element();
                    };
                    div()
                        .h(gpui_kit::px(24.))
                        .child(row.key())
                        .into_any_element()
                },
            ));
            retain_transcript_list(
                &mut self.list,
                ListState::new(self.history.len(), ListAlignment::Bottom, gpui_kit::px(0.)),
                rows,
                cx,
            );
            let list = self.list.clone();
            self.viewport.update(cx, |viewport, cx| {
                viewport.set_content("task".into(), list, None, key.palette, cx);
            });
            self.key = Some(key);
        }
        let composer = if self.fixed_composer_height {
            div()
                .h(gpui_kit::px(48.))
                .flex_shrink_0()
                .child(Textarea::new(&self.composer))
                .into_any_element()
        } else {
            Textarea::new(&self.composer).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                self.viewport
                    .clone()
                    .cached(StyleRefinement::default().flex_1().min_h_0()),
            )
            .child(
                self.animation
                    .clone()
                    .cached(StyleRefinement::default().h(gpui_kit::px(20.)).w_full()),
            )
            .child(composer)
    }
}
fn mount(
    cx: &mut TestAppContext,
    row_count: usize,
) -> (Entity<IsolationHarness>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    cx.add_window_view(move |window, cx| {
        let composer = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 12));
        let input_changes = Rc::new(Cell::new(0));
        let changes = input_changes.clone();
        let change = cx.subscribe(&composer, move |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                changes.set(changes.get() + 1);
                // Same ancestor invalidation as the real draft-persistence
                // subscriber. The cached transcript sibling must survive it.
                cx.notify();
            }
        });
        composer.read(cx).focus_handle(cx).focus(window, cx);
        let viewport = cx.new(|_| TranscriptViewport {
            task_id: "task".into(),
            load_earlier: Rc::new(|_, _| ()),
            load_unmeasured_history: Rc::new(|_, _| ()),
            render_count: 0,
            list: None,
            error: None,
            palette: design::palette(&Value::Null, WindowAppearance::Dark),
        });
        IsolationHarness {
            snapshot: Rc::new(json!({"tasks":[{"id":"task"}]})),
            history: Rc::new(
                (0..row_count)
                    .map(|index| {
                        TimelineRow::Message(Rc::new(json!({"id":format!("row-{index}")})))
                    })
                    .collect(),
            ),
            key: None,
            list: None,
            viewport,
            composer,
            fixed_composer_height: true,
            animation: cx.new(|_| AnimationSibling { ticks: 0 }),
            preparation_visits: Rc::new(Cell::new(0)),
            row_renders: Rc::new(Cell::new(0)),
            input_changes,
            _input_change: change,
        }
    })
}
fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}
fn assert_typing_isolated(cx: &mut TestAppContext, count: usize) {
    let (host, visual) = mount(cx, count);
    draw(visual);
    let mounted = host.read_with(visual, |host, cx| {
        assert!(
            host.row_renders.get() > 0,
            "the real virtual list must have nonzero bounds and render visible history rows"
        );
        (
            host.preparation_visits.get(),
            host.viewport.read(cx).render_count,
        )
    });
    // GPUI explicitly refreshes the entire window when input modality first
    // switches from Mouse to Keyboard (for focus-visible and hover styling).
    // Prime that transition with a real nonediting native key, then measure
    // ordinary typing separately from the required global refresh.
    visual.simulate_keystrokes("right");
    draw(visual);
    let baseline = host.read_with(visual, |host, cx| {
        assert_eq!(
            host.preparation_visits.get(),
            mounted.0,
            "the required keyboard-modality refresh must not reprepare history"
        );
        assert_eq!(
            host.input_changes.get(),
            0,
            "priming modality cannot edit the draft"
        );
        assert!(host.composer.read(cx).value().is_empty());
        (
            host.preparation_visits.get(),
            host.viewport.read(cx).render_count,
            host.row_renders.get(),
        )
    });
    for letter in ["t", "y", "p", "i", "n", "g"] {
        visual.simulate_input(letter);
        host.update(visual, |host, cx| {
            host.animation.update(cx, |animation, cx| {
                animation.ticks += 1;
                cx.notify();
            });
        });
        draw(visual);
        host.read_with(visual, |host, cx| {
            assert_eq!(
                host.preparation_visits.get(),
                baseline.0,
                "native key {letter:?} and its animation frame cannot visit history"
            );
            assert_eq!(
                host.viewport.read(cx).render_count,
                baseline.1,
                "native key {letter:?} and its animation frame must replay the viewport"
            );
            assert_eq!(
                host.row_renders.get(),
                baseline.2,
                "native key {letter:?} and its animation frame cannot rerender visible rows"
            );
        });
    }
    host.read_with(visual, |host, cx| {
        assert_eq!(host.composer.read(cx).value().as_ref(), "typing");
        assert_eq!(
            host.input_changes.get(),
            6,
            "real native changes must reach draft persistence"
        );
        assert_eq!(
            host.preparation_visits.get(),
            baseline.0,
            "ordinary typing/animation cannot revisit even one history row"
        );
        assert_eq!(
            host.viewport.read(cx).render_count,
            baseline.1,
            "composer and animation siblings cannot rerender the transcript viewport"
        );
    });
    // Content changes must still update the retained transcript after typing.
    host.update(visual, |host, cx| {
        let mut changed = host.history.as_ref().clone();
        changed.push(TimelineRow::Message(Rc::new(
            json!({"id":"new-live-message"}),
        )));
        host.history = Rc::new(changed);
        cx.notify();
    });
    draw(visual);
    host.read_with(visual, |host, cx| {
        assert_eq!(host.preparation_visits.get(), baseline.0 + count + 1);
        assert!(host.viewport.read(cx).render_count > baseline.1);
    });
}
#[gpui::test]
fn typing_and_animation_reuse_transcript_with_small_history(cx: &mut TestAppContext) {
    assert_typing_isolated(cx, 10);
}
#[gpui::test]
fn typing_and_animation_reuse_transcript_with_large_history(cx: &mut TestAppContext) {
    assert_typing_isolated(cx, 50_000);
}
#[gpui::test]
fn composer_growth_and_editing_preserve_transcript_preparation(cx: &mut TestAppContext) {
    let (host, visual) = mount(cx, 1_000);
    host.update(visual, |host, cx| {
        host.fixed_composer_height = false;
        cx.notify();
    });
    draw(visual);
    let visits = host.read_with(visual, |host, _| host.preparation_visits.get());
    visual.simulate_input("line one\nline two\nline three");
    draw(visual);
    visual.simulate_keystrokes("backspace");
    draw(visual);
    host.read_with(visual, |host, cx| {
        assert_eq!(
            host.composer.read(cx).value().as_ref(),
            "line one\nline two\nline thre"
        );
        assert_eq!(
            host.preparation_visits.get(),
            visits,
            "a resized viewport may redraw visible rows but cannot reprepare history"
        );
    });
}

#[::core::prelude::v1::test]
fn preparation_key_tracks_sources_and_semantic_context_without_deep_equality() {
    let snapshot = Rc::new(json!({"tasks":[]}));
    let rows = Rc::new(vec![TimelineRow::Message(Rc::new(json!({"id":"one"})))]);
    let base = fixture_key(snapshot.clone(), rows.clone());
    assert!(base == fixture_key(snapshot.clone(), rows.clone()));
    assert!(base != fixture_key(Rc::new(snapshot.as_ref().clone()), rows.clone()));
    assert!(base != fixture_key(snapshot.clone(), Rc::new(rows.as_ref().clone())));
    let mut boundary = base.clone();
    boundary.history_top_reached = true;
    assert!(base != boundary);
    let mut disclosure = base.clone();
    disclosure.disclosure_generation += 1;
    assert!(base != disclosure);
    let mut width = base.clone();
    width.width -= 20.;
    assert!(base != width);
    let mut error = base.clone();
    error.history_error = Some("history unavailable".into());
    assert!(base != error);
}

struct DeferredTaskHarness {
    selected: String,
    requested: Rc<Cell<usize>>,
}
#[gpui::test]
fn deferred_history_scroll_cannot_fetch_after_task_selection_changes(cx: &mut TestAppContext) {
    let requests = Rc::new(Cell::new(0));
    let count = requests.clone();
    let owner = cx.new(|_| DeferredTaskHarness {
        selected: "original-task".into(),
        requested: count,
    });
    cx.update(|cx| {
        defer_for_task(
            owner.downgrade(),
            "original-task".into(),
            |owner| Some(&owner.selected),
            |owner, _| owner.requested.set(owner.requested.get() + 1),
            cx,
        );
        owner.update(cx, |owner, _| owner.selected = "new-task".into());
    });
    cx.run_until_parked();
    assert_eq!(
        requests.get(),
        0,
        "old scroll must not request the newly selected task's history"
    );
    cx.update(|cx| {
        defer_for_task(
            owner.downgrade(),
            "new-task".into(),
            |owner| Some(&owner.selected),
            |owner, _| owner.requested.set(owner.requested.get() + 1),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        requests.get(),
        1,
        "a current task scroll must still perform its history action"
    );
}

/// This uses the real ChatPane draft-restoration method and native textarea
/// change subscription. The fixture facade is permanently disconnected; the
/// terminal event below is observed locally and never dispatched to a process.
#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn restored_slash_draft_tracks_native_edits_and_rejects_stale_restore(cx: &mut TestAppContext) {
    use super::super::{ChatEvent, ChatPane};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let snapshot = Rc::new(json!({
        "settings": {"theme":"dark", "accent":"#6594db"},
        "tasks": [{"id":"synthetic-restore-task", "agentId":"synthetic-restore-agent",
            "title":"Synthetic draft restore", "provider":"codex", "status":"idle", "queue":[]}],
        "agents": [{"id":"synthetic-restore-agent", "name":"Synthetic fixture agent", "provider":"codex"}],
        "messages":[], "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
    }));
    let (pane, visual) = cx.add_window_view(move |window, cx| {
        let mut pane =
            ChatPane::performance_fixture("synthetic-restore-task".into(), snapshot, window, cx);
        assert!(!pane.slash.read(cx).is_open());
        assert!(pane.restore_draft("/terminal ", window, cx));
        assert_eq!(pane.composer.read(cx).value().as_ref(), "/terminal ");
        assert!(
            pane.slash.read(cx).is_open(),
            "restoring a slash draft must synchronize its palette immediately"
        );
        assert!(!pane.draft_touched, "restoration is not a local edit");
        // Native multiline set_value resets the caret to the start. This
        // scenario explicitly chooses to continue typing after the restored
        // command rather than changing production restore focus/caret policy.
        assert_eq!(pane.composer.read(cx).cursor(), 0);
        pane.composer.update(cx, |input, cx| {
            input.set_cursor_position(gpui_kit::component::input::Position::new(0, 10), window, cx)
        });
        assert_eq!(pane.composer.read(cx).cursor(), 10);
        pane
    });
    let terminals = Rc::new(RefCell::new(Vec::new()));
    let observed = terminals.clone();
    let _terminal_observer = visual.update(|_, cx| {
        cx.subscribe(&pane, move |_, event: &ChatEvent, _| {
            if let ChatEvent::Terminal { task_id, command } = event {
                observed
                    .borrow_mut()
                    .push((task_id.clone(), command.clone()));
            }
        })
    });
    draw(visual);
    visual.simulate_input("echo fixture");
    draw(visual);
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            assert!(
                pane.draft_touched,
                "the real native change event must mark local ownership of the draft"
            );
            assert_eq!(
                pane.composer.read(cx).value().as_ref(),
                "/terminal echo fixture"
            );
            assert!(pane.slash.read(cx).is_open());
            assert!(!pane.restore_draft("/terminal stale", window, cx));
            assert_eq!(
                pane.composer.read(cx).value().as_ref(),
                "/terminal echo fixture",
                "a later persisted draft cannot overwrite the locally edited value"
            );
            assert!(
                pane.slash
                    .update(cx, |slash, cx| slash.submit_typed_input(cx))
            );
        });
    });
    draw(visual);
    assert_eq!(
        terminals.borrow().as_slice(),
        &[(
            "synthetic-restore-task".to_owned(),
            Some("echo fixture".to_owned()),
        )],
        "palette submission must use the restored task and the current native edit, never the stale restore"
    );
}

#[cfg(feature = "performance-profiling")]
fn retention_snapshot(unrelated_rows: usize) -> Value {
    let mut messages = vec![json!({
        "id":"quiet-message", "taskId":"quiet-task", "role":"assistant",
        "text":"Quiet task history", "createdAt":1,
    })];
    let payload = "Unrelated resident history. ".repeat(64);
    messages.extend((0..unrelated_rows).map(|index| {
        json!({
            "id":format!("retired-task-{index}"), "taskId":"retired-task",
            "role":"assistant", "text":payload, "createdAt":index as i64 + 2,
        })
    }));
    json!({
        "settings":{"theme":"dark", "accent":"#6594db"},
        "tasks":[{"id":"quiet-task", "agentId":"quiet-agent", "title":"Quiet task",
            "provider":"codex", "status":"idle", "queue":[], "hostId":"fixture-host", "cwd":""}],
        "agents":[{"id":"quiet-agent", "name":"Original fixture agent", "provider":"codex"}],
        "messages":messages, "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
    })
}

#[cfg(feature = "performance-profiling")]
fn mount_retention_pane(
    cx: &mut TestAppContext,
    snapshot: Rc<Value>,
) -> (Entity<super::super::ChatPane>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    cx.add_window_view(move |window, cx| {
        super::super::ChatPane::performance_fixture("quiet-task".into(), snapshot, window, cx)
    })
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn unaffected_publication_releases_old_global_history_without_repreparing_transcript(
    cx: &mut TestAppContext,
) {
    let original = Rc::new(retention_snapshot(10_000));
    let old_global = Rc::downgrade(&original);
    let (pane, visual) = mount_retention_pane(cx, original);
    draw(visual);
    let renderer = pane.read_with(visual, |pane, cx| {
        pane.detail_transcript_cache
            .as_ref()
            .unwrap()
            .read(cx)
            .rows
            .clone()
    });
    let latest = Rc::new(retention_snapshot(0));
    pane.update(visual, |pane, cx| {
        pane.advance_revision(latest.clone(), Some("synthetic-2".into()), cx);
        // Exercise a later ancestor render too. The unaffected source update
        // itself performs no notify, but ordinary input can invalidate it.
        cx.notify();
    });
    assert!(
        old_global.upgrade().is_none(),
        "the pane, companion, and preparation key must release the old global snapshot and its 10k unrelated rows"
    );
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        let after = &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows;
        assert!(
            Rc::ptr_eq(&renderer, after),
            "unrelated publication cannot replace the prepared row renderer"
        );
        assert!(std::rc::Weak::ptr_eq(
            &pane.transcript_preparation_key.as_ref().unwrap().snapshot,
            &Rc::downgrade(&latest)
        ));
        assert_eq!(pane.revision.as_deref(), Some("synthetic-2"));
        assert_eq!(pane.transcripts["quiet-task"].messages.len(), 1);
    });
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn hidden_substantive_sync_still_requires_preparation_after_unaffected_publication(
    cx: &mut TestAppContext,
) {
    let original = Rc::new(retention_snapshot(10_000));
    let old_global = Rc::downgrade(&original);
    let (pane, visual) = mount_retention_pane(cx, original);
    draw(visual);
    let renderer = pane.read_with(visual, |pane, cx| {
        pane.detail_transcript_cache
            .as_ref()
            .unwrap()
            .read(cx)
            .rows
            .clone()
    });
    let mut changed = retention_snapshot(0);
    changed["agents"][0]["name"] = json!("New fixture agent label");
    let substantive = Rc::new(changed.clone());
    let latest = Rc::new(changed);
    pane.update(visual, |pane, cx| {
        // No intervening draw: a hidden pane's prepared renderer is still
        // bound to the original agent label when a quiet update follows.
        pane.set_dock_visible(false, cx);
        pane.sync_prepared(
            substantive.clone(),
            Some("synthetic-2".into()),
            std::sync::Arc::new(vec![]),
            std::sync::Arc::new(std::collections::HashMap::new()),
            cx,
        );
        pane.advance_revision(latest.clone(), Some("synthetic-3".into()), cx);
        assert!(
            !std::rc::Weak::ptr_eq(
                &pane.transcript_preparation_key.as_ref().unwrap().snapshot,
                &Rc::downgrade(&latest)
            ),
            "advancing an unrelated source cannot erase the hidden substantive invalidation"
        );
        pane.set_dock_visible(true, cx);
    });
    assert!(
        old_global.upgrade().is_none(),
        "an invalid preparation key preserves identity without retaining historical payloads"
    );
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        let after = &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows;
        assert!(
            !Rc::ptr_eq(&renderer, after),
            "the newly visible pane must prepare its changed metadata exactly when rendered"
        );
        assert!(std::rc::Weak::ptr_eq(
            &pane.transcript_preparation_key.as_ref().unwrap().snapshot,
            &Rc::downgrade(&latest)
        ));
        assert_eq!(
            pane.snapshot["agents"][0]["name"],
            "New fixture agent label"
        );
    });
}
