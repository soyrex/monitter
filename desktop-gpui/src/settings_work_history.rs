//! Private, owner-local Work History. No history or configuration enters Snapshot.
use crate::design::{Palette, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::{
    component::{
        Disableable,
        button::*,
        checkbox::Checkbox,
        input::{Input, InputState},
        switch::Switch,
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::HashSet, thread};

const PAGE_SIZE: usize = 32;
const MAX_ROWS: usize = 256;
const MIB: u64 = 1024 * 1024;

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn timestamp(value: &Value) -> String {
    value
        .as_i64()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "Unknown time".into())
}
fn date_boundary(value: &str, end: bool) -> Result<Option<i64>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let date = chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map_err(|_| "Enter dates as YYYY-MM-DD (UTC).".to_owned())?;
    let start = date
        .and_hms_opt(0, 0, 0)
        .ok_or("Invalid date.")?
        .and_utc()
        .timestamp_millis();
    Ok(Some(if end { start + 86_400_000 - 1 } else { start }))
}
fn filtered_query(
    project: &str,
    agent: &str,
    outcome: &str,
    from: &str,
    through: &str,
) -> Result<Value, String> {
    let from = date_boundary(from, false)?;
    let through = date_boundary(through, true)?;
    if from.zip(through).is_some_and(|(start, end)| start > end) {
        return Err("The end date must be on or after the start date.".into());
    }
    if from
        .zip(through)
        .is_some_and(|(start, end)| end - start > 366 * 86_400_000)
    {
        return Err("Choose a date range of at most 366 days.".into());
    }
    if from.is_none() && through.is_none() && project.trim().is_empty() {
        return Err("Choose a date range or a project to bound history and exports.".into());
    }
    let mut query = json!({"timezone":"UTC", "limit":PAGE_SIZE});
    for (key, value) in [
        ("projectId", project),
        ("agentId", agent),
        ("outcome", outcome),
    ] {
        let value = value.trim();
        if value.len() > 128 || value.chars().any(char::is_control) {
            return Err(
                "Filter values must be 128 bytes or fewer and contain no control characters."
                    .into(),
            );
        }
        if !value.is_empty() {
            query[key] = json!(value);
        }
    }
    if let Some(from) = from {
        query["from"] = json!(from);
    }
    if let Some(through) = through {
        query["through"] = json!(through);
    }
    Ok(query)
}
/// Clone the complete owner settings, including future fields, before changing a switch.
fn with_enabled(settings: &Value, enabled: bool) -> Result<Value, String> {
    if !settings.is_object() {
        return Err("Load collection settings first.".into());
    }
    let mut next = settings.clone();
    next["enabled"] = json!(enabled);
    Ok(next)
}
fn with_limits(settings: &Value, values: &[String]) -> Result<Value, String> {
    let mut next = with_enabled(settings, settings["enabled"].as_bool().unwrap_or(false))?;
    let numbers = values
        .iter()
        .map(|value| {
            value
                .trim()
                .parse::<u64>()
                .map_err(|_| "Enter whole numbers for retention and storage limits.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if numbers.len() != 4
        || numbers[0] > 365
        || numbers[1] < numbers[0]
        || numbers[1] > 3650
        || numbers[2] < 1
        || numbers[3] < numbers[2]
        || numbers[3] > 65536
    {
        return Err("Raw retention: 0–365 days; summary retention: raw retention–3650 days; storage: 1–65536 MiB, soft limit no larger than hard limit.".into());
    }
    next["rawRetentionDays"] = json!(numbers[0]);
    next["minuteRetentionDays"] = json!(numbers[1]);
    next["softLimitBytes"] = json!(numbers[2] * MIB);
    next["hardLimitBytes"] = json!(numbers[3] * MIB);
    Ok(next)
}
fn selected_runs(selected: &HashSet<String>) -> Vec<String> {
    let mut ids: Vec<_> = selected
        .iter()
        .filter(|id| !id.is_empty())
        .cloned()
        .collect();
    ids.sort();
    ids
}
fn confirmed_selection(
    selected: &HashSet<String>,
    confirmation: Option<&Vec<String>>,
) -> Option<Vec<String>> {
    let ids = selected_runs(selected);
    (!ids.is_empty() && confirmation == Some(&ids)).then_some(ids)
}
fn note(value: impl Into<SharedString>, palette: Palette) -> Div {
    super::settings_hint(value, palette)
}
fn field(label: &str, input: &Entity<InputState>) -> Div {
    div()
        .min_w(px(150.))
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(label.to_owned())
        .child(
            Input::new(input)
                .aria_label(label.to_owned())
                .text_size(crate::settings::settings_text_size(0.86)),
        )
}

pub(super) struct WorkHistoryPane {
    service: DesktopService,
    settings: Option<Value>,
    sync_limits: bool,
    health: Option<Value>,
    summary: Option<Value>,
    entries: Vec<Value>,
    next_cursor: Option<String>,
    coverage: Vec<String>,
    query: Value,
    filters: Vec<Entity<InputState>>,
    limits: Vec<Entity<InputState>>,
    selected: HashSet<String>,
    confirmation: Option<Vec<String>>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    export: Option<Value>,
    choosing_export: bool,
}
impl WorkHistoryPane {
    pub(super) fn new(
        service: DesktopService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let today = chrono::Utc::now().date_naive();
        let from = (today - chrono::Duration::days(29))
            .format("%Y-%m-%d")
            .to_string();
        let through = today.format("%Y-%m-%d").to_string();
        let query = filtered_query("", "", "", &from, &through).expect("UTC date range is valid");
        let filters = [
            "Project ID (all if blank)",
            "Agent ID (all if blank)",
            "Outcome (all if blank)",
            "From YYYY-MM-DD",
            "Through YYYY-MM-DD",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, placeholder)| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(match index {
                        3 => from.clone(),
                        4 => through.clone(),
                        _ => String::new(),
                    })
            })
        })
        .collect();
        let limits = [
            "Raw retention days",
            "Summary retention days",
            "Soft limit MiB",
            "Hard limit MiB",
        ]
        .into_iter()
        .map(|placeholder| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)))
        .collect();
        let mut pane = Self {
            service,
            settings: None,
            sync_limits: false,
            health: None,
            summary: None,
            entries: Vec::new(),
            next_cursor: None,
            coverage: Vec::new(),
            query,
            filters,
            limits,
            selected: HashSet::new(),
            confirmation: None,
            busy: false,
            error: None,
            notice: None,
            export: None,
            choosing_export: false,
        };
        pane.refresh(false, cx);
        pane
    }
    pub(super) fn rehost_to_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        for input in self.filters.iter().chain(self.limits.iter()) {
            input.update(cx, |input, cx| input.rehost_to_window(window, cx))?;
        }
        Ok(())
    }
    fn refresh(&mut self, append: bool, cx: &mut Context<Self>) {
        let mut query = self.query.clone();
        if append {
            if self.entries.len() >= MAX_ROWS {
                return;
            }
            let Some(cursor) = &self.next_cursor else {
                return;
            };
            query["cursor"] = json!(cursor);
        }
        self.invoke(if append { "more" } else { "refresh" }, query, cx);
    }
    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        let values: Vec<String> = self
            .filters
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        match filtered_query(&values[0], &values[1], &values[2], &values[3], &values[4]) {
            Ok(query) => {
                self.query = query;
                self.confirmation = None;
                self.selected.clear();
                self.refresh(false, cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    fn configure_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let result = self
            .settings
            .as_ref()
            .ok_or_else(|| "Load collection settings first.".to_owned())
            .and_then(|settings| with_enabled(settings, enabled));
        self.configure(result, cx);
    }
    fn save_limits(&mut self, cx: &mut Context<Self>) {
        let values = self
            .limits
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect::<Vec<_>>();
        let result = self
            .settings
            .as_ref()
            .ok_or_else(|| "Load collection settings first.".to_owned())
            .and_then(|settings| with_limits(settings, &values));
        self.configure(result, cx);
    }
    fn configure(&mut self, result: Result<Value, String>, cx: &mut Context<Self>) {
        match result {
            Ok(value) => self.invoke("configure", value, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    /// All owner calls, including reads, use a bounded-stack worker rather than the UI thread.
    fn invoke(&mut self, operation: &'static str, args: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let service = self.service.clone();
        let (tx, rx) = oneshot::channel();
        let spawn = thread::Builder::new()
            .name("gpui-work-history".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = if matches!(operation, "refresh" | "more") {
                    let mut bundle = json!({});
                    let mut errors = Vec::new();
                    // Keep successful settings/health available even when the history DB is unavailable.
                    for (key, command, input) in [
                        ("settings", "settings", json!({})),
                        ("health", "health", json!({})),
                        ("page", "query", args.clone()),
                        ("summary", "summary", args),
                    ] {
                        match service.work_log_owner(command, input) {
                            Ok(value) => bundle[key] = value,
                            Err(error) => errors.push(format!("{command}: {error}")),
                        }
                    }
                    bundle["errors"] = json!(errors);
                    Ok(bundle)
                } else {
                    service.work_log_owner(operation, args)
                };
                let _ = tx.send(result);
            });
        if let Err(error) = spawn {
            self.busy = false;
            self.error = Some(format!("Could not start Work History worker: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane, cx| {
            let result = rx
                .await
                .unwrap_or_else(|_| Err("Work History worker stopped before replying.".into()));
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Ok(value) if matches!(operation, "refresh" | "more") => {
                        if let Some(settings) =
                            value.get("settings").and_then(|v| v.get("settings"))
                        {
                            if pane.settings.as_ref() != Some(settings) {
                                pane.sync_limits = true;
                            }
                            pane.settings = Some(settings.clone());
                        }
                        if let Some(health) = value.get("health") {
                            pane.health = Some(health.clone());
                        }
                        if let Some(summary) = value.get("summary") {
                            pane.summary = Some(summary.clone());
                        }
                        if let Some(page) = value.get("page") {
                            if operation == "refresh" {
                                pane.entries.clear();
                                pane.selected.clear();
                                pane.confirmation = None;
                            }
                            if let Some(entries) = page["entries"].as_array() {
                                pane.entries.extend(
                                    entries
                                        .iter()
                                        .take(MAX_ROWS.saturating_sub(pane.entries.len()))
                                        .cloned(),
                                );
                            }
                            pane.next_cursor = page["nextCursor"].as_str().map(str::to_owned);
                            pane.coverage = page["coverage"]
                                .as_array()
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .map(str::to_owned)
                                        .collect()
                                })
                                .unwrap_or_default();
                        } else if operation == "refresh" {
                            // A failed new query must never display rows from an earlier filter.
                            pane.entries.clear();
                            pane.next_cursor = None;
                            pane.coverage.clear();
                            pane.summary = None;
                            pane.selected.clear();
                            pane.confirmation = None;
                        }
                        let errors = value["errors"]
                            .as_array()
                            .map(|errors| {
                                errors
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            })
                            .unwrap_or_default();
                        if !errors.is_empty() {
                            pane.error = Some(errors);
                        }
                    }
                    Ok(value) if operation == "configure" => {
                        pane.settings = Some(value);
                        pane.sync_limits = true;
                        pane.notice =
                            Some("Collection settings saved locally and applied now.".into());
                        pane.refresh(false, cx);
                    }
                    Ok(_) if operation == "deleteHistory" => {
                        pane.confirmation = None;
                        pane.selected.clear();
                        pane.notice = Some("Selected run history deleted locally.".into());
                        pane.refresh(false, cx);
                    }
                    Ok(value)
                        if matches!(operation, "export" | "exportStatus" | "cancelExport") =>
                    {
                        pane.export = Some(value);
                        pane.poll_export(cx);
                    }
                    Ok(_) => {}
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn poll_export(&self, cx: &mut Context<Self>) {
        let Some(export) = self
            .export
            .as_ref()
            .filter(|value| value["status"] == "running")
        else {
            return;
        };
        let id = text(export, "exportId");
        cx.spawn(async move |pane, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(750))
                .await;
            let _ =
                pane.update(cx, |pane, cx| {
                    if pane.export.as_ref().is_some_and(|value| {
                        value["exportId"] == id && value["status"] == "running"
                    }) {
                        if pane.busy {
                            pane.poll_export(cx);
                        } else {
                            pane.invoke("exportStatus", json!({"exportId":id}), cx);
                        }
                    }
                });
        })
        .detach();
    }
    fn choose_export(&mut self, format: &'static str, cx: &mut Context<Self>) {
        if self.busy
            || self.choosing_export
            || self
                .export
                .as_ref()
                .is_some_and(|v| v["status"] == "running")
        {
            return;
        }
        self.choosing_export = true;
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let name = format!(
            "monitter-work-history-{}.{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            format
        );
        let prompt = cx.prompt_for_new_path(&home, Some(&name));
        let query = self.query.clone();
        cx.spawn(async move |pane, cx| {
            let selected = prompt.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.choosing_export = false;
                match selected {
                    Ok(Ok(Some(path))) => pane.invoke(
                        "export",
                        json!({"query":query, "path":path, "format":format}),
                        cx,
                    ),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        pane.error = Some(format!("Could not open save dialog: {error}"))
                    }
                    Err(error) => pane.error = Some(format!("Save dialog closed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(ids) = confirmed_selection(&self.selected, self.confirmation.as_ref()) {
            self.invoke("deleteHistory", json!({"runIds":ids}), cx);
        }
    }
    pub(super) fn render(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
        presentation: &super::SearchCardPresentation,
    ) -> AnyElement {
        if self.sync_limits {
            if let Some(settings) = &self.settings {
                for (input, value) in self.limits.iter().zip([
                    settings["rawRetentionDays"].as_u64().unwrap_or(30),
                    settings["minuteRetentionDays"].as_u64().unwrap_or(365),
                    settings["softLimitBytes"].as_u64().unwrap_or(MIB) / MIB,
                    settings["hardLimitBytes"].as_u64().unwrap_or(MIB) / MIB,
                ]) {
                    input.update(cx, |input, cx| {
                        input.set_value(value.to_string(), window, cx)
                    });
                }
            }
            self.sync_limits = false;
        }
        let disabled = self.busy || self.choosing_export;
        let enabled = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings["enabled"] == true);
        let mut collection = presentation.card("Work History collection", "Private history stored on this desktop. Collection is prospective; earlier work is not reconstructed. Agent access requires separate owner grants.", palette)
            .child(div().flex().items_center().justify_between().gap(px(12.))
                .child(div().child("Collect new agent work"))
                .child(Switch::new("work-history-enabled").checked(enabled).disabled(disabled || self.settings.is_none())
                    .accessibility_label("Collect new agent work locally")
                    .on_change(cx.listener(|pane, enabled, _, cx| pane.configure_enabled(*enabled, cx)))))
            .child(note(if enabled { "Collection is enabled for new activity. Refresh to check collector health." } else { "Collection is paused. Existing history stays available; activity while paused is not backfilled." }, palette));
        if let Some(health) = &self.health {
            collection = collection.child(note(format!("Collector: {}{} · last write: {}", text(health, "state"), health["reason"].as_str().map(|reason| format!(" · {reason}")).unwrap_or_default(), timestamp(&health["lastWriteAt"])), palette))
                .child(note(format!("Storage: {:.1} MiB database + {:.1} MiB WAL · {} queued records ({:.1} KiB) · {} dropped resource observations", health["databaseBytes"].as_u64().unwrap_or(0) as f64 / MIB as f64, health["walBytes"].as_u64().unwrap_or(0) as f64 / MIB as f64, health["pendingRows"].as_u64().unwrap_or(0), health["pendingBytes"].as_u64().unwrap_or(0) as f64 / 1024., health["droppedResources"].as_u64().unwrap_or(0)), palette));
        }
        let mut limits = div().flex().flex_wrap().gap(px(10.));
        for (label, input) in [
            "Raw retention (days)",
            "Summary retention (days)",
            "Soft storage limit (MiB)",
            "Hard storage limit (MiB)",
        ]
        .iter()
        .zip(&self.limits)
        {
            limits = limits.child(field(label, input));
        }
        collection = collection.child(limits)
            .child(note("Retention removes older detail and summaries automatically. Storage pressure can remove history sooner. Raw: 0–365 days; summaries: up to 3650 days; storage: up to 65536 MiB.", palette))
            .child(Button::new("work-history-save-limits").compact().label("Save retention & limits").disabled(disabled || self.settings.is_none()).on_click(cx.listener(|pane, _, _, cx| pane.save_limits(cx))));
        let mut filters = div().flex().flex_wrap().gap(px(10.));
        for (label, input) in [
            "Project ID",
            "Agent ID",
            "Outcome",
            "From (UTC)",
            "Through (UTC)",
        ]
        .iter()
        .zip(&self.filters)
        {
            filters = filters.child(field(label, input));
        }
        let mut history = presentation.card("Recorded work", "Starts with the last 30 UTC days. Filter by exact project or agent ID, outcome, and inclusive UTC dates. Keep a date range or a project to bound history. Apply filters before exporting.", palette)
            .child(filters)
            .child(div().flex().flex_wrap().gap(px(8.))
                .child(Button::new("work-history-apply").compact().label("Apply filters").disabled(disabled).on_click(cx.listener(|pane, _, _, cx| pane.apply_filters(cx))))
                .child(Button::new("work-history-refresh").compact().label(if self.busy { "Loading…" } else { "Refresh" }).disabled(disabled).on_click(cx.listener(|pane, _, _, cx| pane.refresh(false, cx)))));
        if let Some(summary) = &self.summary {
            history = history.child(note(format!("{} runs · {:.1} agent elapsed hours · {:.1} project covered hours · {} runs without usage · {} resolution", summary["runs"].as_u64().unwrap_or(0), summary["agentElapsedMs"].as_u64().unwrap_or(0) as f64 / 3_600_000., summary["projectCoveredMs"].as_u64().unwrap_or(0) as f64 / 3_600_000., summary["missingUsageRuns"].as_u64().unwrap_or(0), text(summary, "resolution")), palette))
                .child(note("Elapsed time measures agent activity, not human effort. Coverage and usage may be incomplete.", palette));
        }
        if !self.coverage.is_empty() {
            history = history.child(note(
                format!("Coverage: {}", self.coverage.join(", ")),
                palette,
            ));
        }
        if self.entries.is_empty() && !self.busy {
            history = history.child(note(if self.error.is_some() { "History could not be loaded. Review the error and refresh." } else if enabled { "No recorded work matches these filters. Only activity collected after enabling Work History appears here." } else { "No recorded work matches these filters. Enable collection to record future activity." }, palette));
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let run = &entry["run"];
            let event = &entry["event"];
            let id = text(run, "runId");
            let selected = self.selected.contains(&id);
            let completed = run["endedAt"].is_number();
            let title = text(run, "title");
            let project = run["projectName"].as_str().unwrap_or("No project");
            let outcome = run["outcome"].as_str().unwrap_or("In progress / unknown");
            history = history.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .child(
                        Checkbox::new(format!("work-history-select-{index}"))
                            .label(if title.is_empty() {
                                "Untitled run".to_owned()
                            } else {
                                title
                            })
                            .checked(selected)
                            .disabled(disabled || !completed)
                            .on_change(cx.listener(move |pane, checked, _, cx| {
                                if *checked {
                                    pane.selected.insert(id.clone());
                                } else {
                                    pane.selected.remove(&id);
                                }
                                pane.confirmation = None;
                                cx.notify();
                            })),
                    )
                    .child(note(
                        format!(
                            "{} · {} · {} · {}",
                            project,
                            text(run, "agentName"),
                            outcome,
                            timestamp(&event["occurredAt"])
                        ),
                        palette,
                    ))
                    .child(note(
                        format!(
                            "Project ID: {} · Agent ID: {}",
                            text(run, "projectId"),
                            text(run, "agentId")
                        ),
                        palette,
                    ))
                    .child(note(
                        format!(
                            "{} · {} · {}",
                            text(event, "kind"),
                            text(event, "summary"),
                            text(event, "coverage")
                        ),
                        palette,
                    ))
                    .when(!completed, |row| {
                        row.child(note(
                            "This run is still active; its history cannot be deleted yet.",
                            palette,
                        ))
                    }),
            );
        }
        history = history
            .child(note(
                format!(
                    "Showing {} events (maximum {} per view). Narrow filters for older history.",
                    self.entries.len(),
                    MAX_ROWS
                ),
                palette,
            ))
            .child(
                Button::new("work-history-more")
                    .compact()
                    .label("Load more")
                    .disabled(
                        disabled || self.next_cursor.is_none() || self.entries.len() >= MAX_ROWS,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.refresh(true, cx))),
            );
        let exporting = self
            .export
            .as_ref()
            .is_some_and(|export| export["status"] == "running");
        let mut export = presentation.card("Export & delete", "Export all history matching the applied filters to a new private local file. Choose a directory owned by you; existing files are never overwritten. Deletion removes all recorded events for selected runs.", palette)
            .child(div().flex().flex_wrap().gap(px(8.))
                .child(Button::new("work-history-export-json").compact().label("Export JSON…").disabled(disabled || exporting).on_click(cx.listener(|pane, _, _, cx| pane.choose_export("json", cx))))
                .child(Button::new("work-history-export-csv").compact().label("Export CSV…").disabled(disabled || exporting).on_click(cx.listener(|pane, _, _, cx| pane.choose_export("csv", cx))))
                .child(Button::new("work-history-delete").compact().label(format!("Delete {} selected runs…", self.selected.len())).disabled(disabled || self.selected.is_empty()).on_click(cx.listener(|pane, _, _, cx| { pane.confirmation = Some(selected_runs(&pane.selected)); cx.notify(); }))));
        if let Some(result) = &self.export {
            let receipt = &result["receipt"];
            export = export.child(note(
                format!(
                    "Export: {}{}{}",
                    text(result, "status"),
                    result["error"]
                        .as_str()
                        .map(|error| format!(" · {error}"))
                        .unwrap_or_default(),
                    receipt["path"]
                        .as_str()
                        .map(|path| format!(
                            " · {} entries · {path}",
                            receipt["entries"].as_u64().unwrap_or(0)
                        ))
                        .unwrap_or_default()
                ),
                palette,
            ));
            if exporting {
                let id = text(result, "exportId");
                let check_id = id.clone();
                export = export.child(
                    Button::new("work-history-check-export")
                        .compact()
                        .label("Check export status")
                        .disabled(disabled)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.invoke("exportStatus", json!({"exportId":check_id}), cx)
                        })),
                );
                export = export.child(
                    Button::new("work-history-cancel-export")
                        .compact()
                        .label("Cancel export")
                        .disabled(disabled)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.invoke("cancelExport", json!({"exportId":id}), cx)
                        })),
                );
            }
        }
        if let Some(ids) = &self.confirmation {
            export = export.child(note(format!("Permanently delete local history for {} selected runs? This cannot be undone. Agent sessions and transcripts are preserved.", ids.len()), palette))
                .child(div().flex().gap(px(8.))
                    .child(Button::new("work-history-confirm-delete").compact().label("Permanently delete selected history").disabled(disabled).on_click(cx.listener(|pane, _, _, cx| pane.confirm_delete(cx))))
                    .child(Button::new("work-history-cancel-delete").compact().label("Cancel").disabled(disabled).on_click(cx.listener(|pane, _, _, cx| { pane.confirmation = None; cx.notify(); }))));
        }
        let mut pane = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(16.))
            .text_size(crate::settings::settings_text_size(0.86));
        if let Some(error) = &self.error {
            pane = pane.child(note(format!("Work History error: {error}"), palette));
        }
        if let Some(notice) = &self.notice {
            pane = pane.child(note(notice.clone(), palette));
        }
        pane.child(collection)
            .child(history)
            .child(export)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn toggle_preserves_all_owner_settings() {
        let before = json!({"enabled":false,"rawRetentionDays":11,"minuteRetentionDays":222,"softLimitBytes":12345678,"hardLimitBytes":87654321,"futureField":{"keep":true}});
        let mut expected = before.clone();
        expected["enabled"] = json!(true);
        assert_eq!(with_enabled(&before, true).unwrap(), expected);
        assert!(!before["enabled"].as_bool().unwrap());
        assert!(with_enabled(&Value::Null, true).is_err());
    }
    #[::core::prelude::v1::test]
    fn filters_are_bounded_and_dates_are_inclusive_utc() {
        let query = filtered_query(" p1 ", "a1", "succeeded", "2026-10-18", "2026-10-18").unwrap();
        assert_eq!(query["projectId"], "p1");
        assert_eq!(query["limit"], PAGE_SIZE);
        assert_eq!(
            query["through"].as_i64().unwrap() - query["from"].as_i64().unwrap(),
            86_399_999
        );
        assert_eq!(query["timezone"], "UTC");
        assert!(query.get("cursor").is_none());
        assert!(filtered_query("", "", "", "2026-10-19", "2026-10-18").is_err());
        assert!(filtered_query("", "", "", "yesterday", "").is_err());
        assert!(filtered_query("", "", "", "2025-01-01", "2026-10-18").is_err());
        assert!(filtered_query("", "", "", "", "").is_err());
        assert!(filtered_query(&"x".repeat(129), "", "", "", "").is_err());
        assert!(filtered_query(&"é".repeat(65), "", "", "", "").is_err());
        assert!(
            filtered_query("", "", "", "2026-10-18", "")
                .unwrap()
                .get("projectId")
                .is_none()
        );
    }
    #[::core::prelude::v1::test]
    fn deletion_requires_the_same_nonempty_confirmed_selection() {
        let selected = ["a".to_owned(), "b".to_owned()].into_iter().collect();
        let confirmed = vec!["a".to_owned(), "b".to_owned()];
        assert!(confirmed_selection(&selected, None).is_none());
        assert_eq!(
            confirmed_selection(&selected, Some(&confirmed)),
            Some(confirmed)
        );
        assert!(confirmed_selection(&selected, Some(&vec!["a".into()])).is_none());
        assert!(confirmed_selection(&HashSet::new(), Some(&Vec::new())).is_none());
    }
    #[::core::prelude::v1::test]
    fn retention_validation_and_selection_are_explicit() {
        let original = json!({"enabled":true,"future":42});
        let next =
            with_limits(&original, &["30", "365", "1024", "2048"].map(str::to_owned)).unwrap();
        assert_eq!(next["future"], 42);
        assert_eq!(next["hardLimitBytes"], 2048 * MIB);
        assert!(with_limits(&original, &["366", "365", "1", "2"].map(str::to_owned)).is_err());
        assert!(with_limits(&original, &["30", "29", "1", "2"].map(str::to_owned)).is_err());
        assert!(with_limits(&original, &["30", "365", "3", "2"].map(str::to_owned)).is_err());
        assert_eq!(
            selected_runs(&["b".into(), "a".into(), "".into()].into_iter().collect()),
            vec!["a", "b"]
        );
    }
}
