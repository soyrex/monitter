//! Explicit profiling-build-only native UI. Synthetic data, detached backend,
//! in-memory drafts, and no disk-backed preferences or agent commands.
use crate::{
    chat::ChatPane,
    design,
    docking::{DockWorkspace, PaneSpec},
    streaming_display,
};
use gpui_kit::component::{
    ActiveTheme as _, Placement,
    dock::{DockPlacement, InsertTarget},
};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Render, Styled as _, TitlebarOptions, Window, WindowBounds, WindowOptions, canvas, div, point,
    prelude::*, rgb, size,
};
use monitter_gpui_desktop::native_publication::{
    self, PreparedChildTranscripts, PreparedPublication, PublicationChanges, PublicationIndex,
    PublicationQueue,
};
use monitter_lib::model::Snapshot;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

// Fixture-only observer: no timers, repaint requests, or focus changes.
thread_local! {
    static WINDOW_OBSERVATION: std::cell::RefCell<Option<WindowObservation>> = const { std::cell::RefCell::new(None) };
    static WORKLOAD_UPDATES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

struct WindowObservation {
    started: std::time::Instant,
    last_seconds: f64,
    active: bool,
    visible: bool,
    active_seconds: f64,
    visible_seconds: f64,
    all_active_visible: bool,
    transitions: Vec<Value>,
    workload_start: u64,
}
impl WindowObservation {
    fn record(&mut self, seconds: f64, active: bool, visible: bool) {
        let elapsed = (seconds - self.last_seconds).max(0.0);
        if self.active { self.active_seconds += elapsed; }
        if self.visible { self.visible_seconds += elapsed; }
        if active != self.active || visible != self.visible {
            self.transitions.push(json!({"elapsedSeconds": seconds, "active": active, "visible": visible}));
        }
        self.last_seconds = seconds;
        self.active = active;
        self.visible = visible;
        self.all_active_visible &= active && visible;
    }
}

fn observe_window_conditions(window: &Window) {
    WINDOW_OBSERVATION.with_borrow_mut(|observation| {
        if let Some(observation) = observation {
            observation.record(observation.started.elapsed().as_secs_f64(), window.is_window_active(), window.is_visible());
        }
    });
}

pub(crate) fn begin_window_observation(window: &Window) {
    let active = window.is_window_active();
    let visible = window.is_visible();
    WINDOW_OBSERVATION.with_borrow_mut(|slot| {
        *slot = Some(WindowObservation {
            started: std::time::Instant::now(), last_seconds: 0.0, active, visible,
            active_seconds: 0.0, visible_seconds: 0.0, all_active_visible: active && visible,
            transitions: vec![json!({"elapsedSeconds": 0.0, "active": active, "visible": visible})],
            workload_start: WORKLOAD_UPDATES.get(),
        });
    });
}

pub(crate) fn finish_window_observation(window: &Window) -> Value {
    observe_window_conditions(window);
    WINDOW_OBSERVATION.with_borrow_mut(|slot| {
        let Some(observation) = slot.take() else { return Value::Null; };
        json!({
            "schemaVersion": 1,
            "durationSeconds": observation.last_seconds,
            "activeSeconds": observation.active_seconds,
            "visibleSeconds": observation.visible_seconds,
            "fullyActiveVisible": observation.all_active_visible,
            "transitions": observation.transitions,
            "workloadUpdateCallbacks": WORKLOAD_UPDATES.get().saturating_sub(observation.workload_start),
            "workloadCounterMeaning": "streaming-body: fixture update callbacks delivered (body existence is not proven); pipeline-updates: publications committed; idle: zero",
            "limitation": "Platform activation/occlusion callbacks bracket the measured observation; partial covering is still visible. No physical display or workload correctness guarantee."
        })
    })
}

fn record_workload_update() {
    WORKLOAD_UPDATES.set(WORKLOAD_UPDATES.get().saturating_add(1));
}

#[cfg(test)]
mod window_observation_tests {
    use super::*;

    fn observation(active: bool, visible: bool) -> WindowObservation {
        WindowObservation {
            started: std::time::Instant::now(), last_seconds: 0.0, active, visible,
            active_seconds: 0.0, visible_seconds: 0.0, all_active_visible: active && visible,
            transitions: Vec::new(), workload_start: 0,
        }
    }

    #[test]
    fn hidden_interval_is_retained_after_reactivation() {
        let mut state = observation(true, true);
        state.record(10.0, true, false);
        state.record(20.0, true, true);
        state.record(60.0, true, true);
        assert_eq!(state.active_seconds, 60.0);
        assert_eq!(state.visible_seconds, 50.0);
        assert!(!state.all_active_visible);
        assert_eq!(state.transitions.len(), 2);
    }

    #[test]
    fn foreground_and_visibility_are_independent() {
        let mut state = observation(false, true);
        state.record(5.0, true, true);
        state.record(60.0, true, true);
        assert_eq!(state.active_seconds, 55.0);
        assert_eq!(state.visible_seconds, 60.0);
        assert!(!state.all_active_visible);
        let mut state = observation(true, true);
        state.record(60.0, true, true);
        assert!(state.all_active_visible);
        assert_eq!(state.active_seconds, 60.0);
        assert_eq!(state.visible_seconds, 60.0);
    }
}

const TASK: &str = "synthetic-ui-performance-task";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Static,
    Idle,
    StreamingBody,
    PipelineUpdates,
    BrowserAdjacent,
    Motion,
    Minimap,
}
impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Static => "static + active sparkles",
            Self::Idle => "idle",
            Self::Minimap => "chat minimap: synthetic message sizes and roles",
            Self::StreamingBody => "streaming-body + active sparkles",
            Self::PipelineUpdates => "pipeline-updates + active sparkles",
            Self::BrowserAdjacent => "browser-adjacent + active sparkles",
            Self::Motion => "motion controls + retained chats + native browser",
        }
    }
}
struct PerformanceScenario {
    dock: Entity<DockWorkspace>,
    chats: HashMap<String, Entity<ChatPane>>,
    snapshot: Rc<Value>,
    source: Option<Snapshot>,
    publication: PublicationQueue,
    index: Option<Arc<PublicationIndex>>,
    records: HashMap<String, Arc<Vec<Value>>>,
    child_transcripts: HashMap<String, PreparedChildTranscripts>,
    sequence: u64,
    scenario: Scenario,
    rows: usize,
    palette: design::Palette,
    invalid_option: Option<String>,
    sidebar_width: f32,
    sidebar_target: f32,
    sidebar_motion: crate::layout_motion::WidthMotion,
    sidebar_frame_pending: bool,
    sidebar_callbacks: u64,
    disclosure_open: bool,
    disclosure_expanded: bool,
}
impl Render for PerformanceScenario {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::native_pane_highlight::begin_frame(window);
        div().size_full().relative().flex().flex_col().bg(rgb(self.palette.paper))
            .font_family(cx.theme().font_family.clone()).text_color(rgb(self.palette.ink))
            .child(div().w_full().flex_shrink_0().px_4().py_2().bg(rgb(self.palette.card))
                .border_b_1().border_color(rgb(self.palette.line))
                .child("Performance fixture — synthetic data / no backend")
                .child(div().text_size(gpui_kit::px(11.)).text_color(rgb(self.palette.muted))
                    .child(format!("{} · {} panes × {} history rows · drafts stay in memory; send/voice/attachments disabled",
                        self.scenario.name(), self.chats.len(), self.rows)))
                .children(self.invalid_option.as_ref().map(|option|
                    div().child(format!("Unknown scenario {option:?}; showing static fixture.")))))
            .when(self.scenario == Scenario::Minimap, |view| view.child(div().flex().gap_2().px_4().py_2()
                .child(gpui_kit::component::button::Button::new("minimap-fixture-older").label("Preview older messages")
                    .on_click(cx.listener(|this, _, _, cx| { if let Some(chat) = this.chats.values().next() { chat.update(cx, |pane, cx| pane.minimap_fixture_scroll(Some(15), cx)); } })))
                .child(gpui_kit::component::button::Button::new("minimap-fixture-latest").label("Return to latest")
                    .on_click(cx.listener(|this, _, _, cx| { if let Some(chat) = this.chats.values().next() { chat.update(cx, |pane, cx| pane.minimap_fixture_scroll(None, cx)); } })))
                .child(gpui_kit::component::button::Button::new("minimap-fixture-marker").label("Preview marker jump")
                    .on_click(cx.listener(|this, _, _, cx| { if let Some(chat) = this.chats.values().next() { chat.update(cx, |pane, cx| pane.minimap_fixture_marker_jump(5, cx)); } })))
            ))
            .when(self.scenario == Scenario::Motion, |view| view.child(self.motion_controls(cx)))
            .when(self.scenario == Scenario::Motion && self.disclosure_open, |view| view.child(self.disclosure_fixture(window, cx)))
            .child(div().flex_1().min_h_0().flex()
                .when(self.scenario == Scenario::Motion && self.sidebar_width > 0., |view| {
                    view.child(div().w(gpui_kit::px(self.sidebar_width)).h_full().flex_shrink_0()
                        .overflow_hidden().bg(rgb(self.palette.card)).border_r_1().border_color(rgb(self.palette.line))
                        .child(div().p_2().whitespace_nowrap().child(if self.sidebar_target <= 48. { "◆" } else { "Synthetic motion rail" })))
                })
                .child(div().flex_1().min_w_0().size_full().child(self.dock.clone())))
            .child(gpui_kit::deferred(canvas(|_,_,_|(),|_,_,window,_|crate::native_pane_highlight::finish_frame(window))
                .absolute().inset_0()).with_priority(5))
    }
}
impl PerformanceScenario {
    fn motion_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{Sizable as _, button::Button};
        let modes = crate::native_motion::MotionPreference::ALL;
        let mode_buttons = modes.into_iter().map(|mode| {
            Button::new(format!("fixture-motion-{}", mode.id()))
                .compact()
                .label(mode.label())
                .on_click(cx.listener(move |scene, _, window, cx| {
                    crate::native_motion::set_preference_for_ui_fixture(mode, cx);
                    if !crate::native_motion::motion_allowed() {
                        scene.sidebar_width = scene.sidebar_target;
                        scene.sidebar_motion.cancel();
                    }
                    window.refresh();
                    cx.notify();
                }))
        });
        let sidebar_buttons = [("Expanded", 252.), ("Compact", 48.), ("Hidden", 0.)]
            .into_iter()
            .map(|(label, width)| {
                Button::new(format!("fixture-sidebar-{label}"))
                    .compact()
                    .label(label)
                    .on_click(cx.listener(move |scene, _, window, cx| {
                        scene.sidebar_target = width;
                        scene.sidebar_motion.start(
                            scene.sidebar_width,
                            width,
                            std::time::Instant::now(),
                            crate::native_motion::motion_allowed(),
                        );
                        if scene.sidebar_motion.active() {
                            scene.schedule_sidebar_frame(window, cx);
                        } else {
                            scene.sidebar_width = width;
                        }
                        cx.notify();
                    }))
            });
        div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .children(mode_buttons)
            .children(sidebar_buttons)
            .child(
                Button::new("fixture-equalize")
                    .compact()
                    .label("Equalise panes")
                    .on_click(cx.listener(|scene, _, window, cx| {
                        scene.dock.update(cx, |dock, cx| {
                            dock.equalize_workspace(window, cx);
                        });
                    })),
            )
            .child(
                Button::new("fixture-disclosure")
                    .compact()
                    .label("Disclosure")
                    .on_click(cx.listener(|scene, _, _, cx| {
                        scene.disclosure_open = !scene.disclosure_open;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("fixture-completion")
                    .compact()
                    .label("Complete task")
                    .on_click(cx.listener(|scene, _, _, cx| scene.complete_fixture_task(cx))),
            )
            .child(div().text_size(gpui_kit::px(11.)).child(format!(
                "{} · rail callbacks {} · {}",
                crate::native_motion::current().label(),
                self.sidebar_callbacks,
                if self.sidebar_motion.active() {
                    "moving"
                } else {
                    "settled"
                }
            )))
    }
    fn disclosure_fixture(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let owner = cx.weak_entity();
        crate::approval_prompt::render_approval_prompt(
            &json!({"id":"synthetic-disclosure", "tool":"Synthetic tool", "summary":"Synthetic disclosure fixture — decisions disabled", "detail":"{\"fixture\":true,\"message\":\"Disconnected native UI; no command will run.\"}"}),
            &self.snapshot["tasks"][0],
            &self.snapshot,
            self.palette,
            cx.theme().font_family.clone(),
            gpui_kit::px(12.),
            true,
            self.disclosure_expanded,
            false,
            Rc::new(|_, _, _| {}),
            Rc::new(move |_, cx| {
                let _ = owner.update(cx, |scene, cx| {
                    scene.disclosure_expanded = !scene.disclosure_expanded;
                    cx.notify();
                });
            }),
            window,
            cx,
        )
    }
    fn complete_fixture_task(&mut self, cx: &mut Context<Self>) {
        if self.snapshot["tasks"][0]["status"] == "completed" {
            return;
        }
        let mut snapshot = self.snapshot.as_ref().clone();
        snapshot["tasks"][0]["status"] = json!("completed");
        self.snapshot = Rc::new(snapshot);
        crate::activity_effects::sync(&self.snapshot, self.palette, cx);
        for chat in self.chats.values() {
            chat.update(cx, |chat, cx| {
                chat.sync(
                    self.snapshot.clone(),
                    Some("synthetic-completed".into()),
                    cx,
                )
            });
        }
        cx.notify();
    }
    fn schedule_sidebar_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_frame_pending || !self.sidebar_motion.active() {
            return;
        }
        self.sidebar_frame_pending = true;
        let owner = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = owner.update(cx, |scene, cx| {
                scene.sidebar_frame_pending = false;
                scene.sidebar_callbacks += 1;
                if !scene.sidebar_motion.active() {
                    return;
                }
                if !crate::native_motion::motion_allowed() {
                    scene.sidebar_width = scene.sidebar_target;
                    scene.sidebar_motion.cancel();
                } else if let Some(width) = scene.sidebar_motion.sample(std::time::Instant::now()) {
                    scene.sidebar_width = width;
                }
                if scene.sidebar_motion.active() {
                    scene.schedule_sidebar_frame(window, cx);
                }
                cx.notify();
            });
        });
    }
    fn queue_pipeline_update(&mut self, cx: &mut Context<Self>) {
        // Match production's backpressure: do not advance the source revision
        // faster than one preparation can finish and install it.
        if self.publication.busy {
            return;
        }
        let Some(source) = self.source.as_mut() else {
            return;
        };
        self.sequence = self.sequence.wrapping_add(1);
        // Codex/ACP stream flushes update both the message and Task.updatedAt.
        // Rotate through tasks to exercise dependency-scoped publication.
        let pane = self.sequence as usize % self.chats.len();
        let task = format!("{TASK}-{pane}");
        let message = if pane == 0 {
            "fixture-stream".to_owned()
        } else {
            format!("fixture-stream-{pane}")
        };
        let text = format!(
            "Synthetic publication {}. {}",
            self.sequence,
            "Native pipeline fixture output. ".repeat((self.sequence as usize % 40) + 1)
        );
        source
            .messages
            .update_by_id(&message, |message| message.text = text)
            .expect("synthetic streaming message index");
        source
            .tasks
            .iter_mut()
            .find(|row| row.id == task)
            .expect("synthetic streaming task")
            .updated_at = chrono::Utc::now().timestamp_millis();
        self.publication.enqueue(PublicationChanges {
            message_tasks: HashSet::from([task]),
            message_ids: HashSet::from([message]),
            ..Default::default()
        });
        self.start_publication(cx);
    }
    fn start_publication(&mut self, cx: &mut Context<Self>) {
        let Some((generation, changes)) = self.publication.start() else {
            return;
        };
        let source = self.source.as_ref().unwrap().clone();
        let panes: HashSet<_> = self.chats.keys().cloned().collect();
        let visible: HashSet<_> = self
            .chats
            .iter()
            .filter_map(|(id, pane)| {
                pane.read(cx)
                    .wants_history_publication()
                    .then(|| id.clone())
            })
            .collect();
        let previous = self.index.clone();
        let previous_records = self.records.clone();
        let previous_transcripts = self.child_transcripts.clone();
        let worker_changes = changes.clone();
        let worker = cx.background_executor().spawn(async move {
            let mut prepared = native_publication::prepare_visible(
                source,
                panes.clone(),
                visible,
                &worker_changes,
                previous,
            )?;
            for id in &panes {
                let records =
                    if prepared.sync_tasks.contains(id) || !previous_records.contains_key(id) {
                        Arc::new(crate::collaboration::records(&prepared.snapshot, id))
                    } else {
                        previous_records[id].clone()
                    };
                let transcripts =
                    if prepared.sync_tasks.contains(id) || !previous_transcripts.contains_key(id) {
                        crate::collaboration::prepare_child_transcripts_indexed(
                            &prepared.snapshot,
                            &prepared.index,
                            &records,
                            previous_transcripts.get(id),
                        )
                    } else {
                        previous_transcripts[id].clone()
                    };
                prepared.collaboration_records.insert(id.clone(), records);
                prepared
                    .collaboration_transcripts
                    .insert(id.clone(), transcripts);
            }
            Ok::<_, String>(prepared)
        });
        cx.spawn(async move |scene, cx| {
            let result = worker.await;
            let _ = scene.update(cx, |scene, cx| {
                if scene.publication.finish(generation, changes.clone()) {
                    match result {
                        Ok(prepared) => {
                            scene.commit_publication(prepared, generation, &changes, cx)
                        }
                        Err(error) => eprintln!("Synthetic pipeline preparation failed: {error}"),
                    }
                }
                scene.start_publication(cx);
            });
        })
        .detach();
    }
    fn commit_publication(
        &mut self,
        mut prepared: PreparedPublication,
        generation: u64,
        changes: &PublicationChanges,
        cx: &mut Context<Self>,
    ) {
        record_workload_update();
        self.index = Some(prepared.index.clone());
        self.records = prepared.collaboration_records;
        self.child_transcripts = prepared.collaboration_transcripts;
        let snapshot = Rc::new(prepared.snapshot);
        let retired_snapshot = std::mem::replace(&mut self.snapshot, snapshot.clone());
        let revision = Some(format!("synthetic-{generation}"));
        crate::activity_effects::sync(&snapshot, self.palette, cx);
        for (id, pane) in &self.chats {
            if !prepared.sync_tasks.contains(id) {
                pane.update(cx, |pane, cx| {
                    pane.advance_revision(snapshot.clone(), revision.clone(), cx)
                });
                continue;
            }
            let history = prepared.histories.remove(id);
            pane.update(cx, |pane, cx| {
                let patches = prepared
                    .body_only_tasks
                    .contains(id)
                    .then(|| prepared.message_updates.remove(id).unwrap_or_default());
                let boundary = changes.task_boundary_index(
                    id,
                    revision.as_deref(),
                    &snapshot,
                    self.index.as_ref().unwrap(),
                );
                pane.publish_indexed(
                    snapshot.clone(),
                    revision.clone(),
                    self.records[id].clone(),
                    self.child_transcripts[id].clone(),
                    prepared.transcript_context_unchanged_tasks.contains(id),
                    history,
                    changes.reset,
                    boundary.map(|boundary| boundary.next_before_id.clone()),
                    self.index.clone(),
                    patches,
                    prepared.lazy_history_tasks.contains(id),
                    cx,
                );
            });
        }
        crate::retire_snapshot(retired_snapshot, cx);
    }
}
/// Startup has no painted focus path yet. Resolve the group from the retained
/// center topology, which is authoritative immediately after pane insertion.
fn fixture_group_node(
    dock: &DockWorkspace,
    id: &str,
    cx: &App,
) -> Option<gpui_kit::component::dock::NodeId> {
    let panel = dock.panel_key(id)?;
    dock.area()
        .read(cx)
        .layout(DockPlacement::Center)?
        .find_panel_node(panel)
}

