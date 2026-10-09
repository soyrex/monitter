//! Anchored rich popovers for the composer permission and model controls.
use super::*;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    popover::Popover,
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    switch::Switch,
    ActiveTheme as _, Selectable as _, h_flex, v_flex,
};
use gpui_kit::assets::IconName;

fn popup_px(value: f32) -> gpui_kit::Pixels {
    design::ui_scaled_px(value)
}

fn effort_slider_state(levels: &[Value], selected_index: usize) -> Option<SliderState> {
    if levels.len() < 2 {
        return None;
    }
    let max_index = (levels.len() - 1) as f32;
    Some(
        SliderState::new()
            .max(max_index)
            .min(0.)
            .step(1.)
            .default_value(selected_index.min(levels.len() - 1) as f32),
    )
}

fn popup_note(text: String, font_ratio: f32) -> PopupMenuItem {
    let accessible = text.clone();
    PopupMenuItem::element(move |_, cx| {
        div()
            .w(popup_px(260.))
            .min_w_0()
            .py(popup_px(design::space::TIGHT))
            .whitespace_normal()
            .font_family(cx.theme().font_family.clone())
            .text_size(popup_px(11. * font_ratio))
            .line_height(popup_px(15. * font_ratio))
            .text_color(cx.theme().muted_foreground)
            .child(text.clone())
    })
    .accessibility_label(accessible)
    .disabled(true)
}

fn popup_heading(text: &'static str, font_ratio: f32) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        div()
            .w_full()
            .py(popup_px(design::space::TIGHT))
            .font_family(cx.theme().font_family.clone())
            .text_size(popup_px(12. * font_ratio))
            .line_height(popup_px(16. * font_ratio))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(text)
    })
    .accessibility_label(text)
    .disabled(true)
}

fn model_note(text: impl Into<String>, font_ratio: f32, cx: &Context<ModelPopupState>) -> AnyElement {
    let text = text.into();
    div()
        .w_full()
        .min_w_0()
        .whitespace_normal()
        .py(popup_px(design::space::TIGHT))
        .font_family(cx.theme().font_family.clone())
        .text_size(popup_px(11. * font_ratio))
        .line_height(popup_px(15. * font_ratio))
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn rich_menu_item(
    title: String,
    description: String,
    accessibility_label: String,
    selected: bool,
    disabled: bool,
    truncate_description: bool,
    font_ratio: f32,
) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        let mut title_element = div()
            .w_full()
            .min_w_0()
            .truncate()
            .font_family(cx.theme().font_family.clone())
            .text_size(popup_px(12. * font_ratio))
            .line_height(popup_px(15. * font_ratio))
            .font_weight(FontWeight::MEDIUM)
            .child(title.clone());
        if disabled {
            title_element = title_element.text_color(cx.theme().muted_foreground);
        }
        let mut label = v_flex()
            .flex_1()
            .min_w_0()
            .gap_y(popup_px(design::space::TIGHT))
            .font_family(cx.theme().font_family.clone())
            .child(title_element);
        if !description.trim().is_empty() {
            let mut detail = div()
                .w_full()
                .min_w_0()
                .font_family(cx.theme().font_family.clone())
                .text_size(popup_px(11. * font_ratio))
                .line_height(popup_px(14. * font_ratio))
                .text_color(cx.theme().muted_foreground)
                .child(description.clone());
            if truncate_description {
                detail = detail.truncate();
            } else {
                detail = detail.whitespace_normal();
            }
            if disabled {
                detail = detail.text_color(cx.theme().muted_foreground);
            }
            label = label.child(detail);
        }
        h_flex()
            .w_full()
            .min_w_0()
            .min_h(popup_px(if description.trim().is_empty() {
                30.
            } else {
                44.
            }))
            .items_center()
            .child(label)
    })
    .accessibility_label(accessibility_label)
    .checked(selected)
    .disabled(disabled)
}

