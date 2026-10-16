//! Exercise real retained GPUI siblings and native input, with history-sized
//! preparation work counted separately from the virtual list's visible rows.
use super::super::{StableRowRenderer, StableTranscriptList, retain_transcript_list};
use super::{TranscriptPreparationKey, TranscriptViewport, defer_for_task};
use crate::{design, transcript::TimelineRow};
use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ListAlignment, ListState,
    ParentElement as _, InteractiveElement as _, Render, StyleRefinement, Styled as _, Subscription, TestAppContext,
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
    history: Rc<im::Vector<TimelineRow>>,
    key: Option<TranscriptPreparationKey>,
    list: Option<Entity<StableTranscriptList>>,
    viewport: Entity<TranscriptViewport>,
    composer: Entity<TextareaState>,
    fixed_composer_height: bool,
    working: bool,
    animation: Entity<AnimationSibling>,
    preparation_visits: Rc<Cell<usize>>,
    row_renders: Rc<Cell<usize>>,
    input_changes: Rc<Cell<usize>>,
    _input_change: Subscription,
}
fn fixture_key(snapshot: Rc<Value>, rows: Rc<im::Vector<TimelineRow>>) -> TranscriptPreparationKey {
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
                move |index: usize, _: &mut Window, _: &mut App, _: f32| {
                    rendered.set(rendered.get() + 1);
                    let Some(row) = history.get(index) else {
                        return div().into_any_element();
                    };
                    div()
                        .debug_selector(move || format!("isolation-row-{index}"))
                        .h(gpui_kit::px(24.))
                        .child(row.key())
                        .into_any_element()
                },
            ));
            retain_transcript_list(
                &mut self.list,
                ListState::new(self.history.len(), ListAlignment::Bottom, gpui_kit::px(0.)),
                rows,
                Rc::new(Vec::new()),
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
            .child(super::super::working_clearance_spacer(self.working))
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
            selection_group: gpui_kit::base::TextSelectionGroupId::new(),
            task_id: "task".into(),
            owner: None,
            minimap_visible: false,
            minimap_exit: None, minimap_jump: None,
            minimap_emphasis: super::minimap::Emphasis::default(),
            minimap_bottom_inset: gpui_kit::px(0.),
            load_earlier: Rc::new(|_, _| ()),
            load_unmeasured_history: Rc::new(|_, _| ()),
            render_count: 0,
            list: None,
            error: None,
            palette: design::palette(&Value::Null, WindowAppearance::Dark),
            geometry: Rc::new(Cell::new(super::TranscriptGeometry::default())),
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
            working: false,
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
        changed.push_back(TimelineRow::Message(Rc::new(
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
    let rows = Rc::new(im::vector![TimelineRow::Message(Rc::new(json!({"id":"one"})))]);
    let base = fixture_key(snapshot.clone(), rows.clone());
    assert!(base == fixture_key(snapshot.clone(), rows.clone()));
    assert!(base != fixture_key(Rc::new(snapshot.as_ref().clone()), rows.clone()));
    assert!(base != fixture_key(snapshot.clone(), Rc::new(rows.as_ref().clone())));
    let mut boundary = base.clone();
    boundary.history_top_reached = true;
    assert!(base != boundary);
    let mut width = base.clone();
    width.width -= 20.;
    width.companion_width += 20.;
    assert!(base == width, "scalar widths only reflow visible rows");
    let mut compact = base.clone();
    compact.compact_detail = !compact.compact_detail;
    assert!(base != compact, "content mode remains a semantic change");
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
            false,
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

#[cfg(feature = "performance-profiling")]
fn observe_fixture_terminal_events(
    pane: &Entity<super::super::ChatPane>,
    visual: &mut VisualTestContext,
) -> (Rc<RefCell<Vec<(String, Option<String>)>>>, Subscription) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let subscription = visual.update(|_, cx| {
        cx.subscribe(pane, move |_, event: &super::super::ChatEvent, _| {
            if let super::super::ChatEvent::Terminal { task_id, command } = event {
                observed
                    .borrow_mut()
                    .push((task_id.clone(), command.clone()));
            }
        })
    });
    (events, subscription)
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn cleared_slash_draft_cannot_repeat_terminal_and_explicit_send_rechecks_editor(
    cx: &mut TestAppContext,
) {
    let (pane, visual) = mount_retention_pane(cx, Rc::new(retention_snapshot(0)));
    let (events, _subscription) = observe_fixture_terminal_events(&pane, visual);
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            assert!(pane.restore_draft("/terminal echo fixture", window, cx));
            assert!(pane.submit_current_slash_input(cx));
        });
    });
    draw(visual);
    assert_eq!(
        events.borrow().as_slice(),
        &[("quiet-task".into(), Some("echo fixture".into()))]
    );
    pane.update(visual, |pane, cx| {
        assert!(pane.composer.read(cx).value().is_empty());
        assert!(!pane.slash.read(cx).is_open());
        assert!(
            !pane
                .slash
                .update(cx, |slash, cx| slash.submit_typed_input(cx)),
            "programmatic clear must also clear the palette's cached command"
        );
        assert!(
            !pane.submit_current_slash_input(cx),
            "an empty native editor cannot repeat the last terminal command"
        );
        // Simulate a future untracked programmatic writer leaving its derived
        // cache stale. Explicit Send must still treat the editor as authority.
        pane.slash.update(cx, |slash, cx| {
            slash.set_input("/terminal stale command", cx)
        });
        assert!(!pane.submit_current_slash_input(cx));
        assert!(!pane.slash.read(cx).is_open());
    });
    draw(visual);
    assert_eq!(
        events.borrow().len(),
        1,
        "only the original local terminal event is permitted; no process is launched by this fixture"
    );
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn voice_programmatic_draft_updates_palette_arguments_before_submission(cx: &mut TestAppContext) {
    let (pane, visual) = mount_retention_pane(cx, Rc::new(retention_snapshot(0)));
    let (events, _subscription) = observe_fixture_terminal_events(&pane, visual);
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            assert!(pane.restore_draft("/terminal ", window, cx));
            // Emit the captured result through the real subscription. The
            // disconnected fixture never requests recording or transcription.
            pane.voice.update(cx, |_, cx| {
                cx.emit(crate::chat_voice::VoiceComposerEvent::TranscriptReady {
                    task_id: "quiet-task".into(),
                    draft: "/terminal ".into(),
                    text: "echo dictation".into(),
                })
            });
        });
    });
    draw(visual);
    pane.update(visual, |pane, cx| {
        assert_eq!(
            pane.composer.read(cx).value().as_ref(),
            "/terminal \necho dictation"
        );
        assert!(pane.slash.read(cx).is_open());
        // Deliberately use the palette's cached submission rather than the
        // lazy Send helper, proving its displayed draft is already current.
        assert!(
            pane.slash
                .update(cx, |slash, cx| slash.submit_typed_input(cx))
        );
    });
    draw(visual);
    assert_eq!(
        events.borrow().as_slice(),
        &[("quiet-task".into(), Some("echo dictation".into()))]
    );
    pane.read_with(visual, |pane, cx| {
        assert!(pane.composer.read(cx).value().is_empty());
        assert!(!pane.slash.read(cx).is_open());
    });
}


