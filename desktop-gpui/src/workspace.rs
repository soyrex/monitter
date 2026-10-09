//! Owner project editor and coordination board, backed by the existing service.
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
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
static REQUEST: AtomicU64 = AtomicU64::new(0);
fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_owned()
}
fn items(v: &Value, k: &str) -> Vec<Value> {
    v[k].as_array().cloned().unwrap_or_default()
}

pub struct WorkspacePane {
    service: DesktopService,
    snapshot: Value,
    selected: Option<String>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    guidance: Entity<TextareaState>,
    note: Entity<TextareaState>,
    folders: Vec<(String, String, Entity<InputState>)>,
    busy: bool,
    error: Option<String>,
    pending_note: Option<(String, String, String)>,
    posted_note: Option<(String, String)>,
}
impl WorkspacePane {
    pub fn new(
        service: DesktopService,
        snapshot: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let folders = items(&snapshot, "hosts")
            .into_iter()
            .map(|h| {
                (
                    text(&h, "id"),
                    text(&h, "name"),
                    cx.new(|cx| {
                        InputState::new(window, cx).placeholder("Workspace folder (optional)")
                    }),
                )
            })
            .collect();
        Self {
            service,
            snapshot,
            selected: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Project name")),
            description: cx.new(|cx| InputState::new(window, cx).placeholder("Description")),
            guidance: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("Optional project coordination guidance")
            }),
            note: cx
                .new(|cx| TextareaState::new(window, cx).placeholder("Post a coordination note…")),
            folders,
            busy: false,
            error: None,
            pending_note: None,
            posted_note: None,
        }
    }
    pub fn sync(&mut self, snapshot: Value, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        cx.notify();
    }
    fn select(&mut self, project: Value, window: &mut Window, cx: &mut Context<Self>) {
        let id = text(&project, "id");
        self.selected = (!id.is_empty()).then_some(id);
        self.name
            .update(cx, |s, cx| s.set_value(text(&project, "name"), window, cx));
        self.description.update(cx, |s, cx| {
            s.set_value(text(&project, "description"), window, cx)
        });
        self.guidance.update(cx, |s, cx| {
            s.set_value(text(&project, "boardGuidanceText"), window, cx)
        });
        self.note.update(cx, |s, cx| s.set_value("", window, cx));
        for (id, _, input) in &self.folders {
            let cwd = items(&project, "workspaces")
                .iter()
                .find(|w| text(w, "hostId") == *id)
                .map(|w| text(w, "cwd"))
                .unwrap_or_default();
            input.update(cx, |s, cx| s.set_value(cwd, window, cx));
        }
        self.pending_note = None;
        self.error = None;
        cx.notify();
    }
    fn invoke(&mut self, command: &'static str, args: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let service = self.service.clone();
        let work = cx
            .background_executor()
            .spawn(async move { service.invoke(command, args) });
        cx.spawn(async move |view, cx| {
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(snapshot) => {
                        view.snapshot = snapshot;
                        if command == "post_project_board_note" {
                            view.posted_note =
                                view.pending_note.take().map(|(id, body, _)| (id, body));
                        }
                    }
                    Err(e) => view.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let mut project = items(&self.snapshot, "projects")
            .into_iter()
            .find(|p| Some(text(p, "id")) == self.selected)
            .unwrap_or(json!({"id":"","icon":"folder","color":"#7c8aff"}));
        project["name"] = json!(self.name.read(cx).value().to_string());
        project["description"] = json!(self.description.read(cx).value().to_string());
        let guidance = self.guidance.read(cx).value().to_string();
        project["boardGuidanceText"] = if guidance.trim().is_empty() {
            Value::Null
        } else {
            json!(guidance)
        };
        project["workspaces"] = json!(
            self.folders
                .iter()
                .filter_map(|(id, _, input)| {
                    let cwd = input.read(cx).value().to_string();
                    (!cwd.trim().is_empty()).then(|| json!({"hostId":id,"cwd":cwd}))
                })
                .collect::<Vec<_>>()
        );
        self.invoke("save_project", json!({"project":project}), cx);
    }
    fn post(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let note = self.note.read(cx).value().to_string();
        if note.trim().is_empty() {
            return;
        }
        // Preserve the request identity after an uncertain failure. Edits create
        // a new request; the backend enforces idempotence and note size limits.
        let request = match &self.pending_note {
            Some((project, body, key)) if project == &id && body == &note => key.clone(),
            _ => format!(
                "gpui-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                REQUEST.fetch_add(1, Ordering::Relaxed)
            ),
        };
        self.pending_note = Some((id.clone(), note.clone(), request.clone()));
        self.invoke(
            "post_project_board_note",
            json!({"projectId":id,"text":note,"requestId":request}),
            cx,
        );
    }
}
impl Render for WorkspacePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((id, body)) = self.posted_note.take() {
            if self.selected.as_ref() == Some(&id) && self.note.read(cx).value().as_ref() == body {
                self.note.update(cx, |s, cx| s.set_value("", window, cx));
            }
        }
        let mut list = div().flex().flex_col().gap_2().w(px(220.)).child(
            Button::new("new-project")
                .label("New project")
                .disabled(self.busy)
                .on_click(cx.listener(|v, _, w, cx| v.select(json!({}), w, cx))),
        );
        for p in items(&self.snapshot, "projects") {
            list = list.child(
                Button::new(SharedString::from(format!("project-{}", text(&p, "id"))))
                    .label(text(&p, "name"))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |v, _, w, cx| v.select(p.clone(), w, cx))),
            );
        }
        let mut detail = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xl().child("Project workspace"))
            .child(Input::new(&self.name).disabled(self.busy))
            .child(Input::new(&self.description).disabled(self.busy));
        for (_, name, input) in &self.folders {
            detail = detail
                .child(name.clone())
                .child(Input::new(input).disabled(self.busy));
        }
        detail = detail
            .child("Board guidance")
            .child(Textarea::new(&self.guidance))
            .child(
                Button::new("save-project")
                    .label("Save project")
                    .disabled(self.busy)
                    .on_click(cx.listener(|v, _, _, cx| v.save(cx))),
            );
        if let Some(id) = &self.selected {
            detail = detail.child(div().mt_4().text_xl().child("Coordination board"));
            if self.snapshot["settings"]["projectBoardEnabled"] == true {
                for note in items(&self.snapshot, "projectBoardMessages")
                    .into_iter()
                    .filter(|n| text(n, "projectId") == *id)
                {
                    detail =
                        detail.child(div().p_3().rounded_md().bg(crate::design::surface_color(0x202737)).child(format!(
                            "#{} · {}\n{}",
                            note["sequence"],
                            text(&note, "authorName"),
                            text(&note, "text")
                        )));
                }
                detail = detail.child(Textarea::new(&self.note)).child(
                    Button::new("post-note")
                        .label("Post note")
                        .disabled(self.busy)
                        .on_click(cx.listener(|v, _, _, cx| v.post(cx))),
                );
            } else {
                detail = detail.child("Project boards are disabled in settings.");
            }
        }
        if let Some(error) = &self.error {
            detail = detail.child(div().text_color(rgb(0xff9292)).child(error.clone()));
        }
        div()
            .id("project-workspace")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .gap_4()
            .child(list)
            .child(detail)
    }
}
