//! Anchored rich popovers for the composer permission and model controls.
use super::*;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    popover::Popover,
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    switch::Switch,
    ActiveTheme as _, Selectable as _, ThemeStyled as _, h_flex, v_flex,
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
            .debug_selector({ let title = title.clone(); move || format!("permission-choice-{title}") })
            .w_full()
            .min_w_0()
            .min_h(popup_px(if description.trim().is_empty() {
                38.
            } else {
                52.
            }))
            .items_center()
            .py(popup_px(design::space::TIGHT))
            .child(label)
    })
    .accessibility_label(accessibility_label)
    .checked(selected)
    .disabled(disabled)
}

fn permission_choice_icon(level: &str) -> &'static [u8] {
    match level {
        "read-only" => icon_assets::Eye.1,
        "auto-approve-edits" => icon_assets::Pencil.1,
        _ => icon_assets::ShieldCheck.1,
    }
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
    let sandbox = pane.model_save.sandbox.as_ref().unwrap_or(&task)
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
                .check_side(gpui_kit::component::Side::Right)
                .header("Access permissions", if applies_next { "Applies to the next turn." } else { "" });
            if disabled {
                menu = menu.item(popup_note("Available after this run finishes.".to_owned(), font_ratio));
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
                    .icon(Icon::default().data(permission_choice_icon(&level)).with_size(popup_px(16.)))
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
    open_generation: u64,
    font_ratio: f32,
    saving: bool,
    save_error: Option<String>,
    catalog: Option<Value>,
    settings: Value,
    loading: bool,
    error: Option<String>,
    disabled: bool,
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

    fn set_props(&mut self, props: ModelPickerProps, window: &mut Window, cx: &mut Context<Self>) {
        if self.props == props {
            return;
        }
        let hide_search = self.props.catalog != props.catalog
            && props.catalog.as_ref().and_then(|catalog| catalog["models"].as_array())
                .is_some_and(|models| models.len() <= 5);
        if hide_search || self.props.open_generation != props.open_generation {
            self.search.update(cx, |search, cx| search.set_value("", window, cx));
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
            if pane.task_id == task_id && expected_task == task_id && !pane.model_disabled()
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
            if pane.task_id == task_id && !pane.model_disabled() && supported
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
            if pane.task_id == task_id && !pane.model_disabled() {
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .debug_selector(|| "composer-model-picker".into())
            .w(popup_px(360.))
            // Scroll containers have no intrinsic minimum height. Give the
            // nested native popover flex wrappers a definite viewport extent.
            .h(popup_px(520.).min((window.viewport_size().height - popup_px(64.)).max(popup_px(1.))))
            .flex_shrink_0()
            .overflow_hidden()
            .gap_y(popup_px(design::space::TIGHT))
            .p(popup_px(design::space::INSET))
            .popover_style(cx)
            .font_family(font_family)
            .text_size(popup_px(12. * font_ratio))
            .child(
                h_flex()
                    .debug_selector(|| "model-picker-toolbar".into())
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_x(popup_px(design::space::TIGHT))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(popup_px(14. * font_ratio))
                            .child("Available models"),
                    )
                    .child(
                        Button::new("model-picker-refresh")
                            .compact()
                            .ghost()
                            .icon(Icon::new(IconName::RefreshCw))
                            .accessibility_label("Refresh models")
                            .disabled(props.loading || props.disabled)
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
            content = content.child(model_note("Model settings are unavailable for this chat.", font_ratio, cx));
        } else if props.loading && props.catalog.is_none() {
            content = content.child(model_note("Reading harness models…", font_ratio, cx));
        }
        if props.saving {
            content = content.child(model_note(format!("Model change queued · {}", props.settings["model"].as_str().filter(|model| !model.is_empty()).unwrap_or("Harness default")), font_ratio, cx));
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
            content = content.child(div().flex_shrink_0().debug_selector(|| "model-picker-search".into())
                .child(Input::new(&self.search).text_size(popup_px(12. * font_ratio)).aria_label("Find a model")));
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
            let row_selector = format!("model-choice-{id}");
            div().w_full().flex_shrink_0().debug_selector(move || row_selector.clone()).child(Button::new(SharedString::from(format!("model-choice-{id}")))
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
                .disabled(props.disabled)
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
                }))
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
                    .debug_selector(|| "model-picker-model-list".into())
                    .role(Role::RadioGroup)
                    .aria_label("Available models")
                    .flex_1()
                    .min_h_0()
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
                    .flex_shrink_0()
                    .gap_y(popup_px(design::space::TIGHT))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt(popup_px(design::space::TIGHT))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Reasoning effort"))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(effort_label)),
                    )
                    .when_some(self.effort_slider.as_ref(), |section, slider| {
                        section.child(Slider::new(slider).disabled(props.disabled || efforts.len() < 2))
                    })
                    .when(!effort_description.is_empty(), |section| {
                        section.child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .text_size(popup_px(11. * font_ratio))
                                .line_height(popup_px(15. * font_ratio))
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
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap_x(popup_px(design::space::GROUP))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_y(popup_px(design::space::TIGHT))
                            .child(div().font_weight(FontWeight::MEDIUM).child("Fast mode"))
                    )
                    .child(
                        Switch::new(SharedString::from(format!("fast-mode-{}", props.task_id)))
                            .checked(props.settings.get("fastMode").and_then(Value::as_bool) == Some(true))
                            .disabled(props.disabled)
                            .accessibility_label("Fast mode")
                            .tooltip(fast_description)
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
                    .flex_shrink_0()
                    .compact()
                    .text()
                    .text_size(popup_px(11. * font_ratio))
                    .label("Use harness defaults")
                    .disabled(props.disabled)
                    .accessibility_label("Use harness defaults")
                    .on_click(move |_, _, cx| {
                        popup_state.update(cx, |state, cx| {
                            let pane = state.pane.clone();
                            let task = state.props.task_id.clone();
                            pane.update(cx, |pane, cx| {
                                if pane.task_id == task && !pane.model_disabled()
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
        open_generation: pane.model_menu_open_generation,
        font_ratio: interface_font_ratio(&pane.snapshot),
        saving: pane.model_save.desired.is_some(),
        save_error: pane.model_save.error.clone().or_else(|| pane.save_error.clone()),
        catalog: pane.catalog.clone(),
        settings: pane.selected_settings(&task),
        loading: pane.catalog_loading,
        error: pane.catalog_error.clone(),
        disabled: pane.model_disabled(),
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
                        pane.set_model_picker_open(*open, cx);
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
            panel.update(cx, |state, panel_cx| state.set_props(props, window, panel_cx));
            panel
        })
        .into_any_element()
}

#[cfg(test)]
mod effort_slider_tests {
    use ::core::prelude::v1::test;
    use super::{effort_slider_state, interface_font_ratio, permission_choice_icon};

    #[test]
    fn permission_choices_use_distinct_native_svg_icons() {
        let read = permission_choice_icon("read-only");
        let edit = permission_choice_icon("auto-approve-edits");
        let full = permission_choice_icon("full-access");
        assert_ne!(read, edit);
        assert_ne!(edit, full);
        assert_ne!(read, full);
        for icon in [read, edit, full] {
            assert!(std::str::from_utf8(icon).unwrap().contains("<svg"));
        }
    }

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

#[cfg(all(test, feature = "visual-export"))]
mod native_model_popup_tests {
    use ::core::prelude::v1::test;
    use super::{model_menu, permissions_menu, ComposerFooterPane};
    use gpui::InteractiveElement as _;
    use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window, div, px, size};
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use monitter_lib::experimental_desktop::DesktopService;
    use serde_json::json;

    struct BottomComposer { pane: Entity<ComposerFooterPane> }
    impl Render for BottomComposer {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette = crate::design::palette(&json!({"theme":"dark"}), gpui_kit::WindowAppearance::Dark);
            let menu = self.pane.update(cx, |pane, cx| model_menu(pane, Button::new("fixture-model-trigger").label("Model"), palette, window, cx));
            div().size_full().flex().flex_col().justify_end().items_end().p(px(16.))
                .child(div().debug_selector(|| "bottom-composer-trigger".into()).child(menu))
        }
    }
    #[gpui_kit::test]
    fn model_popup_bottom_composer_retains_usable_viewport_and_search(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let pane_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = pane_slot.clone();
        let (_, visual) = cx.add_window_view(move |window, cx| {
            let pane = cx.new(|_| ComposerFooterPane {
                service: DesktopService::disconnected_for_ui_fixture(),
                snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"popup-task","provider":"codex","status":"idle","archived":false,"model":"model-0"}],"agents":[]}),
                task_id: "popup-task".into(), context_cleared_at: None,
                catalog: Some(json!({"current":{"model":"model-0"},"models":(0..8).map(|index| json!({"id":format!("model-{index}"),"name":format!("Model {index}"),"description":"A model supplied by the loaded catalog.","supportsFast":true,"reasoningEfforts":[{"id":"low","name":"Low","description":"Fast reasoning for straightforward tasks."},{"id":"high","name":"High","description":"More reasoning for difficult tasks."}]})).collect::<Vec<_>>()})),
                usage_overview: None, catalog_loading: false, usage_loading: false, saving: false, model_save: Default::default(), model_save_queues: Default::default(), busy: false,
                catalog_error: None, usage_error: None, save_error: None, model_menu_open: false, model_menu_open_generation: 0,
                catalog_generation: 0, usage_refresh: Default::default(), usage_clock: std::time::Instant::now(), save_generation: 0,
                observed_task_marker: None, composed_palette: None, available_width_design: None, retained_height: None,
            });
            *slot.borrow_mut() = Some(pane.clone());
            let harness = cx.new(|_| BottomComposer { pane });
            gpui_kit::component::Root::new(harness, window, cx)
        });
        visual.simulate_resize(size(px(1000.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let trigger = visual.debug_bounds("bottom-composer-trigger").unwrap();
        assert!(trigger.top() > px(700.));
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let panel = visual.debug_bounds("composer-model-picker").expect("full retained popover opened");
        assert!(panel.size.height >= px(400.), "popup collapsed: {panel:?}");
        assert!(panel.top() >= px(0.) && panel.bottom() <= px(800.));
        assert!(visual.debug_bounds("model-picker-header").is_none(), "redundant title block removed");
        let toolbar = visual.debug_bounds("model-picker-toolbar").expect("model toolbar visible");
        assert!(toolbar.top() >= panel.top() && toolbar.bottom() <= panel.bottom());
        let search = visual.debug_bounds("model-picker-search").expect("search visible");
        let list = visual.debug_bounds("model-picker-model-list").expect("models visible");
        let first = visual.debug_bounds("model-choice-model-0").expect("first model hitbox");
        assert!(search.size.height >= px(24.) && search.top() >= panel.top() && search.bottom() <= panel.bottom());
        assert!(list.size.height > panel.size.height * 0.5 && list.bottom() <= panel.bottom(),
            "models should occupy most of the menu: list={list:?}, panel={panel:?}");
        assert!(first.size.height >= px(44.) && first.bottom() <= list.bottom());
        let first_top = first.top();
        visual.simulate_event(gpui_kit::ScrollWheelEvent {
            position: list.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
            ..Default::default()
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let shifted = visual.debug_bounds("model-choice-model-0").expect("retained first model");
        assert!(shifted.top() < first_top, "model list must scroll independently");
        assert_eq!(visual.debug_bounds("model-picker-toolbar").unwrap(), toolbar,
            "scrolling model choices must keep controls in place");
        visual.simulate_click(search.center(), Default::default());
        visual.simulate_input("Model 7");
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("model-choice-model-0").is_none());
        assert!(visual.debug_bounds("model-choice-model-7").is_some());
        // Native close/reopen must start with all choices, while a refresh in
        // the same opening continues to preserve a usable search query.
        visual.simulate_click(gpui::point(px(10.), px(10.)), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let trigger = visual.debug_bounds("bottom-composer-trigger").expect("trigger rendered");
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        for selector in ["model-choice-model-0", "model-choice-model-1", "model-choice-model-2", "model-choice-model-3", "model-choice-model-4", "model-choice-model-5", "model-choice-model-6", "model-choice-model-7"] {
            assert!(visual.debug_bounds(selector).is_some(),
                "new opening resets search for {selector}");
        }
        let search = visual.debug_bounds("model-picker-search").expect("search remains available");
        visual.simulate_click(search.center(), Default::default());
        visual.simulate_input("Model 7");
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("model-choice-model-0").is_none());
        visual.simulate_resize(size(px(650.), px(460.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let small = visual.debug_bounds("composer-model-picker").unwrap();
        assert!(small.size.height > px(250.) && small.size.height <= px(396.), "{small:?}");
        assert!(small.top() >= px(0.) && small.bottom() <= px(460.));
        let pane = pane_slot.borrow().clone().expect("fixture pane");
        let generation = pane.read_with(visual, |pane, _| pane.catalog_generation);
        assert_eq!(generation, 2, "each opening revalidates once");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(pane.read_with(visual, |pane, _| pane.catalog_generation), generation,
            "ordinary renders cannot refetch the catalog");
        pane.update(visual, |pane, cx| {
            pane.catalog = Some(json!({"current":{"model":"model-0"},"models":(0..3).map(|index|
                json!({"id":format!("model-{index}"),"name":format!("Model {index}"),"description":"Refreshed catalog."})).collect::<Vec<_>>()}));
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("model-picker-search").is_none(), "small catalogs hide search");
        for selector in ["model-choice-model-0", "model-choice-model-1", "model-choice-model-2"] {
            assert!(visual.debug_bounds(selector).is_some(),
                "hidden old search must not exclude refreshed {selector}");
        }
        pane.read_with(visual, |pane, _| {
            assert_eq!(pane.snapshot["tasks"][0]["model"], "model-0");
            assert_eq!(pane.save_generation, 0, "refresh cannot save model settings");
            assert!(!pane.saving);
        });
    }
    struct BottomPermissions { pane: Entity<ComposerFooterPane> }
    impl Render for BottomPermissions {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette = crate::design::palette(&json!({"theme":"dark"}), gpui_kit::WindowAppearance::Dark);
            let menu = self.pane.update(cx, |pane, cx| permissions_menu(pane, Button::new("fixture-permissions-trigger").label("Read only"), palette, window, cx));
            div().size_full().flex().flex_col().justify_end().items_end().p(px(16.))
                .child(div().debug_selector(|| "bottom-permissions-trigger".into()).child(menu))
        }
    }
    #[gpui_kit::test]
    fn permissions_popup_native_rows_keep_icons_and_trailing_check_with_header_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|_| ComposerFooterPane {
                service: DesktopService::disconnected_for_ui_fixture(),
                snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"permission-task","provider":"codex","status":"running","archived":false,"sandbox":"read-only"}],"agents":[]}),
                task_id: "permission-task".into(), context_cleared_at: None, catalog: None, usage_overview: None,
                catalog_loading: false, usage_loading: false, saving: false, model_save: Default::default(), model_save_queues: Default::default(), busy: false,
                catalog_error: None, usage_error: None, save_error: None, model_menu_open: false, model_menu_open_generation: 0,
                catalog_generation: 0, usage_refresh: Default::default(), usage_clock: std::time::Instant::now(), save_generation: 0,
                observed_task_marker: None, composed_palette: None, available_width_design: None, retained_height: None,
            });
            retained = Some(pane.clone());
            let fixture = cx.new(|_| BottomPermissions { pane });
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        visual.simulate_resize(size(px(1000.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let trigger = visual.debug_bounds("bottom-permissions-trigger").unwrap();
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let heading = visual.debug_bounds("popup-header-title-Access permissions").expect("real menu heading");
        let note = visual.debug_bounds("popup-header-subtitle-Access permissions").expect("next-turn subtitle");
        assert!(note.size.height > px(10.));
        assert!(note.top() >= heading.bottom() && note.top() - heading.bottom() <= px(8.));
        let read = visual.debug_bounds("permission-choice-Read only").unwrap();
        let edit = visual.debug_bounds("permission-choice-Auto-approve edits").unwrap();
        let full = visual.debug_bounds("permission-choice-Full access").unwrap();
        assert!(read.top() > note.bottom());
        for row in [read, edit, full] { assert!(row.size.height >= crate::design::ui_scaled_px(52.)); }
        assert!(edit.top() > read.bottom() && full.top() > edit.bottom());
        let selected_slot = visual.debug_bounds("popup-menu-element-row-0").unwrap();
        let unselected_slot = visual.debug_bounds("popup-menu-element-row-1").unwrap();
        // Native leading icon slots survive selection; only the checked row
        // reserves space after its content for the right-side check glyph.
        assert!(read.left() - selected_slot.left() >= px(10.));
        assert!(edit.left() - unselected_slot.left() >= px(10.));
        assert!(selected_slot.right() - read.right() >= px(10.));
        assert!(unselected_slot.right() - edit.right() < px(10.));
        visual.read(|cx| {
            let pane = retained.as_ref().unwrap().read(cx);
            assert_eq!(pane.snapshot["tasks"][0]["sandbox"], "read-only");
            assert!(!pane.saving && pane.save_generation == 0, "opening never sends permission writes");
        });
    }

}