#[cfg(feature = "performance-profiling")]
fn streaming_snapshot() -> Value {
    let mut snapshot = retention_snapshot(0);
    snapshot["messages"] = Value::Array((0..10_000).map(|index| json!({
        "id":format!("stream-{index}"), "taskId":"quiet-task", "role":"assistant",
        "text":format!("Synthetic retained history {index}"), "createdAt":index + 1,
        "streamStatus":if index == 9_980 || index == 9_999 { "streaming" } else { "complete" },
    })).collect());
    snapshot
}

#[cfg(feature = "performance-profiling")]
fn publish_stream_fixture(
    pane: &Entity<super::super::ChatPane>, visual: &mut VisualTestContext, snapshot: &Value,
) {
    pane.update(visual, |pane, cx| {
        pane.sync_prepared(Rc::new(snapshot.clone()), Some("synthetic-stream".into()),
            std::sync::Arc::new(vec![]), std::sync::Arc::new(std::collections::HashMap::new()), true, cx);
        pane.reconcile_history(snapshot["messages"].as_array().unwrap().clone(), false, cx);
    });
    draw(visual);
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn streaming_bodies_reuse_preparation_and_remount_current_text(cx: &mut TestAppContext) {
    let mut snapshot = streaming_snapshot();
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot.clone()));
    visual.update(|window, cx| {
        pane.read(cx).composer.read(cx).focus_handle(cx).focus(window, cx);
    });
    draw(visual);
    visual.simulate_keystrokes("right");
    visual.simulate_input("preserved draft");
    draw(visual);
    let baseline = super::super::transcript_preparation_counts();
    let (renderer, offscreen_body, tail_body, cursor) = pane.read_with(visual, |pane, cx| {
        let offscreen = pane.streaming_bodies["stream-9980"].clone();
        assert_eq!(offscreen.read(cx).render_count, 0, "the older body starts outside the viewport");
        (pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows.clone(), offscreen,
            pane.streaming_bodies["stream-9999"].clone(), pane.composer.read(cx).cursor())
    });
    for version in 1..=3 {
        snapshot["messages"][9_980]["text"] = json!(format!("Offscreen streamed body version {version}"));
        snapshot["messages"][9_999]["text"] = json!(format!("Visible streamed body version {version}"));
        publish_stream_fixture(&pane, visual, &snapshot);
        assert_eq!(super::super::transcript_preparation_counts(), baseline,
            "text-only streaming must perform zero transcript preparations and history visits");
        pane.read_with(visual, |pane, cx| {
            let state = &pane.transcripts["quiet-task"];
            assert!(Rc::ptr_eq(&renderer, &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows));
            assert_eq!(offscreen_body.read(cx).source()["text"], snapshot["messages"][9_980]["text"]);
            assert_eq!(tail_body.read(cx).source()["text"], snapshot["messages"][9_999]["text"]);
            assert!(Rc::ptr_eq(offscreen_body.read(cx).source(), &state.messages[9_980]));
            let TimelineRow::Message(row) = &state.rows[9_980] else { panic!("expected message") };
            assert!(Rc::ptr_eq(row, &state.messages[9_980]), "canonical flat rows must not retain stale bodies");
            let crate::transcript::ConversationItem::Message(projected) = &state.projection[9_980]
                else { panic!("expected projected message") };
            assert!(Rc::ptr_eq(projected, &state.messages[9_980]));
            assert_eq!(pane.composer.read(cx).value().as_ref(), "preserved draft");
            assert_eq!(pane.composer.read(cx).cursor(), cursor);
        });
    }
    pane.update(visual, |pane, cx| {
        // This is the exact action used by the retained Copy click handler.
        pane.copy_message_text("quiet-task", "stream-9980", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "Offscreen streamed body version 3");
    });
    let scroll_position = pane.read_with(visual, |pane, _| {
        pane.decoration_bounds.chat_column.get().unwrap().origin + gpui::point(gpui::px(50.), gpui::px(50.))
    });
    // Unknown virtual rows carry zero cumulative height. Native wheel input
    // reveals another measured slice at each edge; it cannot jump to message0
    // using one huge pixel delta. Reach this nearby offscreen row explicitly.
    for _ in 0..8 {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position:scroll_position, delta:gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(1_000.))),
            ..Default::default()
        });
        draw(visual);
        if offscreen_body.read_with(visual, |body, _| body.render_count) > 0 { break; }
    }
    pane.read_with(visual, |_, cx| {
        assert!(offscreen_body.read(cx).render_count > 0, "the old cached row really remounted in the native viewport");
        assert_eq!(offscreen_body.read(cx).source()["text"], "Offscreen streamed body version 3");
    });
    assert_eq!(super::super::transcript_preparation_counts(), baseline);

    // Completion must rebuild status/actions/markdown even though the worker
    // permits an own-message update in otherwise unchanged row context.
    let tail_renders = tail_body.read_with(visual, |body, _| body.render_count);
    pane.update(visual, |pane, cx| pane.finish_bottom_jump(cx));
    draw(visual);
    assert!(tail_body.read_with(visual, |body, _| body.render_count) > tail_renders,
        "production back-to-bottom must invalidate the retained list and remount its visible tail body");
    assert_eq!(super::super::transcript_preparation_counts(), baseline,
        "a programmatic scroll must reuse prepared transcript wrappers");
    snapshot["messages"][9_999]["streamStatus"] = json!("complete");
    publish_stream_fixture(&pane, visual, &snapshot);
    assert!(super::super::transcript_preparation_counts().0 > baseline.0);
    pane.read_with(visual, |pane, cx| {
        assert_eq!(tail_body.read(cx).source()["streamStatus"], "complete");
        assert!(!Rc::ptr_eq(&renderer, &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows));
    });
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn streaming_bodies_preserve_scrollback_until_reader_follows(cx: &mut TestAppContext) {
    let mut snapshot = streaming_snapshot();
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot.clone()));
    draw(visual);
    let scroll_position = pane.read_with(visual, |pane, _| {
        pane.decoration_bounds.chat_column.get().unwrap().origin + gpui::point(gpui::px(50.), gpui::px(50.))
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position:scroll_position, delta:gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(1_000.))),
        ..Default::default()
    });
    draw(visual);
    let (anchor, body) = pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["quiet-task"];
        assert!(!state.list.is_following_tail());
        (state.list.logical_scroll_top(), pane.streaming_bodies["stream-9999"].clone())
    });
    snapshot["messages"][9_999]["text"] = json!("Held latest body while reader is scrolled back");
    publish_stream_fixture(&pane, visual, &snapshot);
    pane.read_with(visual, |pane, cx| {
        let state = &pane.transcripts["quiet-task"];
        assert!(state.has_pending_live());
        assert_eq!(state.list.logical_scroll_top().item_ix, anchor.item_ix);
        assert_eq!(state.list.logical_scroll_top().offset_in_item, anchor.offset_in_item);
        assert_ne!(state.messages[9_999]["text"], snapshot["messages"][9_999]["text"]);
        assert_ne!(body.read(cx).source()["text"], snapshot["messages"][9_999]["text"]);
    });
    pane.update(visual, |pane, cx| pane.finish_bottom_jump(cx));
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.transcripts["quiet-task"].has_pending_live());
        assert_eq!(body.read(cx).source()["text"], snapshot["messages"][9_999]["text"]);
    });
}


