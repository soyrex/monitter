//! A project shelf: capture and browse ideas without leaving the agent workspace.
use crate::design::{self, ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable, Icon, Sizable,
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{DropdownMenu as _, PopupMenuItem},
        text::TextView,
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::rc::Rc;

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}
fn rows(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
fn author(v: &Value) -> String {
    let name = text(v, "authorName");
    if name.is_empty() {
        "Author not recorded".into()
    } else {
        name
    }
}
fn status(v: &Value) -> &'static str {
    match v["status"].as_str() {
        Some("inPlan") => "In plan",
        Some("done") => "Done",
        Some("dropped") => "Dropped",
        _ => "Open",
    }
}
fn status_icon(value: &str) -> IconName {
    match value {
        "done" => IconName::CircleCheck,
        "inPlan" => IconName::CircleDot,
        _ => IconName::Circle,
    }
}
fn excerpt(v: &Value) -> String {
    text(v, "markdown")
        .lines()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
        .trim()
        .trim_start_matches('#')
        .trim()
        .chars()
        .take(160)
        .collect()
}
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Mine,
    Agents,
}
fn visible(mut ideas: Vec<Value>, filter: Filter) -> Vec<Value> {
    ideas.retain(|v| match filter {
        Filter::All => true,
        Filter::Mine => v["authorKind"] == "person",
        Filter::Agents => v["authorKind"] == "agent",
    });
    ideas.sort_by_key(|v| std::cmp::Reverse(v["createdAt"].as_i64().unwrap_or(0)));
    ideas
}
fn group(v: &Value) -> &'static str {
    let Some(date) = crate::timestamps::local_datetime(v["createdAt"].as_i64().unwrap_or(0)) else {
        return "Earlier";
    };
    let today = chrono::Local::now().date_naive();
    let days = (today - date.date_naive()).num_days();
    if days == 0 {
        "Today"
    } else if (1..7).contains(&days) {
        "This week"
    } else {
        "Earlier"
    }
}
#[derive(Clone)]
pub enum IdeasEvent {
    Split { vertical: bool },
    SendToAgent { task_id: String, idea: Value },
    MakeWorkPlan { task_id: String, idea: Value },
    OpenSource(String),
}
impl EventEmitter<IdeasEvent> for IdeasPane {}
pub struct IdeasPane {
    focus: FocusHandle,
    dock_group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
    service: DesktopService,
    snapshot: Rc<Value>,
    project: Option<String>,
    selected: Option<String>,
    capture: Entity<InputState>,
    title: Entity<InputState>,
    note: Entity<TextareaState>,
    tags: Entity<InputState>,
    tags_editing: bool,
    search: Entity<InputState>,
    search_visible: bool,
    edit_status: String,
    filter: Filter,
    markdown: bool,
    history: bool,
    task_picker: Option<bool>,
    busy: bool,
    error: Option<String>,
    edit_revision: u64,
    clear_capture: Option<String>,
    pending_save: Option<(String, u64)>,
    _subscriptions: Vec<Subscription>,
}
impl IdeasPane {
    pub fn new(
        service: DesktopService,
        snapshot: Rc<Value>,
        preferred_project: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let capture = cx.new(|cx| InputState::new(window, cx).placeholder("Capture an idea…"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search ideas…"));
        let search_subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let subscription = cx.subscribe_in(&capture, window, |pane, _, event, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false
                }
            ) && !pane
                .capture
                .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            {
                pane.capture(cx);
            }
        });
        let projects = rows(&snapshot, "projects");
        let project = preferred_project
            .filter(|id| projects.iter().any(|p| text(p, "id") == *id))
            .or_else(|| projects.first().map(|p| text(p, "id")));
        Self {
            focus: cx.focus_handle(),
            dock_group: None,
            service,
            snapshot,
            project,
            selected: None,
            capture,
            title: cx.new(|cx| InputState::new(window, cx)),
            note: cx.new(|cx| TextareaState::new(window, cx).placeholder("Add a Markdown note…")),
            tags_editing: false,
            tags: cx.new(|cx| InputState::new(window, cx).placeholder("Tags, separated by commas")),
            search,
            search_visible: false,
            edit_status: "open".into(),
            filter: Filter::All,
            markdown: false,
            history: false,
            task_picker: None,
            busy: false,
            error: None,
            edit_revision: 0,
            clear_capture: None,
            pending_save: None,
            _subscriptions: vec![subscription, search_subscription],
        }
    }
    pub(crate) fn set_dock_group(
        &mut self,
        group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
        cx: &mut Context<Self>,
    ) {
        self.dock_group = group;
        cx.notify();
    }
    pub fn sync(&mut self, snapshot: Rc<Value>, cx: &mut Context<Self>) {
        let mut snapshot = snapshot;
        if let Some(project_id) = self.project.as_deref() {
            let current = self.project();
            if let Some(incoming) = snapshot["projects"].as_array().and_then(|projects| {
                projects
                    .iter()
                    .find(|p| p["id"].as_str() == Some(project_id))
            }) {
                if incoming["ideasRevision"].as_u64().unwrap_or(0)
                    < current["ideasRevision"].as_u64().unwrap_or(0)
                {
                    // Publications can lag a successful save. Preserve only the newer
                    // idea document while still accepting unrelated snapshot changes.
                    let mut merged = (*snapshot).clone();
                    if let Some(projects) = merged["projects"].as_array_mut() {
                        if let Some(project) = projects
                            .iter_mut()
                            .find(|p| p["id"].as_str() == Some(project_id))
                        {
                            project["ideas"] = current["ideas"].clone();
                            project["ideasRevision"] = current["ideasRevision"].clone();
                        }
                    }
                    snapshot = Rc::new(merged);
                }
            }
        }
        let changed = self.snapshot["projects"] != snapshot["projects"]
            || self.snapshot["tasks"] != snapshot["tasks"]
            || self.snapshot["agents"] != snapshot["agents"]
            || self.snapshot["settings"] != snapshot["settings"];
        self.snapshot = snapshot;
        self.finish_published_save(cx);
        if changed {
            cx.notify();
        }
    }
    fn project(&self) -> Value {
        rows(&self.snapshot, "projects")
            .into_iter()
            .find(|p| Some(text(p, "id")) == self.project)
            .unwrap_or(Value::Null)
    }
    fn finish_published_save(&mut self, cx: &mut Context<Self>) {
        let Some((project_id, revision)) = self.pending_save.as_ref() else { return; };
        let published = rows(&self.snapshot, "projects").into_iter()
            .find(|p| p["id"].as_str() == Some(project_id.as_str()));
        if !published.as_ref().is_some_and(|p| p["ideasRevision"].as_u64().unwrap_or(0) >= *revision) { return; }
        if self.project.as_ref() == Some(project_id) {
            // Keep the exact acknowledged revision: a later external edit must
            // still reject this editor's stale draft rather than overwrite it.
            self.edit_revision = *revision;
            if let Some(idea) = published.as_ref().and_then(|p| p["ideas"].as_array())
                .and_then(|ideas| ideas.iter().find(|i| Some(text(i, "id")) == self.selected)) {
                self.edit_status = idea["status"].as_str().unwrap_or("open").into();
            }
        }
        self.pending_save = None;
        self.busy = false;
        cx.notify();
    }
    fn open(&mut self, idea: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(text(&idea, "id"));
        self.edit_revision = self.project()["ideasRevision"].as_u64().unwrap_or(0);
        self.title
            .update(cx, |s, cx| s.set_value(text(&idea, "title"), window, cx));
        self.note
            .update(cx, |s, cx| s.set_value(text(&idea, "markdown"), window, cx));
        self.tags.update(cx, |s, cx| {
            s.set_value(
                rows(&idea, "tags")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                window,
                cx,
            )
        });
        self.edit_status = idea["status"].as_str().unwrap_or("open").into();
        self.markdown = false;
        self.history = false;
        self.task_picker = None;
        self.error = None;
        cx.notify();
    }
    fn capture(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let title = self.capture.read(cx).value().trim().to_owned();
        if title.is_empty() {
            return;
        }
        let project = self.project();
        let mut ideas = rows(&project, "ideas");
        ideas.push(json!({"id":monitter_lib::model::id(),"title":title,"markdown":"","createdAt":0,"updatedAt":0}));
        self.clear_capture = Some(title);
        self.persist(ideas, project["ideasRevision"].as_u64().unwrap_or(0), cx);
    }
    fn save(&mut self, drop: bool, cx: &mut Context<Self>) {
        let mut ideas = rows(&self.project(), "ideas");
        if let Some(idea) = ideas
            .iter_mut()
            .find(|i| Some(text(i, "id")) == self.selected)
        {
            idea["title"] = json!(self.title.read(cx).value().to_string());
            idea["markdown"] = json!(self.note.read(cx).value().to_string());
            idea["tags"] = json!(
                self.tags
                    .read(cx)
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            );
            idea["status"] = json!(self.edit_status);
            if drop {
                idea["status"] = json!("dropped");
            }
        }
        self.persist(ideas, self.edit_revision, cx);
    }
    fn persist(&mut self, ideas: Vec<Value>, revision: u64, cx: &mut Context<Self>) {
        if self.busy || self.project.is_none() {
            return;
        }
        self.busy = true;
        self.error = None;
        let ideas = match ideas.into_iter().map(serde_json::from_value).collect::<Result<Vec<monitter_lib::model::ProjectIdea>, _>>() {
            Ok(ideas) => ideas,
            Err(_) => { self.busy = false; self.error = Some("The idea could not be encoded. Your text is retained.".into()); cx.notify(); return; }
        };
        let service = self.service.clone();
        let project = self.project.clone().unwrap();
        let saved_project = project.clone();
        let work = cx
            .background_executor()
            .spawn(async move { service.save_project_ideas(&saved_project, ideas, revision) });
        cx.spawn(async move |view, cx| {
            let result = work.await;
            let _ = view.update(cx, |v, cx| {
                match result {
                    Ok(revision) => {
                        v.pending_save = Some((project, revision));
                        v.finish_published_save(cx);
                    }
                    Err(e) => {
                        v.busy = false;
                        v.error = Some(e);
                        v.clear_capture = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Focusable for IdeasPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for IdeasPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.busy && self.error.is_none() {
            if let Some(value) = self.clear_capture.take() {
                if self.capture.read(cx).value().as_ref() == value {
                    self.capture.update(cx, |s, cx| s.set_value("", window, cx));
                }
            }
        }
        let p = design::palette(&self.snapshot["settings"], window.appearance());
        let font_scale = f32::from(cx.theme().font_size) / 14.;
        let project = self.project();
        let query = if self.search_visible {
            self.search.read(cx).value().to_lowercase()
        } else {
            String::new()
        };
        let ideas: Vec<_> = visible(rows(&project, "ideas"), self.filter)
            .into_iter()
            .filter(|i| {
                query.is_empty()
                    || format!("{} {} {}", text(i, "title"), text(i, "markdown"), author(i))
                        .to_lowercase()
                        .contains(&query)
            })
            .collect();
        let mut body = div()
            .id("ideas-pane")
            .debug_selector(|| "ideas-pane".into())
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .bg(rgb(p.panel))
            .text_color(rgb(p.ink))
            .font_family(cx.theme().font_family.clone())
            .text_size(gpui_kit::px(12. * font_scale));
        let project_owner = cx.weak_entity();
        let project_choices = rows(&self.snapshot, "projects");
        let project_selector = Button::new("ideas-project-selector")
            .debug_selector(|| "ideas-project-selector".into())
            .compact()
            .ghost()
            .max_w(px(110.))
            .label(if project.is_null() {
                "Project ▾".into()
            } else {
                format!("{} ▾", text(&project, "name"))
            })
            .disabled(self.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for project in &project_choices {
                    let id = text(project, "id");
                    let owner = project_owner.clone();
                    menu = menu.item(PopupMenuItem::new(text(project, "name")).on_click(
                        move |_, _, cx| {
                            let _ = owner.update(cx, |v, cx| {
                                v.project = Some(id.clone());
                                v.selected = None;
                                v.error = None;
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
        let identity = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(gpui_kit::px(13. * font_scale))
                    .child("Ideas"),
            )
            .child(div()
                .flex_shrink_0()
                .font_family(design::FONT_MONO)
                .text_size(gpui_kit::px(11. * font_scale))
                .text_color(rgb(p.text(2)))
                .child(rows(&project, "ideas").len().to_string()));
        let context = self
            .dock_group
            .as_ref()
            .and_then(|g| g.upgrade())
            .map(|g| g.read(cx).context(cx));
        let identity = if let Some(context) = &context {
            gpui_kit::component::dock::pane_drag_region(identity, context, cx).into_any_element()
        } else {
            identity.into_any_element()
        };
        let mut header = div()
            .id("ideas-header")
            .debug_selector(|| "ideas-header".into())
            .h(px(48.))
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(rgb(p.line))
            .child(project_selector)
            .child(identity);
        if let Some(context) = context {
            let group = self.dock_group.clone();
            let split_owner = cx.weak_entity();
            header = header
                .child(
                    div()
                        .flex_shrink_0()
                        .debug_selector(|| "ideas-pane-move".into())
                        .child(gpui_kit::component::dock::pane_move_control(&context, cx)),
                )
                .child(
                    Button::new("ideas-pane-menu")
                        .compact()
                        .small()
                        .ghost()
                        .label("⋯")
                        .w(px(28.))
                        .h(px(28.))
                        .p_0()
                        .debug_selector(|| "ideas-pane-menu".into())
                        .tooltip("Split and manage pane")
                        .dropdown_menu(move |menu, window, cx| {
                            let Some(group) = group.as_ref().and_then(|g| g.upgrade()) else {
                                return menu;
                            };
                            let right_owner = split_owner.clone();
                            let below_owner = split_owner.clone();
                            let menu = menu
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div()
                                            .debug_selector(|| "ideas-split-right".into())
                                            .child("Split right")
                                            .into_any_element()
                                    })
                                    .accessibility_label("Split right")
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = right_owner.update(cx, |_, cx| {
                                                cx.emit(IdeasEvent::Split { vertical: true })
                                            });
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div()
                                            .debug_selector(|| "ideas-split-below".into())
                                            .child("Split below")
                                            .into_any_element()
                                    })
                                    .accessibility_label("Split below")
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = below_owner.update(cx, |_, cx| {
                                                cx.emit(IdeasEvent::Split { vertical: false })
                                            });
                                        },
                                    ),
                                )
                                .separator();
                            gpui_kit::component::dock::pane_menu(
                                menu,
                                &group.read(cx).context(cx),
                                window,
                                cx,
                            )
                        }),
                )
                .child(
                    gpui_kit::component::dock::pane_close_control(&context, cx)
                        .w(px(24.))
                        .h(px(24.))
                        .p_0()
                        .debug_selector(|| "ideas-pane-close".into()),
                );
        }
        body = body.child(header);
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
                    .child(" Your text is retained. Reopen the idea to load the latest revision."),
            );
        }
        if let Some(selected) = self.selected.clone() {
            if let Some(idea) = ideas
                .iter()
                .find(|v| text(v, "id") == selected)
                .cloned()
                .or_else(|| {
                    rows(&project, "ideas")
                        .into_iter()
                        .find(|v| text(v, "id") == selected)
                })
            {
                let index = ideas
                    .iter()
                    .position(|i| text(i, "id") == selected)
                    .unwrap_or(0);
                let mut nav = div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .h(px(28.))
                    .child(
                        Button::new("ideas-back")
                            .compact()
                            .small()
                            .ghost()
                            .label("‹")
                            .tooltip("Back to ideas")
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.selected = None;
                                v.error = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(gpui_kit::px(10. * font_scale))
                            .text_color(rgb(p.muted))
                            .child(selected.clone()),
                    );
                for (label, offset) in [("Prev", -1isize), ("Next", 1)] {
                    let next = index
                        .checked_add_signed(offset)
                        .and_then(|i| ideas.get(i))
                        .cloned();
                    nav = nav.child(
                        Button::new(label)
                            .compact()
                            .ghost()
                            .label(if offset < 0 { "⌃" } else { "⌄" })
                            .tooltip(label)
                            .disabled(next.is_none() || self.busy)
                            .on_click(cx.listener(move |v, _, w, cx| {
                                if let Some(i) = next.clone() {
                                    v.open(i, w, cx);
                                }
                            })),
                    );
                }
                body = body.child(nav);
                let metadata = |label: &str, value: String| {
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .w(px(84.))
                                .flex_shrink_0()
                                .text_color(rgb(p.text(2)))
                                .child(label.to_owned()),
                        )
                        .child(div().flex_1().min_w_0()
                            .font_family(design::FONT_MONO)
                            .text_size(gpui_kit::px(12. * font_scale))
                            .text_color(rgb(p.text(1)))
                            .child(value))
                };
                let mut detail = div()
                    .id("idea-detail")
                    .debug_selector(|| "idea-detail".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p(px(16.))
                    .child(
                        Input::new(&self.title)
                            .appearance(false)
                            .bordered(false)
                            .h(px(36.))
                            .text_size(gpui_kit::px(20. * font_scale))
                            .font_weight(FontWeight::SEMIBOLD)
                            .disabled(self.busy),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_shrink_0()
                                    .text_color(rgb(p.text(2)))
                                    .child("Author"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .w(px(18.))
                                            .h(px(18.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(idea["authorKind"] == "agent", |d| d.rounded_sm()
                                                .text_color(rgb(p.paper))
                                                .bg(linear_gradient(135.,
                                                    linear_color_stop(rgb(p.agent()), 0.),
                                                    linear_color_stop(rgb(design::mix_rgb(p.agent(), p.paper, 0.3)), 1.),
                                                ))
                                                .child(Icon::new(IconName::Terminal).size(px(10.))))
                                            .when(idea["authorKind"] != "agent", |d| d.rounded_full()
                                                .bg(rgb(p.fill(3)))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(gpui_kit::px(10. * font_scale))
                                                .text_color(rgb(p.text(0)))
                                                .child(author(&idea)
                                                    .chars()
                                                    .next()
                                                    .map(|c| c.to_string())
                                                    .unwrap_or_default())),
                                    )
                                    .child(author(&idea))
                                    .when(idea["authorKind"] == "agent", |row| row.child(
                                        div().font_family(design::FONT_MONO)
                                            .text_size(gpui_kit::px(11. * font_scale))
                                            .text_color(rgb(p.agent()))
                                            .child("agent"),
                                    )),
                            ),
                    )
                    .child(metadata(
                        "Created",
                        crate::timestamps::datetime_label(idea["createdAt"].as_i64().unwrap_or(0)),
                    ))
                    .child(metadata("Updated", {
                        let mut updated = crate::timestamps::datetime_label(
                            idea["updatedAt"].as_i64().unwrap_or(0),
                        );
                        let actor = &idea["updatedBy"];
                        let name = text(actor, "name");
                        if !name.is_empty()
                            && (actor["kind"] != idea["authorKind"]
                                || actor["id"] != idea["authorId"]
                                || actor["name"] != idea["authorName"])
                        {
                            updated.push_str(&format!(" · {name}"));
                        }
                        updated
                    }));
                let owner = cx.weak_entity();
                let selected_status = status(&json!({"status":self.edit_status}));
                detail = detail.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(84.)).text_color(rgb(p.text(2))).child("Status"))
                        .child(
                            Button::new("idea-status-selector")
                                .compact()
                                .small()
                                .outline()
                                .icon(status_icon(&self.edit_status))
                                .h(px(24.))
                                .rounded(px(6.))
                                .bg(rgb(p.soft))
                                .label(format!("{selected_status} ▾"))
                                .disabled(self.busy)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (value, label) in [
                                        ("open", "Open"),
                                        ("inPlan", "In plan"),
                                        ("done", "Done"),
                                        ("dropped", "Dropped"),
                                    ] {
                                        let owner = owner.clone();
                                        menu = menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, _, cx| {
                                                let _ = owner.update(cx, |v, cx| {
                                                    v.edit_status = value.into();
                                                    cx.notify();
                                                });
                                            },
                                        ));
                                    }
                                    menu
                                }),
                        ),
                );
                if let Some(source) = idea["sourceTaskId"].as_str() {
                    let source = source.to_owned();
                    let name = rows(&self.snapshot, "tasks")
                        .iter()
                        .find(|t| text(t, "id") == source)
                        .map(|t| text(t, "title"))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Open source agent".into());
                    detail = detail.child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_shrink_0()
                                    .text_color(rgb(p.text(2)))
                                    .child("Source"),
                            )
                            .child(
                                Button::new("idea-source")
                                    .max_w(px(200.))
                                    .overflow_hidden()
                                    .compact()
                                    .small()
                                    .outline()
                                    .h(px(24.))
                                    .rounded(px(6.))
                                    .icon(IconName::ArrowUpRight)
                                    .border_color(rgb(design::mix_rgb(p.panel, p.agent(), 0.38)))
                                    .bg(rgb(design::mix_rgb(p.panel, p.agent(), 0.10)))
                                    .text_color(rgb(p.agent()))
                                    .label(name)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(IdeasEvent::OpenSource(source.clone()))
                                    })),
                            ),
                    );
                }
                let tags = self.tags.read(cx).value().to_string();
                let mut tag_values = div().flex_1().min_w_0().flex().flex_wrap().gap_1();
                if self.tags_editing {
                    tag_values = tag_values.child(
                        Input::new(&self.tags)
                            .appearance(false)
                            .bordered(false)
                            .h(px(26.))
                            .disabled(self.busy),
                    );
                } else {
                    for tag in tags.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        tag_values = tag_values.child(
                            div()
                                .px(px(6.))
                                .rounded(px(4.))
                                .bg(rgb(p.fill(0)))
                                .text_size(gpui_kit::px(11. * font_scale))
                                .text_color(rgb(p.text(2)))
                                .child(tag.to_owned()),
                        );
                    }
                }
                tag_values = tag_values.child(
                    Button::new("idea-edit-tags")
                        .compact()
                        .small()
                        .ghost()
                        .h(px(22.))
                        .label(if self.tags_editing { "Done" } else { "+ Edit" })
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.tags_editing = !v.tags_editing;
                            cx.notify();
                        })),
                );
                detail = detail.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(84.))
                                .flex_shrink_0()
                                .text_color(rgb(p.text(2)))
                                .child("Tags"),
                        )
                        .child(tag_values),
                );
                let mut tabs = div()
                    .border_t_1()
                    .border_color(rgb(p.line))
                    .pt_3()
                    .mt_2()
                    .flex()
                    .gap_1()
                    .child(div().flex_1().font_family(design::FONT_MONO)
                        .text_size(gpui_kit::px(11. * font_scale))
                        .text_color(rgb(p.text(3))).child("NOTE"));
                tabs = tabs.child(
                    Button::new("idea-save")
                        .compact()
                        .small()
                        .ghost()
                        .h(px(24.))
                        .debug_selector(|| "idea-save".into())
                        .label(if self.busy { "Saving…" } else { "Save" })
                        .disabled(self.busy)
                        .on_click(cx.listener(|v, _, _, cx| v.save(false, cx))),
                );
                let mut segments = div()
                    .flex()
                    .items_center()
                    .p(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(p.line));
                for (label, md) in [("Preview", false), ("Markdown", true)] {
                    segments = segments.child(
                        Button::new(label)
                            .compact()
                            .small()
                            .ghost()
                            .h(px(22.))
                            .px_2()
                            .rounded(px(4.))
                            .when(self.markdown == md, |b| b.bg(rgb(p.soft)))
                            .debug_selector(move || label.into())
                            .label(label)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.markdown = md;
                                cx.notify();
                            })),
                    );
                }
                detail = detail.child(tabs.child(segments));
                if self.markdown {
                    detail = detail.child(div().min_h(px(230.)).child(Textarea::new(&self.note)));
                } else {
                    detail = detail.child(TextView::markdown(
                        SharedString::from(format!("idea-preview-{selected}")),
                        self.note.read(cx).value().to_string(),
                    ));
                }
                let history = rows(&idea, "history");
                detail = detail.child(div().flex_1().min_h(px(12.))).child(
                    Button::new("idea-history")
                        .compact()
                        .small()
                        .ghost()
                        .text_color(rgb(p.muted))
                        .label(format!("History · {} edits", history.len()))
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.history = !v.history;
                            cx.notify();
                        })),
                );
                if self.history {
                    for edit in history.into_iter().rev() {
                        detail = detail.child(
                            div()
                                .border_t_1()
                                .border_color(rgb(p.line))
                                .py_2()
                                .child(format!(
                                    "{} · {}",
                                    crate::timestamps::datetime_label(
                                        edit["editedAt"].as_i64().unwrap_or(0)
                                    ),
                                    text(&edit["editedBy"], "name")
                                ))
                                .child(text(&edit, "title"))
                                .child(text(&edit, "markdown")),
                        );
                    }
                }
                body = body.child(detail);
                if let Some(make_plan) = self.task_picker {
                    let tasks: Vec<_> = rows(&self.snapshot, "tasks")
                        .into_iter()
                        .filter(|t| {
                            t["projectId"].as_str() == self.project.as_deref()
                                && !t["archived"].as_bool().unwrap_or(false)
                                && !t["hidden"].as_bool().unwrap_or(false)
                                && !rows(&self.snapshot, "agents").iter().any(|a| {
                                    a["id"] == t["agentId"]
                                        && (a["internal"].as_bool().unwrap_or(false)
                                            || a["hidden"].as_bool().unwrap_or(false))
                                })
                        })
                        .collect();
                    let mut picker = div().flex().flex_col().gap_1().p_3().child(if make_plan {
                        "Choose an agent for the work plan"
                    } else {
                        "Send idea to agent"
                    });
                    if tasks.is_empty() {
                        picker = picker.child("No agent panes in this project.");
                    }
                    for task in tasks {
                        let task_id = text(&task, "id");
                        let mut draft = idea.clone();
                        draft["title"] = json!(self.title.read(cx).value().to_string());
                        draft["markdown"] = json!(self.note.read(cx).value().to_string());
                        draft["status"] = json!(self.edit_status);
                        draft["tags"] = json!(
                            self.tags
                                .read(cx)
                                .value()
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                        );
                        picker = picker.child(
                            Button::new(SharedString::from(format!("idea-target-{task_id}")))
                                .label(text(&task, "title"))
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    if make_plan {
                                        cx.emit(IdeasEvent::MakeWorkPlan {
                                            task_id: task_id.clone(),
                                            idea: draft.clone(),
                                        });
                                    } else {
                                        cx.emit(IdeasEvent::SendToAgent {
                                            task_id: task_id.clone(),
                                            idea: draft.clone(),
                                        });
                                    }
                                    v.task_picker = None;
                                    cx.notify();
                                })),
                        );
                    }
                    picker =
                        picker.child(Button::new("idea-target-cancel").label("Cancel").on_click(
                            cx.listener(|v, _, _, cx| {
                                v.task_picker = None;
                                cx.notify();
                            }),
                        ));
                    body = body.child(picker);
                }
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_1()
                        .min_h(px(52.))
                        .px(px(16.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(rgb(p.hairline(0)))
                        .bg(rgb(p.fill(0)))
                        .child(
                            Button::new("idea-send")
                                .compact()
                                .small()
                                .outline()
                                .h(px(30.))
                                .icon(IconName::ArrowUpRight)
                                .text_color(rgb(p.accent))
                                .border_color(rgb(p.glow()))
                                .bg(rgb(design::mix_rgb(p.panel, p.accent, 0.14)))
                                .debug_selector(|| "idea-send".into())
                                .label("Send to agent")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.task_picker = Some(false);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("idea-plan")
                                .compact()
                                .small()
                                .outline()
                                .h(px(30.))
                                .debug_selector(|| "idea-plan".into())
                                .label("Make work plan")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.task_picker = Some(true);
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("idea-drop")
                                .compact()
                                .small()
                                .ghost()
                                .h(px(30.))
                                .text_color(rgb(p.muted))
                                .debug_selector(|| "idea-drop".into())
                                .label("Drop")
                                .disabled(self.busy)
                                .on_click(cx.listener(|v, _, _, cx| v.save(true, cx))),
                        ),
                );
            } else {
                body = body.child(div().p_3().child("This idea is no longer in the project."));
            }
        } else {
            body = body.child(
                div().px_3().py_2().child(
                    Input::new(&self.capture)
                        .h(px(40.))
                        .bg(rgb(p.fill(0)))
                        .border_1()
                        .border_color(rgb(p.hairline(1)))
                        .rounded(px(10.))
                        .prefix(div().text_color(rgb(p.text(2))).child("+"))
                        .suffix(
                            div()
                                .px(gpui_kit::px(5.))
                                .rounded(gpui_kit::px(4.))
                                .border_1()
                                .border_color(rgb(p.hairline(0)))
                                .font_family(crate::design::FONT_MONO)
                                .text_size(gpui_kit::px(11. * font_scale))
                                .text_color(rgb(p.text(2)))
                                .child("↵"),
                        )
                        .disabled(self.busy || self.project.is_none()),
                ),
            );
            if self.search_visible {
                body = body.child(
                    div()
                        .debug_selector(|| "ideas-search-input".into())
                        .px_3()
                        .pb_2()
                        .child(Input::new(&self.search)),
                );
            }
            let mut filters = div().flex().items_center().gap_1().px_3().py_2().child(
                Button::new("ideas-search-toggle")
                    .compact()
                    .small()
                    .ghost()
                    .w(px(24.))
                    .h(px(24.))
                    .p_0()
                    .icon(gpui_kit::assets::IconName::Search)
                    .tooltip("Search ideas")
                    .accessibility_label("Search ideas")
                    .debug_selector(|| "ideas-search-toggle".into())
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.search_visible = !v.search_visible;
                        cx.notify();
                    })),
            );
            for (label, filter) in [
                ("All", Filter::All),
                ("Mine", Filter::Mine),
                ("Agents", Filter::Agents),
            ] {
                filters = filters.child(
                    Button::new(label)
                        .compact()
                        .small()
                        .ghost()
                        .h(px(24.))
                        .rounded_full()
                        .px_2()
                        .text_size(gpui_kit::px(12. * font_scale))
                        .border_1()
                        .border_color(rgb(p.hairline(0)))
                        .when(self.filter == filter, |b| b.bg(rgb(p.fill(1))).border_color(rgb(p.hairline(1))))
                        .label(label)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.filter = filter;
                            cx.notify();
                        })),
                );
            }
            body = body.child(
                filters.child(div().flex_1()).child(
                    div()
                        .text_color(rgb(p.text(2)))
                        .text_size(gpui_kit::px(12. * font_scale))
                        .child("Newest ↓"),
                ),
            );
            let mut list = div()
                .id("ideas-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll();
            let mut previous = "";
            if ideas.is_empty() {
                list = list.child(div().p_3().text_color(rgb(p.muted)).child(
                    if self.project.is_none() {
                        "Choose a project to capture ideas."
                    } else {
                        "No ideas here yet."
                    },
                ));
            }
            for idea in ideas {
                let section = group(&idea);
                if section != previous {
                    list = list.child(
                        div()
                            .px(gpui_kit::px(16.))
                            .pt(gpui_kit::px(8.))
                            .pb(gpui_kit::px(4.))
                            .font_family(crate::design::FONT_MONO)
                            .text_size(gpui_kit::px(11. * font_scale))
                            .text_color(rgb(p.text(2)))
                            .child(section.to_uppercase()),
                    );
                    previous = section;
                }
                let id = text(&idea, "id");
                let clicked = idea.clone();
                let icon = status_icon(idea["status"].as_str().unwrap_or_default());
                let icon_color = match idea["status"].as_str() {
                    Some("done") => p.ok(),
                    Some("inPlan") => p.accent,
                    Some("dropped") => p.text(3),
                    _ => p.text(2),
                };
                let dropped = idea["status"] == "dropped";
                let is_agent = idea["authorKind"] == "agent";
                let initial = if idea["authorKind"] == "person" {
                    author(&idea)
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default()
                } else {
                    "?".into()
                };
                let avatar = div()
                    .w(px(18.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(dropped, |s| s.opacity(0.6))
                    .when(is_agent, |s| s.rounded_sm()
                        .text_color(rgb(p.paper))
                        .bg(linear_gradient(135.,
                            linear_color_stop(rgb(p.agent()), 0.),
                            linear_color_stop(rgb(design::mix_rgb(p.agent(), p.paper, 0.3)), 1.),
                        ))
                        .child(Icon::new(IconName::Terminal).size(px(10.))))
                    .when(!is_agent, |s| s.rounded_full()
                        .bg(rgb(p.fill(3)))
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(p.text(0)))
                        .child(initial));
                list = list.child(
                    div()
                        .id(SharedString::from(format!("idea-{id}")))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("idea-{id}")
                        })
                        .cursor_pointer()
                        .border_b_1()
                        .border_color(rgb(p.hairline(0)))
                        .px(gpui_kit::px(16.))
                        .py(gpui_kit::px(12.))
                        .hover(move |s| s.bg(rgb(p.fill(1))))
                        .on_click(cx.listener(move |v, _, w, cx| v.open(clicked.clone(), w, cx)))
                        .child(
                            div()
                                .flex()
                                .gap(gpui_kit::px(12.))
                                .child(div().text_color(rgb(icon_color))
                                    .child(Icon::new(icon).size(gpui_kit::px(14. * font_scale))))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_size(gpui_kit::px(14. * font_scale))
                                        .text_color(rgb(if dropped { p.text(2) } else { p.text(0) }))
                                        .min_w_0()
                                        .truncate()
                                        .when(dropped, |s| s.line_through())
                                        .child(text(&idea, "title")),
                                ),
                        )
                        .child(
                            div()
                                .pl(gpui_kit::px(24.))
                                .mt_1()
                                .text_size(gpui_kit::px(12. * font_scale))
                                .text_color(rgb(p.text(2)))
                                .truncate()
                                .child(excerpt(&idea)),
                        )
                        .child(
                            div()
                                .pl(gpui_kit::px(24.))
                                .mt_1()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(gpui_kit::px(12. * font_scale))
                                .text_color(rgb(p.text(1)))
                                .child(avatar)
                                .child(div().min_w_0().truncate().child(author(&idea)))
                                .children(
                                    rows(&idea, "tags")
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .take(3)
                                        .map(|tag| {
                                            div()
                                                .px_1()
                                                .rounded_sm()
                                                .bg(rgb(p.fill(0)))
                                                .child(tag.to_owned())
                                        })
                                        .collect::<Vec<_>>(),
                                )
                                .child(div().flex_1())
                                .child(div().font_family(crate::design::FONT_MONO)
                                    .text_size(gpui_kit::px(11. * font_scale))
                                    .text_color(rgb(p.text(3))).child(crate::timestamps::local_datetime(
                                        idea["createdAt"].as_i64().unwrap_or(0),
                                    )
                                    .map(|d| {
                                        if group(&idea) == "Today" {
                                            d.format("%H:%M").to_string()
                                        } else {
                                            d.format("%b %-d").to_string()
                                        }
                                    })
                                    .unwrap_or_default())),
                        ),
                );
            }
            body = body.child(list).child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(p.hairline(0)))
                    .text_color(rgb(p.text(2)))
                    .child("Stored in this project · Enter captures an idea"),
            );
        }
        body
    }
}
#[cfg(test)]
mod tests {
    use super::{Filter, IdeasPane, author, excerpt, rows, status, visible};
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
        Styled as _, TestAppContext, Window, div,
    };
    use monitter_lib::experimental_desktop::DesktopService;
    use serde_json::json;
    use std::rc::Rc;
    #[::core::prelude::v1::test]
    fn filters_do_not_claim_legacy_authorship_and_sort_newest() {
        let legacy = json!({"id":"old","createdAt":1});
        let human = json!({"id":"human","createdAt":3,"authorKind":"person"});
        let agent = json!({"id":"agent","createdAt":2,"authorKind":"agent"});
        let all = vec![legacy.clone(), human, agent];
        assert_eq!(visible(all.clone(), Filter::All)[0]["id"], "human");
        assert_eq!(visible(all.clone(), Filter::Mine).len(), 1);
        assert_eq!(visible(all, Filter::Agents)[0]["id"], "agent");
        assert_eq!(author(&legacy), "Author not recorded");
    }
    #[::core::prelude::v1::test]
    fn excerpt_is_one_bounded_nonempty_line() {
        assert_eq!(
            excerpt(&json!({"markdown":"\n### Why\nsecret second line"})),
            "Why"
        );
        assert_eq!(excerpt(&json!({"markdown":"a".repeat(400)})).len(), 160);
        assert_eq!(status(&json!({"status":"inPlan"})), "In plan");
    }
    #[gpui_kit::test]
    fn native_ideas_ack_waits_for_publication_in_either_order(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| IdeasPane::new(DesktopService::disconnected_for_ui_fixture(),
                Rc::new(json!({"settings":{},"projects":[{"id":"project","name":"Ideas","ideasRevision":0,"ideas":[]}]})),
                Some("project".into()), window, cx));
            gpui_kit::component::Root::new(pane, window, cx)
        });
        visual.update(|window, cx| {
            let retained = cx.new(|cx| IdeasPane::new(DesktopService::disconnected_for_ui_fixture(),
                Rc::new(json!({"projects":[{"id":"project","ideasRevision":0,"ideas":[]}]})),
                Some("project".into()), window, cx));
            retained.update(cx, |pane, cx| {
                pane.busy = true;
                pane.pending_save = Some(("project".into(), 1));
                pane.finish_published_save(cx);
                assert!(pane.busy, "ack alone must not enable a second stale capture");
                pane.sync(Rc::new(json!({"projects":[{"id":"project","ideasRevision":1,"ideas":[{"id":"one","title":"Saved"}]}]})), cx);
                assert!(!pane.busy);
                assert_eq!(pane.edit_revision, 1);
                assert!(pane.pending_save.is_none());
                pane.busy = true;
                pane.sync(Rc::new(json!({"projects":[{"id":"project","ideasRevision":2,"ideas":[]}]})), cx);
                assert!(pane.busy, "publication before ack still waits for confirmation");
                pane.pending_save = Some(("project".into(), 2));
                pane.finish_published_save(cx);
                assert!(!pane.busy);
                assert_eq!(pane.edit_revision, 2);
            });
        });
    }
    #[gpui_kit::test]
    fn native_narrow_pane_preserves_drafts_and_rejects_stale_publications(cx: &mut TestAppContext) {
        use gpui_kit::test::TestWindowExt;
        cx.update(gpui_kit::init);
        let snapshot = Rc::new(
            json!({"settings":{},"projects":[{"id":"project","name":"Example","ideasRevision":4,"ideas":[{"id":"one","title":"An idea","markdown":"Original note","createdAt":1,"updatedAt":1}]}]}),
        );
        let mut pane = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let entity = cx.new(|cx| {
                IdeasPane::new(
                    DesktopService::disconnected_for_ui_fixture(),
                    snapshot.clone(),
                    Some("project".into()),
                    window,
                    cx,
                )
            });
            pane = Some(entity.clone());
            let view = cx.new(|_| NarrowIdeasFixture { pane: entity });
            gpui_kit::component::Root::new(view, window, cx)
        });
        let pane = pane.unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let list_bounds = visual
            .debug_bounds("ideas-pane")
            .expect("list pane mounted");
        let row_bounds = visual.debug_bounds("idea-one").expect("idea row rendered");
        assert_eq!(list_bounds.size.width, gpui_kit::px(320.));
        assert!(row_bounds.right() <= list_bounds.right());
        let project_bounds = visual
            .debug_bounds("ideas-project-selector")
            .expect("project selector mounted left");
        let header_bounds = visual.debug_bounds("ideas-header").expect("header mounted");
        let search_bounds = visual
            .debug_bounds("ideas-search-toggle")
            .expect("search mounted in secondary toolbar");
        assert!(project_bounds.left() >= header_bounds.left());
        assert!(search_bounds.top() >= header_bounds.bottom());
        assert!(visual.debug_bounds("ideas-search-input").is_none());
        visual.simulate_click(search_bounds.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("ideas-search-input").is_some());
        visual.update(|window, cx| {
            pane.update(cx, |pane, cx| {
                pane.capture
                    .update(cx, |s, cx| s.set_value("Unsubmitted capture", window, cx));
                let mut stale = (*snapshot).clone();
                stale["projects"][0]["ideasRevision"] = json!(3);
                stale["projects"][0]["ideas"] = json!([]);
                pane.sync(Rc::new(stale), cx);
                assert_eq!(pane.project()["ideasRevision"], 4);
                assert_eq!(rows(&pane.project(), "ideas").len(), 1);
                assert_eq!(
                    pane.capture.read(cx).value().as_ref(),
                    "Unsubmitted capture"
                );
                pane.open(rows(&pane.project(), "ideas")[0].clone(), window, cx);
                pane.note
                    .update(cx, |s, cx| s.set_value("Unsaved **Markdown**", window, cx));
            });
            window.draw(cx).clear(cx);
        });
        for id in ["Markdown", "Preview"] {
            let bounds = visual
                .debug_bounds(id)
                .expect("detail control is visible at 320px");
            assert!(bounds.size.width <= gpui_kit::px(320.));
            visual.simulate_click(bounds.center(), Modifiers::none());
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        visual.update(|_, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.note.read(cx).value().as_ref(), "Unsaved **Markdown**");
            assert_eq!(pane.edit_status, "open");
            assert_eq!(
                pane.capture.read(cx).value().as_ref(),
                "Unsubmitted capture"
            );
        });
        let bounds = visual.debug_bounds("ideas-pane").expect("pane mounted");
        assert_eq!(bounds.size.width, gpui_kit::px(320.));
        for id in ["idea-save", "idea-send", "idea-plan", "idea-drop"] {
            let button = visual
                .debug_bounds(id)
                .expect("bottom action fits narrow pane");
            assert!(button.left() >= bounds.left() && button.right() <= bounds.right());
        }
    }
    struct NarrowIdeasFixture {
        pane: Entity<IdeasPane>,
    }
    impl Render for NarrowIdeasFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(gpui_kit::px(320.))
                .h(gpui_kit::px(800.))
                .child(self.pane.clone())
        }
    }
    #[gpui_kit::test]
    fn mounted_dock_header_moves_splits_and_closes_only_ideas(cx: &mut TestAppContext) {
        use crate::docking::{DockWorkspace, PaneFactory, PaneSpec};
        use gpui_kit::{Focusable as _, component::dock::DockPlacement, test::TestWindowExt};
        use std::cell::RefCell;
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let factory: PaneFactory = Rc::new(|_, _, _| None);
            let dock = cx.new(|cx| DockWorkspace::new("ideas-header-fixture", factory, window, cx));
            workspace = Some(dock.clone());
            gpui_kit::component::Root::new(dock, window, cx)
        });
        visual.simulate_resize(gpui_kit::size(gpui_kit::px(640.), gpui_kit::px(850.)));
        let workspace = workspace.unwrap();
        let splits = Rc::new(RefCell::new(Vec::new()));
        let ideas=visual.update(|window,cx|{
            let snapshot=Rc::new(json!({"settings":{},"projects":[{"id":"project","name":"Example","ideasRevision":0,"ideas":[]}]}));
            let ideas=cx.new(|cx|IdeasPane::new(DesktopService::disconnected_for_ui_fixture(),snapshot,Some("project".into()),window,cx));
            let recorded=splits.clone();
            cx.subscribe(&ideas,move|_,event: &super::IdeasEvent,_|{if let super::IdeasEvent::Split{vertical}=event {recorded.borrow_mut().push(*vertical);}}).detach();
            let ideas_focus=ideas.read(cx).focus_handle(cx);
            let chat=cx.new(|_| NeighborFixture);
            let chat_focus=cx.focus_handle();
            workspace.update(cx,|dock,cx|{
                dock.add(PaneSpec{id:"chat:neighbor".into(),title:"Agent".into(),content:chat.into(),focus:chat_focus,lifecycle:None,closable:true},DockPlacement::Center,None,window,cx).unwrap();
                let group = dock.group_for_pane("chat:neighbor", cx);
                dock.add_browser_beside(PaneSpec{id:"route:ideas".into(),title:"Ideas".into(),content:ideas.clone().into(),focus:ideas_focus,lifecycle:None,closable:true},group,window,cx).unwrap();
            });ideas
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let header = visual
            .debug_bounds("ideas-header")
            .expect("real dock content header mounted");
        assert!(header.size.width <= gpui_kit::px(320.));
        for selector in [
            "ideas-project-selector",
            "ideas-pane-move",
            "ideas-pane-menu",
            "ideas-pane-close",
        ] {
            let control = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} visible"));
            assert!(
                control.left() >= header.left() && control.right() <= header.right(),
                "{selector} fits narrow header"
            );
            assert!(control.top() >= header.top() && control.bottom() <= header.bottom());
        }
        let project = visual.debug_bounds("ideas-project-selector").unwrap();
        let movement = visual.debug_bounds("ideas-pane-move").unwrap();
        assert!(project.right() <= movement.left());
        assert!(visual.debug_bounds("ideas-search-toggle").unwrap().top() >= header.bottom());
        visual.read(|cx| {
            assert!(
                ideas
                    .read(cx)
                    .dock_group
                    .as_ref()
                    .and_then(|g| g.upgrade())
                    .is_some()
            )
        });
        assert!(
            visual
                .debug_bounds("dock-pane-header-drag-region")
                .is_some()
        );
        let menu = visual.debug_bounds("ideas-pane-menu").unwrap();
        visual.simulate_click(menu.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let right = visual
            .debug_bounds("ideas-split-right")
            .expect("split right offered by actual menu");
        assert!(visual.debug_bounds("ideas-split-below").is_some());
        visual.simulate_click(right.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(*splits.borrow(), vec![true]);
        visual.update(|window, cx| {
            workspace.update(cx, |dock, cx| {
                dock.focus("chat:neighbor", window, cx);
            })
        });
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let close = visual
            .debug_bounds("ideas-pane-close")
            .expect("close remains mounted while neighbor focused");
        visual.simulate_click(close.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.read(|cx| {
            let ids = DockWorkspace::saved_ids(&workspace.read(cx).serialize(cx).unwrap()).unwrap();
            assert!(
                !ids.iter().any(|id| id == "route:ideas"),
                "closed pane is absent from actual layout"
            );
            assert!(
                ids.iter().any(|id| id == "chat:neighbor"),
                "neighbor remains in actual layout"
            );
            assert!(
                ideas.read(cx).dock_group.is_none(),
                "closed editor releases owning dock context"
            );
        });
    }
    struct NeighborFixture;
    impl Render for NeighborFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child("Agent remains open")
        }
    }
}
