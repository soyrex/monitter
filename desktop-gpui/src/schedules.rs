//! Native schedule controls. Backend validation and dispatch remain authoritative;
//! this entity only edits drafts and calls the typed owner service off the UI thread.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{
        Disableable,
        button::*,
        input::{Input, InputState, Textarea, TextareaState},
    },
    prelude::*,
    *,
};
use monitter_lib::{
    experimental_desktop::{DesktopService, ScheduleAction, ScheduleInput},
    model::{OverlapPolicy, ScheduleMode},
};
use serde_json::Value;
use std::collections::HashMap;

fn field(row: &Value, key: &str) -> String {
    row[key].as_str().unwrap_or_default().to_owned()
}
fn rows(snapshot: &Value, key: &str) -> Vec<Value> {
    snapshot[key].as_array().cloned().unwrap_or_default()
}
const PRESETS: &[(&str, &str)] = &[
    ("every_5_minutes", "Every 5 minutes"),
    ("every_15_minutes", "Every 15 minutes"),
    ("every_30_minutes", "Every 30 minutes"),
    ("hourly", "Hourly"),
    ("daily_9am", "Daily at 09:00"),
    ("weekday_mornings", "Weekdays at 09:00"),
    ("weekly_monday", "Monday at 09:00"),
    ("monthly_first", "First of month at 09:00"),
];