#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn indexed_stream_chunks_hold_scrollback_without_full_history_fallback(cx: &mut TestAppContext) {
    let snapshot = streaming_snapshot();
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot.clone()));
    draw(visual);
    let position = pane.read_with(visual, |pane, _| {
        pane.decoration_bounds.chat_column.get().unwrap().origin
            + gpui::point(gpui::px(50.), gpui::px(50.))
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(1_000.))),
        ..Default::default()
    });
    draw(visual);
    let baseline = super::super::transcript_preparation_counts();
    let (messages, anchor, body) = pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["quiet-task"];
        assert!(!state.list.is_following_tail());
        (state.messages.clone(), state.list.logical_scroll_top(), pane.streaming_bodies["stream-9999"].clone())
    });
    let mut patch = snapshot["messages"][9_999].clone();
    for chunk in 0..20 {
        patch["text"] = json!(format!("Held point update {chunk}"));
        pane.update(visual, |pane, cx| {
            // Production reaches this only after background structural proof.
            pane.transcript_context_unchanged = true;
            assert!(pane.reconcile_body_patches(&[patch.clone()], cx),
                "accepted body patch must not request full-history conversion");
        });
        draw(visual);
    }
    assert_eq!(super::super::transcript_preparation_counts(), baseline);
    pane.read_with(visual, |pane, cx| {
        let state = &pane.transcripts["quiet-task"];
        assert!(state.has_pending_live());
        assert!(Rc::ptr_eq(&state.messages, &messages));
        assert_eq!(state.list.logical_scroll_top().item_ix, anchor.item_ix);
        assert_eq!(state.list.logical_scroll_top().offset_in_item, anchor.offset_in_item);
        assert_ne!(body.read(cx).source()["text"], patch["text"]);
    });
    pane.update(visual, |pane, cx| pane.finish_bottom_jump(cx));
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.transcripts["quiet-task"].has_pending_live());
        assert_eq!(body.read(cx).source()["text"], "Held point update 19");
    });
}

