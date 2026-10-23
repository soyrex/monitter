//! Retained native controls for the task composer footer.
//!
//! The parent owns composer and Git layout. Drop `render_context_bar(...)`
//! inside the composer box, at its bottom edge, and `render_footer(...)`
//! beneath it. The pane owns only task-scoped permissions, model catalog,
//! System One routing status and reported context usage.
//!
//! Frozen integration API:
//! - `ComposerFooterPane::new(service, snapshot, task_id, window, cx)`
//! - `set_snapshot(snapshot, task_id, cx)` when canonical task metadata changes
//! - `set_context_messages(messages, cx)` with messages for the selected task
//! - `set_busy(busy, cx)` while parent-owned sends/settings writes are active
//! - subscribe to `ComposerFooterEvent::SnapshotUpdated` and merge the returned
//!   canonical snapshot through the parent's existing snapshot revision gate.
//! - `render_footer(palette, window, cx)` and
//!   `render_context_bar(palette) -> AnyElement` for the two layout slots.
//!
//! All choices come from the selected task, its agent, backend capability
//! rules, and the real `get_model_catalog` / `get_usage_overview` responses.
//! No renderer-side model IDs, permission modes, or context limits are guessed.
#![forbid(unsafe_code)]

use crate::design::{self, ui_scaled_px as px};
use gpui_kit::component::tooltip::ManagedTooltipExt as _;
use gpui_kit::{
    assets::__private as icon_assets,
    component::{
        ActiveTheme as _, Disableable, Icon, Sizable, Size,
        button::{Button, ButtonCustomVariant, ButtonVariants},
        tooltip::Tooltip,
    },
    prelude::FluentBuilder,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[path = "composer_popup.rs"]
mod composer_popup;
#[path = "model_save_queue.rs"]
mod model_save_queue;
use model_save_queue::{ModelSaveQueue, SaveOutcome};
#[path = "composer_footer_usage.rs"]
mod usage_refresh;
use usage_refresh::UsageRefresh;

#[derive(Clone, Debug)]
pub enum ComposerFooterEvent {
    SnapshotUpdated { task_id: String, snapshot: Value },
}

fn footer_snapshot(snapshot: &Value) -> Value {
    json!({"settings": snapshot["settings"], "tasks": snapshot["tasks"], "agents": snapshot["agents"]})
}

pub struct ComposerFooterPane {
    service: DesktopService,
    snapshot: Value,
    task_id: String,
    context_cleared_at: Option<i64>,
    catalog: Option<Value>,
    usage_overview: Option<Value>,
    catalog_loading: bool,
    usage_loading: bool,
    saving: bool,
    model_save: ModelSaveQueue,
    model_save_queues: std::collections::HashMap<String, ModelSaveQueue>,
    busy: bool,
    catalog_error: Option<String>,
    usage_error: Option<String>,
    save_error: Option<String>,
    model_menu_open: bool,
    model_menu_open_generation: u64,
    catalog_generation: u64,
    usage_refresh: UsageRefresh,
    usage_clock: Instant,
    save_generation: u64,
    observed_task_marker: Option<(String, String)>,
    composed_palette: Option<crate::design::Palette>,
    available_width_design: Option<f32>,
    retained_height: Option<Pixels>,
}

impl EventEmitter<ComposerFooterEvent> for ComposerFooterPane {}

impl ComposerFooterPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        task_id: impl Into<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self {
            service,
            snapshot: footer_snapshot(snapshot),
            task_id: task_id.into(),
            context_cleared_at: None,
            catalog: None,
            usage_overview: None,
            catalog_loading: false,
            usage_loading: false,
            saving: false,
            model_save: ModelSaveQueue::default(),
            model_save_queues: Default::default(),
            busy: false,
            catalog_error: None,
            usage_error: None,
            save_error: None,
            model_menu_open: false,
            model_menu_open_generation: 0,
            catalog_generation: 0,
            usage_refresh: UsageRefresh::default(),
            usage_clock: Instant::now(),
            save_generation: 0,
            observed_task_marker: None,
            composed_palette: None,
            available_width_design: None,
            retained_height: None,
        };
        pane.observed_task_marker = pane.task_marker();
        pane.refresh_catalog(false, cx);
        pane.refresh_usage(cx);
        pane
    }

    /// Keep selection state scoped to the same task while accepting the
    /// parent's canonical snapshot. A task switch invalidates async catalog
    /// results before requesting the new exact target.
    pub fn set_snapshot(
        &mut self,
        snapshot: &Value,
        task_id: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let task_id = task_id.into();
        let changed_task = self.task_id != task_id;
        let previous_marker = self.observed_task_marker.clone();
        let next_snapshot = footer_snapshot(snapshot);
        let snapshot_changed = self.snapshot != next_snapshot;
        self.snapshot = next_snapshot;
        if changed_task {
            self.model_menu_open = false;
            let next_queue = self.model_save_queues.remove(&task_id).unwrap_or_default();
            let previous_queue = std::mem::replace(&mut self.model_save, next_queue);
            if previous_queue.sandbox.is_some()
                || previous_queue.desired.is_some()
                || previous_queue.working
                || previous_queue.error.is_some()
            {
                self.model_save_queues
                    .insert(self.task_id.clone(), previous_queue);
            }
            self.task_id = task_id;
            self.context_cleared_at = None;
            self.catalog = None;
            self.catalog_error = None;
            self.save_error = None;
            self.catalog_generation = self.catalog_generation.wrapping_add(1);
            self.saving = false;
            self.save_generation = self.save_generation.wrapping_add(1);
            self.refresh_catalog(false, cx);
        } else if self.catalog.is_none() && !self.catalog_loading && self.catalog_error.is_none() {
            self.refresh_catalog(false, cx);
        }
        // Every successful authority refresh can resume queued intent.
        self.model_save.reconcile(&self.snapshot, &self.task_id);
        let pending: Vec<_> = self.model_save_queues.keys().cloned().collect();
        for task in &pending {
            if let Some(queue) = self.model_save_queues.get_mut(task) {
                queue.reconcile(&self.snapshot, task);
            }
        }
        self.start_model_save(cx);
        for task in pending {
            self.start_model_save_for(task, cx);
        }
        let marker = self.task_marker();
        self.observed_task_marker = marker.clone();
        let marker_changed = previous_marker != marker;
        // Own status/task transitions stay prompt, even if a prior cache read
        // is in flight. Timestamp-only stream markers use the normal budget.
        self.drive_usage_refresh(cx);
        if changed_task || snapshot_changed || marker_changed {
            cx.notify();
        }
    }

    pub fn set_context_messages(
        &mut self,
        messages: Vec<std::rc::Rc<Value>>,
        cx: &mut Context<Self>,
    ) {
        self.context_cleared_at = messages
            .iter()
            .filter(|message| {
                message["taskId"].as_str() == Some(self.task_id.as_str())
                    && message["role"].as_str() == Some("system")
                    && message["text"].as_str() == Some("Context Cleared")
                    && message
                        .get("senderAgentId")
                        .is_none_or(|v| v.is_null() || v.as_str() == Some(""))
                    && message
                        .get("collaborationId")
                        .is_none_or(|v| v.is_null() || v.as_str() == Some(""))
                    && message
                        .get("attachments")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty)
            })
            .filter_map(|message| message["createdAt"].as_i64())
            .max();
        cx.notify();
    }

    /// Keep the bar in step with the app's shared, cache-backed usage view.
    pub fn set_usage_overview(&mut self, overview: &Value, cx: &mut Context<Self>) {
        self.usage_refresh.publish(self.usage_clock.elapsed());
        self.usage_overview = Some(overview.clone());
        self.usage_error = None;
        self.usage_loading = false;
        cx.notify();
    }

    /// Called from the desktop's one-second UI pulse. The overview is read
    /// from the local cache only. Ordinary activity respects a 65-second budget;
    /// task/status transitions and explicit refreshes can drain sooner.
    pub fn tick_usage(&mut self, cx: &mut Context<Self>) {
        self.drive_usage_refresh(cx);
    }

    fn task_marker(&self) -> Option<(String, String)> {
        self.selected_task().map(|task| {
            (
                task.get("updatedAt")
                    .and_then(Value::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                task.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
    }

    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        if !busy {
            self.start_model_save(cx);
            self.drive_usage_refresh(cx);
        }
        cx.notify();
    }

    /// Open the same anchored model menu used by the composer control when a
    /// slash command asks the user to choose a model.
    pub fn open_model_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        _palette: Option<crate::design::Palette>,
    ) {
        gpui_kit::component::surface_backdrop::set_active(
            gpui_kit::ElementId::Name(
                format!("composer-model-selection-popover-{}", cx.entity_id()).into(),
            ),
            true,
            window,
            cx,
        );
        self.set_model_picker_open(true, cx);
    }

    /// Refresh once per opening while retaining the current choices and draft.
    pub(super) fn set_model_picker_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.model_menu_open == open {
            return;
        }
        self.model_menu_open = open;
        if open {
            self.model_menu_open_generation = self.model_menu_open_generation.wrapping_add(1);
        }
        if open && !self.catalog_loading {
            self.refresh_catalog(true, cx);
        }
        cx.notify();
    }

    fn selected_task(&self) -> Option<Value> {
        self.snapshot
            .get("tasks")?
            .as_array()?
            .iter()
            .find(|task| task.get("id").and_then(Value::as_str) == Some(self.task_id.as_str()))
            .cloned()
    }

    fn selected_agent(&self, task: &Value) -> Option<Value> {
        let agent_id = task.get("agentId")?.as_str()?;
        self.snapshot
            .get("agents")?
            .as_array()?
            .iter()
            .find(|agent| agent.get("id").and_then(Value::as_str) == Some(agent_id))
            .cloned()
    }

    fn refresh_catalog(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(task) = self.selected_task() else {
            return;
        };
        if task
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return;
        }
        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        let generation = self.catalog_generation;
        let service = self.service.clone();
        let task_id = self.task_id.clone();
        let codex_home = if task.get("provider").and_then(Value::as_str) == Some("codex") {
            task.get("codexHome").cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let target = json!({
            "taskId": task_id.clone(),
            "codexHome": codex_home,
            "refresh": force,
        });
        self.catalog_loading = true;
        self.catalog_error = None;
        let work = cx
            .background_executor()
            .spawn(async move { service.invoke("get_model_catalog", json!({"target": target})) });
        cx.spawn(async move |entity, cx| {
            let result = work.await;
            let _ = entity.update(cx, |pane, cx| {
                if pane.catalog_generation != generation || pane.task_id != task_id {
                    return;
                }
                pane.catalog_loading = false;
                match result {
                    Ok(catalog) => pane.catalog = Some(catalog),
                    Err(reason) => pane.catalog_error = Some(reason),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        self.usage_refresh.request_urgent();
        self.drive_usage_refresh(cx);
    }

    fn drive_usage_refresh(&mut self, cx: &mut Context<Self>) {
        self.usage_refresh.observe(
            &self.task_id,
            self.observed_task_marker
                .as_ref()
                .map(|(_, status)| status.as_str()),
        );
        if self.busy || self.saving {
            return;
        }
        let Some(request) = self.usage_refresh.begin(self.usage_clock.elapsed()) else {
            return;
        };
        let service = self.service.clone();
        // Cache reads can wait behind a provider refresh in the backend. Never
        // occupy GPUI's shared GCD executor while waiting on that blocking IPC:
        // retained footers can otherwise starve command acknowledgements.
        let (sender, receiver) = futures::channel::oneshot::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("monitter-composer-usage".into())
            .spawn(move || {
                let _ = sender.send(service.invoke(
                    "get_usage_overview", json!({"policy":"cache-only"}),
                ));
            })
        {
            self.usage_refresh.complete(&request, self.usage_clock.elapsed());
            self.usage_loading = false;
            self.usage_error = Some(format!("Could not start usage reader: {error}"));
            cx.notify();
            return;
        }
        self.usage_loading = true;
        self.usage_error = None;
        cx.spawn(async move |entity, cx| {
            let result = receiver.await.unwrap_or_else(|_| {
                Err("Composer usage reader stopped before responding.".into())
            });
            let _ = entity.update(cx, |pane, cx| {
                if pane
                    .usage_refresh
                    .complete(&request, pane.usage_clock.elapsed())
                {
                    pane.usage_loading = false;
                    match result {
                        Ok(overview) => pane.usage_overview = Some(overview),
                        Err(reason) => pane.usage_error = Some(reason),
                    }
                    cx.notify();
                }
                pane.drive_usage_refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn save_sandbox(&mut self, sandbox: String, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        self.model_save.enqueue_sandbox(sandbox);
        self.save_error = None;
        self.start_model_save(cx);
        cx.notify();
    }

    fn save_model(&mut self, settings: Value, cx: &mut Context<Self>) {
        if self.model_disabled() {
            return;
        }
        self.model_save.enqueue(settings);
        self.save_error = None;
        self.start_model_save(cx);
        cx.notify();
    }

    fn model_queue(&self, task: &str) -> Option<&ModelSaveQueue> {
        if self.task_id == task {
            Some(&self.model_save)
        } else {
            self.model_save_queues.get(task)
        }
    }
    fn model_queue_mut(&mut self, task: &str) -> Option<&mut ModelSaveQueue> {
        if self.task_id == task {
            Some(&mut self.model_save)
        } else {
            self.model_save_queues.get_mut(task)
        }
    }
    fn start_model_save(&mut self, cx: &mut Context<Self>) {
        self.start_model_save_for(self.task_id.clone(), cx);
    }
    fn start_model_save_for(&mut self, task_id: String, cx: &mut Context<Self>) {
        let task = self.snapshot["tasks"].as_array().and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["id"].as_str() == Some(task_id.as_str()))
        });
        if !model_save_queue::available(task) {
            return;
        }
        let access_waits = task.is_some_and(|task| {
            task["status"].as_str() == Some("running") && task["provider"].as_str() != Some("codex")
        });
        let Some(queue) = self.model_queue_mut(&task_id) else {
            return;
        };
        if access_waits && queue.desired.is_none() && queue.sandbox.is_some() {
            return;
        }
        if queue.begin().is_none() {
            return;
        }
        let epoch = queue.epoch;
        let retry = queue.attempts;
        let service = self.service.clone();
        cx.spawn(async move |entity, cx| {
            if retry > 0 {
                cx.background_executor()
                    .timer(Duration::from_millis(if retry == 1 { 150 } else { 450 }))
                    .await;
            }
            // The queue remains bound to its original task, even when this
            // retained footer is now displaying a different chat.
            let sent = match entity.update(cx, |pane, _| {
                let queue = pane.model_queue(&task_id)?;
                if queue.epoch != epoch {
                    return None;
                }
                queue.pending()
            }) {
                Ok(Some(settings)) => settings,
                _ => return,
            };
            let request = sent.clone();
            let request_task = task_id.clone();
            let work = cx.background_executor().spawn(async move {
                let mut service = service;
                let result = if let Some(sandbox) = request.get("sandbox") {
                    service.invoke(
                        "set_task_sandbox",
                        json!({"taskId":request_task,"sandbox":sandbox}),
                    )
                } else {
                    service.invoke(
                        "set_task_model_settings",
                        json!({"taskId":request_task,"settings":request}),
                    )
                };
                if result
                    .as_ref()
                    .err()
                    .is_some_and(|error| model_save_queue::recoverable(error))
                {
                    if let Ok(reconnected) = DesktopService::connect() {
                        service = reconnected;
                    }
                }
                let outcome =
                    model_save_queue::after_write(result, &request_task, &request, || {
                        service.invoke("get_ui_snapshot", json!({}))
                    });
                (service, outcome)
            });
            let (service, outcome) = work.await;
            let _ = entity.update(cx, |pane, cx| {
                let Some(queue) = pane.model_queue_mut(&task_id) else {
                    return;
                };
                if queue.epoch != epoch {
                    return;
                }
                let snapshot = match outcome {
                    SaveOutcome::Saved(snapshot) => {
                        queue.saved(&sent);
                        queue.error = None;
                        Some(snapshot)
                    }
                    SaveOutcome::Interrupted(snapshot) => {
                        queue.interrupted();
                        snapshot
                    }
                    SaveOutcome::Deferred => {
                        queue.working = false;
                        queue.paused = true;
                        None
                    }
                    SaveOutcome::Rejected(error) => {
                        queue.rejected(&sent);
                        queue.error = Some(error);
                        None
                    }
                };
                pane.service = service;
                if pane.task_id == task_id {
                    if let Some(snapshot) = snapshot {
                        pane.accept_save_result(Ok(snapshot), cx);
                    }
                    cx.notify();
                }
                pane.start_model_save_for(task_id.clone(), cx);
            });
        })
        .detach();
    }

    fn model_disabled(&self) -> bool {
        !model_save_queue::available(self.selected_task().as_ref())
    }

    fn model_pending_status(&self) -> Option<String> {
        let Some(settings) = self.model_save.desired.as_ref() else {
            return self
                .model_save
                .sandbox
                .as_ref()
                .map(|_| "Access change queued".into());
        };
        let model = settings["model"]
            .as_str()
            .filter(|model| !model.is_empty())
            .unwrap_or("Harness default");
        let effort = settings["reasoningEffort"]
            .as_str()
            .map(|effort| format!(" · {effort}"))
            .unwrap_or_default();
        let fast = if settings["fastMode"].as_bool() == Some(true) {
            " · Fast"
        } else {
            ""
        };
        Some(format!("Model change queued · {model}{effort}{fast}"))
    }

    fn accept_save_result(&mut self, result: Result<Value, String>, cx: &mut Context<Self>) {
        match result {
            Ok(snapshot) => {
                self.snapshot = footer_snapshot(&snapshot);
                let current_settings = self.selected_task().map(|task| {
                    let configured = task.get("modelSettings").cloned().unwrap_or(Value::Null);
                    if configured.get("model").and_then(Value::as_str).is_some_and(|model| !model.is_empty()) {
                        configured
                    } else {
                        json!({"model":task.get("model").and_then(Value::as_str).unwrap_or_default(),"reasoningEffort":null,"fastMode":null})
                    }
                });
                if let Some(catalog) = &mut self.catalog {
                    if let Some(current) = current_settings {
                        catalog["current"] = current;
                    }
                }
                cx.emit(ComposerFooterEvent::SnapshotUpdated {
                    task_id: self.task_id.clone(),
                    snapshot,
                });
            }
            Err(reason) => self.save_error = Some(reason),
        }
        cx.notify();
    }

    fn disabled(&self) -> bool {
        self.model_disabled()
    }

    fn selected_settings(&self, task: &Value) -> Value {
        if let Some(settings) = &self.model_save.desired {
            return settings.clone();
        }
        if task["modelSettings"].is_object() {
            return model_save_queue::normalized(&task["modelSettings"]);
        }
        // An explicit empty canonical model means harness defaults. A stale
        // cached catalog must not resurrect the previous choice after reset.
        if let Some(model) = task.get("model").and_then(Value::as_str) {
            return model_save_queue::normalized(&json!({"model":model}));
        }
        model_save_queue::normalized(
            self.catalog
                .as_ref()
                .and_then(|catalog| catalog.get("current"))
                .unwrap_or(&Value::Null),
        )
    }

    fn context_usage(&self, task: Option<&Value>) -> ContextUsage {
        let Some(task) = task else {
            return ContextUsage::Unavailable("Context usage is unavailable.".into());
        };
        if self.usage_overview.is_none() {
            if let Some(error) = self
                .usage_error
                .as_deref()
                .and_then(crate::chat::connection_recovery::visible_error)
            {
                return ContextUsage::Unavailable(format!(
                    "Context usage could not be read: {error}"
                ));
            }
            if self.usage_loading {
                return ContextUsage::Unavailable("Reading reported context usage…".into());
            }
        }
        let task_id = task.get("id").and_then(Value::as_str).unwrap_or_default();
        let cleared_at = self.context_cleared_at;
        let configured_model = task
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let mut runs = self
            .usage_overview
            .as_ref()
            .and_then(|overview| overview.get("recentRuns"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|run| {
                run.get("taskId").and_then(Value::as_str) == Some(task_id)
                    && run
                        .get("configuredModel")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        == configured_model
                    && cleared_at.is_none_or(|cleared| {
                        run.get("startedAt")
                            .and_then(Value::as_i64)
                            .unwrap_or_default()
                            >= cleared
                    })
                    && run.get("context").is_some_and(|context| {
                        let used = context.get("used").and_then(Value::as_f64).unwrap_or(-1.0);
                        let size = context
                            .get("size")
                            .and_then(Value::as_f64)
                            .unwrap_or_default();
                        used.is_finite() && used >= 0.0 && size.is_finite() && size > 0.0
                    })
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| {
            std::cmp::Reverse(
                run.get("startedAt")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            )
        });
        let Some(context) = runs.first().and_then(|run| run.get("context")) else {
            let reason = if let Some(error) = self
                .usage_error
                .as_deref()
                .and_then(crate::chat::connection_recovery::visible_error)
            {
                return ContextUsage::Unavailable(format!(
                    "Context usage could not be read: {error}"
                ));
            } else if cleared_at.is_some() {
                "Context usage is unavailable after the context was cleared, until this model reports its new context window."
            } else {
                "Context usage is unavailable until this model reports its context window."
            };
            return ContextUsage::Unavailable(reason.into());
        };
        let used = context
            .get("used")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let size = context
            .get("size")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        ContextUsage::Available {
            used,
            size,
            percent: (used / size * 100.0).clamp(0.0, 100.0),
            model: configured_model.to_owned(),
        }
    }

    pub fn render_context_bar(&self, palette: crate::design::Palette) -> AnyElement {
        render_context_bar(&self.context_usage(self.selected_task().as_ref()), palette)
    }

    /// Retain the measured toolbar height: GPUI cached view roots need an
    /// explicit layout size because their child tree is replayed in prepaint.
    pub fn cached_style(&self) -> StyleRefinement {
        StyleRefinement::default()
            .w_full()
            .h(self.retained_height.unwrap_or(px(32.)))
    }

    pub fn set_appearance(
        &mut self,
        palette: crate::design::Palette,
        width: Option<f32>,
        cx: &mut Context<Self>,
    ) {
        if self.composed_palette != Some(palette) || self.available_width_design != width {
            self.composed_palette = Some(palette);
            self.available_width_design = width;
            cx.notify();
        }
    }

    pub fn render_footer(
        &self,
        palette: Option<crate::design::Palette>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_footer_for_width(palette, None, window, cx)
    }

    pub fn render_footer_for_width(
        &self,
        palette: Option<crate::design::Palette>,
        available_width_design: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let compact_footer = available_width_design.is_some_and(|width| width < 520.0);
        let interface_font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.0)
            .clamp(8.0, 32.0) as f32
            / 14.0;
        let Some(task) = self.selected_task() else {
            return div().id("composer-footer-unavailable").into_any_element();
        };
        let provider = task
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("opencode");
        let sandbox = self
            .model_save
            .sandbox
            .as_ref()
            .unwrap_or(&task)
            .get("sandbox")
            .and_then(Value::as_str)
            .unwrap_or("harness-configured");
        let agent = self.selected_agent(&task);
        let acp = task
            .get("acp")
            .filter(|value| !value.is_null())
            .or_else(|| agent.as_ref().and_then(|value| value.get("acp")))
            .unwrap_or(&Value::Null);
        let selected = normalized_permission(provider, acp, sandbox);
        let jev_mode = agent
            .as_ref()
            .and_then(|a| a.get("jevRouting"))
            .and_then(Value::as_str)
            .unwrap_or("inherit");
        let jev_mode = if jev_mode == "inherit" {
            self.snapshot["settings"]["systemOneRouting"].as_str().unwrap_or("off")
        } else { jev_mode };
        let settings = self.selected_settings(&task);
        let catalog = self.catalog.clone();
        let catalog_loading = self.catalog_loading;
        let catalog_error = self
            .catalog_error
            .as_deref()
            .and_then(crate::chat::connection_recovery::visible_error)
            .map(str::to_owned);
        let save_error = self
            .model_save
            .error
            .clone()
            .or_else(|| self.save_error.clone())
            .filter(|error| crate::chat::connection_recovery::visible_error(error).is_some());
        let disabled = self.disabled();
        let applies_next_turn =
            task.get("status").and_then(Value::as_str) == Some("running") && provider == "codex";
        // Tauri applies interface scale as WebView zoom, so its CSS-pixel
        // spacing and icon dimensions scale alongside its interface text.
        let backend_limits_choices = ["read-only", "auto-approve-edits", "full-access"]
            .into_iter()
            .any(|level| {
                permission_supported(provider, acp, level)
                    && !backend_accepts_sandbox(provider, &stored_permission(provider, level))
            });
        let permission_hint = if backend_limits_choices {
            "Some permission levels are not supported by this task's current harness."
        } else {
            permission_hint(provider, acp)
        };
        let model_label = catalog
            .as_ref()
            .and_then(|c| c.get("models"))
            .and_then(Value::as_array)
            .and_then(|models| {
                models.iter().find(|model| {
                    model.get("id").and_then(Value::as_str)
                        == settings.get("model").and_then(Value::as_str)
                })
            })
            .and_then(|model| model.get("name"))
            .and_then(Value::as_str)
            .or_else(|| settings.get("model").and_then(Value::as_str))
            .filter(|value| !value.is_empty())
            .unwrap_or("Harness default");
        let measure = |text: &str| {
            window
                .text_system()
                .shape_line(
                    text.to_owned().into(),
                    px(11. * interface_font_ratio),
                    &[TextRun {
                        len: text.len(),
                        font: font(cx.theme().font_family.clone()),
                        color: rgb(palette.muted).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .x_for_index(text.len())
        };
        let permission_width =
            measure(permission_label(selected)) + px(14. + design::space::TIGHT + 14.);
        let model_reserve = if compact_footer && jev_mode == "off" {
            px(28.)
        } else if compact_footer {
            (measure(model_label) + px(64.)).min(px(220.))
        } else {
            px(220.)
        };
        let error_reserve = save_error.as_deref().map(&measure).unwrap_or(px(0.))
            + catalog_error.as_deref().map(&measure).unwrap_or(px(0.));
        let permission_compact = available_width_design.is_some_and(|width| {
            !permission_label_fits(
                f32::from(px(width)),
                f32::from(permission_width),
                f32::from(
                    model_reserve
                        + error_reserve
                        + px(design::space::INSET + 2. * design::space::TIGHT),
                ),
            )
        });
        let footer = div()
            .id("composer-footer-controls")
            .w_full()
            .min_h(px(40.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(design::space::INSET))
            .px(px(if compact_footer { 8. } else { 16. }))
            .font_family(cx.theme().font_family.clone())
            .text_size(design::ui_scaled_px(12.0 * interface_font_ratio))
            .text_color(rgb(palette.text(1)));
        let access_content = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(design::space::TIGHT))
            .when(permission_compact, |content| content.justify_center())
            .child(
                Icon::default()
                    .data(if self.model_save.sandbox.is_some() {
                        icon_assets::LoaderCircle.1
                    } else {
                        icon_assets::Shield.1
                    })
                    .with_size(px(14.)),
            )
            .when(!permission_compact, |content| {
                content.child(
                    div()
                        .flex_shrink_0()
                        .debug_selector(|| "composer-permission-state-label".into())
                        .font_family(cx.theme().font_family.clone())
                        .text_size(design::ui_scaled_px(12.0 * interface_font_ratio))
                        .whitespace_nowrap()
                        .child(permission_label(selected)),
                )
            });
        let access = Button::new("composer-permissions")
            .with_size(Size::Small)
            .compact()
            .h_auto()
            .min_h(px(0.))
            .px(px(7.))
            .py(px(5.))
            .flex_shrink_0()
            .when(!permission_compact, |button| {
                button.w(permission_width).min_w(permission_width)
            })
            .when(permission_compact, |button| {
                button
                    .w(px(28.))
                    .h(px(28.))
                    .min_w(px(28.))
                    .max_w(px(28.))
                    .min_h(px(28.))
                    .px(px(0.))
                    .py(px(0.))
                    .justify_center()
            })
            .rounded(px(design::radius::CONTROL))
            .font_family(cx.theme().font_family.clone())
            .text_size(design::ui_scaled_px(11.0 * interface_font_ratio))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(rgba(0x00000000).into())
                    .foreground(rgb(palette.text(1)).into())
                    .hover(crate::design::surface_color(palette.fill(2)).into())
                    .active(crate::design::surface_color(palette.fill(3)).into()),
            )
            .accessibility_label(format!(
                "Access Permissions: {}",
                permission_label(selected)
            ))
            .child(access_content)
            .tooltip(if disabled {
                "Available after this run finishes."
            } else if applies_next_turn {
                "Applies to the next turn."
            } else if backend_limits_choices {
                "Some permission levels are not supported by this task's current harness."
            } else {
                "Permissions for this chat."
            })
            .disabled(disabled);
        let access = composer_popup::permissions_menu(self, access, palette, window, cx);
        let mut left = div()
            .flex()
            .items_center()
            .gap(px(design::space::TIGHT))
            .child(access);
        if let Some(error) = save_error {
            left = left.child(div().text_color(rgb(palette.err())).child(error));
        }
        if let Some(status) = self.model_pending_status() {
            left = left.child(
                div()
                    .min_w_0()
                    .max_w(px(200.))
                    .text_size(px(10. * interface_font_ratio))
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .child(status),
            );
        }
        let jev_enabled = jev_mode != "off";
        let selected_model = catalog
            .as_ref()
            .and_then(|c| c.get("models"))
            .and_then(Value::as_array)
            .and_then(|models| {
                models.iter().find(|model| {
                    model.get("id").and_then(Value::as_str)
                        == settings.get("model").and_then(Value::as_str)
                })
            });
        let effort = settings
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .or_else(|| {
                selected_model.and_then(|model| model.get("defaultEffort").and_then(Value::as_str))
            })
            .or_else(|| {
                selected_model
                    .and_then(|model| model.get("reasoningEfforts").and_then(Value::as_array))
                    .and_then(|levels| levels.first())
                    .and_then(|level| level.get("id"))
                    .and_then(Value::as_str)
            });
        let fast = selected_model
            .is_some_and(|model| model.get("supportsFast").and_then(Value::as_bool) == Some(true))
            && settings.get("fastMode").and_then(Value::as_bool) == Some(true);
        let model_button = Button::new("composer-model-selection")
            .with_size(Size::Small)
            .compact()
            .h_auto()
            .min_h(px(0.))
            .px(px(7.))
            .py(px(5.))
            .max_w(px(220.))
            .min_w_0()
            .when(compact_footer, |button| {
                if jev_enabled {
                    button
                        .h(px(28.))
                        .min_h(px(28.))
                        .px(px(7.))
                        .py(px(0.))
                        .max_w(px(220.))
                } else {
                    button
                        .w(px(28.))
                        .h(px(28.))
                        .min_w(px(28.))
                        .max_w(px(28.))
                        .min_h(px(28.))
                        .px(px(0.))
                        .py(px(0.))
                        .justify_center()
                }
            })
            .rounded(px(design::radius::CONTROL))
            .font_family(cx.theme().font_family.clone())
            .text_size(design::ui_scaled_px(11.0 * interface_font_ratio))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(rgba(0x00000000).into())
                    .foreground(rgb(palette.text(1)).into())
                    .hover(crate::design::surface_color(palette.fill(2)).into())
                    .active(crate::design::surface_color(palette.fill(3)).into()),
            )
            .accessibility_label(format!(
                "{}Model: {}",
                if jev_enabled {
                    "System One routing enabled for new chats. "
                } else {
                    ""
                },
                model_label
            ))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(design::space::TIGHT))
                    .when(jev_enabled, |this| {
                        this.child(
                            div()
                                .px(px(5.))
                                .py(px(2.))
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(palette.glow()))
                                .text_color(rgb(palette.accent))
                                .font_family(design::FONT_MONO)
                                .text_size(design::ui_scaled_px(10. * interface_font_ratio))
                                .child("SYS-1"),
                        )
                    })
                    .child(
                        Icon::default()
                            .data(if catalog_loading || self.model_save.desired.is_some() {
                                icon_assets::LoaderCircle.1
                            } else {
                                icon_assets::Brain.1
                            })
                            .with_size(px(14.)),
                    )
                    .when(fast && !compact_footer, |this| {
                        this.child(
                            div().ml(-px(5.)).child(
                                Icon::default()
                                    .data(icon_assets::Zap.1)
                                    .with_size(px(11.))
                                    .text_color(rgb(palette.accent)),
                            ),
                        )
                    })
                    .when(!compact_footer || jev_enabled, |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(cx.theme().font_family.clone())
                                .text_size(design::ui_scaled_px(11.0 * interface_font_ratio))
                                .truncate()
                                .child(model_label.to_owned()),
                        )
                    })
                    .when(!compact_footer && effort.is_some(), |this| {
                        this.child(
                            div()
                                .text_size(design::ui_scaled_px(11.0 * interface_font_ratio))
                                .text_color(rgb(palette.text(2)))
                                .child(effort.unwrap_or_default().to_owned()),
                        )
                    }),
            )
            .tooltip(if disabled {
                "Model can be changed when this run finishes."
            } else if applies_next_turn {
                "Applies to the next turn."
            } else {
                "Choose a model and reasoning effort."
            })
            .disabled(self.model_disabled());
        let model_button = composer_popup::model_menu(self, model_button, palette, window, cx);
        let mut right = div().flex().items_center().gap(px(design::space::TIGHT));
        if let Some(error) = catalog_error {
            right = right.child(div().text_color(rgb(palette.err())).child(error));
        }
        right = right.child(model_button);
        footer.child(left).child(right).into_any_element()
    }
}

impl Render for ComposerFooterPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let toolbar = self.render_footer_for_width(
            self.composed_palette,
            self.available_width_design,
            window,
            cx,
        );
        div().relative().w_full().child(toolbar).child(
            canvas(
                move |bounds, window, cx| {
                    let owner = owner.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = owner.update(cx, |footer, cx| {
                            if footer.retained_height.is_none_or(|height| {
                                (height - bounds.size.height).abs() > gpui_kit::px(0.25)
                            }) {
                                footer.retained_height = Some(bounds.size.height);
                                cx.notify();
                            }
                        });
                    });
                },
                |_, _, _, _| (),
            )
            .absolute()
            .size_full(),
        )
    }
}

