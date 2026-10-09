//! Bounded, explicit fork configuration for a selected transcript message.
//!
//! The owner opens this pane inside a GPUI dialog and handles its Created and
//! Cancelled events. This pane only reads a model catalog and invokes
//! `fork_task`; it never sends a prompt or changes the source task's settings.
use crate::design::{self, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::{
    component::{Disableable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};

const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum ForkDialogEvent {
    Created(Value),
    Cancelled,
}

impl EventEmitter<ForkDialogEvent> for ForkDialogPane {}

pub struct ForkDialogPane {
    service: DesktopService,
    snapshot: Value,
    source_task_id: String,
    through_message_id: String,
    source_agent_id: String,
    palette: design::Palette,
    selected_agent_id: String,
    /// None means the selected target agent's default model.
    model_settings: Option<Value>,
    catalog: Option<Value>,
    catalog_agent_id: Option<String>,
    catalog_loading_agent_id: Option<String>,
    catalog_error: Option<String>,
    catalog_generation: u64,
    busy: bool,
    error: Option<String>,
}

fn field(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn visible_agents(snapshot: &Value) -> Vec<Value> {
    snapshot["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|agent| agent["internal"] != true && agent["archived"] != true)
        .cloned()
        .collect()
}

fn source_model_settings(task: &Value) -> Option<Value> {
    if task["modelSettings"].is_object() {
        return Some(task["modelSettings"].clone());
    }
    let model = field(task, "model");
    (!model.is_empty()).then(|| {
        json!({
            "model": model,
            "reasoningEffort": null,
            "fastMode": null,
        })
    })
}

impl ForkDialogPane {
    pub fn new(
        service: DesktopService,
        snapshot: std::rc::Rc<Value>,
        source_task_id: String,
        through_message_id: String,
        palette: design::Palette,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        let source_task = snapshot["tasks"]
            .as_array()
            .and_then(|tasks| {
                tasks
                    .iter()
                    .find(|task| field(task, "id") == source_task_id)
            })
            .cloned()
            .unwrap_or(Value::Null);
        let source_agent_id = field(&source_task, "agentId");
        let agents = visible_agents(&snapshot);
        let selected_agent_id = if agents
            .iter()
            .any(|agent| field(agent, "id") == source_agent_id)
        {
            source_agent_id.clone()
        } else {
            agents.first().map(|agent| field(agent, "id")).unwrap_or_default()
        };
        let model_settings = (selected_agent_id == source_agent_id)
            .then(|| source_model_settings(&source_task))
            .flatten();

        Self {
            service,
            snapshot: snapshot.as_ref().clone(),
            source_task_id,
            through_message_id,
            source_agent_id,
            palette,
            selected_agent_id,
            model_settings,
            catalog: None,
            catalog_agent_id: None,
            catalog_loading_agent_id: None,
            catalog_error: None,
            catalog_generation: 0,
            busy: false,
            error: None,
        }
    }

    fn selected_agent(&self) -> Option<Value> {
        visible_agents(&self.snapshot)
            .into_iter()
            .find(|agent| field(agent, "id") == self.selected_agent_id)
    }

    fn request_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(agent) = self.selected_agent() else {
            self.catalog = None;
            self.catalog_agent_id = None;
            self.catalog_loading_agent_id = None;
            self.catalog_error = Some("Choose an available agent to read its models.".into());
            return;
        };
        let agent_id = field(&agent, "id");
        if self.catalog_agent_id.as_deref() == Some(agent_id.as_str())
            || self.catalog_loading_agent_id.as_deref() == Some(agent_id.as_str())
        {
            return;
        }

        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        let generation = self.catalog_generation;
        self.catalog = None;
        self.catalog_agent_id = None;
        self.catalog_loading_agent_id = Some(agent_id.clone());
        self.catalog_error = None;

        let project_id = self
            .snapshot["tasks"]
            .as_array()
            .and_then(|tasks| {
                tasks
                    .iter()
                    .find(|task| field(task, "id") == self.source_task_id)
            })
            .and_then(|task| task["projectId"].as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let codex_home = (field(&agent, "provider") == "codex")
            .then(|| agent["codexHome"].as_str().map(str::to_owned))
            .flatten();
        let service = self.service.clone();
        let source_task_id = self.source_task_id.clone();
        let (sender, receiver) = oneshot::channel();
        let worker_agent_id = agent_id.clone();
        let started = std::thread::Builder::new()
            .name("monitter-fork-model-catalog".into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service.invoke(
                    "get_model_catalog",
                    json!({
                        "target": {
                            "agentId": worker_agent_id,
                            "projectId": project_id,
                            "codexHome": codex_home,
                        }
                    }),
                );
                let _ = sender.send(result);
            });
        if let Err(error) = started {
            self.catalog_loading_agent_id = None;
            self.catalog_error = Some(format!("Could not start model lookup: {error}"));
            cx.notify();
            return;
        }

        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Model lookup worker closed before returning.".into()));
            let _ = pane.update(cx, |pane, cx| {
                if pane.catalog_generation != generation
                    || pane.selected_agent_id != agent_id
                    || pane.source_task_id != source_task_id
                {
                    return;
                }
                pane.catalog_loading_agent_id = None;
                match result {
                    Ok(catalog) => {
                        pane.catalog_agent_id = Some(agent_id);
                        pane.catalog = Some(catalog);
                        pane.catalog_error = None;
                    }
                    Err(error) => {
                        pane.catalog_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_agent(&mut self, agent_id: String, cx: &mut Context<Self>) {
        if self.busy
            || agent_id == self.selected_agent_id
            || !visible_agents(&self.snapshot)
                .iter()
                .any(|agent| field(agent, "id") == agent_id)
        {
            return;
        }
        self.selected_agent_id = agent_id.clone();
        self.model_settings = if agent_id == self.source_agent_id {
            self.snapshot["tasks"]
                .as_array()
                .and_then(|tasks| {
                    tasks
                        .iter()
                        .find(|task| field(task, "id") == self.source_task_id)
                })
                .and_then(source_model_settings)
        } else {
            None
        };
        self.error = None;
        self.request_catalog(cx);
        cx.notify();
    }

    fn choose_default_model(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.model_settings.is_none() {
            return;
        }
        self.model_settings = None;
        self.error = None;
        cx.notify();
    }

    fn choose_model(&mut self, model: &Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let model_id = field(model, "id");
        if model_id.is_empty() {
            return;
        }
        self.model_settings = Some(json!({
            "model": model_id,
            "reasoningEffort": model["defaultEffort"].as_str(),
            "fastMode": null,
        }));
        self.error = None;
        cx.notify();
    }

    fn choose_retained_model(&mut self, settings: Value, cx: &mut Context<Self>) {
        if self.busy || field(&settings, "model").is_empty() {
            return;
        }
        self.model_settings = Some(settings);
        self.error = None;
        cx.notify();
    }

    pub fn request_cancel(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.catalog_generation = self.catalog_generation.wrapping_add(1);
        cx.emit(ForkDialogEvent::Cancelled);
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy
            || self.source_task_id.is_empty()
            || self.through_message_id.is_empty()
            || self.selected_agent_id.is_empty()
            || self.selected_agent().is_none()
        {
            return;
        }
        let service = self.service.clone();
        let input = json!({
            "sourceTaskId": self.source_task_id,
            "throughMessageId": self.through_message_id,
            "agentId": self.selected_agent_id,
            "modelSettings": self.model_settings,
        });
        let (sender, receiver) = oneshot::channel();
        let started = std::thread::Builder::new()
            .name("monitter-create-chat-fork".into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service.invoke("fork_task", json!({"input": input}));
                let _ = sender.send(result);
            });
        if let Err(error) = started {
            self.error = Some(format!("Could not start fork creation: {error}"));
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Fork worker closed before returning.".into()));
            let _ = pane.update(cx, |pane, cx| {
                match result {
                    Ok(task) => {
                        // Keep ownership of the modal until the parent refreshes
                        // its canonical snapshot and opens the returned task.
                        pane.busy = true;
                        cx.emit(ForkDialogEvent::Created(task));
                    }
                    Err(error) => {
                        pane.busy = false;
                        pane.error = Some(format!("Could not fork this chat: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_models(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected_model = self.model_settings.as_ref().map(|settings| field(settings, "model"));
        let agent_model = self.selected_agent().map(|agent| field(&agent, "model"));
        let catalog_model = self
            .catalog
            .as_ref()
            .filter(|_| self.catalog_agent_id.as_deref() == Some(self.selected_agent_id.as_str()))
            .map(|catalog| field(&catalog["current"], "model"));
        let default_label = catalog_model
            .as_deref()
            .filter(|model| !model.is_empty())
            .or_else(|| agent_model.as_deref().filter(|model| !model.is_empty()))
            .map_or_else(|| "Agent default".to_owned(), |model| format!("Agent default · {model}"));
        let default_entity = cx.entity();
        let mut choices = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                Button::new("fork-model-default")
                    .w_full()
                    .text()
                    .compact()
                    .disabled(self.busy)
                    .accessibility_label(format!(
                        "{}{}",
                        if self.model_settings.is_none() { "Selected: " } else { "" },
                        default_label
                    ))
                    .label(if self.model_settings.is_none() {
                        format!("✓ {default_label}")
                    } else {
                        default_label
                    })
                    .on_click(cx.listener(|pane, _, _, cx| pane.choose_default_model(cx))),
            );

        let catalog = self
            .catalog
            .as_ref()
            .filter(|_| self.catalog_agent_id.as_deref() == Some(self.selected_agent_id.as_str()));
        if let Some(models) = catalog.and_then(|value| value["models"].as_array()) {
            for model in models {
                let model_id = field(model, "id");
                if model_id.is_empty() {
                    continue;
                }
                let name = if field(model, "name").is_empty() {
                    model_id.clone()
                } else {
                    field(model, "name")
                };
                let selected = selected_model.as_deref() == Some(model_id.as_str());
                let entity = default_entity.clone();
                let chosen_model = model.clone();
                choices = choices.child(
                    Button::new(SharedString::from(format!("fork-model-{}", model_id)))
                        .w_full()
                        .text()
                        .compact()
                        .disabled(self.busy)
                        .accessibility_label(format!(
                            "{}{}",
                            if selected { "Selected: " } else { "" },
                            name
                        ))
                        .label(if selected { format!("✓ {name}") } else { name })
                        .on_click(move |_, _, cx| {
                            let model = chosen_model.clone();
                            let _ = entity.update(cx, |pane, cx| pane.choose_model(&model, cx));
                        }),
                );
            }
        }
        if let Some(selected_model) = selected_model.as_deref()
            && catalog.is_none_or(|catalog| {
                !catalog["models"].as_array().is_some_and(|models| {
                    models.iter().any(|model| field(model, "id") == selected_model)
                })
            })
        {
            let selected = selected_model.to_owned();
            let retained_settings = self.model_settings.clone().unwrap_or(Value::Null);
            let entity = default_entity.clone();
            choices = choices.child(
                Button::new("fork-model-current")
                    .w_full()
                    .text()
                    .compact()
                    .disabled(self.busy)
                    .accessibility_label(format!("Selected: Current model · {selected}"))
                    .label(format!("✓ Current model · {selected}"))
                    .on_click(move |_, _, cx| {
                        let settings = retained_settings.clone();
                        let _ = entity.update(cx, |pane, cx| {
                            pane.choose_retained_model(settings, cx)
                        });
                    }),
            );
        }
        choices.into_any_element()
    }
}

impl Render for ForkDialogPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.catalog_agent_id.as_deref() != Some(self.selected_agent_id.as_str())
            && self.catalog_loading_agent_id.as_deref() != Some(self.selected_agent_id.as_str())
            && self.catalog_error.is_none()
        {
            self.request_catalog(cx);
        }

        let palette = self.palette;
        let agents = visible_agents(&self.snapshot);
        let mut content = div()
            .id("fork-dialog-pane")
            .w(px(440.))
            .max_w(px(560.))
            .max_h(px(560.))
            .overflow_y_scroll()
            .text_color(rgb(palette.ink))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p_3()
            .font_family("Montserrat")
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child("Copies visible user and agent messages through the selected message. The source session, tools, attachments, approvals, and credentials are not copied. On your first send, the new agent receives the latest 32 copied messages, up to 2,500 characters each, as context."),
            );

        let mut agent_choices = div().flex().flex_wrap().gap(px(5.));
        for agent in agents {
            let agent_id = field(&agent, "id");
            let name = field(&agent, "name");
            let provider = field(&agent, "provider");
            let selected = self.selected_agent_id == agent_id;
            let pane = cx.entity();
            let value = agent_id.clone();
            agent_choices = agent_choices.child(
                Button::new(SharedString::from(format!("fork-agent-{agent_id}")))
                    .compact()
                    .disabled(self.busy)
                    .accessibility_label(format!(
                        "{}{} · {}",
                        if selected { "Selected: " } else { "" },
                        name,
                        provider
                    ))
                    .label(if selected {
                        format!("✓ {name} · {provider}")
                    } else {
                        format!("{name} · {provider}")
                    })
                    .on_click(move |_, _, cx| {
                        let value = value.clone();
                        let _ = pane.update(cx, |pane, cx| pane.select_agent(value, cx));
                    }),
            );
        }
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().text_size(px(11.)).text_color(rgb(palette.muted)).child("Continue with"))
                .child(agent_choices),
        );

        let catalog_matches = self.catalog_agent_id.as_deref() == Some(self.selected_agent_id.as_str());
        if self.catalog_loading_agent_id.as_deref() == Some(self.selected_agent_id.as_str()) {
            content = content.child(
                div()
                    .id("fork-catalog-loading")
                    .role(Role::Status)
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child("Reading available models…"),
            );
        }
        if let Some(catalog) = self.catalog.as_ref().filter(|_| catalog_matches)
            && let Some(warning) = catalog["warning"].as_str().filter(|text| !text.is_empty())
        {
            content = content.child(
                div()
                    .id("fork-catalog-warning")
                    .role(Role::Status)
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child(warning.to_owned()),
            );
        }
        if self
            .catalog
            .as_ref()
            .filter(|_| catalog_matches)
            .is_some_and(|catalog| {
                catalog["models"]
                    .as_array()
                    .is_some_and(|models| models.is_empty())
            })
        {
            content = content.child(
                div()
                    .id("fork-catalog-empty")
                    .role(Role::Status)
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child("No model overrides were reported. The agent default remains available."),
            );
        }
        if let Some(error) = &self.catalog_error {
            let has_fallback = self.model_settings.is_none()
                || self
                    .model_settings
                    .as_ref()
                    .is_some_and(|settings| !field(settings, "model").is_empty());
            let message = if has_fallback {
                format!("Could not read models: {error}. The agent default or retained current model remains selectable.")
            } else {
                format!("Could not read models: {error}.")
            };
            content = content.child(
                div()
                    .id("fork-catalog-error")
                    .role(Role::Status)
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .child(message),
            );
        }

        let model_choices = self.render_models(cx);
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().text_size(px(11.)).text_color(rgb(palette.muted)).child("Model for the new chat"))
                .child(model_choices),
        );

        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .id("fork-dialog-error")
                    .role(Role::Alert)
                    .text_size(px(11.))
                    .text_color(rgb(0xc44c79))
                    .child(error.clone()),
            );
        }

        let submit_disabled = self.busy || self.selected_agent_id.is_empty() || self.selected_agent().is_none();
        content.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .border_t_1()
                .border_color(rgb(palette.line))
                .pt(px(8.))
                .child(
                    Button::new("fork-cancel")
                        .outline()
                        .label("Cancel")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.request_cancel(cx))),
                )
                .child(
                    Button::new("fork-submit")
                        .primary()
                        .label(if self.busy { "Creating fork…" } else { "Create fork" })
                        .disabled(submit_disabled)
                        .on_click(cx.listener(|pane, _, _, cx| pane.submit(cx))),
                ),
        )
    }
}