pub(super) fn permissions_menu(
    pane: &ComposerFooterPane,
    trigger: Button,
    _palette: design::Palette,
    _window: &mut Window,
    cx: &mut Context<ComposerFooterPane>,
) -> AnyElement {
    let Some(task) = pane.selected_task() else {
        return trigger.into_any_element();
    };
    let font_ratio = interface_font_ratio(&pane.snapshot);
    let provider = task
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("opencode")
        .to_owned();
    let acp = task
        .get("acp")
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| {
            pane.selected_agent(&task)
                .and_then(|agent| agent.get("acp").cloned())
        })
        .unwrap_or(Value::Null);
    let sandbox = task
        .get("sandbox")
        .and_then(Value::as_str)
        .unwrap_or("harness-configured");
    let selected = normalized_permission(&provider, &acp, sandbox).to_owned();
    let disabled = pane.disabled();
    let applies_next =
        task.get("status").and_then(Value::as_str) == Some("running") && provider == "codex";
    let backend_limits_choices = ["read-only", "auto-approve-edits", "full-access"]
        .into_iter()
        .any(|level| {
            permission_supported(&provider, &acp, level)
                && !backend_accepts_sandbox(&provider, &stored_permission(&provider, level))
        });
    let hint = if backend_limits_choices {
        "Some permission levels are not supported by this task's current harness."
    } else {
        permission_hint(&provider, &acp)
    };
    let choices = ["read-only", "auto-approve-edits", "full-access"]
        .into_iter()
        .map(|level| {
            let stored = stored_permission(&provider, level);
            (
                level.to_owned(),
                permission_supported(&provider, &acp, level)
                    && backend_accepts_sandbox(&provider, &stored),
                stored,
            )
        })
        .collect::<Vec<_>>();
    let task_id = pane.task_id.clone();
    let entity = cx.entity();
    let menu_button = trigger
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
            menu = menu
                .max_w(popup_px(300.))
                .max_h(popup_px(440.))
                .scrollable(true)
                .item(popup_heading("Access permissions", font_ratio));
            if disabled || applies_next {
                menu = menu.item(popup_note(if disabled {
                    "Available after this run finishes.".to_owned()
                } else {
                    "Applies to the next turn.".to_owned()
                }, font_ratio));
            }
            for (level, supported, stored) in choices.iter().cloned() {
                let chosen = selected == level;
                let blocked = disabled || !supported;
                let label = permission_label(&level);
                let description = permission_description(&level);
                let row_description = if supported {
                    description.to_owned()
                } else {
                    "Not supported by this task's harness.".to_owned()
                };
                let row_accessible = format!(
                    "{}{} — {}",
                    if chosen { "Selected: " } else { "" },
                    label,
                    row_description
                );
                let row_entity = entity.clone();
                let row_task = task_id.clone();
                menu = menu.item(
                    rich_menu_item(
                        label.to_owned(),
                        row_description,
                        row_accessible,
                        chosen,
                        blocked,
                        false,
                        font_ratio,
                    )
                    .on_click(move |_, _, cx| {
                        let entity = row_entity.clone();
                        let task = row_task.clone();
                        let stored = stored.clone();
                        let _ = entity.update(cx, |pane, cx| {
                            if pane.task_id == task {
                                pane.save_sandbox(stored, cx);
                            }
                        });
                    }),
                );
            }
            menu.item(popup_note(hint.to_owned(), font_ratio))
        })
        .into_any_element();
    div()
        .font_family(cx.theme().font_family.clone())
        .text_size(popup_px(12. * font_ratio))
        .child(menu_button)
        .into_any_element()
}

fn interface_font_ratio(snapshot: &Value) -> f32 {
    snapshot["settings"]["interfaceFontSize"].as_f64().unwrap_or(14.).clamp(8., 32.) as f32 / 14.
}

#[derive(Clone, PartialEq)]
struct ModelPickerProps {
    task_id: String,
    font_ratio: f32,
    saving: bool,
    save_error: Option<String>,
    catalog: Option<Value>,
    settings: Value,
    loading: bool,
    error: Option<String>,
    disabled: bool,
    applies_next_turn: bool,
    jev: String,
}