/// The context bar and toolbar are composer siblings with distinct layout.
/// Both retain the same authoritative footer state without eagerly rebuilding
/// model menus and usage metadata on every native input event.
pub struct ComposerContextBar {
    footer: Entity<ComposerFooterPane>,
    palette: crate::design::Palette,
    _footer_change: Subscription,
}
impl ComposerContextBar {
    pub fn new(
        footer: Entity<ComposerFooterPane>,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) -> Self {
        let change = cx.observe(&footer, |_, _, cx| cx.notify());
        Self {
            footer,
            palette,
            _footer_change: change,
        }
    }
    pub fn set_appearance(&mut self, palette: crate::design::Palette, cx: &mut Context<Self>) {
        if self.palette != palette {
            self.palette = palette;
            cx.notify();
        }
    }
}
impl Render for ComposerContextBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.footer.read(cx).render_context_bar(self.palette)
    }
}

#[derive(Clone, Debug)]
enum ContextUsage {
    Available {
        used: f64,
        size: f64,
        percent: f64,
        model: String,
    },
    Unavailable(String),
}

fn permission_capabilities(provider: &str, acp: &Value) -> (bool, bool, bool) {
    let command = acp
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let key = if provider == "claude" {
        "claude-code"
    } else if provider == "codex" {
        "codex"
    } else if provider == "hermes" || provider == "acp" && command.contains("hermes") {
        "hermes"
    } else if provider == "acp" && (command.contains("gemini") || command.contains("agy")) {
        "gemini"
    } else if provider == "acp" && (command.contains("mcode") || command.contains("minimax")) {
        "minimax"
    } else if provider == "acp" && command.contains("mona") {
        "mona"
    } else {
        "opencode"
    };
    match key {
        "hermes" => (false, true, false),
        "gemini" => (false, true, true),
        _ => (true, true, true),
    }
}