#[cfg(feature = "performance-profiling")]
fn active_computer_fixture_snapshot() -> Value {
    let mut snapshot = retention_snapshot(0);
    snapshot["tasks"][0]["status"] = json!("running");
    let mut history: Vec<_> = (0..100).map(|index| json!({
        "id":format!("old-turn-{index}"), "taskId":"quiet-task", "role":"user",
        "text":"Older synthetic turn", "createdAt":index - 200,
    })).collect();
    history.push(json!({"id":"active-user", "taskId":"quiet-task", "role":"user",
        "text":"Synthetic current turn", "createdAt":100}));
    history.push(json!({"id":"active-body", "taskId":"quiet-task", "role":"assistant",
        "text":"Synthetic body before update", "streamStatus":"streaming", "createdAt":200}));
    snapshot["messages"] = Value::Array(history);
    snapshot["events"] = json!([{"id":"synthetic-computer-start", "taskId":"quiet-task",
        "kind":"computer", "createdAt":150, "detail":json!({
            "id":"fixture-computer", "phase":"started", "summary":"Synthetic active computer",
        }).to_string()}]);
    snapshot
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn activity_body_deferral_skips_only_validated_bodies_and_refreshes_held_new_turn(cx: &mut TestAppContext) {
    let mut snapshot = active_computer_fixture_snapshot();
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot.clone()));
    draw(visual);
    let baseline = pane.read_with(visual, |pane, cx| {
        assert_eq!(pane.activity.read(cx).tools(), &["Synthetic active computer"]);
        pane.activity_context_refreshes
    });
    for version in 1..=3 {
        snapshot["messages"][101]["text"] = json!(format!("Synthetic updated body {version}"));
        publish_stream_fixture(&pane, visual, &snapshot);
        pane.read_with(visual, |pane, cx| {
            assert_eq!(pane.activity_context_refreshes, baseline,
                "the successful exact body proof must cancel activity work, including its deferred fallback");
            assert_eq!(pane.activity.read(cx).tools(), &["Synthetic active computer"]);
            assert_eq!(pane.streaming_bodies["active-body"].read(cx).source()["text"], snapshot["messages"][101]["text"]);
        });
    }
    // Incoming message order, metadata, and completion each fail the proof.
    snapshot["messages"].as_array_mut().unwrap().swap(0, 1);
    publish_stream_fixture(&pane, visual, &snapshot);
    assert_eq!(pane.read_with(visual, |pane, _| pane.activity_context_refreshes), baseline + 1);
    snapshot["messages"][101]["responseMetadata"] = json!({"model":"synthetic-model"});
    publish_stream_fixture(&pane, visual, &snapshot);
    assert_eq!(pane.read_with(visual, |pane, _| pane.activity_context_refreshes), baseline + 2);
    snapshot["messages"][101]["streamStatus"] = json!("complete");
    publish_stream_fixture(&pane, visual, &snapshot);
    assert_eq!(pane.read_with(visual, |pane, _| pane.activity_context_refreshes), baseline + 3);

    let scroll_position = pane.read_with(visual, |pane, _| {
        pane.decoration_bounds.chat_column.get().unwrap().origin + gpui::point(gpui::px(50.), gpui::px(50.))
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position:scroll_position, delta:gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(1_000.))),
        ..Default::default()
    });
    draw(visual);
    let (held_messages, anchor) = pane.read_with(visual, |pane, _| {
        let state = &pane.transcripts["quiet-task"];
        assert!(!state.list.is_following_tail());
        (state.messages.clone(), state.list.logical_scroll_top())
    });
    snapshot["messages"].as_array_mut().unwrap().push(json!({"id":"next-user", "taskId":"quiet-task",
        "role":"user", "text":"Synthetic next turn", "createdAt":300}));
    pane.update(visual, |pane, cx| {
        pane.sync_prepared(Rc::new(snapshot.clone()), Some("synthetic-next-turn".into()),
            std::sync::Arc::new(vec![]), std::sync::Arc::new(std::collections::HashMap::new()), true, cx);
        pane.reconcile_history(snapshot["messages"].as_array().unwrap().clone(), false, cx);
        assert_eq!(pane.activity_context_refreshes, baseline + 4);
        assert!(pane.activity.read(cx).tools().is_empty(),
            "new user current-turn facts refresh immediately despite a frozen scrollback transcript");
        let state = &pane.transcripts["quiet-task"];
        assert!(state.has_pending_live());
        assert!(Rc::ptr_eq(&state.messages, &held_messages));
        assert_eq!(state.list.logical_scroll_top().item_ix, anchor.item_ix);
        assert_eq!(state.list.logical_scroll_top().offset_in_item, anchor.offset_in_item);
    });
    draw(visual);
    assert_eq!(pane.read_with(visual, |pane, _| pane.activity_context_refreshes), baseline + 4,
        "the queued callback must not repeat an already-drained fallback");
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn activity_body_deferral_refreshes_sync_only_and_changed_event_publications(cx: &mut TestAppContext) {
    let mut snapshot = active_computer_fixture_snapshot();
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot.clone()));
    draw(visual);
    let baseline = pane.read_with(visual, |pane, _| pane.activity_context_refreshes);
    snapshot["messages"].as_array_mut().unwrap().push(json!({"id":"next-user", "taskId":"quiet-task",
        "role":"user", "text":"Synthetic sync-only next turn", "createdAt":300}));
    pane.update(visual, |pane, cx| {
        pane.sync_prepared(Rc::new(snapshot.clone()), Some("synthetic-sync-only".into()),
            std::sync::Arc::new(vec![]), std::sync::Arc::new(std::collections::HashMap::new()), true, cx);
        assert!(pane.activity_refresh_pending);
        assert_eq!(pane.activity_context_refreshes, baseline);
    });
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.activity_refresh_pending);
        assert_eq!(pane.activity_context_refreshes, baseline + 1);
        assert!(pane.activity.read(cx).tools().is_empty(), "sync-only callers must not leave old activity facts indefinitely");
    });
    snapshot["events"].as_array_mut().unwrap().push(json!({"id":"next-computer", "taskId":"quiet-task",
        "kind":"computer", "createdAt":350, "detail":json!({
            "id":"fixture-next-computer", "phase":"started", "summary":"Synthetic next computer",
        }).to_string()}));
    pane.update(visual, |pane, cx| {
        // The worker's unchanged-context flag is false when an event changes.
        pane.sync_prepared(Rc::new(snapshot.clone()), Some("synthetic-changed-event".into()),
            std::sync::Arc::new(vec![]), std::sync::Arc::new(std::collections::HashMap::new()), false, cx);
        assert_eq!(pane.activity_context_refreshes, baseline + 2);
        assert_eq!(pane.activity.read(cx).tools(), &["Synthetic next computer"]);
        assert!(!pane.activity_refresh_pending);
    });
    draw(visual);
    assert_eq!(pane.read_with(visual, |pane, _| pane.activity_context_refreshes), baseline + 2);
}