struct ModelPopupState {
    props: ModelPickerProps,
    pane: Entity<ComposerFooterPane>,
    search: Entity<InputState>,
    effort_slider: Option<Entity<SliderState>>,
    effort_signature: String,
    search_observation: Option<gpui_kit::Subscription>,
    slider_observation: Option<gpui_kit::Subscription>,
    slider_subscription: Option<gpui_kit::Subscription>,
}

impl ModelPopupState {
    fn new(
        props: ModelPickerProps,
        pane: Entity<ComposerFooterPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a model…"));
        let mut state = Self {
            props,
            pane,
            search,
            effort_slider: None,
            effort_signature: String::new(),
            search_observation: None,
            slider_observation: None,
            slider_subscription: None,
        };
        state.search_observation = Some(cx.observe(&state.search, |_, _, cx| cx.notify()));
        state.sync_effort_slider(cx);
        if state.models().len() > 5 {
            state.search.update(cx, |search, cx| search.focus(window, cx));
        }
        state
    }

    fn set_props(&mut self, props: ModelPickerProps, cx: &mut Context<Self>) {
        if self.props == props {
            return;
        }
        self.props = props;
        self.sync_effort_slider(cx);
        cx.notify();
    }

    fn models(&self) -> Vec<Value> {
        self.props
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.get("models"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn selected_model(&self) -> Option<Value> {
        let selected = self.props.settings.get("model")?.as_str()?;
        self.models().into_iter().find(|model| {
            model.get("id").and_then(Value::as_str) == Some(selected)
        })
    }

    fn effort_levels(&self) -> Vec<Value> {
        self.selected_model()
            .and_then(|model| model.get("reasoningEfforts").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    }

    fn selected_effort_id(&self, levels: &[Value]) -> String {
        let selected_model = self.selected_model();
        self.props
            .settings
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| selected_model.as_ref().and_then(|model| model.get("defaultEffort").and_then(Value::as_str)).map(str::to_owned))
            .or_else(|| levels.first().and_then(|level| level.get("id").and_then(Value::as_str)).map(str::to_owned))
            .unwrap_or_default()
    }

    fn sync_effort_slider(&mut self, cx: &mut Context<Self>) {
        let levels = self.effort_levels();
        let model_id = self
            .props
            .settings
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let selected_effort = self.selected_effort_id(&levels);
        let level_ids = levels
            .iter()
            .filter_map(|level| level.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let signature = format!("{model_id}:{level_ids}:{selected_effort}");
        if signature == self.effort_signature {
            return;
        }
        self.effort_signature = signature;
        self.slider_observation = None;
        self.slider_subscription = None;
        self.effort_slider = if levels.len() < 2 {
            None
        } else {
            let selected_index = levels
                .iter()
                .position(|level| level.get("id").and_then(Value::as_str) == Some(&selected_effort))
                .unwrap_or(0);
            let Some(slider_state) = effort_slider_state(&levels, selected_index) else {
                return;
            };
            let slider = cx.new(|_| slider_state);
            self.slider_observation = Some(cx.observe(&slider, |_, _, cx| cx.notify()));
            self.slider_subscription = Some(cx.subscribe(
                &slider,
                |state, _, event: &SliderEvent, cx| {
                    if let SliderEvent::Release(value) = event {
                        state.save_effort(value.end(), cx);
                    } else {
                        cx.notify();
                    }
                },
            ));
            Some(slider)
        };
    }

    fn save_effort(&mut self, index: f32, cx: &mut Context<Self>) {
        let levels = self.effort_levels();
        let expected_model = self.props.settings.get("model").and_then(Value::as_str).unwrap_or_default().to_owned();
        let expected_task = self.props.task_id.clone();
        let Some(level) = levels.get(index.round().clamp(0., levels.len().saturating_sub(1) as f32) as usize) else {
            return;
        };
        let Some(id) = level.get("id").and_then(Value::as_str) else {
            return;
        };
        let pane = self.pane.clone();
        let task_id = self.props.task_id.clone();
        pane.update(cx, |pane, cx| {
            let Some(task) = pane.selected_task() else { return; };
            let current = pane.selected_settings(&task);
            let still_supported = pane.catalog.as_ref()
                .and_then(|catalog| catalog.get("models"))
                .and_then(Value::as_array)
                .and_then(|models| models.iter().find(|model| model.get("id").and_then(Value::as_str) == Some(expected_model.as_str())))
                .and_then(|model| model.get("reasoningEfforts"))
                .and_then(Value::as_array)
                .is_some_and(|levels| levels.iter().any(|candidate| candidate.get("id").and_then(Value::as_str) == Some(id)));
            if pane.task_id == task_id && expected_task == task_id && !pane.disabled()
                && current.get("model").and_then(Value::as_str) == Some(expected_model.as_str())
                && still_supported
            {
                let mut next = current;
                next["reasoningEffort"] = json!(id);
                pane.save_model(next, cx);
            }
        });
    }

    fn save_fast_mode(&self, enabled: bool, cx: &mut Context<Self>) {
        let pane = self.pane.clone();
        let task_id = self.props.task_id.clone();
        let expected_model = self.props.settings.get("model").and_then(Value::as_str).unwrap_or_default().to_owned();
        pane.update(cx, |pane, cx| {
            let Some(task) = pane.selected_task() else { return; };
            let settings = pane.selected_settings(&task);
            let supported = pane.catalog.as_ref()
                .and_then(|catalog| catalog.get("models"))
                .and_then(Value::as_array)
                .and_then(|models| models.iter().find(|model| model.get("id").and_then(Value::as_str) == settings.get("model").and_then(Value::as_str)))
                .and_then(|model| model.get("supportsFast"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if pane.task_id == task_id && !pane.disabled() && supported
                && settings.get("model").and_then(Value::as_str) == Some(expected_model.as_str())
            {
                let mut next = settings;
                next["fastMode"] = json!(enabled);
                pane.save_model(next, cx);
            }
        });
    }

    fn choose_model(&self, id: String, cx: &mut Context<Self>) {
        let pane = self.pane.clone();
        let task_id = self.props.task_id.clone();
        pane.update(cx, |pane, cx| {
            let Some(task) = pane.selected_task() else { return; };
            let current = pane.selected_settings(&task);
            let Some(model) = pane.catalog.as_ref()
                .and_then(|catalog| catalog.get("models"))
                .and_then(Value::as_array)
                .and_then(|models| models.iter().find(|model| model.get("id").and_then(Value::as_str) == Some(id.as_str())))
            else { return; };
            let supports_effort = model.get("reasoningEfforts").and_then(Value::as_array).is_some_and(|levels| !levels.is_empty());
            let supports_fast = model.get("supportsFast").and_then(Value::as_bool).unwrap_or(false);
            let current_id = current.get("model").and_then(Value::as_str).unwrap_or_default();
            let mut effort = if id == current_id {
                current.get("reasoningEffort").cloned().unwrap_or(Value::Null)
            } else {
                model.get("defaultEffort").cloned().filter(|value| !value.is_null()).unwrap_or(Value::Null)
            };
            if !supports_effort { effort = Value::Null; }
            let fast = if supports_fast { current.get("fastMode").cloned().unwrap_or(Value::Null) } else { Value::Null };
            if pane.task_id == task_id && !pane.disabled() {
                pane.model_menu_open = false;
                pane.save_model(json!({"model":id,"reasoningEffort":effort,"fastMode":fast}), cx);
            }
        });
    }

    fn close(&self, cx: &mut Context<Self>) {
        let pane = self.pane.clone();
        let task_id = self.props.task_id.clone();
        pane.update(cx, |pane, cx| {
            if pane.task_id == task_id {
                pane.model_menu_open = false;
                cx.notify();
            }
        });
    }
}

impl Render for ModelPopupState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let props = self.props.clone();
        let font_ratio = props.font_ratio;
        let models = self.models();
        let selected_model = self.selected_model();
        let efforts = self.effort_levels();
        let effort_id = self.selected_effort_id(&efforts);
        let saved_effort_index = efforts
            .iter()
            .position(|level| level.get("id").and_then(Value::as_str) == Some(&effort_id))
            .unwrap_or(0);
        let effort_index = self.effort_slider.as_ref().map_or(saved_effort_index, |slider| {
            slider.read(cx).value().end().round().clamp(0., efforts.len().saturating_sub(1) as f32) as usize
        });
        let effort = efforts.get(effort_index).cloned();
        let query = self.search.read(cx).value().to_string().to_lowercase();
        let visible_models = models
            .iter()
            .filter(|model| {
                ["name", "id", "description"].into_iter().any(|key| {
                    model
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let popup_state = cx.entity();
        let pane = self.pane.clone();

        let font_family = cx.theme().font_family.clone();
        let mut content = v_flex()
            .id("composer-model-picker")
            .w(popup_px(360.))
            .max_h(popup_px(520.))
            .overflow_y_scroll()
            .gap_y(popup_px(design::space::GROUP))
            .p(popup_px(design::space::INSET))
            .rounded(popup_px(design::radius::CARD))
            .border_1()
            .border_color(cx.theme().border)
            .bg(gpui_kit::Background::from(cx.theme().popover).backdrop_blur(20.))
            .text_color(cx.theme().popover_foreground)
            .font_family(font_family)
            .text_size(popup_px(12. * font_ratio))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_x(popup_px(design::space::TIGHT))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(popup_px(14. * font_ratio))
                            .child("Model"),
                    )
                    .child(
                        Button::new("model-picker-refresh")
                            .compact()
                            .ghost()
                            .icon(Icon::new(IconName::RefreshCw))
                            .accessibility_label("Refresh models")
                            .disabled(props.loading || props.disabled || props.saving)
                            .on_click({
                                let pane = pane.clone();
                                let task = props.task_id.clone();
                                move |_, _, cx| {
                                    pane.update(cx, |pane, cx| {
                                        if pane.task_id == task {
                                            pane.refresh_catalog(true, cx);
                                        }
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("model-picker-close")
                            .compact()
                            .ghost()
                            .icon(Icon::new(IconName::X))
                            .accessibility_label("Close model picker")
                            .on_click({
                                let popup_state = popup_state.clone();
                                move |_, _, cx| {
                                    popup_state.update(cx, |state, cx| state.close(cx));
                                }
                            }),
                    ),
            );

        if props.disabled && !props.saving {
            content = content.child(model_note("Available after this run finishes.", font_ratio, cx));
        } else if props.applies_next_turn {
            content = content.child(model_note("Applies to the next turn.", font_ratio, cx));
        } else if props.loading && props.catalog.is_none() {
            content = content.child(model_note("Reading harness models…", font_ratio, cx));
        }
        if props.jev != "off" {
            content = content.child(model_note(if props.jev == "safe_auto" {
                "Jev routing · automatic when confidence is sufficient".to_owned()
            } else {
                "Jev routing · model recommendation".to_owned()
            }, font_ratio, cx)).child(model_note("Jev can affect model selection for new chats. It never changes permissions.", font_ratio, cx));
        }
        if props.saving {
            content = content.child(model_note("Saving model settings…", font_ratio, cx));
        }
        if let Some(error) = props.save_error.as_ref() {
            content = content.child(model_note(format!("Could not save model settings: {error}"), font_ratio, cx));
        }
        if let Some(error) = props.error.as_ref() {
            content = content.child(model_note(format!("Could not load models: {error}"), font_ratio, cx));
        }
        if let Some(warning) = props
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.get("warning"))
            .and_then(Value::as_str)
            .filter(|warning| !warning.is_empty())
        {
            content = content.child(model_note(warning.to_owned(), font_ratio, cx));
        }

        if models.len() > 5 {
            content = content.child(Input::new(&self.search).text_size(popup_px(12. * font_ratio)).aria_label("Find a model"));
        }
        let visible_models_empty = visible_models.is_empty();
        let model_rows = visible_models.iter().cloned().map(|model| {
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_owned();
            let description = model
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let chosen = props.settings.get("model").and_then(Value::as_str) == Some(id.as_str());
            let accessible = format!(
                "{}{}{}",
                if chosen { "Selected: " } else { "" },
                name,
                if description.is_empty() {
                    String::new()
                } else {
                    format!(" — {description}")
                }
            );
            let popup_state = popup_state.clone();
            Button::new(SharedString::from(format!("model-choice-{id}")))
                .ghost()
                .text_size(popup_px(12. * font_ratio))
                .w_full()
                .min_h(popup_px(if description.is_empty() { 44. } else { 62. }))
                .px(popup_px(design::space::INSET))
                .py(popup_px(design::space::TIGHT))
                .rounded(popup_px(design::radius::CONTROL))
                .role(Role::RadioButton)
                .selected(chosen)
                .accessibility_label(accessible)
                .disabled(props.disabled || props.saving)
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .justify_between()
                        .gap_x(popup_px(design::space::GROUP))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_y(popup_px(design::space::TIGHT))
                                .child(
                                    div()
                                        .w_full()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(name),
                                )
                                .when(!description.is_empty(), |label| {
                                    label.child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .text_size(popup_px(11. * font_ratio))
                                            .line_height(popup_px(15. * font_ratio))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(description),
                                    )
                                }),
                        )
                        .child(
                            Icon::new(IconName::Check)
                                .with_size(popup_px(16.))
                                .opacity(if chosen { 1. } else { 0. }),
                        ),
                )
                .on_click(move |_, _, cx| {
                    popup_state.update(cx, |state, cx| {
                        state.choose_model(id.clone(), cx);
                    });
                })
        });

        if props.catalog.is_some() && models.is_empty() {
            content = content.child(model_note(
                "This harness does not provide a model catalog. The existing model is unchanged."
                    .to_owned(),
                font_ratio,
                cx,
            ));
        } else if !models.is_empty() && visible_models_empty {
            content = content.child(model_note("No models match this search.", font_ratio, cx));
        } else if !visible_models_empty {
            content = content.child(
                div()
                    .id("model-picker-model-list")
                    .role(Role::RadioGroup)
                    .aria_label("Available models")
                    .max_h(popup_px(245.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_y(popup_px(design::space::TIGHT))
                    .children(model_rows),
            );
        }

        if !efforts.is_empty() {
            let effort_label = effort
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(&effort_id)
                .to_owned();
            let effort_description = effort
                .as_ref()
                .and_then(|value| value.get("description"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            content = content.child(
                v_flex()
                    .w_full()
                    .gap_y(popup_px(design::space::TIGHT))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt(popup_px(design::space::SECTION))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Reasoning effort"))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(effort_label)),
                    )
                    .when_some(self.effort_slider.as_ref(), |section, slider| {
                        section.child(Slider::new(slider).disabled(props.disabled || props.saving || efforts.len() < 2))
                    })
                    .when(!effort_description.is_empty(), |section| {
                        section.child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .whitespace_normal()
                                .child(effort_description),
                        )
                    }),
            );
        }

        if selected_model
            .as_ref()
            .and_then(|model| model.get("supportsFast"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            let fast_description = selected_model
                .as_ref()
                .and_then(|model| model.get("fastDescription"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or("Faster responses with increased usage.")
                .to_owned();
            let popup_state = popup_state.clone();
            content = content.child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_x(popup_px(design::space::GROUP))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_y(popup_px(design::space::TIGHT))
                            .child(div().font_weight(FontWeight::MEDIUM).child("Fast mode"))
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(fast_description),
                            ),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("fast-mode-{}", props.task_id)))
                            .checked(props.settings.get("fastMode").and_then(Value::as_bool) == Some(true))
                            .disabled(props.disabled || props.saving)
                            .accessibility_label("Fast mode")
                            .on_change(move |enabled, _, cx| {
                                popup_state.update(cx, |state, state_cx| state.save_fast_mode(*enabled, state_cx));
                            }),
                    ),
            );
        }

        if props
            .settings
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|model| !model.is_empty())
        {
            let popup_state = popup_state.clone();
            content = content.child(
                Button::new("model-picker-reset-defaults")
                    .text()
                    .text_size(popup_px(11. * font_ratio))
                    .label("Use harness defaults")
                    .disabled(props.disabled || props.saving)
                    .accessibility_label("Use harness defaults")
                    .on_click(move |_, _, cx| {
                        popup_state.update(cx, |state, cx| {
                            let pane = state.pane.clone();
                            let task = state.props.task_id.clone();
                            pane.update(cx, |pane, cx| {
                                if pane.task_id == task && !pane.disabled()
                                    && pane.selected_task().is_some_and(|task| {
                                        pane.selected_settings(&task).get("model")
                                            .and_then(Value::as_str).is_some_and(|id| !id.is_empty())
                                    })
                                {
                                    pane.save_model(
                                        json!({"model":"","reasoningEffort":null,"fastMode":null}),
                                        cx,
                                    );
                                }
                            });
                        });
                    }),
            );
        }
        content
    }
}

pub(super) fn model_menu(
    pane: &ComposerFooterPane,
    trigger: Button,
    _palette: design::Palette,
    _window: &mut Window,
    cx: &mut Context<ComposerFooterPane>,
) -> AnyElement {
    let Some(task) = pane.selected_task() else {
        return trigger.into_any_element();
    };
    let task_id = pane.task_id.clone();
    let props = ModelPickerProps {
        task_id: pane.task_id.clone(),
        font_ratio: interface_font_ratio(&pane.snapshot),
        saving: pane.saving,
        save_error: pane.save_error.clone(),
        catalog: pane.catalog.clone(),
        settings: pane.selected_settings(&task),
        loading: pane.catalog_loading,
        error: pane.catalog_error.clone(),
        disabled: pane.disabled(),
        applies_next_turn: task.get("status").and_then(Value::as_str) == Some("running")
            && task.get("provider").and_then(Value::as_str) == Some("codex"),
        jev: pane
            .selected_agent(&task)
            .and_then(|agent| agent.get("jevRouting").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "off".to_owned()),
    };
    let is_open = pane.model_menu_open;
    let entity = cx.entity();
    let state_key = format!("composer-model-selection-panel-{}-{task_id}", entity.entity_id());
    Popover::new(SharedString::from(format!("composer-model-selection-popover-{}", entity.entity_id())))
        .appearance(false)
        .trigger(trigger)
        .anchor(Anchor::BottomRight)
        .open(is_open)
        .on_open_change({
            let entity = entity.clone();
            let task_id = task_id.clone();
            move |open, _, cx| {
                let entity = entity.clone();
                let _ = entity.update(cx, |pane, cx| {
                    if pane.task_id == task_id {
                        pane.model_menu_open = *open;
                        cx.notify();
                    }
                });
            }
        })
        .overlay_closable(true)
        .content(move |_, window, cx| {
            let props = props.clone();
            let pane = entity.clone();
            let factory_props = props.clone();
            let factory_pane = pane.clone();
            let panel = window.use_keyed_state(
                SharedString::from(state_key.clone()),
                cx,
                move |window, panel_cx| ModelPopupState::new(factory_props, factory_pane, window, panel_cx),
            );
            panel.update(cx, |state, panel_cx| state.set_props(props, panel_cx));
            panel
        })
        .into_any_element()
}

#[cfg(test)]
mod effort_slider_tests {
    use super::{effort_slider_state, interface_font_ratio};

    #[test]
    fn menu_font_tracks_interface_settings_with_default_and_bounds() {
        assert_eq!(interface_font_ratio(&serde_json::json!({})), 1.);
        assert_eq!(interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":21}})), 1.5);
        assert_eq!(interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":100}})), 32. / 14.);
        assert_eq!(interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":1}})), 8. / 14.);
    }
    use gpui_kit::component::slider::SliderValue;
    use serde_json::json;

    #[test]
    fn zero_or_one_effort_level_has_no_degenerate_slider() {
        assert!(effort_slider_state(&[], 0).is_none());
        assert!(effort_slider_state(&[json!({"id":"low"})], 0).is_none());
    }

    #[test]
    fn multiple_effort_levels_map_to_ordered_integer_indices() {
        let levels = [json!({"id":"low"}), json!({"id":"mid"}), json!({"id":"high"})];
        let state = effort_slider_state(&levels, 1).expect("multiple levels have a slider");

        assert_eq!(state.min_value(), 0.);
        assert_eq!(state.max_value(), 2.);
        assert_eq!(state.value(), SliderValue::Single(1.));
        assert_eq!(state.step_value(), 1.);
    }
}