fn permission_supported(provider: &str, acp: &Value, level: &str) -> bool {
    let caps = permission_capabilities(provider, acp);
    match level {
        "read-only" => caps.0,
        "auto-approve-edits" => caps.1,
        "full-access" => caps.2,
        _ => false,
    }
}

/// Mirror the backend's `valid_sandbox_for_provider` contract so the picker
/// never offers a UI capability that the native command will reject.
fn backend_accepts_sandbox(provider: &str, sandbox: &str) -> bool {
    match provider {
        "codex" => matches!(sandbox, "read-only" | "workspace-write" | "yolo"),
        "claude" => matches!(sandbox, "harness-configured" | "yolo"),
        "opencode" | "hermes" => sandbox == "harness-configured",
        "acp" => matches!(sandbox, "harness-configured" | "yolo"),
        _ => false,
    }
}

fn normalized_permission(provider: &str, acp: &Value, sandbox: &str) -> &'static str {
    let preferred = match sandbox {
        "read-only" => "read-only",
        "workspace-write" => "auto-approve-edits",
        "yolo" => "full-access",
        "harness-configured" if provider == "codex" => "read-only",
        _ => "auto-approve-edits",
    };
    if permission_supported(provider, acp, preferred) {
        preferred
    } else if permission_supported(provider, acp, "auto-approve-edits") {
        "auto-approve-edits"
    } else if permission_supported(provider, acp, "full-access") {
        "full-access"
    } else {
        "auto-approve-edits"
    }
}

