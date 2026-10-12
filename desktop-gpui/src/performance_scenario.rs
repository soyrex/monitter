//! Explicit profiling-build-only native UI. Synthetic data, detached backend,
//! in-memory drafts, and no disk-backed preferences or agent commands.
use crate::{
    chat::ChatPane,
    design,
    docking::{DockWorkspace, PaneSpec},
    streaming_display,
};
use gpui_kit::component::{ActiveTheme as _, dock::DockPlacement};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Render, Styled as _, TitlebarOptions, Window, WindowBounds, WindowOptions, canvas, div, point,
    rgb, size,
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

const TASK: &str = "synthetic-ui-performance-task";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Static,
    Idle,
    StreamingBody,
    PipelineUpdates,
    BrowserAdjacent,
}
impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Static => "static + active sparkles",
            Self::Idle => "idle",
            Self::StreamingBody => "streaming-body + active sparkles",
            Self::PipelineUpdates => "pipeline-updates + active sparkles",
            Self::BrowserAdjacent => "browser-adjacent + active sparkles",
        }
    }
}
struct PerformanceScenario {
    dock: Entity<DockWorkspace>,
    chats: HashMap<String, Entity<ChatPane>>,
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
            .child(div().flex_1().min_h_0()
                .child(self.dock.clone()))
            .child(gpui_kit::deferred(canvas(|_,_,_|(),|_,_,window,_|crate::native_pane_highlight::finish_frame(window))
                .absolute().inset_0()).with_priority(5))
    }
}
impl PerformanceScenario {
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
        let previous = self.index.clone();
        let previous_records = self.records.clone();
        let previous_transcripts = self.child_transcripts.clone();
        let worker_changes = changes.clone();
        let worker = cx.background_executor().spawn(async move {
            let mut prepared =
                native_publication::prepare(source, panes.clone(), &worker_changes, previous)?;
            for id in &panes {
                let records =
                    if prepared.sync_tasks.contains(id) || !previous_records.contains_key(id) {
                        Arc::new(crate::collaboration::records(&prepared.snapshot, id))
                    } else {
                        previous_records[id].clone()
                    };
                let transcripts =
                    if prepared.sync_tasks.contains(id) || !previous_transcripts.contains_key(id) {
                        crate::collaboration::prepare_child_transcripts(
                            &prepared.snapshot,
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
                        Ok(prepared) => scene.commit_publication(prepared, generation, cx),
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
        cx: &mut Context<Self>,
    ) {
        self.index = Some(prepared.index);
        self.records = prepared.collaboration_records;
        self.child_transcripts = prepared.collaboration_transcripts;
        let snapshot = Rc::new(prepared.snapshot);
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
                pane.sync_prepared(
                    snapshot.clone(),
                    revision.clone(),
                    self.records[id].clone(),
                    self.child_transcripts[id].clone(),
                    cx,
                );
                if let Some(messages) = history {
                    pane.reconcile_history(messages, false, cx);
                }
            });
        }
    }
}
fn snapshot(scenario: Scenario, row_count: usize, pane_count: usize) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    let mut messages = Vec::new();
    let mut tasks = Vec::new();
    for pane in 0..pane_count {
        let task = format!("{TASK}-{pane}");
        tasks.push(json!({"id":task,"title":format!("Synthetic performance pane {}",pane+1),
            "agentId":"synthetic-agent","status":if scenario==Scenario::Idle {"idle"} else {"running"},
            "createdAt":now,"updatedAt":now,"queue":[],"hostId":"synthetic-host",
            "cwd":"","provider":"codex","model":"synthetic","sandbox":"read-only"}));
        messages.extend((0..row_count).map(|index| json!({
            "id":format!("synthetic-message-{pane}-{index}"),"taskId":task,
            "role":if index%2==0 {"user"} else {"assistant"},
            "text":format!("Synthetic history row {index}. Isolated native fixture for retained layout, selection, scrolling and input."),
            "createdAt":now-(row_count-index) as i64*1_000,
        })));
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
    let (scenario, invalid_option) = match option.as_str() {
        "static" | "active" => (Scenario::Static, None),
        "idle" => (Scenario::Idle, None),
        "streaming-body" | "streaming" => (Scenario::StreamingBody, None),
        "pipeline-updates" => (Scenario::PipelineUpdates, None),
        "browser-adjacent" => (Scenario::BrowserAdjacent, None),
        _ => (Scenario::Static, Some(option)),
    };
    let row_count = std::env::var("MONITTER_GPUI_PERF_HISTORY_ROWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000)
        .clamp(1_000, 100_000);
    let pane_count = if scenario == Scenario::PipelineUpdates {
        4
    } else {
        1
    };
    crate::surface_blend::initialize_for_ui_fixture();
    crate::native_motion::initialize_for_ui_fixture(cx);
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
    let snapshot = Rc::new(snapshot(scenario, row_count, pane_count));
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
                let placement = match pane_index {
                    0 => DockPlacement::Center,
                    1 => DockPlacement::Right,
                    2 => DockPlacement::Bottom,
                    _ => DockPlacement::Left,
                };
                dock.update(cx, |dock, cx| {
                    dock.add(spec, placement, Some(gpui_kit::px(380.)), window, cx)
                })
                .expect("add synthetic native dock pane");
                chats.insert(task, chat);
            }
            if scenario == Scenario::BrowserAdjacent {
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
                        DockPlacement::Right,
                        Some(gpui_kit::px(480.)),
                        window,
                        cx,
                    )
                })
                .expect("add incognito static browser beside synthetic chat");
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
                            .update(cx, |chat, cx| chat.advance_performance_stream(&text, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            let scene = cx.new(|_| PerformanceScenario {
                dock,
                chats,
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
            });
            if scenario == Scenario::PipelineUpdates {
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
    cx.activate(true);
    true
}

#[cfg(test)]
mod tests {
    use super::{Scenario, snapshot};
    use monitter_lib::model::Snapshot;
    #[cfg(unix)]
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
    fn pipeline_fixture_is_a_valid_typed_snapshot_with_four_independent_histories() {
        let value = snapshot(Scenario::PipelineUpdates, 1_000, 4);
        let source: Snapshot =
            serde_json::from_value(value).expect("fixture uses production contract");
        assert_eq!(source.tasks.len(), 4);
        assert_eq!(source.messages.len(), 4_004);
        assert!(source.hosts.is_empty());
        assert!(source.tasks.iter().all(|task| task.cwd.is_empty()));
    }
}
