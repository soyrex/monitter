//! Task-scoped project assignment through the existing set_task_project API.
//! This picker changes only the task's project link; its working folder and
//! native session remain under the backend's existing preservation contract.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    base::Selectable as _,
    component::{Disableable, WindowExt, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};

const PROJECT_PAGE_SIZE: usize = 256;
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum TaskProjectPickerEvent {
    /// The parent refreshes its revisioned snapshot; the unversioned mutation
    /// response must not overwrite a newer journal snapshot.
    RefreshRequired {
        task_id: String,
    },
    Dismissed {
        task_id: String,
    },
}

impl EventEmitter<TaskProjectPickerEvent> for TaskProjectPicker {}

pub struct TaskProjectPicker {
    service: DesktopService,
    snapshot: Value,
    task_id: String,
    palette: crate::design::Palette,
    open: bool,
    busy: bool,
    generation: u64,
    error: Option<String>,
    visible_project_limit: usize,
}

impl TaskProjectPicker {
    pub fn new(service: DesktopService, palette: crate::design::Palette) -> Self {
        Self {
            service,
            snapshot: Value::Null,
            task_id: String::new(),
            palette,
            open: false,
            busy: false,
            generation: 0,
            error: None,
            visible_project_limit: PROJECT_PAGE_SIZE,
        }
    }