fn stored_permission(provider: &str, level: &str) -> String {
    match level {
        "read-only" => "read-only".into(),
        "auto-approve-edits" if provider == "codex" => "workspace-write".into(),
        "auto-approve-edits" => "harness-configured".into(),
        "full-access" => "yolo".into(),
        _ => "harness-configured".into(),
    }
}

fn permission_label_fits(available: f32, label_control: f32, reserved: f32) -> bool {
    available.is_finite() && available >= label_control + reserved
}

fn permission_label(level: &str) -> &'static str {
    match level {
        "read-only" => "Read only",
        "full-access" => "Full access",
        _ => "Auto-approve edits",
    }
}

fn permission_description(level: &str) -> &'static str {
    match level {
        "read-only" => "Can read files. Can’t make changes.",
        "full-access" => "Can run any command, edit anything, no prompts.",
        _ => "Edits workspace files without asking. Won’t run arbitrary commands.",
    }
}

fn permission_hint(provider: &str, acp: &Value) -> &'static str {
    let command = acp
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    if provider == "codex" {
        "Codex supports all three levels."
    } else if provider == "claude" {
        "Claude Code supports all three levels."
    } else if provider == "opencode" {
        "OpenCode supports all three levels."
    } else if provider == "hermes" || command.contains("hermes") {
        "Hermes Agent currently uses its configured ACP permission policy."
    } else if command.contains("gemini") || command.contains("agy") {
        "Gemini doesn’t support Read only — its lowest setting still allows edits."
    } else if command.contains("mcode") || command.contains("minimax") {
        "MiniMax supports all three levels."
    } else if command.contains("mona") {
        "Mona supports all three levels."
    } else {
        "The selected harness controls its configured permission behavior."
    }
}