#[gpui::test]
fn working_clearance_moves_native_tail_without_replacing_history(cx: &mut TestAppContext) {
    let (host, visual) = mount(cx, 100);
    draw(visual);
    let list = host.read_with(visual, |host, _| host.list.clone().unwrap());
    let state = list.read_with(visual, |list, _| list.state.clone());
    // Measure the fixture once so viewport comparisons use identical row
    // geometry. Scrollbar extent excludes padding, so use painted tail bounds.
    state.clone().measure_all();
    state.set_follow_mode(gpui::FollowMode::Tail);
    list.update(visual, |_, cx| cx.notify());
    draw(visual);
    assert!(state.is_following_tail(), "fixture starts in production tail-follow mode");
    let idle_tail_bottom = visual.debug_bounds("isolation-row-99").expect("idle tail rendered").bottom();
    host.update(visual, |host, cx| { host.working = true; cx.notify(); });
    draw(visual);
    assert!(state.is_following_tail(), "adding clearance preserves tail follow");
    let working_tail_bottom = visual.debug_bounds("isolation-row-99").expect("working tail rendered").bottom();
    assert_eq!(working_tail_bottom, idle_tail_bottom - design::ui_scaled_px(48.),
        "the final transcript row must actually move above the working controls");
    let viewport_bottom = visual.debug_bounds("retained-transcript-viewport").expect("viewport rendered").bottom();
    assert_eq!(working_tail_bottom, viewport_bottom, "tail fills the shortened viewport");
    let clearance = visual.debug_bounds("working-clearance-spacer").expect("clearance rendered");
    assert_eq!(clearance.top(), viewport_bottom);
    assert_eq!(clearance.size.height, design::ui_scaled_px(48.),
        "working controls have a real 48px empty region below the final row");
    state.scroll_by(gpui_kit::px(-120.));
    list.update(visual, |_, cx| cx.notify());
    draw(visual);
    let scrollback = state.logical_scroll_top();
    host.update(visual, |host, cx| { host.working = false; cx.notify(); });
    draw(visual);
    let after = state.logical_scroll_top();
    assert_eq!(after.item_ix, scrollback.item_ix, "removing clearance preserves the reader's history anchor");
    assert_eq!(after.offset_in_item, scrollback.offset_in_item);
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn response_disclosure_reuses_transcript_preparation_and_renderer(cx: &mut TestAppContext) {
    let mut snapshot = retention_snapshot(0);
    snapshot["messages"][0]["responseMetadata"] = json!({"model":"fixture-model", "inputTokens":12});
    let (pane, visual) = mount_retention_pane(cx, Rc::new(snapshot));
    draw(visual);
    let baseline = super::super::transcript_preparation_counts();
    let renderer = pane.read_with(visual, |pane, cx| pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows.clone());
    for expanded in [true, false] {
        let bounds = visual.debug_bounds("response-metadata-toggle-quiet-message").expect("metadata toggle");
        visual.simulate_click(bounds.center(), Default::default());
        draw(visual);
        assert_eq!(super::super::transcript_preparation_counts(), baseline, "disclosure must not revisit history");
        pane.read_with(visual, |pane, cx| {
            assert_eq!(pane.response_details_expanded.borrow().contains("quiet-message"), expanded);
            assert!(Rc::ptr_eq(&renderer, &pane.detail_transcript_cache.as_ref().unwrap().read(cx).rows));
        });
        assert_eq!(visual.debug_bounds("response-details-quiet-message").is_some(), expanded);
    }
}

#[cfg(feature = "performance-profiling")]
#[gpui::test]
fn hidden_indexed_patch_supersedes_earlier_materialized_history(cx: &mut TestAppContext) {
    use monitter_gpui_desktop::native_publication::{self, PublicationChanges};
    use std::{collections::{HashMap, HashSet}, sync::Arc};
    fn index(source: &Value) -> Arc<native_publication::PublicationIndex> {
        let mut typed = monitter_lib::model::default_snapshot();
        typed.messages = source["messages"].as_array().unwrap().iter()
            .map(|row| serde_json::from_value(row.clone()).unwrap()).collect::<Vec<_>>().into();
        native_publication::prepare_visible(typed, HashSet::from(["quiet-task".into()]),
            HashSet::new(), &PublicationChanges::default(), None).unwrap().index
    }
    struct IndexedVisibilityHarness {
        pane: Entity<super::super::ChatPane>,
        shown: bool,
    }
    impl Render for IndexedVisibilityHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let frame = div().size_full();
            if self.shown {
                frame.child(self.pane.clone().cached(StyleRefinement::default().size_full()))
            } else { frame }
        }
    }
    let initial = retention_snapshot(0);
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let mounted = Rc::new(initial.clone());
    let (host, visual) = cx.add_window_view(move |window, cx| {
        let pane = cx.new(|cx| super::super::ChatPane::performance_fixture("quiet-task".into(), mounted, window, cx));
        IndexedVisibilityHarness { pane, shown:true }
    });
    let pane = host.read_with(visual, |host, _| host.pane.clone());
    draw(visual);
    pane.update(visual, |pane, cx| pane.set_dock_visible(false, cx));
    draw(visual);
    // A hidden production dock removes the cached pane from the rendered tree.
    // A mounted ChatPane root would correctly drain its publication when drawn.
    host.update(visual, |host, cx| { host.shown = false; cx.notify(); });
    draw(visual);
    let canonical = pane.read_with(visual, |pane, _| pane.transcripts["quiet-task"].messages.clone());
    let mut structural = initial.clone();
    structural["messages"].as_array_mut().unwrap().push(json!({
        "id":"new-indexed-body", "taskId":"quiet-task", "role":"assistant",
        "text":"Earlier materialized body", "streamStatus":"streaming", "createdAt":2,
    }));
    let first_index = index(&structural);
    let mut compact = structural.clone();
    compact.as_object_mut().unwrap().remove("messages");
    pane.update(visual, |pane, cx| pane.publish_indexed(Rc::new(compact), Some("indexed-1".into()),
        Arc::new(vec![]), Arc::new(HashMap::new()), false,
        Some(structural["messages"].as_array().unwrap().clone()), false, None,
        Some(first_index), None, true, cx));
    let mut latest = structural.clone();
    latest["messages"][1]["text"] = json!("Latest indexed body after hidden patch");
    let latest_index = index(&latest);
    let patch = latest["messages"][1].clone();
    let mut compact = latest.clone();
    compact.as_object_mut().unwrap().remove("messages");
    pane.update(visual, |pane, cx| pane.publish_indexed(Rc::new(compact), Some("indexed-2".into()),
        Arc::new(vec![]), Arc::new(HashMap::new()), true, None, false, None,
        Some(latest_index), Some(vec![patch]), true, cx));
    pane.read_with(visual, |pane, _| {
        assert!(Rc::ptr_eq(&canonical, &pane.transcripts["quiet-task"].messages), "hidden publications stay deferred");
        assert!(pane.deferred_publication.as_ref().unwrap().history.is_none(), "stale materialized history is discarded");
    });
    pane.update(visual, |pane, cx| pane.set_dock_visible(true, cx));
    host.update(visual, |host, cx| { host.shown = true; cx.notify(); });
    draw(visual);
    pane.read_with(visual, |pane, cx| {
        let rows = &pane.transcripts["quiet-task"].messages;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["text"], latest["messages"][1]["text"]);
        assert_eq!(pane.streaming_bodies["new-indexed-body"].read(cx).source()["text"], latest["messages"][1]["text"]);
        assert!(pane.deferred_publication.is_none());
        assert!(pane.snapshot.get("messages").is_none(), "compact snapshots do not need a global history fallback");
    });
}