fn snapshot(scenario: Scenario, row_count: usize, pane_count: usize) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    let mut messages = Vec::new();
    let mut tasks = Vec::new();
    for pane in 0..pane_count {
        let task = format!("{TASK}-{pane}");
        tasks.push(json!({"id":task,"title":format!("Synthetic performance pane {}",pane+1),
            "agentId":"synthetic-agent","status":if matches!(scenario, Scenario::Idle | Scenario::Minimap) {"idle"} else {"running"},
            "createdAt":now,"updatedAt":now,"queue":[],"hostId":"synthetic-host",
            "cwd":"","provider":"codex","model":"synthetic","sandbox":"read-only"}));
        messages.extend((0..row_count).map(|index| {
            let minimap = scenario == Scenario::Minimap;
            let user = if minimap { index % 3 == 0 } else { index % 2 == 0 };
            let text = if minimap {
                let label = if user { "User prompt" } else if index % 3 == 2 { "Agent closing answer" } else { "Agent progress" };
                let detail = "This is clearly labelled synthetic text for checking the compact chat minimap, message lengths, hover previews and navigation. ";
                format!("{label} {index}.\n\n{}", detail.repeat(if index % 7 == 0 { 18 } else if index % 4 == 0 { 5 } else { 1 }))
            } else {
                format!("Synthetic history row {index}. Isolated native fixture for retained layout, selection, scrolling and input.")
            };
            let mut message = json!({
                "id":format!("synthetic-message-{pane}-{index}"),"taskId":task,
                "role":if user {"user"} else {"assistant"}, "text":text,
                "createdAt":now-(row_count-index) as i64*1_000,
            });
            if minimap && index % 3 == 2 { message["phase"] = json!("final_answer"); }
            message
        }));
        if matches!(
            scenario,
            Scenario::StreamingBody | Scenario::PipelineUpdates
        ) {
            messages.push(json!({"id":if pane==0 {"fixture-stream".to_owned()} else {format!("fixture-stream-{pane}")},
                "taskId":task,"role":"assistant","text":"Synthetic streaming body. ",
                "streamStatus":"streaming","createdAt":now}));
        }
    }
    json!({
        "settings":{"theme":"dark","accent":"#6594db","activityAnimation":"sparkles",
            "chatFontSize":13,"interfaceFontSize":14,"compressToolCalls":true,
            "userName":"Synthetic operator","showActivePaneBorder":true},
        "tasks":tasks,
        "agents":[{"id":"synthetic-agent","name":"Synthetic fixture agent","provider":"codex",
            "description":"Synthetic only","instructions":"","model":"synthetic","hostId":"synthetic-host",
            "cwd":"","color":"#6594db","sandbox":"read-only"}],
        "messages":messages,"channels":[],"events":[],"approvalRequests":[],"sharing":{},"hosts":[],"schedules":[],
    })
}
/// Call after GPUI initialization, before normal preference/backend setup.
/// Even an invalid but present option remains isolated from the live profile.
pub fn open_if_requested(cx: &mut App) -> bool {
    let Some(option) = std::env::var_os("MONITTER_GPUI_PERF_SCENARIO") else {
        return false;
    };
    let option = option.to_string_lossy().into_owned();
    if option == "surface" {
        crate::surface_preview::open(cx);
        return true;
    }
    let (scenario, invalid_option) = match option.as_str() {
        "static" | "active" => (Scenario::Static, None),
        "idle" => (Scenario::Idle, None),
        "streaming-body" | "streaming" => (Scenario::StreamingBody, None),
        "pipeline-updates" => (Scenario::PipelineUpdates, None),
        "browser-adjacent" => (Scenario::BrowserAdjacent, None),
        "motion" => (Scenario::Motion, None),
        "minimap" => (Scenario::Minimap, None),
        _ => (Scenario::Static, Some(option)),
    };
    let row_count = std::env::var("MONITTER_GPUI_PERF_HISTORY_ROWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000)
        .clamp(
            if scenario == Scenario::Minimap {
                12
            } else {
                1_000
            },
            100_000,
        );
    let pane_count = match scenario {
        Scenario::PipelineUpdates => 4,
        Scenario::Motion => 3,
        _ => 1,
    };
    crate::surface_blend::initialize_for_ui_fixture();
    crate::native_motion::initialize_for_ui_fixture(cx);
    if let Some(mode) = std::env::var("MONITTER_GPUI_PERF_MOTION")
        .ok()
        .and_then(|value| crate::native_motion::MotionPreference::parse(&value))
    {
        crate::native_motion::set_preference_for_ui_fixture(mode, cx);
    }
    // Production applies the native theme through SettingsView before drawing.
    // The isolated scene bypasses preferences, so initialize component colors.
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    cx.set_global(streaming_display::Selection {
        options: streaming_display::Options {
            mode: if matches!(
                scenario,
                Scenario::StreamingBody | Scenario::PipelineUpdates
            ) {
                streaming_display::Mode::Typing
            } else {
                streaming_display::Mode::AsReceived
            },
            characters_per_second: 600,
        },
        error: None,
    });
    if let Err(error) = monitter_gpui_desktop::fonts::register(cx) {
        eprintln!("{error}");
    }
    let mut fixture_snapshot = snapshot(scenario, row_count, pane_count);
    // Explicitly synthetic, disconnected preview of a blocking permission dialog.
    if std::env::var_os("MONITTER_GPUI_APPROVAL_PREVIEW").is_some() {
        fixture_snapshot["approvalRequests"] = json!([{
            "id":"synthetic-approval-preview", "taskId":format!("{TASK}-0"),
            "status":"pending", "provider":"codex", "tool":"Command execution",
            "summary":"/bin/zsh -c python3", "risk":"high", "rememberable":true,
            "detail":json!({"command":"/bin/zsh -c \"python3 - <<'PY'\nfrom pathlib import Path\n\nproject = Path('synthetic-fixture')\nfor file in project.glob('*.rs'):\n    print(file.name)\nPY\"", "reason":"Synthetic preview only. No command will run."}).to_string()
        }]);
    }
    let snapshot = Rc::new(fixture_snapshot);
    let palette = design::palette(&snapshot["settings"], gpui_kit::WindowAppearance::Dark);
    crate::activity_effects::sync(&snapshot, palette, cx);
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(gpui_kit::px(1200.), gpui_kit::px(820.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Monitter — isolated performance fixture".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(gpui_kit::px(12.), gpui_kit::px(18.))),
            }),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            let source = if scenario == Scenario::PipelineUpdates {
                Some(
                    serde_json::from_value::<Snapshot>(snapshot.as_ref().clone())
                        .expect("fully synthetic pipeline snapshot"),
                )
            } else {
                None
            };
            let dock = cx.new(|cx| {
                DockWorkspace::new(
                    "isolated-performance-fixture",
                    Rc::new(|_, _, _| None),
                    window,
                    cx,
                )
            });
            dock.update(cx, |dock, cx| {
                dock.set_pane_appearance(&snapshot["settings"], palette, cx);
                dock.set_tab_appearance(&snapshot["settings"], design::interface_scale(), cx);
            });
            let mut chats = HashMap::new();
            for pane_index in 0..pane_count {
                let task = format!("{TASK}-{pane_index}");
                let chat = cx.new(|cx| {
                    ChatPane::performance_fixture(task.clone(), snapshot.clone(), window, cx)
                });
                let content = chat.clone();
                let lifecycle_task = task.clone();
                let spec = PaneSpec {
                    id: format!("chat:{task}"),
                    title: format!("Synthetic pane {}", pane_index + 1).into(),
                    content: chat.clone().into(),
                    focus: chat.focus_handle(cx),
                    closable: false,
                    lifecycle: Some(Rc::new(move |event, _, cx| {
                        if let crate::docking::PaneLifecycle::Visible(visible) = event {
                            content.update(cx, |chat, cx| chat.set_dock_visible(visible, cx));
                            crate::activity_effects::set_pane_visible(&lifecycle_task, visible, cx);
                        }
                    })),
                };
                let placement = if scenario == Scenario::Motion {
                    DockPlacement::Center
                } else {
                    match pane_index {
                        0 => DockPlacement::Center,
                        1 => DockPlacement::Right,
                        2 => DockPlacement::Bottom,
                        _ => DockPlacement::Left,
                    }
                };
                dock.update(cx, |dock, cx| {
                    dock.add(spec, placement, Some(gpui_kit::px(380.)), window, cx)
                })
                .expect("add synthetic native dock pane");
                if scenario == Scenario::Motion && pane_index > 0 {
                    dock.update(cx, |dock, cx| {
                        let node = fixture_group_node(dock, &format!("chat:{TASK}-0"), cx)
                            .expect("retained fixture group");
                        assert!(dock.move_pane(
                            &format!("chat:{task}"),
                            InsertTarget::Split {
                                node,
                                placement: if pane_index == 1 {
                                    Placement::Right
                                } else {
                                    Placement::Bottom
                                },
                                size: Some(gpui_kit::px(if pane_index == 1 { 290. } else { 220. })),
                            },
                            window,
                            cx
                        ));
                    });
                }
                chats.insert(task, chat);
            }
            if matches!(scenario, Scenario::BrowserAdjacent | Scenario::Motion) {
                let browser =
                    cx.new(|cx| crate::browser::BrowserTab::performance_fixture(window, cx));
                let native = browser.clone();
                let spec = PaneSpec {
                    id: "browser:synthetic-isolated".into(),
                    title: "Static browser fixture".into(),
                    content: browser.clone().into(),
                    focus: browser.focus_handle(cx),
                    closable: false,
                    lifecycle: Some(Rc::new(move |event, _, cx| match event {
                        crate::docking::PaneLifecycle::Visible(true) => {
                            native.update(cx, |tab, cx| tab.show(cx))
                        }
                        crate::docking::PaneLifecycle::Visible(false) => {
                            native.update(cx, |tab, cx| tab.hide(cx))
                        }
                        crate::docking::PaneLifecycle::Removed => {
                            native.update(cx, |tab, cx| tab.close(cx))
                        }
                        _ => {}
                    })),
                };
                dock.update(cx, |dock, cx| {
                    dock.add(
                        spec,
                        if scenario == Scenario::Motion {
                            DockPlacement::Center
                        } else {
                            DockPlacement::Right
                        },
                        Some(gpui_kit::px(480.)),
                        window,
                        cx,
                    )
                })
                .expect("add incognito static browser beside synthetic chat");
                if scenario == Scenario::Motion {
                    dock.update(cx, |dock, cx| {
                        let node = fixture_group_node(dock, &format!("chat:{TASK}-1"), cx)
                            .expect("retained fixture chat group");
                        assert!(dock.move_pane(
                            "browser:synthetic-isolated",
                            InsertTarget::Split {
                                node,
                                placement: Placement::Right,
                                size: Some(gpui_kit::px(340.)),
                            },
                            window,
                            cx
                        ));
                    });
                }
            }
            let first = format!("{TASK}-0");
            chats[&first]
                .read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            if scenario == Scenario::StreamingBody {
                let owner = chats[&first].downgrade();
                cx.spawn(async move |cx| {
                    let mut sequence = 0u64;
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await;
                        sequence += 1;
                        let text = format!(
                            "Synthetic streaming body, update {sequence}. {}",
                            "Live fixture text. ".repeat((sequence as usize % 80) + 1)
                        );
                        if owner
                            .update(cx, |chat, cx| {
                                chat.advance_performance_stream(&text, cx);
                                record_workload_update();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            let scene = cx.new(|cx| {
                cx.observe_global::<crate::native_motion::MotionSelection>(
                    |scene: &mut PerformanceScenario, cx| {
                        if !crate::native_motion::motion_allowed() {
                            scene.sidebar_width = scene.sidebar_target;
                            scene.sidebar_motion.cancel();
                        }
                        cx.notify();
                    },
                )
                .detach();
                PerformanceScenario {
                    dock,
                    chats,
                    snapshot,
                    source,
                    publication: PublicationQueue::default(),
                    index: None,
                    records: HashMap::new(),
                    child_transcripts: HashMap::new(),
                    sequence: 0,
                    scenario,
                    rows: row_count,
                    palette,
                    invalid_option,
                    sidebar_width: 252.,
                    sidebar_target: 252.,
                    sidebar_motion: Default::default(),
                    sidebar_frame_pending: false,
                    sidebar_callbacks: 0,
                    disclosure_open: false,
                    disclosure_expanded: false,
                }
            });
            scene.update(cx, |_, cx| {
                cx.observe_window_activation(window, |_: &mut PerformanceScenario, window, _| {
                    observe_window_conditions(window);
                }).detach();
                cx.observe_window_visibility(window, |_: &mut PerformanceScenario, _, window, _| {
                    observe_window_conditions(window);
                }).detach();
            });
            if scenario == Scenario::PipelineUpdates {
                scene.update(cx, |scene, cx| {
                    scene.publication.enqueue(PublicationChanges {
                        reset: true,
                        ..Default::default()
                    });
                    scene.start_publication(cx);
                });
                let owner = scene.downgrade();
                cx.spawn(async move |cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        if owner
                            .update(cx, |scene, cx| scene.queue_pipeline_update(cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            scene
        },
    )
    .expect("open isolated GPUI performance fixture");
    crate::performance_metrics::schedule(handle, cx);
    #[cfg(feature = "visual-export")]
    {
        crate::visual_export::install(cx);
        crate::visual_export::schedule(handle, cx);
    }
    cx.activate(true);
    true
}

#[cfg(test)]
mod tests {
    use super::{Scenario, snapshot};
    use monitter_lib::model::Snapshot;
    #[cfg(unix)]
    #[test]
    fn opening_idle_parent_prepares_child_history_from_compact_source() {
        use monitter_gpui_desktop::native_publication::{PublicationChanges, prepare};
        use std::collections::HashSet;
        let mut value = snapshot(Scenario::PipelineUpdates, 10, 2);
        let parent = value["tasks"][0]["id"].as_str().unwrap().to_owned();
        let child = value["tasks"][1]["id"].as_str().unwrap().to_owned();
        value["collaborations"] = serde_json::json!([{"id":"idle-child","kind":"delegation","fromAgentId":"fixture-agent","fromTaskId":parent,"toTaskId":child,"toAgentId":"fixture-agent","requestId":"idle-request","text":"child","status":"completed","createdAt":0,"updatedAt":0}]);
        let source: Snapshot = serde_json::from_value(value).unwrap();
        let initial = prepare(
            source.clone(),
            HashSet::new(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let opened = prepare(
            source,
            HashSet::from([parent.clone()]),
            &PublicationChanges {
                message_tasks: HashSet::from([parent.clone()]),
                ..Default::default()
            },
            Some(initial.index),
        )
        .unwrap();
        assert!(opened.sync_tasks.contains(&parent));
        assert!(opened.snapshot["messages"].as_array().unwrap().is_empty());
        let records = crate::collaboration::records(&opened.snapshot, &parent);
        assert!(!records.is_empty());
        let transcripts = crate::collaboration::prepare_child_transcripts_indexed(
            &opened.snapshot,
            &opened.index,
            &records,
            None,
        );
        assert!(transcripts.values().any(|entries| !entries.is_empty()));
    }
    #[test]
    fn fixture_service_rejects_reads_commands_and_subscriptions() {
        let service =
            monitter_lib::experimental_desktop::DesktopService::disconnected_for_ui_fixture();
        let disconnected = "The experimental GUI client is disconnected.";
        assert_eq!(service.server_instance_id(), "synthetic-ui-fixture");
        assert_eq!(service.read_ui_changes(None).unwrap_err(), disconnected);
        assert_eq!(
            service
                .read_task_messages("synthetic-task", None, Some(1))
                .unwrap_err(),
            disconnected
        );
        assert_eq!(
            service
                .invoke("fixture-no-op", serde_json::Value::Null)
                .unwrap_err(),
            disconnected
        );
        assert_eq!(service.subscribe_changes().unwrap_err(), disconnected);
    }
    #[test]
    fn motion_fixture_is_detached_with_three_independent_chat_histories() {
        let value = snapshot(Scenario::Motion, 1_000, 3);
        let source: Snapshot = serde_json::from_value(value).expect("typed motion fixture");
        assert_eq!(source.tasks.len(), 3);
        assert_eq!(source.messages.len(), 3_000);
        assert!(source.hosts.is_empty());
        assert!(source.tasks.iter().all(|task| task.cwd.is_empty()));
    }
    #[test]
    fn pipeline_fixture_is_a_valid_typed_snapshot_with_four_independent_histories() {
        let value = snapshot(Scenario::PipelineUpdates, 1_000, 4);
        let source: Snapshot =
            serde_json::from_value(value).expect("fixture uses production contract");
        assert_eq!(source.tasks.len(), 4);
        assert_eq!(source.messages.len(), 4_004);
        assert!(source.hosts.is_empty());
        assert!(source.tasks.iter().all(|task| task.cwd.is_empty()));
    }

    struct StartupPane(std::rc::Rc<std::cell::Cell<usize>>);
    impl gpui_kit::Render for StartupPane {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            self.0.set(self.0.get() + 1);
            gpui_kit::div()
        }
    }

    #[gpui_kit::test]
    fn motion_startup_assembles_four_visible_groups_before_first_paint(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{
            AppContext as _,
            component::{
                Placement,
                dock::{DockPlacement, InsertTarget},
            },
        };
        cx.update(|cx| gpui_kit::init(cx));
        let (_, visual) = cx.add_window_view(|window, cx| {
            let dock = cx.new(|cx| {
                let renders = std::rc::Rc::new(std::cell::Cell::new(0));
                let mut dock = crate::docking::DockWorkspace::new(
                    "motion-startup-test",
                    std::rc::Rc::new(|_, _, _| None),
                    window,
                    cx,
                );
                let ids = [
                    "chat:startup-0",
                    "chat:startup-1",
                    "chat:startup-2",
                    "browser:startup",
                ];
                for (index, id) in ids.iter().enumerate() {
                    let content = cx.new(|_| StartupPane(renders.clone()));
                    dock.add(
                        crate::docking::PaneSpec {
                            id: (*id).into(),
                            title: "Synthetic startup pane".into(),
                            content: content.into(),
                            focus: cx.focus_handle(),
                            closable: false,
                            lifecycle: None,
                        },
                        DockPlacement::Center,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                    if index > 0 {
                        let anchor = if index == 3 { ids[1] } else { ids[0] };
                        let node = super::fixture_group_node(&dock, anchor, cx)
                            .expect("topology exists before focus has a painted path");
                        assert!(dock.move_pane(
                            id,
                            InsertTarget::Split {
                                node,
                                placement: if index == 2 {
                                    Placement::Bottom
                                } else {
                                    Placement::Right
                                },
                                size: Some(gpui_kit::px(if index == 2 { 220. } else { 290. })),
                            },
                            window,
                            cx
                        ));
                    }
                }
                assert_eq!(
                    renders.get(),
                    0,
                    "startup assembly must not depend on any pane render"
                );
                let groups: std::collections::HashSet<_> = ids
                    .iter()
                    .map(|id| super::fixture_group_node(&dock, id, cx).unwrap())
                    .collect();
                assert_eq!(
                    groups.len(),
                    4,
                    "all chats and browser occupy independent retained groups"
                );
                dock
            });
            gpui_kit::component::Root::new(dock, window, cx)
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }
}
