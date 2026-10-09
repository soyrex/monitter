//! Native archived-chat search, restore, and confirmed permanent deletion.
//! Deletion eligibility and native-file cleanup are always revalidated by Service.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{
        Disableable,
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

pub struct ArchivedChatsPane {
    service: DesktopService,
    snapshot: Value,
    query: Entity<InputState>,
    tasks: Vec<Value>,
    list: ListState,
    pending_delete: Option<Value>,
    preview: Option<Value>,
    preview_loading: bool,
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
        cx.notify();
    }

    /// A new visit starts without carrying a previous destructive confirmation.
    /// Keep an in-flight mutation's busy state until its actual response arrives.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.pending_delete = None;
        self.preview = None;
        self.preview_loading = false;
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
            .gap_3()
            .p_4()
            .bg(crate::design::surface_color(palette.paper))
            .text_color(rgb(palette.ink))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Archived chats"),
            )
            .child(Input::new(&self.query));
        if let Some(error) = &self.error {
            root = root.child(div().text_color(rgb(0xdc2626)).child(error.clone()));
        }
        if let Some(task) = &self.pending_delete {
            let title = field(task, "title");
            let provider = field(task, "provider");
            let native_session = field(task, "nativeSessionId");
            let mut confirm = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_md()
                .bg(crate::design::surface_color(palette.card))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Permanently delete ‘{title}’?")),
                )
                .child(div().text_color(rgb(palette.muted)).child(format!(
                    "{provider} · {}",
                    hosts.get(&field(task, "hostId"))
                        .map(|host| field(host, "name"))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "Unknown host".into())
                )))
                .child("This removes the saved Monitter chat and transcript.");
            if !native_session.is_empty() {
                confirm = confirm.child(div().text_sm().child(native_session));
            }
            if self.preview_loading {
                confirm = confirm.child(
                    div()
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
                                    .text_xs()
                                    .child(file.as_str().unwrap_or_default().to_owned()),
                            );
                        }
                    }
                } else {
                    confirm = confirm.child(div().text_color(rgb(palette.muted)).child(format!(
                        "Native session files will be kept: {}",
                        field(preview, "reason")
                    )));
                }
            }
            confirm = confirm.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("archived-delete-cancel")
                            .label("Cancel")
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
                            .with_variant(ButtonVariant::Danger)
                            .disabled(self.busy || self.preview_loading)
                            .on_click(cx.listener(|pane, _, _, cx| pane.confirm_delete(cx))),
                    ),
            );
            root = root.child(confirm);
        }

        let tasks = self.tasks.clone();
        let pane = cx.weak_entity();
        let busy = self.busy;
        let preview_loading = self.preview_loading;
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
            let weak = pane.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .p_3()
                .mb_2()
                .rounded_md()
                .bg(crate::design::surface_color(palette.card))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(metadata),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("restore-{id}")))
                                .label("Restore")
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
                                .label("Delete…")
                                .with_variant(ButtonVariant::Danger)
                                .disabled(busy || preview_loading)
                                .on_click(move |_, _, app| {
                                    let _ = weak
                                        .update(app, |pane, cx| pane.begin_delete(id.clone(), cx));
                                }),
                        ),
                )
                .into_any_element()
        });
        if self.tasks.is_empty() {
            root.child(
                div()
                    .flex_1()
                    .text_color(rgb(palette.muted))
                    .child("No archived chats match this search."),
            )
        } else {
            root.child(rows.flex_1().min_h(px(0.)))
        }
    }
}