fn render_context_bar(usage: &ContextUsage, palette: crate::design::Palette) -> AnyElement {
    let (percent, label, meter_value, unavailable) = match usage {
        ContextUsage::Available {
            used,
            size,
            percent,
            model,
        } => {
            let format = format_token_count;
            (
                *percent,
                format!(
                    "Context: {} of {} tokens used ({}%){}",
                    format(*used),
                    format(*size),
                    percent.round() as u64,
                    if model.is_empty() {
                        String::new()
                    } else {
                        format!(". {model}")
                    }
                ),
                format!("{} of {} tokens used", format(*used), format(*size)),
                false,
            )
        }
        ContextUsage::Unavailable(reason) => (0.0, reason.clone(), reason.clone(), true),
    };
    let percent = percent.clamp(0.0, 100.0);
    let color = context_color(percent, palette);
    // Div overflow clips rectangular bounds only. Paint the intersection of
    // this bottom stripe with the composer's rounded inner outline, then mask
    // the filled portion by the percentage; it cannot leak through a corner.
    let radius = (f32::from(px(design::radius::CONTROL)) - 1.).max(0.);
    let fill = canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let radius = radius.min(width / 2.);
            let point_at = |x: f32, y: f32| {
                point(
                    bounds.origin.x + gpui_kit::px(x),
                    bounds.origin.y + gpui_kit::px(y),
                )
            };
            let inset = |y: f32| {
                let dy = (radius - height + y).max(0.);
                radius - (radius * radius - dy * dy).max(0.).sqrt()
            };
            let mut shape = PathBuilder::fill();
            shape.move_to(point_at(inset(0.), 0.));
            shape.line_to(point_at(width - inset(0.), 0.));
            for step in 1..=16 {
                let y = height * step as f32 / 16.;
                shape.line_to(point_at(width - inset(y), y));
            }
            shape.line_to(point_at(inset(height), height));
            for step in (0..16).rev() {
                let y = height * step as f32 / 16.;
                shape.line_to(point_at(inset(y), y));
            }
            shape.close();
            if let Ok(path) = shape.build() {
                window.paint_path(path.clone(), design::surface_color(palette.wash(palette.paper)));
                let mask = ContentMask::new(Bounds::new(
                    bounds.origin,
                    size(
                        bounds.size.width * (percent as f32 / 100.),
                        bounds.size.height,
                    ),
                ));
                window.with_content_mask(Some(mask), |window| window.paint_path(path, rgb(color)));
            }
        },
    )
    .size_full();
    let mut bar = div()
        .id("composer-context-usage")
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(px(4.))
        .overflow_hidden()
        .aria_label(label.clone())
        .managed_tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx));
    if unavailable {
        bar = bar
            .role(gpui_kit::accesskit::Role::Status)
            .aria_value(meter_value);
        bar = bar.opacity(0.7);
    } else {
        bar = bar
            .role(gpui_kit::accesskit::Role::Meter)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(100.0)
            .aria_numeric_value(percent)
            .aria_value(meter_value);
    }
    bar = bar.child(fill);
    bar.into_any_element()
}