    /// Supply the latest canonical snapshot and task context from the owner.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        task_id: impl Into<String>,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) {
        let task_id = task_id.into();
        let changed =
            task_id != self.task_id || self.snapshot != *snapshot || self.palette != palette;
        if task_id != self.task_id {
            self.generation = self.generation.wrapping_add(1);
            self.busy = false;
            self.error = None;
            self.visible_project_limit = PROJECT_PAGE_SIZE;
        }
        if !changed {
            return;
        }
        self.snapshot = snapshot.clone();
        self.task_id = task_id;
        self.palette = palette;
        cx.notify();
    }

    pub fn open_in_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.open = true;
        self.error = None;
        let entity = cx.entity();
        let dismiss_entity = entity.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let dismiss_entity = dismiss_entity.clone();
            dialog
                .title("Move chat to project")
                .width(px(420.))
                .overlay_closable(false)
                .close_button(false)
                .keyboard(true)
                .on_cancel(move |_, _, cx| {
                    let entity = dismiss_entity.clone();
                    let _ = entity.update(cx, |picker, cx| picker.request_dismiss(cx));
                    // Owner handles Dismissed and closes this native dialog.
                    false
                })
                .child(entity.clone())
        });
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.request_dismiss(cx);
    }

    fn request_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.open = false;
        self.error = None;
        cx.emit(TaskProjectPickerEvent::Dismissed {
            task_id: self.task_id.clone(),
        });
        cx.notify();
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.open {
            return div().into_any_element();
        }

        let palette = self.palette;
        let task_id = self.task_id.clone();
        let task = self.snapshot["tasks"].as_array().and_then(|tasks| {
            tasks
                .iter()
                .find(|task| task["id"].as_str() == Some(&task_id))
        });
        let current_project = task
            .and_then(|task| task["projectId"].as_str())
            .filter(|project_id| !project_id.is_empty())
            .map(str::to_owned);
        let projects = self.snapshot["projects"]
            .as_array()
            .map(|projects| {
                projects
                    .iter()
                    .take(self.visible_project_limit)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let project_count = self.snapshot["projects"].as_array().map_or(0, Vec::len);
        let mut content = div()
            .id("task-project-picker")
            .max_h(px(480.))
            .overflow_y_scroll()
            .text_color(rgb(palette.ink))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(
                        "Changing the project keeps this chat’s working folder and native session.",
                    ),
            );

        if task.is_none() {
            content = content.child(
                div()
                    .id("task-project-picker-missing-task")
                    .role(Role::Status)
                    .text_color(rgb(0xbd655b))
                    .child("The selected chat is no longer available."),
            );
        } else {
            let none_selected = current_project.is_none();
            let none_clicked = cx.listener(|picker, _, _, cx| picker.assign(None, cx));
            if project_count == 0 {
                content = content.child(
                    div()
                        .id("task-project-picker-no-projects")
                        .role(Role::Status)
                        .text_color(rgb(palette.muted))
                        .child("No projects are available. Create a project in Projects first."),
                );
            }
            content = content.child(project_choice(
                "task-project-none",
                "No project",
                none_selected,
                self.busy,
                palette,
                cx,
                none_clicked,
            ));
            for project in projects {
                let id = project["id"].as_str().unwrap_or_default().to_owned();
                if id.is_empty() {
                    continue;
                }
                let name = bounded_text(project["name"].as_str().unwrap_or("Unnamed project"), 160);
                let selected = current_project.as_deref() == Some(id.as_str());
                let target = id.clone();
                let project_clicked =
                    cx.listener(move |picker, _, _, cx| picker.assign(Some(target.clone()), cx));
                content = content.child(project_choice(
                    format!("task-project-{id}"),
                    &name,
                    selected,
                    self.busy,
                    palette,
                    cx,
                    project_clicked,
                ));
            }
            if project_count > self.visible_project_limit {
                let remaining = project_count - self.visible_project_limit;
                let next_count = self
                    .visible_project_limit
                    .saturating_add(PROJECT_PAGE_SIZE)
                    .min(project_count);
                let label = format!("Show more projects ({remaining} remaining)");
                content = content
                    .child(
                        div()
                            .id("task-project-picker-project-count")
                            .role(Role::Status)
                            .child(format!(
                                "Showing {} of {project_count} projects.",
                                self.visible_project_limit
                            )),
                    )
                    .child(
                        Button::new("task-project-picker-show-more")
                            .compact()
                            .accessibility_label(label.clone())
                            .label(label)
                            .on_click(cx.listener(move |picker, _, _, cx| {
                                picker.visible_project_limit = next_count;
                                cx.notify();
                            })),
                    );
            }
        }

        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .id("task-project-picker-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xbd655b))
                    .child(error.clone()),
            );
        }
        if self.busy {
            content = content.child(
                div()
                    .id("task-project-picker-busy")
                    .role(Role::Status)
                    .text_color(rgb(palette.muted))
                    .child("Updating chat project…"),
            );
        }

        content = content.child(
            div().mt(px(4.)).flex().justify_end().child(
                Button::new("task-project-picker-cancel")
                    .compact()
                    .accessibility_label("Cancel")
                    .disabled(self.busy)
                    .on_click(cx.listener(|picker, _, _, cx| picker.dismiss(cx)))
                    .child(div().text_size(px(12.)).child("Cancel")),
            ),
        );

        content.into_any_element()
    }

    fn assign(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        if self.busy || !self.open || self.task_id.is_empty() {
            return;
        }
        let task_exists = self.snapshot["tasks"].as_array().is_some_and(|tasks| {
            tasks
                .iter()
                .any(|task| task["id"].as_str() == Some(self.task_id.as_str()))
        });
        if !task_exists {
            self.error = Some("The selected chat is no longer available.".into());
            cx.notify();
            return;
        }
        let current_project = self.snapshot["tasks"]
            .as_array()
            .and_then(|tasks| {
                tasks
                    .iter()
                    .find(|task| task["id"].as_str() == Some(self.task_id.as_str()))
            })
            .and_then(|task| task["projectId"].as_str())
            .filter(|id| !id.is_empty());
        if current_project == project_id.as_deref() {
            self.request_dismiss(cx);
            return;
        }
        if let Some(project_id) = &project_id
            && !self.snapshot["projects"]
                .as_array()
                .is_some_and(|projects| {
                    projects
                        .iter()
                        .any(|project| project["id"].as_str() == Some(project_id.as_str()))
                })
        {
            self.error = Some("That project is no longer available. Refresh and try again.".into());
            cx.notify();
            return;
        }

        let task_id = self.task_id.clone();
        let generation = self.generation;
        let expected_project_id = project_id.clone();
        let service = self.service.clone();
        let (sender, receiver) = futures::channel::oneshot::channel();
        let worker_task_id = task_id.clone();
        let started = std::thread::Builder::new()
            .name("monitter-task-project-assignment".into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service.invoke(
                    "set_task_project",
                    json!({"taskId": worker_task_id, "projectId": project_id}),
                );
                let _ = sender.send(result);
            });
        if let Err(error) = started {
            self.error = Some(format!("Could not start project update: {error}"));
            cx.notify();
            return;
        }

        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |picker, cx| {
            let result = receiver
                .await
                .map_err(|_| "Project update ended without a result.".to_owned())
                .and_then(|result| result);
            let _ = picker.update(cx, |picker, cx| {
                match result {
                    Ok(snapshot) => {
                let confirmed = snapshot["tasks"].as_array().is_some_and(|tasks| {
                    tasks.iter().any(|task| {
                        task["id"].as_str() == Some(task_id.as_str())
                            && task.get("projectId").is_some()
                            && task["projectId"].as_str().map(str::to_owned)
                                == expected_project_id
                    })
                        });
                        if !confirmed {
                            if picker.generation == generation && picker.task_id == task_id {
                                picker.busy = false;
                                picker.error = Some(
                                    "The backend response did not confirm the project assignment. Refresh and try again.".into(),
                                );
                                cx.notify();
                            }
                            return;
                        }

                        // The owner must fetch a current revisioned snapshot;
                        // do not apply this unversioned mutation response over
                        // a possibly newer change feed.
                        cx.emit(TaskProjectPickerEvent::RefreshRequired {
                            task_id: task_id.clone(),
                        });
                        if picker.generation == generation && picker.task_id == task_id {
                            picker.busy = false;
                            picker.open = false;
                            picker.error = None;
                        }
                    }
                    Err(error) => {
                        if picker.generation != generation || picker.task_id != task_id {
                            return;
                        }
                        picker.busy = false;
                        picker.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for TaskProjectPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_body(cx)
    }
}

fn project_choice(
    id: impl Into<SharedString>,
    name: &str,
    selected: bool,
    busy: bool,
    palette: crate::design::Palette,
    cx: &App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let id: SharedString = id.into();
    let accessibility_label = if selected {
        format!("{name}, current project")
    } else {
        format!("Move chat to {name}")
    };
    Button::new(id)
        .w_full()
        .h_auto()
        .justify_start()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.line
        }))
        .text_color(rgb(palette.ink))
        .selected(selected)
        .disabled(busy)
        .accessibility_label(accessibility_label)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(
                    rgb(if selected {
                        palette.soft
                    } else {
                        palette.panel
                    })
                    .into(),
                )
                .foreground(rgb(palette.ink).into())
                .hover(crate::design::surface_color(mix_rgb(palette.panel, palette.accent, 0.14)).into())
                .active(crate::design::surface_color(mix_rgb(palette.panel, palette.accent, 0.2)).into()),
        )
        .on_click(on_click)
        .child(
            div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .child(name.to_owned()),
        )
        .into_any_element()
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