#[derive(Clone)]
struct Draft {
    title: String,
    agent_id: String,
    prompt: String,
    cron: String,
    tz: String,
    preset: Option<usize>,
    mode: ScheduleMode,
    overlap: OverlapPolicy,
    enabled: bool,
    failures: String,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            title: String::new(),
            agent_id: String::new(),
            prompt: String::new(),
            cron: "0 9 * * *".into(),
            tz: "host".into(),
            preset: Some(4),
            mode: ScheduleMode::PersistentThread,
            overlap: OverlapPolicy::Queue,
            enabled: false,
            failures: String::new(),
        }
    }
}
impl Draft {
    fn from_row(row: &Value) -> Self {
        Self {
            title: field(row, "title"),
            agent_id: field(row, "agentId"),
            prompt: field(row, "prompt"),
            cron: field(row, "frequency"),
            tz: field(row, "tz"),
            preset: None,
            mode: ScheduleMode::parse(&field(row, "mode"))
                .unwrap_or(ScheduleMode::PersistentThread),
            overlap: OverlapPolicy::parse(&field(row, "overlapPolicy"))
                .unwrap_or(OverlapPolicy::Skip),
            enabled: row["enabled"].as_bool().unwrap_or(false),
            failures: row["maxConsecutiveFailures"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_default(),
        }
    }
    fn input(&self, id: String) -> Result<ScheduleInput, String> {
        let failures = if self.failures.trim().is_empty() {
            None
        } else {
            Some(self.failures.trim().parse::<u32>().map_err(|_| {
                "Auto-pause failures must be a positive whole number, or blank to disable."
                    .to_owned()
            })?)
        };
        if failures == Some(0) {
            return Err("Auto-pause failures must be greater than zero.".into());
        }
        Ok(ScheduleInput {
            id,
            title: self.title.clone(),
            agent_id: self.agent_id.clone(),
            prompt: self.prompt.clone(),
            preset: self.preset.map(|n| PRESETS[n].0.to_owned()),
            frequency: if self.preset.is_none() {
                Some(self.cron.clone())
            } else {
                None
            },
            tz: self.tz.clone(),
            mode: self.mode,
            overlap_policy: Some(if self.mode == ScheduleMode::PersistentThread {
                self.overlap
            } else {
                OverlapPolicy::Skip
            }),
            max_consecutive_failures: failures,
            enabled: self.enabled,
        })
    }
}
enum Operation {
    Save(ScheduleInput),
    Action(String, ScheduleAction),
}

pub struct SchedulePane {
    service: DesktopService,
    snapshot: Value,
    selected: String,
    drafts: HashMap<String, Draft>,
    draft: Draft,
    title: Entity<InputState>,
    prompt: Entity<TextareaState>,
    cron: Entity<InputState>,
    tz: Entity<InputState>,
    failures: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    confirm: Option<&'static str>,
    reset_fields: bool,
    created_unselected: bool,
}
impl SchedulePane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = Draft::default();
        Self {
            service,
            snapshot: snapshot.clone(),
            selected: String::new(),
            drafts: HashMap::new(),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("Schedule title")),
            prompt: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Prompt sent to the chosen agent on each fire")
            }),
            cron: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(draft.cron.clone())
                    .placeholder("Five-field cron")
            }),
            tz: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value("host")
                    .placeholder("host or IANA timezone")
            }),
            failures: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Blank disables automatic pause")
            }),
            draft,
            busy: false,
            error: None,
            notice: None,
            confirm: None,
            reset_fields: false,
            created_unselected: false,
        }
    }
    pub fn set_snapshot(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        self.snapshot = snapshot.clone();
        cx.notify();
    }
    fn capture(&mut self, cx: &Context<Self>) {
        self.draft.title = self.title.read(cx).value().to_string();
        self.draft.prompt = self.prompt.read(cx).value().to_string();
        self.draft.cron = self.cron.read(cx).value().to_string();
        self.draft.tz = self.tz.read(cx).value().to_string();
        self.draft.failures = self.failures.read(cx).value().to_string();
        self.drafts
            .insert(self.selected.clone(), self.draft.clone());
    }
    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.capture(cx);
        self.draft = self.drafts.get(&id).cloned().unwrap_or_else(|| {
            rows(&self.snapshot, "schedules")
                .iter()
                .find(|row| field(row, "id") == id)
                .map(Draft::from_row)
                .unwrap_or_default()
        });
        if !id.is_empty() {
            self.created_unselected = false;
        }
        self.selected = id;
        self.confirm = None;
        self.error = None;
        self.notice = None;
        self.load_fields(window, cx);
        cx.notify();
    }
    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.title.update(cx, |v, cx| {
            v.set_value(self.draft.title.clone(), window, cx)
        });
        self.prompt.update(cx, |v, cx| {
            v.set_value(self.draft.prompt.clone(), window, cx)
        });
        self.cron
            .update(cx, |v, cx| v.set_value(self.draft.cron.clone(), window, cx));
        self.tz
            .update(cx, |v, cx| v.set_value(self.draft.tz.clone(), window, cx));
        self.failures.update(cx, |v, cx| {
            v.set_value(self.draft.failures.clone(), window, cx)
        });
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.created_unselected {
            return;
        }
        self.capture(cx);
        match self.draft.input(self.selected.clone()) {
            Ok(input) => self.request(Operation::Save(input), cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    fn request(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        self.confirm = None;
        let service = self.service.clone();
        let was_new = matches!(&operation,Operation::Save(input) if input.id.is_empty());
        let old_ids: Vec<String> = rows(&self.snapshot, "schedules")
            .iter()
            .map(|row| field(row, "id"))
            .collect();
        let deleted = matches!(&operation, Operation::Action(_, ScheduleAction::Delete));
        let worker = cx.background_executor().spawn(async move {
            let snapshot = match operation {
                Operation::Save(input) => service.save_schedule(input),
                Operation::Action(id, action) => service.schedule_action(id, action),
            }?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Err(error) => pane.error = Some(error),
                    Ok(snapshot) => {
                        pane.snapshot = snapshot;
                        if was_new {
                            let candidates: Vec<Value> = rows(&pane.snapshot, "schedules")
                                .into_iter().filter(|row| !old_ids.contains(&field(row,"id"))
                                    && field(row,"agentId")==pane.draft.agent_id
                                    && field(row,"title")==pane.draft.title
                                    && field(row,"prompt")==pane.draft.prompt).collect();
                            if candidates.len()==1 {
                                let id=field(&candidates[0],"id");
                                pane.drafts.remove(""); pane.selected=id.clone();
                                pane.drafts.insert(id,pane.draft.clone());
                            } else {
                                pane.created_unselected = true;
                                pane.notice=Some("Schedule created; select the saved row in the list before making further edits.".into());
                            }
                        }
                        if deleted {
                            pane.drafts.remove(&pane.selected); pane.selected=String::new();
                            pane.reset_fields=true;
                            pane.notice=Some("Deleted schedule and its run history. Existing tasks remain unchanged.".into());
                        } else if pane.notice.is_none() {
                            pane.notice=Some("Backend accepted the schedule action.".into());
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let id = self.selected.clone();
        let row = rows(&self.snapshot, "schedules")
            .into_iter()
            .find(|row| field(row, "id") == id);
        let enabled = row
            .as_ref()
            .and_then(|row| row["enabled"].as_bool())
            .unwrap_or(false);
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("schedule-save")
                    .label(if id.is_empty() {
                        "Create schedule"
                    } else {
                        "Save draft"
                    })
                    .disabled(self.busy || self.created_unselected)
                    .on_click(cx.listener(|pane, _, _, cx| pane.save(cx))),
            )
            .when(!id.is_empty(), |ui| {
                ui.child(
                    Button::new("schedule-toggle")
                        .label(if enabled { "Pause" } else { "Resume" })
                        .disabled(self.busy || row.is_none())
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.request(
                                Operation::Action(
                                    pane.selected.clone(),
                                    if enabled {
                                        ScheduleAction::Pause
                                    } else {
                                        ScheduleAction::Resume
                                    },
                                ),
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new("schedule-run")
                        .label("Run now…")
                        .disabled(self.busy || row.is_none())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.confirm = Some("run");
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("schedule-delete")
                        .label("Delete…")
                        .disabled(self.busy || row.is_none())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.confirm = Some("delete");
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}
impl Render for SchedulePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A deleted row resets only after a successful backend response.
        if std::mem::take(&mut self.reset_fields) {
            self.draft = self.drafts.get("").cloned().unwrap_or_default();
            self.load_fields(window, cx);
        }
        let mut list = div()
            .id("schedule-list")
            .w(px(230.))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Button::new("new-schedule")
                    .label("New schedule")
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(|pane, _, window, cx| pane.select(String::new(), window, cx)),
                    ),
            );
        for row in rows(&self.snapshot, "schedules") {
            let id = field(&row, "id");
            let chosen = id == self.selected;
            let label = format!(
                "{}{} · {}",
                if chosen { "• " } else { "" },
                field(&row, "title"),
                if row["enabled"].as_bool() == Some(true) {
                    "enabled"
                } else {
                    "paused"
                }
            );
            list = list.child(
                Button::new(SharedString::from(format!("schedule-{id}")))
                    .label(label)
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(move |pane, _, window, cx| pane.select(id.clone(), window, cx)),
                    ),
            );
        }
        let mut editor = div()
            .id("schedule-editor")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child("Schedule title")
            .child(Input::new(&self.title).disabled(self.busy))
            .child("Dispatch agent");
        editor = editor.child(
            Button::new("schedule-reload-draft")
                .label("Discard draft and reload saved values")
                .disabled(self.busy)
                .on_click(cx.listener(|pane, _, window, cx| {
                    pane.draft = rows(&pane.snapshot, "schedules")
                        .iter()
                        .find(|row| field(row, "id") == pane.selected)
                        .map(Draft::from_row)
                        .unwrap_or_default();
                    pane.drafts.remove(&pane.selected);
                    pane.confirm = None;
                    pane.load_fields(window, cx);
                    cx.notify();
                })),
        );
        let agents: Vec<Value> = rows(&self.snapshot, "agents")
            .into_iter()
            .filter(|row| row["internal"].as_bool() != Some(true))
            .collect();
        let mut choices = div().flex().flex_wrap().gap_2();
        for agent in agents.iter() {
            let id = field(agent, "id");
            let selected = self.draft.agent_id == id;
            choices = choices.child(
                Button::new(SharedString::from(format!("schedule-agent-{id}")))
                    .label(format!(
                        "{}{}",
                        if selected { "• " } else { "" },
                        field(agent, "name")
                    ))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |pane, _, _, cx| {
                        pane.draft.agent_id = id.clone();
                        cx.notify();
                    })),
            );
        }
        if !self.draft.agent_id.is_empty()
            && !agents
                .iter()
                .any(|row| field(row, "id") == self.draft.agent_id)
        {
            editor = editor
                .child("Chosen agent no longer exists. Select an available agent before saving.");
        }
        editor = editor
            .child(choices)
            .child("Prompt")
            .child(Textarea::new(&self.prompt).h(px(150.)).disabled(self.busy))
            .child("Frequency — times use the timezone below");
        let mut presets = div().flex().flex_wrap().gap_2();
        for (index, (_, label)) in PRESETS.iter().enumerate() {
            presets = presets.child(
                Button::new(SharedString::from(format!("schedule-preset-{index}")))
                    .label(format!(
                        "{}{}",
                        if self.draft.preset == Some(index) {
                            "• "
                        } else {
                            ""
                        },
                        label
                    ))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |pane, _, _, cx| {
                        pane.draft.preset = Some(index);
                        cx.notify();
                    })),
            );
        }
        presets = presets.child(
            Button::new("schedule-custom-cron")
                .label(if self.draft.preset.is_none() {
                    "• Custom cron"
                } else {
                    "Custom cron"
                })
                .disabled(self.busy)
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.draft.preset = None;
                    cx.notify();
                })),
        );
        editor = editor
            .child(presets)
            .when(self.draft.preset.is_none(), |ui| {
                ui.child(Input::new(&self.cron).disabled(self.busy))
            })
            .child("Timezone (host or IANA, for example Europe/Madrid)")
            .child(Input::new(&self.tz).disabled(self.busy));
        let mut modes = div().flex().flex_wrap().gap_2();
        for (mode, label) in [
            (ScheduleMode::PersistentThread, "Reuse persistent thread"),
            (ScheduleMode::NewThreadPerFire, "New thread per fire"),
            (ScheduleMode::Throwaway, "Throwaway report"),
        ] {
            modes = modes.child(
                Button::new(SharedString::from(format!(
                    "schedule-mode-{}",
                    mode.stored()
                )))
                .label(format!(
                    "{}{}",
                    if self.draft.mode == mode { "• " } else { "" },
                    label
                ))
                .disabled(self.busy)
                .on_click(cx.listener(move |pane, _, _, cx| {
                    pane.draft.mode = mode;
                    if mode != ScheduleMode::PersistentThread {
                        pane.draft.overlap = OverlapPolicy::Skip;
                    }
                    cx.notify();
                })),
            );
        }
        editor = editor
            .child("Execution mode")
            .child(modes)
            .child("Overlap when the prior run is active");
        let mut overlap = div().flex().gap_2();
        for (policy, label) in [
            (OverlapPolicy::Skip, "Skip"),
            (OverlapPolicy::Queue, "Queue follow-up"),
        ] {
            overlap = overlap.child(
                Button::new(SharedString::from(format!(
                    "schedule-overlap-{}",
                    policy.stored()
                )))
                .label(format!(
                    "{}{}",
                    if self.draft.overlap == policy {
                        "• "
                    } else {
                        ""
                    },
                    label
                ))
                .disabled(
                    self.busy
                        || (policy == OverlapPolicy::Queue
                            && self.draft.mode != ScheduleMode::PersistentThread),
                )
                .on_click(cx.listener(move |pane, _, _, cx| {
                    pane.draft.overlap = policy;
                    cx.notify();
                })),
            );
        }
        editor = editor
            .child(overlap)
            .child("Automatic pause after consecutive failures (blank disables)")
            .child(Input::new(&self.failures).disabled(self.busy))
            .child(
                Button::new("schedule-enabled")
                    .label(if self.draft.enabled {
                        "Draft: enabled — click to pause"
                    } else {
                        "Draft: paused — click to enable"
                    })
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.draft.enabled = !pane.draft.enabled;
                        cx.notify();
                    })),
            )
            .child(self.actions(cx));
        if let Some(action) = self.confirm {
            editor=editor.child(if action=="run"{"Run now dispatches the saved schedule immediately, even when paused. Unsaved draft edits are not used."}else{"Delete this schedule and all its run history? Tasks created by past runs remain."})
            .child(div().flex().gap_2().child(Button::new("schedule-confirm").label(if action=="run"{"Confirm run now"}else{"Confirm delete"}).disabled(self.busy).on_click(cx.listener(move|pane,_,_,cx|pane.request(Operation::Action(pane.selected.clone(),if action=="run"{ScheduleAction::RunNow}else{ScheduleAction::Delete}),cx))))
            .child(Button::new("schedule-cancel-action").label("Cancel").on_click(cx.listener(|pane,_,_,cx|{pane.confirm=None;cx.notify();}))));
        }
        if self.busy {
            editor = editor.child("Waiting for the backend…");
        }
        if let Some(error) = &self.error {
            editor = editor.child(
                div().flex().flex_col().text_color(rgb(0xd35e5e)).children(
                    error
                        .lines()
                        .map(|line| div().min_h(px(18.)).child(line.to_owned())),
                ),
            );
        }
        if let Some(notice) = &self.notice {
            editor = editor.child(notice.clone());
        }
        if let Some(row) = rows(&self.snapshot, "schedules")
            .iter()
            .find(|row| field(row, "id") == self.selected)
        {
            editor = editor.child(format!(
                "Saved: {} · {} · failures {} · last fire {}",
                field(row, "frequency"),
                field(row, "tz"),
                row["consecutiveFailures"].as_u64().unwrap_or(0),
                row["lastFireAtMs"]
                    .as_i64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "never".into())
            ));
        }
        editor = editor.child("Run history (newest first; timestamps are Unix milliseconds)");
        let mut history = rows(&self.snapshot, "scheduleRuns");
        history.retain(|run| field(run, "scheduleId") == self.selected);
        history.sort_by_key(|run| std::cmp::Reverse(run["firedAtMs"].as_i64().unwrap_or(0)));
        if history.is_empty() {
            editor = editor.child("No recorded runs for this schedule.");
        }
        for run in history {
            let mut text = format!(
                "{} · {}\n{}",
                run["firedAtMs"].as_i64().unwrap_or(0),
                field(&run, "status"),
                field(&run, "summary")
            );
            if !field(&run, "error").is_empty() {
                text.push_str(&format!("\nError: {}", field(&run, "error")));
            }
            if !field(&run, "taskId").is_empty() {
                text.push_str(&format!("\nTask: {}", field(&run, "taskId")));
            }
            if run["skippedOverlap"].as_bool() == Some(true) {
                text.push_str("\nSkipped because the prior run was active.");
            }
            editor = editor.child(
                div().flex().flex_col().children(
                    text.lines()
                        .map(|line| div().min_h(px(18.)).child(line.to_owned())),
                ),
            );
        }
        div().size_full().flex().gap_3().child(list).child(editor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn preset_and_custom_are_exclusive_and_nonpersistent_cannot_queue() {
        let mut draft = Draft::default();
        draft.mode = ScheduleMode::NewThreadPerFire;
        let input = draft.input("existing".into()).unwrap();
        assert_eq!(input.preset.as_deref(), Some("daily_9am"));
        assert!(input.frequency.is_none());
        assert_eq!(input.overlap_policy, Some(OverlapPolicy::Skip));
        draft.preset = None;
        draft.cron = "*/15 * * * *".into();
        let input = draft.input(String::new()).unwrap();
        assert!(input.preset.is_none());
        assert_eq!(input.frequency.as_deref(), Some("*/15 * * * *"));
    }
    #[::core::prelude::v1::test]
    fn failure_limit_never_silently_disables_invalid_input() {
        let mut draft = Draft::default();
        for invalid in ["-1", "0", "x", "4294967296"] {
            draft.failures = invalid.into();
            assert!(draft.input(String::new()).is_err());
        }
        draft.failures = " 3 ".into();
        assert_eq!(
            draft.input(String::new()).unwrap().max_consecutive_failures,
            Some(3)
        );
        draft.failures = "".into();
        assert_eq!(
            draft.input(String::new()).unwrap().max_consecutive_failures,
            None
        );
        assert!(!draft.enabled, "New schedules require explicit enablement");
    }
}