fn format_token_count(value: f64) -> String {
    let digits = format!("{}", value.round() as u64);
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(digit);
    }
    output
}

fn context_color(percent: f64, palette: design::Palette) -> u32 {
    if percent <= 60.0 {
        palette.ok()
    } else if percent <= 90.0 {
        design::mix_rgb(palette.ok(), palette.warn(), ((percent - 60.0) / 30.0) as f32)
    } else {
        design::mix_rgb(palette.warn(), palette.err(), ((percent - 90.0) / 10.0) as f32)
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod permission_label_layout_tests {
    use super::{ComposerFooterPane, permission_label_fits};
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window, div, px, size,
    };
    use monitter_lib::experimental_desktop::DesktopService;
    use serde_json::json;
    struct Fixture {
        pane: Entity<ComposerFooterPane>,
        width: f32,
    }
    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let controls = self.pane.update(cx, |pane, cx| {
                pane.render_footer_for_width(None, Some(self.width), window, cx)
            });
            div()
                .w(crate::design::ui_scaled_px(self.width))
                .child(controls)
        }
    }
    #[::core::prelude::v1::test]
    fn permission_label_width_budget_uses_actual_space_instead_of_footer_mode() {
        assert!(permission_label_fits(300., 130., 52.));
        assert!(!permission_label_fits(140., 130., 52.));
        assert!(permission_label_fits(182., 130., 52.));
        assert!(!permission_label_fits(f32::NAN, 130., 52.));
    }
    #[gpui_kit::test]
    fn permission_label_native_layout_is_full_in_compact_footer_when_room_exists(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut retained_pane = None;
        let mut retained_fixture = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|_| ComposerFooterPane {
                service: DesktopService::disconnected_for_ui_fixture(),
                snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"label-task","provider":"codex","status":"idle","archived":false,"sandbox":"read-only","model":"fixture-model"}],"agents":[]}),
                task_id: "label-task".into(), context_cleared_at: None, catalog: None, usage_overview: None,
                catalog_loading: false, usage_loading: false, saving: false, model_save: Default::default(), model_save_queues: Default::default(), busy: false,
                catalog_error: None, usage_error: None, save_error: None, model_menu_open: false, model_menu_open_generation: 0,
                catalog_generation: 0, usage_refresh: Default::default(), usage_clock: std::time::Instant::now(), save_generation: 0,
                observed_task_marker: None, composed_palette: None, available_width_design: None, retained_height: None,
            });
            retained_pane = Some(pane.clone());
            let fixture = cx.new(|_| Fixture { pane, width: 300. });
            retained_fixture = Some(fixture.clone());
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        let pane = retained_pane.unwrap();
        let fixture = retained_fixture.unwrap();
        visual.simulate_resize(size(px(900.), px(600.)));
        for sandbox in ["read-only", "workspace-write", "yolo"] {
            for (width, visible) in [(700., true), (300., true), (70., false)] {
                visual.update(|window, cx| {
                    fixture.update(cx, |fixture, cx| {
                        fixture.width = width;
                        cx.notify();
                    });
                    pane.update(cx, |pane, cx| {
                        pane.snapshot["tasks"][0]["sandbox"] = json!(sandbox);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let label = visual.debug_bounds("composer-permission-state-label");
                assert_eq!(label.is_some(), visible, "{sandbox} width {width}");
                if let Some(label) = label {
                    assert!(label.size.width > px(30.));
                    assert!(
                        label.left() >= px(0.)
                            && label.right() <= crate::design::ui_scaled_px(width),
                        "{label:?}"
                    );
                }
            }
        }
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod model_save_tests {
    use super::ComposerFooterPane;
    use gpui_kit::{AppContext as _, TestAppContext};
    use monitter_lib::experimental_desktop::DesktopService;
    use serde_json::{Value, json};
    #[gpui_kit::test]
    fn task_switch_keeps_original_queue_and_model_controls_ignore_busy_state(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let snapshot = json!({"settings":{}, "agents":[], "tasks":[
            {"id":"a","provider":"claude","status":"running","archived":false,"model":"original-a"},
            {"id":"b","provider":"codex","status":"idle","archived":false,"model":"original-b"}
        ]});
        let (pane, visual) = cx.add_window_view(|window, cx| {
            ComposerFooterPane::new(
                DesktopService::disconnected_for_ui_fixture(),
                &snapshot,
                "a",
                window,
                cx,
            )
        });
        visual.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.busy = true;
                pane.saving = true;
                assert!(
                    !pane.disabled(),
                    "permission controls accept queued intent while busy"
                );
                assert!(
                    !pane.model_disabled(),
                    "model controls remain usable during running non-Codex turns and other saves"
                );
                pane.catalog = Some(json!({"models":[{"id":"latest"}]}));
                pane.catalog_loading = true;
                pane.catalog_error = Some("Refresh failed".into());
                assert!(pane.catalog.is_some());
                assert!(
                    !pane.model_disabled(),
                    "cached choices stay usable during catalog refresh/failure"
                );
                pane.model_save.enqueue(json!({"model":"first"}));
                let first = pane.model_save.begin().unwrap();
                pane.model_save
                    .enqueue(json!({"model":"latest","reasoningEffort":"high","fastMode":true}));
                assert_eq!(
                    pane.selected_settings(&pane.selected_task().unwrap())["model"],
                    "latest"
                );
                pane.set_snapshot(&snapshot, "b", cx);
                assert!(pane.model_save.desired.is_none());
                assert_eq!(
                    pane.selected_settings(&pane.selected_task().unwrap())["model"],
                    "original-b"
                );
                let original = pane
                    .model_save_queues
                    .get_mut("a")
                    .expect("original task retains its worker and unsent latest choice");
                assert!(original.working);
                original.saved(&first);
                let next = original.begin().unwrap();
                assert_eq!(next["model"], "latest");
                assert_eq!(next["reasoningEffort"], "high");
                assert_eq!(next["fastMode"], true);
                assert!(
                    pane.model_save.desired.is_none(),
                    "background acknowledgement never changes the visible task queue"
                );
                pane.catalog = Some(json!({"current":{"model":"stale-before-reset"}}));
                let mut reset_snapshot = snapshot.clone();
                reset_snapshot["tasks"][1]["model"] = json!("");
                reset_snapshot["tasks"][1]["modelSettings"] = Value::Null;
                pane.set_snapshot(&reset_snapshot, "b", cx);
                assert_eq!(
                    pane.selected_settings(&pane.selected_task().unwrap())["model"],
                    "",
                    "canonical reset must beat stale catalog.current"
                );
            });
        });
    }
}
