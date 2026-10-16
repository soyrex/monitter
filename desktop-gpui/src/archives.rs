//! Native archived-chat search, restore, and confirmed permanent deletion.
//! Deletion eligibility and native-file cleanup are always revalidated by Service.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Icon, Disableable,
        button::*,
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

pub enum ArchiveEvent {
    Refresh,
}

impl EventEmitter<ArchiveEvent> for ArchivedChatsPane {}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn interface_font_size(settings: &Value) -> Pixels {
    px(settings["interfaceFontSize"]
        .as_f64()
        .unwrap_or(14.)
        .clamp(8., 32.) as f32)
}

pub struct ArchivedChatsPane {
    service: DesktopService,
    snapshot: Value,
    query: Entity<InputState>,
    tasks: Vec<Value>,
    list: ListState,
    pending_delete: Option<Value>,
    preview: Option<Value>,
    preview_loading: bool,
    expanded_summary: Option<String>,
    summary: Option<Result<String, String>>,
    summary_loading: bool,
    summary_request: u64,
    remove_native_files: bool,
    busy: bool,
    error: Option<String>,
    generation: u64,
    _query_change: Subscription,
}

impl ArchivedChatsPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search chat, agent, or host"));
        let query_change = cx.subscribe(&query, |pane, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                pane.rebuild_rows(cx);
                cx.notify();
            }
        });
        let mut pane = Self {
            service,
            snapshot: snapshot.clone(),
            query,
            tasks: vec![],
            list: ListState::new(0, ListAlignment::Top, px(100.)),
            pending_delete: None,
            preview: None,
            preview_loading: false,
            expanded_summary: None,
            summary: None,
            summary_loading: false,
            summary_request: 0,
            remove_native_files: false,
            busy: false,
            error: None,
            generation: 0,
            _query_change: query_change,
        };
        pane.rebuild_rows(cx);
        pane
    }

    pub fn set_snapshot(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        self.snapshot = snapshot.clone();
        self.rebuild_rows(cx);
        self.retain_pending_delete();
        if self.expanded_summary.as_ref().is_some_and(|id| !self.tasks.iter().any(|task| field(task, "id") == *id)) {
            self.expanded_summary = None;
            self.summary = None;
            self.summary_loading = false;
            self.summary_request += 1;
        }
        cx.notify();
    }

    /// A new visit starts without carrying a previous destructive confirmation.
    /// Keep an in-flight mutation's busy state until its actual response arrives.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.pending_delete = None;
        self.preview = None;
        self.preview_loading = false;
        self.expanded_summary = None;
        self.summary = None;
        self.summary_loading = false;
        self.summary_request += 1;
        self.remove_native_files = false;
        self.error = None;
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.rebuild_rows(cx);
        cx.notify();
    }

    fn retain_pending_delete(&mut self) {
        if let Some(pending) = &self.pending_delete {
            let id = field(pending, "id");
            if !self.tasks.iter().any(|task| field(task, "id") == id) {
                self.generation += 1;
                self.pending_delete = None;
                self.preview = None;
                self.preview_loading = false;
                self.remove_native_files = false;
            }
        }
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let query = self
            .query
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_lowercase();
        let agents = rows(&self.snapshot, "agents");
        let hosts = rows(&self.snapshot, "hosts");
        let internal: std::collections::HashSet<String> = agents
            .iter()
            .filter(|agent| agent["internal"].as_bool() == Some(true))
            .map(|agent| field(agent, "id"))
            .collect();
        let mut tasks: Vec<_> = rows(&self.snapshot, "tasks")
            .into_iter()
            .filter(|task| {
                task["archived"].as_bool() == Some(true)
                    && !internal.contains(&field(task, "agentId"))
            })
            .filter(|task| {
                if query.is_empty() {
                    return true;
                }
                let agent = agents
                    .iter()
                    .find(|agent| field(agent, "id") == field(task, "agentId"));
                let host = hosts
                    .iter()
                    .find(|host| field(host, "id") == field(task, "hostId"));
                [
                    field(task, "title"),
                    agent
                        .map(|agent| field(agent, "name"))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| field(task, "archivedAgentName")),
                    host.map(|host| field(host, "name")).unwrap_or_default(),
                ]
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
            })
            .collect();
        tasks.sort_by(|a, b| b["updatedAt"].as_i64().cmp(&a["updatedAt"].as_i64()));
        self.list.reset(tasks.len());
        self.tasks = tasks;
    }

    fn restore(&mut self, task_id: String, cx: &mut Context<Self>) {
        self.mutate(
            "set_task_archived",
            json!({"taskId":task_id,"archived":false}),
            None,
            cx,
        );
    }

    fn toggle_summary(&mut self, task_id: String, cx: &mut Context<Self>) {
        let previous = self.expanded_summary.clone();
        self.summary_request += 1;
        if self.expanded_summary.as_deref() == Some(task_id.as_str()) {
            self.expanded_summary = None;
            self.summary = None;
            self.summary_loading = false;
        } else {
            self.expanded_summary = Some(task_id.clone());
            self.summary = None;
            self.summary_loading = true;
            let request = self.summary_request;
            let service = self.service.clone();
            let requested_id = task_id.clone();
            let worker = cx.background_executor().spawn(async move {
                service.invoke("summarize_archived_task", json!({"taskId": requested_id}))
            });
            cx.spawn(async move |pane, cx| {
                let result = worker.await;
                let _ = pane.update(cx, |pane, cx| {
                    if pane.summary_request != request { return; }
                    pane.summary_loading = false;
                    pane.summary = Some(result.and_then(|value| value["summary"].as_str()
                        .map(str::to_owned).ok_or_else(|| "Backend returned an invalid archive summary.".to_owned())));
                    if let Some(index) = pane.tasks.iter().position(|task| pane.expanded_summary.as_deref() == Some(field(task, "id").as_str())) {
                        pane.list.remeasure_items(index..index + 1);
                    }
                    cx.notify();
                });
            }).detach();
        }
        for id in [previous.as_deref(), self.expanded_summary.as_deref()].into_iter().flatten() {
            if let Some(index) = self.tasks.iter().position(|task| field(task, "id") == id) {
                self.list.remeasure_items(index..index + 1);
            }
        }
        cx.notify();
    }

    fn begin_delete(&mut self, task_id: String, cx: &mut Context<Self>) {
        if self.busy || self.preview_loading {
            return;
        }
        let Some(task) = self
            .tasks
            .iter()
            .find(|task| field(task, "id") == task_id)
            .cloned()
        else {
            return;
        };
        if task["archived"].as_bool() != Some(true) {
            return;
        }
        self.pending_delete = Some(task);
        self.preview = None;
        self.remove_native_files = false;
        self.error = None;
        self.preview_loading = true;
        self.generation += 1;
        let generation = self.generation;
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            service.invoke("preview_task_deletion", json!({"taskId":task_id}))
        });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation {
                    return;
                }
                pane.preview_loading = false;
                match result {
                    Ok(preview) => pane.preview = Some(preview),
                    Err(error) => {
                        pane.preview = Some(json!({"supported":false,"reason":error,"files":[]}));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.pending_delete = None;
        self.preview = None;
        self.preview_loading = false;
        self.remove_native_files = false;
        self.error = None;
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.pending_delete.clone() else {
            return;
        };
        let remove_native = self.remove_native_files
            && self
                .preview
                .as_ref()
                .is_some_and(|preview| preview["supported"].as_bool() == Some(true));
        self.mutate(
            "delete_archived_task",
            json!({"taskId":field(&task,"id"),"removeNativeFiles":remove_native}),
            Some(task),
            cx,
        );
    }

    fn mutate(
        &mut self,
        command: &'static str,
        args: Value,
        deleting: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let service = self.service.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { service.invoke(command, args) });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Ok(snapshot) if snapshot.is_object() => {
                        if deleting.is_some() {
                            pane.pending_delete = None;
                            pane.preview = None;
                            pane.preview_loading = false;
                            pane.remove_native_files = false;
                            pane.generation += 1;
                        }
                        // The root owns revision ordering. A command response can
                        // arrive after a newer service change; never replace the
                        // accepted projection with that older response.
                        cx.emit(ArchiveEvent::Refresh);
                    }
                    Ok(_) => pane.error = Some("Backend returned an invalid snapshot.".into()),
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ArchivedChatsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = crate::design::palette(&self.snapshot["settings"], window.appearance());
        let font_size = interface_font_size(&self.snapshot["settings"]);
        let agents: HashMap<String, Value> = rows(&self.snapshot, "agents")
            .into_iter()
            .map(|agent| (field(&agent, "id"), agent))
            .collect();
        let hosts: HashMap<String, Value> = rows(&self.snapshot, "hosts")
            .into_iter()
            .map(|host| (field(&host, "id"), host))
            .collect();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::design::surface_color(palette.paper))
            .text_color(rgb(palette.ink))
            .text_size(font_size)
            .font_family(cx.theme().font_family.clone())
            .child(
                div()
                    .h(px(42.))
                    .flex_shrink_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(13.))
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .border_blend_mode(crate::surface_blend::separator_mode())
                    .child(Icon::new(IconName::Archive).size(px(22.)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(font_size)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Archived chats"),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .px(px(13.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .border_blend_mode(crate::surface_blend::separator_mode())
                    .child(Input::new(&self.query)),
            );
        if let Some(error) = &self.error {
            root = root.child(div().px(px(13.)).py(px(8.)).text_color(rgb(0xdc2626)).child(error.clone()));
        }
        if let Some(task) = &self.pending_delete {
            let title = field(task, "title");
            let provider = field(task, "provider");
            let native_session = field(task, "nativeSessionId");
            let mut confirm = div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(palette.line))
                .bg(crate::design::surface_color(palette.card))
                .child(
                    div()
                        .text_size(font_size)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Permanently delete ‘{title}’?")),
                )
                .child(div().text_size(font_size).text_color(rgb(palette.muted)).child(format!(
                    "{provider} · {}",
                    hosts.get(&field(task, "hostId"))
                        .map(|host| field(host, "name"))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "Unknown host".into())
                )))
                .child("This removes the saved Monitter chat and transcript.");
            if !native_session.is_empty() {
                confirm = confirm.child(div().text_size(font_size).child(native_session));
            }
            if self.preview_loading {
                confirm = confirm.child(
                    div()
                        .text_size(font_size)
                        .text_color(rgb(palette.muted))
                        .child("Checking native session files…"),
                );
            } else if let Some(preview) = &self.preview {
                if preview["supported"].as_bool() == Some(true) {
                    let files = rows(preview, "files");
                    confirm = confirm.child(
                        Checkbox::new("archived-remove-native-files")
                            .label("Also remove verified native session files from disk")
                            .checked(self.remove_native_files)
                            .disabled(self.busy)
                            .on_change(cx.listener(|pane, checked, _, cx| {
                                pane.remove_native_files = *checked;
                                cx.notify();
                            })),
                    );
                    if self.remove_native_files {
                        for file in files {
                            confirm = confirm.child(
                                div()
                                    .text_size(font_size)
                                    .child(file.as_str().unwrap_or_default().to_owned()),
                            );
                        }
                    }
                } else {
                    confirm = confirm.child(div().text_size(font_size).text_color(rgb(palette.muted)).child(format!(
                        "Native session files will be kept: {}",
                        field(preview, "reason")
                    )));
                }
            }
            confirm = confirm.child(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("archived-delete-cancel")
                            .label("Cancel")
                            .text_size(font_size)
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| pane.cancel_delete(cx))),
                    )
                    .child(
                        Button::new("archived-delete-confirm")
                            .label(if self.busy {
                                "Deleting…"
                            } else {
                                "Delete permanently"
                            })
                            .text_size(font_size)
                            .with_variant(ButtonVariant::Danger)
                            .disabled(self.busy || self.preview_loading)
                            .on_click(cx.listener(|pane, _, _, cx| pane.confirm_delete(cx))),
                    ),
            );
            root = root.child(div().w_full().p(px(13.)).child(confirm));
        }

        let tasks = self.tasks.clone();
        let pane = cx.weak_entity();
        let busy = self.busy;
        let preview_loading = self.preview_loading;
        let expanded_summary = self.expanded_summary.clone();
        let summary = self.summary.clone();
        let summary_loading = self.summary_loading;
        let animate = !cx.reduce_motion() && crate::native_motion::motion_allowed();
        let rows = list(self.list.clone(), move |index, _, _| {
            let task = &tasks[index];
            let id = field(task, "id");
            let title = field(task, "title");
            let agent = agents
                .get(&field(task, "agentId"))
                .map(|agent| field(agent, "name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| field(task, "archivedAgentName"));
            let host = hosts
                .get(&field(task, "hostId"))
                .map(|host| field(host, "name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Unknown host".into());
            let agent_label = if agent.is_empty() {
                "Unknown agent".to_owned()
            } else {
                agent
            };
            let metadata = format!(
                "{} · {}{} · {}",
                agent_label,
                field(task, "provider"),
                if field(task, "model").is_empty() {
                    String::new()
                } else {
                    format!(" · {}", field(task, "model"))
                },
                host
            );
            let provider_icon = crate::branding::provider_for_ink(
                &field(task, "provider"), &field(task, "model"), palette.ink,
            );
            let expanded = expanded_summary.as_deref() == Some(id.as_str());
            let weak = pane.clone();
            let mut row = div()
                .w_full()
                .flex()
                .flex_col()
                .px(px(13.))
                .py(px(12.))
                .when(index + 1 < tasks.len(), |row| row.border_b_1().border_color(rgb(palette.line))
                    .border_blend_mode(crate::surface_blend::separator_mode()))
                .child(div().w_full().flex().items_center().justify_between().gap(px(12.))
                .child(
                    div().flex_shrink_0().w(px(28.)).h(px(28.)).rounded(px(6.))
                        .bg(crate::design::surface_color(palette.soft))
                        .flex().items_center().justify_center()
                        .child(if let Some(image) = provider_icon {
                            img(image).w(px(24.)).h(px(24.)).into_any_element()
                        } else {
                            div().text_size(font_size).child("◇").into_any_element()
                        }),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(font_size)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(font_size)
                                .text_color(rgb(palette.muted))
                                .child(metadata),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("info-{id}")))
                                .icon(IconName::Info).ghost().compact()
                                .accessibility_label("Summarize last 10 messages")
                                .tooltip(if expanded { "Hide summary" } else { "Summarize last 10 messages" })
                                .on_click({
                                    let weak = weak.clone();
                                    let id = id.clone();
                                    move |_, _, app| {
                                        let _ = weak.update(app, |pane, cx| pane.toggle_summary(id.clone(), cx));
                                    }
                                }),
                        )
                        .child(
                            Button::new(SharedString::from(format!("restore-{id}")))
                                .icon(IconName::Undo2).ghost().compact()
                                .accessibility_label("Restore archived chat")
                                .tooltip("Restore archived chat")
                                .with_variant(ButtonVariant::Secondary)
                                .disabled(busy)
                                .on_click({
                                    let weak = weak.clone();
                                    let id = id.clone();
                                    move |_, _, app| {
                                        let _ = weak
                                            .update(app, |pane, cx| pane.restore(id.clone(), cx));
                                    }
                                }),
                        )
                        .child(
                            Button::new(SharedString::from(format!("delete-{id}")))
                                .icon(IconName::Trash).ghost().compact()
                                .accessibility_label("Permanently delete archived chat")
                                .tooltip("Permanently delete archived chat")
                                .with_variant(ButtonVariant::Danger)
                                .disabled(busy || preview_loading)
                                .on_click({
                                    let id = id.clone();
                                    move |_, _, app| {
                                        let _ = weak.update(app, |pane, cx| pane.begin_delete(id.clone(), cx));
                                    }
                                }),
                        ),
                ));
            if expanded {
                let detail = if summary_loading {
                    "Monitter Admin is summarizing the last 10 messages…".to_owned()
                } else {
                    match &summary {
                        Some(Ok(text)) => text.clone(),
                        Some(Err(error)) => format!("Could not summarize this chat: {error}"),
                        None => "Preparing summary…".to_owned(),
                    }
                };
                let detail = div().id(SharedString::from(format!("archive-summary-{id}")))
                    .relative().w_full().mt(px(10.)).pt(px(10.))
                    .border_t_1().border_color(rgb(palette.line))
                    .text_size(font_size).line_height(relative(1.35))
                    .child(detail);
                row = row.child(if animate {
                    detail.with_animation(
                        SharedString::from(format!("archive-summary-reveal-{id}")),
                        Animation::new(Duration::from_millis(180)),
                        |detail, progress| detail.top(px(8. * (1. - progress))).opacity(progress),
                    ).into_any_element()
                } else { detail.into_any_element() });
            }
            row.into_any_element()
        });
        if self.tasks.is_empty() {
            root.child(
                div()
                    .flex_1()
                    .p(px(13.))
                    .text_size(font_size)
                    .text_color(rgb(palette.muted))
                    .child("No archived chats match this search."),
            )
        } else {
            root.child(crate::transcript_edge_fade::TranscriptEdgeFade::for_list(
                rows.w_full().flex_1().min_h(px(0.)),
                self.list.clone(),
                px(24.),
            ))
        }
    }
}
