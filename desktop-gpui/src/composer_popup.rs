//! Anchored rich popovers for the composer permission and model controls.
use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, ThemeStyled as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    popover::Popover,
    slider::SliderState,
    v_flex,
};

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

fn model_note(
    text: impl Into<String>,
    font_ratio: f32,
    cx: &Context<ModelPopupState>,
) -> AnyElement {
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

fn permission_choice_icon(level: &str) -> &'static [u8] {
    match level {
        "read-only" => icon_assets::Eye.1,
        "auto-approve-edits" => icon_assets::Pencil.1,
        _ => icon_assets::ShieldCheck.1,
    }
}

#[derive(Clone, PartialEq)]
struct PermissionChoice {
    level: String,
    supported: bool,
    stored: String,
}

#[derive(Clone, PartialEq)]
struct PermissionPickerProps {
    task_id: String,
    provider: String,
    acp: Value,
    selected: String,
    choices: Vec<PermissionChoice>,
    disabled: bool,
    applies_next: bool,
    hint: String,
    harness_label: String,
    font_ratio: f32,
}

struct PermissionPopupState {
    draft: Option<String>,
    props: PermissionPickerProps,
    pane: Entity<ComposerFooterPane>,
    hovered_level: Option<String>,
    focus: FocusHandle,
}

impl PermissionPopupState {
    fn new(
        props: PermissionPickerProps,
        pane: Entity<ComposerFooterPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = Self {
            draft: None,
            props,
            pane,
            hovered_level: None,
            focus: cx.focus_handle(),
        };
        state.focus.focus(window, cx);
        state
    }

    fn set_props(&mut self, props: PermissionPickerProps, cx: &mut Context<Self>) {
        if self.props != props {
            self.props = props;
            self.hovered_level = None;
            cx.notify();
        }
    }

    fn enabled_choices(&self) -> Vec<&PermissionChoice> {
        self.props
            .choices
            .iter()
            .filter(|choice| choice.supported)
            .collect()
    }

    fn choose(&mut self, level: String, cx: &mut Context<Self>) {
        let Some(choice) = self
            .props
            .choices
            .iter()
            .find(|choice| choice.level == level && choice.supported)
            .cloned()
        else {
            return;
        };
        if self.props.disabled {
            return;
        }
        self.draft = Some(choice.level);
        self.hovered_level = None;
        cx.notify();
    }

    fn apply_draft(&mut self, cx: &mut Context<Self>) {
        let Some(level) = self.draft.take() else { return; };
        if level == self.props.selected || self.props.disabled { return; }
        let Some(choice) = self.props.choices.iter()
            .find(|choice| choice.level == level && choice.supported).cloned() else { return; };
        if self.props.task_id == "permission-picker-preview" {
            self.props.selected = level;
            return;
        }
        let task_id = self.props.task_id.clone();
        let provider = self.props.provider.clone();
        let acp = self.props.acp.clone();
        self.pane.update(cx, |pane, cx| {
            let Some(task) = pane.selected_task() else {
                return;
            };
            if pane.task_id == task_id
                && !pane.disabled()
                && permission_supported(&provider, &acp, &choice.level)
                && backend_accepts_sandbox(&provider, &choice.stored)
            {
                pane.save_sandbox(choice.stored, cx);
            }
        });
    }

    fn navigate(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if key == "escape" {
            self.hovered_level = None;
            cx.notify();
            return false;
        }
        if self.props.disabled {
            return false;
        }
        if key == "enter" {
            if let Some(level) = self.hovered_level.clone() {
                self.choose(level, cx);
            }
            return false;
        }
        if !matches!(key, "left" | "right" | "up" | "down") {
            return false;
        }
        let choices = self.enabled_choices();
        if choices.is_empty() {
            return true;
        }
        let current = self
            .hovered_level
            .as_deref()
            .unwrap_or(self.draft.as_deref().unwrap_or(self.props.selected.as_str()));
        let current = choices
            .iter()
            .position(|choice| choice.level == current)
            .unwrap_or(0);
        let next = match key {
            "left" | "up" => current.saturating_sub(1),
            _ => (current + 1).min(choices.len() - 1),
        };
        self.hovered_level = Some(choices[next].level.clone());
        cx.notify();
        true
    }
}

impl Render for PermissionPopupState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut props = self.props.clone();
        if let Some(draft) = &self.draft { props.selected = draft.clone(); }
        let selected_level = props.selected.clone();
        let caption_level = self
            .hovered_level
            .clone()
            .unwrap_or_else(|| selected_level.clone());
        let caption_choice = props
            .choices
            .iter()
            .find(|choice| choice.level == caption_level)
            .or_else(|| props.choices.first());
        let surface = cx.theme().popover;
        let track = cx.theme().muted;
        let edge = cx.theme().border;
        let secondary = cx.theme().muted_foreground;
        let font_ratio = props.font_ratio;
        let popup_state = cx.entity();
        let risk_color = |level: &str| match level {
            "read-only" => 0x38d39f,
            "auto-approve-edits" => 0xf4a340,
            _ => 0xd85be8,
        };
        let compact = window.viewport_size().width < popup_px(430.);
        let panel_width = if compact { 280. } else { 360. };
        let count = props.choices.len().max(1);
        let rail_width = panel_width - 36.;
        let stop_width = rail_width / count as f32;
        let stop_x = |index: usize| 14. + index as f32 * (rail_width - 28.) / (count - 1).max(1) as f32;
        let selected_index = props
            .choices
            .iter()
            .position(|choice| choice.level == selected_level)
            .unwrap_or(0);
        let hover_index = self
            .hovered_level
            .as_ref()
            .and_then(|level| props.choices.iter().position(|choice| &choice.level == level));

        let stops = props.choices.iter().enumerate().map(|(index, choice)| {
            let active = choice.level == selected_level;
            let ghost = self.hovered_level.as_deref() == Some(choice.level.as_str()) && !active;
            let state = popup_state.clone();
            let hover_state = popup_state.clone();
            let level = choice.level.clone();
            let hover_level = choice.level.clone();
            let label = permission_label(&choice.level).to_owned();
            let label_for_accessibility = label.clone();
            let color = rgb(risk_color(&choice.level));
            let icon_size = popup_px(12.);
            v_flex().relative().w(popup_px(stop_width)).items_center()
                .child(h_flex().w_full().h(popup_px(22.)).gap_x(popup_px(4.))
                    .when(index == 0, |row| row.justify_start())
                    .when(index == count - 1, |row| row.justify_end())
                    .when(index > 0 && index < count - 1, |row| row.justify_center())
                    .text_color(if active || ghost { color.into() } else { secondary })
                    .child(Icon::default().data(permission_choice_icon(&choice.level)).with_size(icon_size))
                    .when(!compact, |row| row.child(div().text_size(popup_px(9. * font_ratio)).whitespace_nowrap().child(label))))
                .child(
                    Button::new(SharedString::from(format!("permission-stop-{}", choice.level)))
                        .ghost()
                        .compact()
                        .p_0()
                        .absolute()
                        .top_0()
                        .w(popup_px(stop_width))
                        .h(popup_px(56.))
                        .role(Role::RadioButton)
                        .selected(false).custom(ButtonCustomVariant::new(cx).color(rgba(0x00000000).into()).hover(rgba(0x00000000).into()).active(rgba(0x00000000).into()))
                        .disabled(props.disabled || !choice.supported)
                        .accessibility_label(format!("{} access", label_for_accessibility))
                        .on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| state.choose(level.clone(), cx));
                        })
                        .on_hover(move |hovered, _, cx| {
                            let _ = hover_state.update(cx, |state, cx| {
                                let next = (*hovered).then(|| hover_level.clone());
                                if state.hovered_level != next {
                                    state.hovered_level = next;
                                    cx.notify();
                                }
                            });
                        }),
                )
                .into_any_element()
        });

        // Reserve the same viewport for every notch, including unsupported choices.
        let caption = caption_choice.map(|choice| {
            let preview = self.hovered_level.is_some();
            let color = rgb(risk_color(&choice.level));
            v_flex()
                .id("permission-preview-caption")
                .w_full()
                .h(popup_px(28. + 76. * font_ratio))
                .flex_shrink_0()
                .overflow_y_scroll()
                .p(popup_px(12.))
                .rounded(popup_px(9.))
                .border_1()
                .border_color(edge)
                .bg(track.opacity(0.62))
                .gap_y(popup_px(4.))
                .child(
                    h_flex()
                        .items_center()
                        .gap_x(popup_px(6.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(
                            Icon::default()
                                .data(permission_choice_icon(&choice.level))
                                .with_size(popup_px(13.)),
                        )
                        .child(permission_label(&choice.level))
                        .when(preview, |row| {
                            row.child(
                                div()
                                    .ml_auto()
                                    .px(popup_px(6.))
                                    .rounded(popup_px(3.))
                                    .bg(cx.theme().muted)
                                    .text_size(popup_px(10. * font_ratio))
                                    .text_color(secondary)
                                    .child("Preview"),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(popup_px(11. * font_ratio))
                        .line_height(popup_px(15. * font_ratio))
                        .text_color(secondary)
                        .child(if choice.supported {
                            permission_description(&choice.level)
                        } else {
                            "Not supported by this task's harness."
                        }),
                )
                .into_any_element()
        });

        v_flex()
            .id("composer-permission-picker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|state, event: &KeyDownEvent, _, cx| {
                if state.navigate(&event.keystroke.key, cx) {
                    cx.stop_propagation();
                }
            }))
            .debug_selector(|| "composer-permission-picker".into())
            .w(popup_px(panel_width))
            .overflow_hidden()
            .popover_style(cx)
            .bg(surface)
            .border_color(edge)
            .rounded(popup_px(14.))
            .font_family(cx.theme().font_family.clone())
            .text_size(popup_px(12. * font_ratio))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .px(popup_px(18.))
                    .py(popup_px(13.))
                    .border_b_1()
                    .border_color(edge)
                    .child(
                        v_flex()
                            .gap_y(popup_px(2.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(popup_px(13. * font_ratio))
                                    .child("Access permissions"),
                            )
                            .child(
                                div()
                                    .text_size(popup_px(10. * font_ratio))
                                    .text_color(secondary)
                                    .child(if props.applies_next {
                                        "Applies to the next turn"
                                    } else {
                                        "Applies to this chat"
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .px(popup_px(7.))
                            .py(popup_px(3.))
                            .rounded(popup_px(4.))
                            .bg(track)
                            .text_size(popup_px(10. * font_ratio))
                            .text_color(secondary)
                            .child(props.harness_label),
                    ),
            )
            .child(
                v_flex()
                    .px(popup_px(18.))
                    .pt(popup_px(12.))
                    .gap_y(popup_px(10.))
                    .child(
                        div()
                            .relative()
                            .w(popup_px(rail_width))
                            .h(popup_px(60.))
                            .child(h_flex().absolute().top_0().left_0().w_full().children(stops))
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(popup_px(30.))
                                    .w(popup_px(rail_width))
                                    .h(popup_px(24.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(edge)
                                    .bg(track)
                                    .when(true, |rail| {
                                        rail.child(
                                            div()
                                                .absolute()
                                                .left(popup_px(6.))
                                                .top(popup_px(5.))
                                                .w(popup_px(stop_x(selected_index) - 6.))
                                                .h(popup_px(12.))
                                                .rounded_full()
                                                .overflow_hidden().flex().children((0..=selected_index).map(|i| div().flex_shrink_0().w(popup_px((stop_x(selected_index) - 6.) / (selected_index + 1) as f32)).h_full().bg(rgb(risk_color(&props.choices[i].level)).opacity(0.58)))),
                                        )
                                    })
                                    .when_some(hover_index.filter(|index| *index != selected_index), |rail, index| {
                                        rail.child(
                                            div()
                                                .absolute()
                                                .left(popup_px(6.))
                                                .top(popup_px(5.))
                                                .w(popup_px(stop_x(index) - 6.))
                                                .h(popup_px(12.))
                                                .rounded_full()
                                                .border_1()
                                                .border_dashed()
                                                .border_color(rgb(risk_color(&props.choices[index].level)).opacity(0.65)),
                                        )
                                    }),
                            )
                            .children(props.choices.iter().enumerate().map(|(index, choice)| {
                                let active = index == selected_index;
                                let ghost = hover_index == Some(index) && !active;
                                let color = rgb(risk_color(&choice.level));
                                let diameter = if !choice.supported { 16. } else if active || ghost { 28. } else { 4. };
                                div()
                                    .absolute()
                                    .left(popup_px(stop_x(index) - diameter / 2.))
                                    .top(popup_px(42. - diameter / 2.))
                                    .w(popup_px(diameter))
                                    .h(popup_px(diameter))
                                    .rounded_full()
                                    .when(active || ghost, |knob| knob.border_2())
                                    .border_color(if active || ghost { color.into() } else { secondary.opacity(0.38) })
                                    .bg(if active { rgb(0xf3f7fa).into() } else if ghost { track.opacity(0.35) } else { secondary.opacity(0.46).into() })
                                    .when(!choice.supported, |dot| dot.bg(surface).rounded_none().text_color(secondary).child("–"))
                                    .into_any_element()
                            })),
                    )
                    .when_some(caption, |content, caption| content.child(caption)),
            )
            .child(
                h_flex()
                    .w_full()
                    .mt(popup_px(12.))
                    .px(popup_px(18.))
                    .py(popup_px(10.))
                    .border_t_1()
                    .border_color(edge)
                    .text_size(popup_px(10. * font_ratio))
                    .text_color(secondary)
                    .child(props.hint),
            )
    }
}

#[cfg(feature = "visual-export")]
pub(crate) fn permission_picker_preview(window: &mut Window, cx: &mut App) -> AnyView {
    let task_id = "permission-picker-preview".to_owned();
    let pane = cx.new(|_| ComposerFooterPane {
        service: DesktopService::disconnected_for_ui_fixture(),
        snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"permission-picker-preview","provider":"codex","status":"idle","archived":false,"sandbox":"workspace-write"}],"agents":[]}).into(),
        task_id: task_id.clone(), context_cleared_at: None, catalog: None, usage_overview: None,
        catalog_loading: false, usage_loading: false, saving: false, model_save: Default::default(), model_save_queues: Default::default(), busy: false,
        catalog_error: None, usage_error: None, save_error: None, model_menu_open: false, model_menu_open_generation: 0,
        catalog_generation: 0, usage_refresh: Default::default(), usage_clock: std::time::Instant::now(), save_generation: 0,
        observed_task_marker: None, composed_palette: None, available_width_design: None, retained_height: None,
    });
    cx.new(|cx| {
        PermissionPopupState::new(
            PermissionPickerProps {
                task_id,
                provider: "codex".to_owned(),
                acp: Value::Null,
                selected: "auto-approve-edits".to_owned(),
                choices: ["read-only", "auto-approve-edits", "full-access"]
                    .into_iter()
                    .map(|level| PermissionChoice {
                        level: level.to_owned(),
                        supported: true,
                        stored: stored_permission("codex", level),
                    })
                    .collect(),
                disabled: false,
                applies_next: false,
                hint: "Codex supports all three levels.".to_owned(),
                harness_label: "Codex".to_owned(),
                font_ratio: 1.,
            },
            pane,
            window,
            cx,
        )
    })
    .into()
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
    let sandbox = pane
        .model_save
        .sandbox
        .as_ref()
        .unwrap_or(&task)
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
            PermissionChoice {
                level: level.to_owned(),
                supported: permission_supported(&provider, &acp, level)
                    && backend_accepts_sandbox(&provider, &stored),
                stored,
            }
        })
        .collect::<Vec<_>>();
    let props = PermissionPickerProps {
        task_id: pane.task_id.clone(),
        provider: provider.clone(),
        acp,
        selected,
        choices,
        disabled,
        applies_next,
        hint: hint.to_owned(),
        harness_label: if provider == "codex" {
            "Codex".to_owned()
        } else {
            provider.clone()
        },
        font_ratio,
    };
    let entity = cx.entity();
    let state_key = format!("composer-permission-picker-{}-{}", entity.entity_id(), props.task_id);
    Popover::new(SharedString::from(format!(
        "composer-permission-picker-popover-{}",
        entity.entity_id()
    )))
    .appearance(false)
    .trigger(trigger)
    .anchor(Anchor::BottomLeft)
    .on_open_change({
        let props = props.clone();
        let entity = entity.clone();
        let state_key = state_key.clone();
        move |open, window, cx| {
            let factory_props = props.clone();
            let factory_pane = entity.clone();
            let panel = window.use_keyed_state(SharedString::from(state_key.clone()), cx,
                move |window, cx| PermissionPopupState::new(factory_props, factory_pane, window, cx));
            panel.update(cx, |state, cx| {
                if *open { state.draft = None; state.hovered_level = None; state.set_props(props.clone(), cx); }
                else { state.apply_draft(cx); }
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
            move |window, panel_cx| {
                PermissionPopupState::new(factory_props, factory_pane, window, panel_cx)
            },
        );
        panel.update(cx, |state, panel_cx| state.set_props(props, panel_cx));
        panel
    })
    .into_any_element()
}

fn interface_font_ratio(snapshot: &Value) -> f32 {
    snapshot["settings"]["interfaceFontSize"]
        .as_f64()
        .unwrap_or(14.)
        .clamp(8., 32.) as f32
        / 14.
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
    draft: Option<Value>,
    props: ModelPickerProps,
    pane: Entity<ComposerFooterPane>,
    search: Entity<InputState>,
    search_observation: Option<gpui_kit::Subscription>,
    hovered_choice: Option<(String, String)>,
    focus: FocusHandle,
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
            draft: None,
            props,
            pane,
            search,
            search_observation: None,
            hovered_choice: None,
            focus: cx.focus_handle(),
        };
        state.search_observation = Some(cx.observe(&state.search, |_, _, cx| cx.notify()));
        if state.models().len() > 7 {
            state
                .search
                .update(cx, |search, cx| search.focus(window, cx));
        }
        if state.models().len() <= 7 { state.focus.focus(window, cx); }
        state
    }

    fn set_props(&mut self, props: ModelPickerProps, window: &mut Window, cx: &mut Context<Self>) {
        if self.props == props {
            return;
        }
        let hide_search = self.props.catalog != props.catalog
            && props
                .catalog
                .as_ref()
                .and_then(|catalog| catalog["models"].as_array())
                .is_some_and(|models| models.len() <= 5);
        if self.props.open_generation != props.open_generation {
            self.draft = None;
            self.hovered_choice = None;
        }
        if hide_search || self.props.open_generation != props.open_generation {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
        }
        self.props = props;
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
        let selected = self.settings().get("model")?.as_str()?;
        self.models()
            .into_iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(selected))
    }

    fn selected_effort_id(&self, levels: &[Value]) -> String {
        let selected_model = self.selected_model();
        self.settings()
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                selected_model
                    .as_ref()
                    .and_then(|model| model.get("defaultEffort").and_then(Value::as_str))
                    .map(str::to_owned)
            })
            .or_else(|| {
                levels
                    .first()
                    .and_then(|level| level.get("id").and_then(Value::as_str))
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    }

    fn settings(&self) -> &Value {
        self.draft.as_ref().unwrap_or(&self.props.settings)
    }

    fn choose_model_effort(&mut self, model_id: String, effort_id: String, cx: &mut Context<Self>) {
        if self.props.disabled { return; }
        let Some(model) = self.models().into_iter().find(|model| model["id"].as_str() == Some(&model_id)) else { return; };
        if !model["reasoningEfforts"].as_array().is_some_and(|levels|
            levels.iter().any(|level| level["id"].as_str() == Some(&effort_id))) { return; }
        let fast = if model["supportsFast"].as_bool() == Some(true) { self.settings()["fastMode"].clone() } else { Value::Null };
        self.draft = Some(json!({"model":model_id,"reasoningEffort":effort_id,"fastMode":fast}));
        self.hovered_choice = None;
        cx.notify();
    }

    fn choose_model_without_effort(&mut self, model_id: String, cx: &mut Context<Self>) {
        if self.props.disabled { return; }
        let Some(model) = self.models().into_iter().find(|model| model["id"].as_str() == Some(&model_id)) else { return; };
        if model["reasoningEfforts"].as_array().is_some_and(|levels| !levels.is_empty()) { return; }
        let fast = if model["supportsFast"].as_bool() == Some(true) { self.settings()["fastMode"].clone() } else { Value::Null };
        self.draft = Some(json!({"model":model_id,"reasoningEffort":null,"fastMode":fast}));
        self.hovered_choice = None;
        cx.notify();
    }

    fn save_fast_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.props.disabled || !self.selected_model().is_some_and(|model| model["supportsFast"].as_bool() == Some(true)) { return; }
        let mut next = self.settings().clone();
        next["fastMode"] = json!(enabled);
        self.draft = Some(next);
        cx.notify();
    }

    fn apply_draft(&mut self, cx: &mut Context<Self>) {
        let Some(mut next) = self.draft.take() else { return; };
        if self.props.disabled || next == self.props.settings { return; }
        if self.props.task_id == "model-picker-preview" {
            self.props.settings = next;
            return;
        }
        let task_id = self.props.task_id.clone();
        self.pane.update(cx, |pane, cx| {
            if pane.task_id != task_id || pane.model_disabled() { return; }
            let Some(task) = pane.selected_task() else { return; };
            if next["model"].as_str() == Some("") {
                if pane.selected_settings(&task) != next { pane.save_model(next, cx); }
                return;
            }
            let Some(model) = pane.catalog.as_ref().and_then(|c| c["models"].as_array())
                .and_then(|models| models.iter().find(|model| model["id"] == next["model"])) else { return; };
            let levels = model["reasoningEfforts"].as_array();
            let supported = if let Some(levels) = levels.filter(|levels| !levels.is_empty()) {
                levels.iter().any(|level| level["id"] == next["reasoningEffort"])
            } else { next["reasoningEffort"].is_null() };
            if !supported { return; }
            if model["supportsFast"].as_bool() != Some(true) { next["fastMode"] = Value::Null; }
            if pane.selected_settings(&task) != next { pane.save_model(next, cx); }
        });
    }

    fn navigate(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if key == "escape" { self.hovered_choice = None; self.close(cx); return true; }
        if self.props.disabled { return false; }
        if key == "enter" {
            if let Some((model, effort)) = self.hovered_choice.take() {
                if effort.is_empty() { self.choose_model_without_effort(model, cx); }
                else { self.choose_model_effort(model, effort, cx); }
            }
            self.close(cx); return true;
        }
        if key == "f" {
            self.save_fast_mode(self.settings()["fastMode"].as_bool() != Some(true), cx);
            return true;
        }
        if !matches!(key, "left" | "right" | "up" | "down") { return false; }
        let query = self.search.read(cx).value().to_lowercase();
        let models: Vec<_> = self.models().into_iter().filter(|model| query.is_empty()
            || ["id", "name", "description"].iter().any(|field| model[*field].as_str().unwrap_or_default().to_lowercase().contains(&query))).collect();
        if models.is_empty() { return true; }
        let selected = self.hovered_choice.clone().unwrap_or_else(|| (
            self.settings()["model"].as_str().unwrap_or_default().to_owned(),
            self.settings()["reasoningEffort"].as_str().unwrap_or_default().to_owned()));
        let mut row = models.iter().position(|model| model["id"].as_str() == Some(&selected.0)).unwrap_or(0);
        if key == "up" { row = row.saturating_sub(1); }
        if key == "down" { row = (row + 1).min(models.len() - 1); }
        let model = &models[row];
        let levels = model["reasoningEfforts"].as_array().cloned().unwrap_or_default();
        let mut column = levels.iter().position(|level| level["id"].as_str() == Some(&selected.1))
            .or_else(|| levels.iter().position(|level| level["id"] == model["defaultEffort"]))
            .unwrap_or(0);
        if key == "left" { column = column.saturating_sub(1); }
        if key == "right" { column = (column + 1).min(levels.len().saturating_sub(1)); }
        self.hovered_choice = Some((model["id"].as_str().unwrap_or_default().to_owned(),
            levels.get(column).and_then(|level| level["id"].as_str()).unwrap_or_default().to_owned()));
        cx.notify(); true
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.apply_draft(cx);
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
        let mut props = self.props.clone();
        props.settings = self.settings().clone();
        let selected_id = props.settings["model"].as_str().unwrap_or_default().to_owned();
        let selected_model = self.selected_model();
        let selected_efforts = selected_model.as_ref().and_then(|model| model["reasoningEfforts"].as_array()).cloned().unwrap_or_default();
        let effort_id = self.selected_effort_id(&selected_efforts);
        let selected_effort = selected_efforts.iter().find(|level| level["id"].as_str() == Some(&effort_id));
        let hovered_choice = self.hovered_choice.clone();
        let caption_model_id = hovered_choice.as_ref().map(|choice| choice.0.as_str()).unwrap_or(&selected_id);
        let caption_effort_id = hovered_choice.as_ref().map(|choice| choice.1.as_str()).unwrap_or(&effort_id);
        let caption_model = self.models().into_iter().find(|model| model["id"].as_str() == Some(caption_model_id));
        let caption_effort = caption_model.as_ref().and_then(|model| model["reasoningEfforts"].as_array()).and_then(|efforts| efforts.iter().find(|effort| effort["id"].as_str() == Some(caption_effort_id)));
        let query = self.search.read(cx).value().to_lowercase();
        let models = self.models().into_iter().filter(|model| {
            ["name", "id", "description"].into_iter().any(|key| model[key].as_str().unwrap_or_default().to_lowercase().contains(&query))
        }).collect::<Vec<_>>();
        let surface = cx.theme().popover;
        let track = cx.theme().muted;
        let edge = cx.theme().border;
        let secondary = cx.theme().muted_foreground;
        let fast_rail_height = (models.len().min(7) as f32 * 36. - 28.).max(64.);
        let font_ratio = props.font_ratio;
        let popup_state = cx.entity();
        let pane = self.pane.clone();
        let harness_label = self.pane.read(cx).selected_task()
            .and_then(|task| task["provider"].as_str().map(str::to_owned))
            .map(|provider| format!("{} harness", if provider == "codex" { "Codex".to_owned() } else { provider }))
            .unwrap_or_else(|| "Harness".to_owned());
        let mut columns = self.models()
            .iter()
            .flat_map(|model| {
                model
                    .get("reasoningEfforts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|level| level.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect::<Vec<_>>();
        columns.sort_by_key(|id| (match id.as_str() {
            "minimal" => 0,
            "low" => 0,
            "medium" => 1,
            "high" => 2,
            "xhigh" | "extra-high" | "extra_high" => 3,
            "pro" | "max" | "ultra" => 4,
            _ => 5,
        }, id.clone()));
        columns.dedup();
        let cell_width = 360. / columns.len().max(5) as f32;
        let effort_color = |index: usize| [0x38d39f, 0xe3d147, 0xf4a340, 0xef5a68, 0xd85be8][match columns.get(index).map(String::as_str).unwrap_or("low") {
            "minimal" | "low" => 0, "medium" => 1, "high" => 2,
            "xhigh" | "extra-high" | "extra_high" => 3, "pro" | "max" | "ultra" => 4, _ => index.min(4),
        }];
        let headers = columns.iter().enumerate().map(|(index, id)| {
                let label = models
                    .iter()
                    .flat_map(|model| {
                        model
                            .get("reasoningEfforts")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                    })
                    .find(|level| level.get("id").and_then(Value::as_str) == Some(id.as_str()))
                    .and_then(|level| level.get("name").and_then(Value::as_str))
                    .unwrap_or(id);
                v_flex().w(popup_px(cell_width))
                    .items_center()
                    .gap_y(popup_px(4.))
                    .child(
                        div()
                            .w(popup_px(18.))
                            .h(popup_px(2.))
                            .bg(rgb(effort_color(index))),
                    )
                    .child(
                        div()
                            .text_size(popup_px(10. * font_ratio))
                            .text_color(if id == caption_effort_id { rgb(effort_color(index)).into() } else { secondary })
                            .child(label.to_owned()),
                    )
            });
        let mut content = v_flex()
            .id("composer-model-picker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|state, event: &KeyDownEvent, _, cx| {
                if state.navigate(&event.keystroke.key, cx) { cx.stop_propagation(); }
            }))
            .debug_selector(|| "composer-model-picker".into())
            .w(popup_px(650.))
            .h(popup_px(240. + models.len().clamp(4, 7) as f32 * 36. + if self.models().len() > 7 { 46. } else { 0. })
                .min((window.viewport_size().height - popup_px(64.)).max(popup_px(1.))))
            .flex_shrink_0()
            .relative()
            .overflow_hidden()
            .popover_style(cx).bg(surface).border_color(edge)
            .rounded(popup_px(16.))
            .font_family(cx.theme().font_family.clone())
            .text_size(popup_px(12. * font_ratio))
            .child(
                h_flex()
                    .debug_selector(|| "model-picker-toolbar".into())
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .px(popup_px(22.))
                    .h(popup_px(60.))
                    .border_b_1()
                    .border_color(edge)
                    .child(
                        v_flex()
                            .w(popup_px(154.))
                            .flex_shrink_0()
                            .gap_y(popup_px(3.))
                            .child(div().font_weight(FontWeight::SEMIBOLD)
                                .text_size(popup_px(14. * font_ratio)).child("Model"))
                            .child(div().text_size(popup_px(10. * font_ratio))
                                .text_color(secondary).child(harness_label)),
                    )
                    .child(h_flex().w(popup_px(360.)).flex_shrink_0().children(headers))
                    .child(
                        Button::new("model-picker-refresh")
                            .ml_auto()
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
            content = content.child(model_note(
                "Model settings are unavailable for this chat.",
                font_ratio,
                cx,
            ));
        } else if props.loading && props.catalog.is_none() {
            content = content.child(model_note("Reading harness models…", font_ratio, cx));
        }
        if props.saving {
            content = content.child(model_note(
                format!(
                    "Model change queued · {}",
                    props.settings["model"]
                        .as_str()
                        .filter(|model| !model.is_empty())
                        .unwrap_or("Harness default")
                ),
                font_ratio,
                cx,
            ));
        }
        if let Some(error) = props.save_error.as_ref() {
            content = content.child(model_note(
                format!("Could not save model settings: {error}"),
                font_ratio,
                cx,
            ));
        }
        if let Some(error) = props.error.as_ref() {
            content = content.child(model_note(
                format!("Could not load models: {error}"),
                font_ratio,
                cx,
            ));
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

        if self.models().len() > 7 {
            content = content.child(
                div()
                    .px(popup_px(22.))
                    .pt(popup_px(10.))
                    .flex_shrink_0()
                    .debug_selector(|| "model-picker-search".into())
                    .child(
                        Input::new(&self.search)
                            .text_size(popup_px(12. * font_ratio))
                            .aria_label("Find a model"),
                    ),
            );
        }
        let model_rows = models.iter().cloned().map(|model| {
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
            let chosen = selected_id == id;
            let supported = model
                .get("reasoningEfforts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let row_selector = format!("model-choice-{id}");
            let supported_columns = supported.iter().filter_map(|level| {
                let id = level["id"].as_str()?;
                columns.iter().position(|column| column == id)
            }).collect::<Vec<_>>();
            let first_column = supported_columns.iter().copied().min().unwrap_or(0);
            let last_column = supported_columns.iter().copied().max().unwrap_or(0);
            let selected_index = columns.iter().position(|column| column == &effort_id).unwrap_or(first_column);
            let hover_column = hovered_choice.as_ref().filter(|choice| choice.0 == id).and_then(|choice| columns.iter().position(|column| *column == choice.1));
            let rail_width = popup_px(((last_column.saturating_sub(first_column) + 1) as f32) * cell_width);
            let dots = (0..columns.len()).map(|column_index| {
                let level = supported.iter().find(|level| level["id"].as_str() == Some(columns[column_index].as_str()));
                let Some(level) = level else {
                    return div().w(popup_px(cell_width)).h(popup_px(34.)).flex().items_center().justify_center()
                        .bg(surface).text_color(secondary).child("–").into_any_element();
                };
                let level_id = level["id"].as_str().unwrap_or_default().to_owned();
                let level_name = level["name"].as_str().unwrap_or(&level_id).to_owned();
                let active = chosen && effort_id == level_id;
                let ghost = hovered_choice.as_ref().is_some_and(|choice| choice.0 == id && choice.1 == level_id) && !active;
                let default_ring = !active && model["defaultEffort"].as_str() == Some(level_id.as_str());
                let cell_model = id.clone();
                let popup_state = popup_state.clone();
                let hover_state = popup_state.clone();
                let hover_model = id.clone();
                let hover_effort = level_id.clone();
                Button::new(SharedString::from(format!("model-effort-{id}-{level_id}"))).ghost().compact()
                    .w(popup_px(cell_width)).h(popup_px(34.)).role(Role::RadioButton).selected(false).disabled(props.disabled)
                    .accessibility_label(format!("{name}, {level_name}")).tooltip(format!("{name} · {level_name}"))
                    .child(div().w(popup_px(if active || ghost { 28. } else if default_ring { 10. } else { 6. })).h(popup_px(if active || ghost { 28. } else if default_ring { 10. } else { 6. })).rounded_full()
                        .when(active || ghost || default_ring, |dot| dot.border_2()).border_color(if active || ghost { rgb(effort_color(column_index)).into() } else { secondary.opacity(0.5) }).bg(if active { rgb(0xf3f7fa).into() } else if ghost { cx.theme().muted.opacity(0.2) } else if default_ring { track } else { secondary.opacity(0.65).into() }))
                    .on_click(move |_, _, cx| popup_state.update(cx, |state, cx| state.choose_model_effort(cell_model.clone(), level_id.clone(), cx)))
                    .on_hover(move |hovered, _, cx| { let _ = hover_state.update(cx, |state, cx| { let next = (*hovered).then(|| (hover_model.clone(), hover_effort.clone())); if state.hovered_choice != next { state.hovered_choice = next; cx.notify(); } }); })
                    .into_any_element()
            });
            let model_name = if supported.is_empty() {
                let state = popup_state.clone(); let model_id = id.clone();
                Button::new(SharedString::from(format!("model-no-effort-{id}"))).ghost().compact().label(name.clone()).disabled(props.disabled)
                    .accessibility_label(format!("{name}, default effort")).tooltip(format!("{name} · harness-selected effort"))
                    .on_click(move |_, _, cx| state.update(cx, |state, cx| state.choose_model_without_effort(model_id.clone(), cx))).into_any_element()
            } else { div().min_w_0().truncate().font_weight(if chosen { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).text_color(if chosen { rgb(effort_color(selected_index)).into() } else { cx.theme().foreground }).child(name.clone()).into_any_element() };
            let effort_controls = if supported.is_empty() || columns.is_empty() {
                div().w(popup_px(360.)).h(popup_px(34.)).flex().items_center().text_color(secondary).child("Default effort").into_any_element()
            } else {
                div().relative().w(popup_px(360.)).h(popup_px(34.))
                    .child(div().absolute().left(popup_px(first_column as f32 * cell_width)).top(popup_px(5.)).w(rail_width).h(popup_px(24.)).rounded_full().border_1().border_color(edge).overflow_hidden().bg(track)
                            .when(chosen, |rail| rail.child(div().absolute().left(popup_px(6.)).top(popup_px(5.)).h(popup_px(12.)).w(popup_px((selected_index.saturating_sub(first_column) as f32 + 0.5) * cell_width - 6.)).rounded_full().overflow_hidden().flex().children((first_column..=selected_index).map(|cost| div().w(popup_px(cell_width)).flex_shrink_0().h_full().bg(rgb(effort_color(cost)).opacity(0.62)))))))
                    .when(hover_column.is_some() && !chosen, |area| area.child(
                        div().absolute().left(popup_px(first_column as f32 * cell_width + 6.))
                            .top(popup_px(11.)).h(popup_px(12.))
                            .w(popup_px((hover_column.unwrap_or(first_column).saturating_sub(first_column) as f32 + 0.5) * cell_width - 6.))
                            .rounded_full().border_1().border_dashed()
                            .border_color(rgb(effort_color(hover_column.unwrap_or(first_column))).opacity(0.6))))
                    .child(div().size_full().flex().children(dots)).into_any_element()
            };
            h_flex()
                .w_full()
                .flex_shrink_0()
                .debug_selector(move || row_selector.clone())
                .h(popup_px(36.))
                .items_center()
                .child(
                    div().w(popup_px(176.)).pl(popup_px(22.))
                        .min_w_0()
                        .child(model_name),
                )
                .child(effort_controls)
                .into_any_element()
        });

        if props.catalog.is_some() && self.models().is_empty() {
            content = content.child(model_note(
                "This harness does not provide a model catalog. The existing model is unchanged."
                    .to_owned(),
                font_ratio,
                cx,
            ));
        } else if !self.models().is_empty() && models.is_empty() {
            content = content.child(model_note("No models match this search.", font_ratio, cx));
        } else if !models.is_empty() {
            content = content
                .child(
                    div()
                        .id("model-picker-model-list")
                        .debug_selector(|| "model-picker-model-list".into())
                        .role(Role::RadioGroup)
                        .aria_label("Model and thinking level")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_y(popup_px(0.))
                        .children(model_rows),
                );
        }

        let supports_fast = selected_model
            .as_ref()
            .and_then(|model| model.get("supportsFast"))
            .and_then(Value::as_bool)
            == Some(true);
        {
            let fast_description = selected_model
                .as_ref()
                .and_then(|model| model.get("fastDescription"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or("Faster responses with increased usage.")
                .to_owned();
            let popup_state = popup_state.clone();
            let fast_on = props.settings.get("fastMode").and_then(Value::as_bool) == Some(true);
            let fast_button = Button::new(SharedString::from(format!("fast-mode-{}", props.task_id))).ghost().compact().p_0()
                .w(popup_px(50.)).h(popup_px(fast_rail_height + 46.)).disabled(props.disabled || !supports_fast).accessibility_label("Fast mode").tooltip(fast_description)
                .child(div().relative().w(popup_px(50.)).h(popup_px(fast_rail_height + 46.)).flex().flex_col().items_center().child(div().text_size(popup_px(10. * font_ratio)).text_color(if fast_on { rgb(0x32c8ff).into() } else { secondary }).child(h_flex().gap_x(popup_px(4.)).child(Icon::default().data(icon_assets::Zap.1).with_size(popup_px(10.))).child("FAST"))).child(div().mt(popup_px(8.)).w(popup_px(50.)).h(popup_px(fast_rail_height)).relative().rounded_full().border_1().border_color(if fast_on { rgb(0x32c8ff).into() } else { edge }).bg(if fast_on { rgb(0x32c8ff).opacity(0.10).into() } else { track }).flex().flex_col().items_center().child(div().absolute().top(popup_px(16.)).bottom(popup_px(16.)).left(popup_px(24.)).w(popup_px(1.)).bg(edge)).when(fast_on, |rail| rail.justify_start()).when(!fast_on, |rail| rail.justify_end()).child(div().m(popup_px(3.)).w(popup_px(34.)).h(popup_px(34.)).rounded_full().border_2().border_color(if fast_on { rgb(0x32c8ff).into() } else { edge }).bg(if fast_on { rgb(0x32c8ff).into() } else { secondary }).flex().items_center().justify_center().text_color(track).child(Icon::default().data(icon_assets::Zap.1).with_size(popup_px(15.))))))
                .on_click(move |_, _, cx| popup_state.update(cx, |state, state_cx| state.save_fast_mode(!fast_on, state_cx)));
            content = content.child(h_flex().absolute().right(popup_px(32.)).top(popup_px(if self.models().len() > 7 { 110. } else { 64. })).child(fast_button));
        }

        if let Some(model) = caption_model.as_ref() {
            let model_name = model.get("name").and_then(Value::as_str).unwrap_or("Model");
            let effort_name = caption_effort
                .and_then(|level| level.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(caption_effort_id);
            let description = [
                model.get("description").and_then(Value::as_str),
                caption_effort.and_then(|level| level.get("description")).and_then(Value::as_str),
            ].into_iter().flatten().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" ");
            // A fixed viewport keeps the matrix and popup anchored as preview text changes.
            content = content.child(v_flex().id("model-preview-caption").mx(popup_px(22.)).mb(popup_px(10.)).w_auto().h(popup_px(28. + 76. * font_ratio)).flex_shrink_0().overflow_y_scroll().p(popup_px(12.))
                    .rounded(popup_px(10.))
                    .bg(track.opacity(0.65))
                    .border_1()
                    .border_color(edge)
                    .gap_y(popup_px(4.))
                    .child(h_flex().gap_x(popup_px(6.)).font_weight(FontWeight::SEMIBOLD).child(model_name.to_owned()).child("·").child(div().text_color(rgb(columns.iter().position(|id| id == caption_effort_id).map(effort_color).unwrap_or(0x82919f))).child(effort_name.to_owned())).when(props.settings["fastMode"].as_bool() == Some(true), |row| row.child(div().px(popup_px(6.)).rounded(popup_px(3.)).bg(rgb(0x32c8ff).opacity(0.15)).text_color(rgb(0x32c8ff)).child("Fast"))).when(hovered_choice.is_some(), |row| row.child(div().ml_auto().px(popup_px(6.)).rounded(popup_px(3.)).bg(cx.theme().muted).text_size(popup_px(10. * font_ratio)).child("Preview"))))
                    .when(!description.is_empty(), |this| {
                        this.child(
                            div()
                                .text_color(secondary)
                                .text_size(popup_px(11. * font_ratio))
                                .child(description.clone()),
                        )
                    }));
        }

        {
            let popup_state = popup_state.clone();
            content = content.child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .px(popup_px(22.))
                    .py(popup_px(12.))
                    .justify_between()
                    .border_t_1()
                    .border_color(edge)
                    .pt(popup_px(8.))
                    .child(
                        Button::new("model-picker-reset-defaults")
                            .flex_shrink_0()
                            .compact()
                            .text()
                            .text_size(popup_px(11. * font_ratio))
                            .icon(Icon::new(IconName::RefreshCw)).label("Use harness defaults")
                            .disabled(props.disabled)
                            .accessibility_label("Use harness defaults")
                            .on_click(move |_, _, cx| {
                                popup_state.update(cx, |state, cx| {
                                    if !state.props.disabled {
                                        state.draft = Some(json!({"model":"","reasoningEffort":null,"fastMode":null}));
                                        state.hovered_choice = None;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        div()
                            .text_size(popup_px(11. * font_ratio))
                            .text_color(secondary)
                            .child("Applies to the next turn"),
                    ),
            );
        }
        content
    }
}

#[cfg(feature = "visual-export")]
pub(crate) fn model_picker_preview(window: &mut Window, cx: &mut App) -> AnyView {
    let task_id = "model-picker-preview".to_owned();
    let catalog = json!({
        "current":{"model":"gpt-6-sol"},
        "models":[
            {"id":"gpt-6-astra","name":"GPT 6 Astra","description":"Advanced reasoning for complex tasks.","supportsFast":true,"fastDescription":"Fast responses with increased usage.","reasoningEfforts":[{"id":"low","name":"Low","description":"Fast reasoning for straightforward tasks."},{"id":"medium","name":"Medium","description":"Balanced reasoning depth and speed."},{"id":"high","name":"High","description":"Greater reasoning depth for complex problems."},{"id":"xhigh","name":"Extra high","description":"Longer reasoning for demanding work."},{"id":"pro","name":"Pro","description":"Maximum reasoning depth."}]},
            {"id":"gpt-6-sol","name":"GPT 6 Sol","description":"Previous generation workhorse model.","defaultEffort":"high","supportsFast":true,"reasoningEfforts":[{"id":"low","name":"Low","description":"Fast reasoning for straightforward tasks."},{"id":"medium","name":"Medium","description":"Balanced reasoning depth and speed."},{"id":"high","name":"High","description":"Greater reasoning depth for complex problems."},{"id":"xhigh","name":"Extra high","description":"Longer reasoning for demanding work."}]},
            {"id":"gpt-6-luna","name":"GPT 6 Luna","description":"Fast and affordable for easier tasks.","supportsFast":true,"reasoningEfforts":[{"id":"low","name":"Low","description":"Fast reasoning for straightforward tasks."},{"id":"medium","name":"Medium","description":"Balanced depth and speed."},{"id":"high","name":"High","description":"Greater reasoning depth for complex problems."}]},
            {"id":"gpt-5.6-sol","name":"GPT 5.6 Sol","description":"Previous generation workhorse model.","supportsFast":false,"reasoningEfforts":[{"id":"low","name":"Low","description":"Fast reasoning for straightforward tasks."},{"id":"medium","name":"Medium","description":"Balanced reasoning depth and speed."},{"id":"high","name":"High","description":"Greater reasoning depth for complex problems."},{"id":"xhigh","name":"Extra high","description":"Longer reasoning for demanding work."}]}
        ]
    });
    let pane = cx.new(|_| ComposerFooterPane {
        service: DesktopService::disconnected_for_ui_fixture(),
        snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"model-picker-preview","provider":"codex","status":"idle","archived":false,"modelSettings":{"model":"gpt-6-sol","reasoningEffort":"high","fastMode":false}}],"agents":[]}).into(),
        task_id: task_id.clone(), context_cleared_at: None, catalog: Some(catalog.clone()), usage_overview: None,
        catalog_loading: false, usage_loading: false, saving: false, model_save: Default::default(), model_save_queues: Default::default(), busy: false,
        catalog_error: None, usage_error: None, save_error: None, model_menu_open: true, model_menu_open_generation: 0,
        catalog_generation: 0, usage_refresh: Default::default(), usage_clock: std::time::Instant::now(), save_generation: 0,
        observed_task_marker: None, composed_palette: None, available_width_design: None, retained_height: None,
    });
    cx.new(|cx| ModelPopupState::new(ModelPickerProps {
        task_id, open_generation: 0, font_ratio: 1., saving: false, save_error: None, catalog: Some(catalog),
        settings: json!({"model":"gpt-6-sol","reasoningEffort":"high","fastMode":false}), loading: false, error: None, disabled: false,
    }, pane, window, cx)).into()
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
        save_error: pane
            .model_save
            .error
            .clone()
            .or_else(|| pane.save_error.clone()),
        catalog: pane.catalog.clone(),
        settings: pane.selected_settings(&task),
        loading: pane.catalog_loading,
        error: pane.catalog_error.clone(),
        disabled: pane.model_disabled(),
    };
    let is_open = pane.model_menu_open;
    let entity = cx.entity();
    let state_key = format!(
        "composer-model-selection-panel-{}-{task_id}",
        entity.entity_id()
    );
    Popover::new(SharedString::from(format!(
        "composer-model-selection-popover-{}",
        entity.entity_id()
    )))
    .appearance(false)
    .trigger(trigger)
    .anchor(Anchor::BottomRight)
    .open(is_open)
    .on_open_change({
        let entity = entity.clone();
        let task_id = task_id.clone();
        let props = props.clone();
        let state_key = state_key.clone();
        move |open, window, cx| {
            if !*open {
                let factory_props = props.clone();
                let factory_pane = entity.clone();
                let panel = window.use_keyed_state(SharedString::from(state_key.clone()), cx,
                    move |window, cx| ModelPopupState::new(factory_props, factory_pane, window, cx));
                panel.update(cx, |state, cx| state.apply_draft(cx));
            }
            let _ = entity.update(cx, |pane, cx| {
                if pane.task_id == task_id { pane.set_model_picker_open(*open, cx); }
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
            move |window, panel_cx| {
                ModelPopupState::new(factory_props, factory_pane, window, panel_cx)
            },
        );
        panel.update(cx, |state, panel_cx| {
            state.set_props(props, window, panel_cx)
        });
        panel
    })
    .into_any_element()
}

#[cfg(test)]
mod effort_slider_tests {
    use super::{effort_slider_state, interface_font_ratio, permission_choice_icon};
    use ::core::prelude::v1::test;

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
        assert_eq!(
            interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":21}})),
            1.5
        );
        assert_eq!(
            interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":100}})),
            32. / 14.
        );
        assert_eq!(
            interface_font_ratio(&serde_json::json!({"settings":{"interfaceFontSize":1}})),
            8. / 14.
        );
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
        let levels = [
            json!({"id":"low"}),
            json!({"id":"mid"}),
            json!({"id":"high"}),
        ];
        let state = effort_slider_state(&levels, 1).expect("multiple levels have a slider");

        assert_eq!(state.min_value(), 0.);
        assert_eq!(state.max_value(), 2.);
        assert_eq!(state.value(), SliderValue::Single(1.));
        assert_eq!(state.step_value(), 1.);
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod native_model_popup_tests {
    use super::{ComposerFooterPane, model_menu, permissions_menu};
    use ::core::prelude::v1::test;
    use gpui::InteractiveElement as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window, div, px, size,
    };
    use monitter_lib::experimental_desktop::DesktopService;
    use serde_json::json;
    use std::rc::Rc;

    struct BottomComposer {
        pane: Entity<ComposerFooterPane>,
    }
    impl Render for BottomComposer {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette =
                crate::design::palette(&json!({"theme":"dark"}), gpui_kit::WindowAppearance::Dark);
            let menu = self.pane.update(cx, |pane, cx| {
                model_menu(
                    pane,
                    Button::new("fixture-model-trigger").label("Model"),
                    palette,
                    window,
                    cx,
                )
            });
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_end()
                .p(px(16.))
                .child(
                    div()
                        .debug_selector(|| "bottom-composer-trigger".into())
                        .child(menu),
                )
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
                snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"popup-task","provider":"codex","status":"idle","archived":false,"model":"model-0"}],"agents":[]}).into(),
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
        let panel = visual
            .debug_bounds("composer-model-picker")
            .expect("full retained popover opened");
        assert!(panel.size.height >= px(400.), "popup collapsed: {panel:?}");
        assert!(panel.top() >= px(0.) && panel.bottom() <= px(800.));
        assert!(
            visual.debug_bounds("model-picker-header").is_none(),
            "redundant title block removed"
        );
        let toolbar = visual
            .debug_bounds("model-picker-toolbar")
            .expect("model toolbar visible");
        assert!(toolbar.top() >= panel.top() && toolbar.bottom() <= panel.bottom());
        let search = visual
            .debug_bounds("model-picker-search")
            .expect("search visible");
        let list = visual
            .debug_bounds("model-picker-model-list")
            .expect("models visible");
        let first = visual
            .debug_bounds("model-choice-model-0")
            .expect("first model hitbox");
        assert!(
            search.size.height >= px(24.)
                && search.top() >= panel.top()
                && search.bottom() <= panel.bottom()
        );
        assert!(
            list.size.height > panel.size.height * 0.5 && list.bottom() <= panel.bottom(),
            "models should occupy most of the menu: list={list:?}, panel={panel:?}"
        );
        assert!(first.size.height >= px(44.) && first.bottom() <= list.bottom());
        let first_top = first.top();
        visual.simulate_event(gpui_kit::ScrollWheelEvent {
            position: list.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
            ..Default::default()
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let shifted = visual
            .debug_bounds("model-choice-model-0")
            .expect("retained first model");
        assert!(
            shifted.top() < first_top,
            "model list must scroll independently"
        );
        assert_eq!(
            visual.debug_bounds("model-picker-toolbar").unwrap(),
            toolbar,
            "scrolling model choices must keep controls in place"
        );
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
        let trigger = visual
            .debug_bounds("bottom-composer-trigger")
            .expect("trigger rendered");
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        for selector in [
            "model-choice-model-0",
            "model-choice-model-1",
            "model-choice-model-2",
            "model-choice-model-3",
            "model-choice-model-4",
            "model-choice-model-5",
            "model-choice-model-6",
            "model-choice-model-7",
        ] {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "new opening resets search for {selector}"
            );
        }
        let search = visual
            .debug_bounds("model-picker-search")
            .expect("search remains available");
        visual.simulate_click(search.center(), Default::default());
        visual.simulate_input("Model 7");
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("model-choice-model-0").is_none());
        visual.simulate_resize(size(px(650.), px(460.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let small = visual.debug_bounds("composer-model-picker").unwrap();
        assert!(
            small.size.height > px(250.) && small.size.height <= px(396.),
            "{small:?}"
        );
        assert!(small.top() >= px(0.) && small.bottom() <= px(460.));
        let pane = pane_slot.borrow().clone().expect("fixture pane");
        let generation = pane.read_with(visual, |pane, _| pane.catalog_generation);
        assert_eq!(generation, 2, "each opening revalidates once");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            pane.read_with(visual, |pane, _| pane.catalog_generation),
            generation,
            "ordinary renders cannot refetch the catalog"
        );
        pane.update(visual, |pane, cx| {
            pane.catalog = Some(json!({"current":{"model":"model-0"},"models":(0..3).map(|index|
                json!({"id":format!("model-{index}"),"name":format!("Model {index}"),"description":"Refreshed catalog."})).collect::<Vec<_>>()}));
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            visual.debug_bounds("model-picker-search").is_none(),
            "small catalogs hide search"
        );
        for selector in [
            "model-choice-model-0",
            "model-choice-model-1",
            "model-choice-model-2",
        ] {
            assert!(
                visual.debug_bounds(selector).is_some(),
                "hidden old search must not exclude refreshed {selector}"
            );
        }
        pane.read_with(visual, |pane, _| {
            assert_eq!(pane.snapshot["tasks"][0]["model"], "model-0");
            assert_eq!(
                pane.save_generation, 0,
                "refresh cannot save model settings"
            );
            assert!(!pane.saving);
        });
    }
    struct BottomPermissions {
        pane: Entity<ComposerFooterPane>,
    }
    impl Render for BottomPermissions {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette =
                crate::design::palette(&json!({"theme":"dark"}), gpui_kit::WindowAppearance::Dark);
            let menu = self.pane.update(cx, |pane, cx| {
                permissions_menu(
                    pane,
                    Button::new("fixture-permissions-trigger").label("Read only"),
                    palette,
                    window,
                    cx,
                )
            });
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .items_end()
                .p(px(16.))
                .child(
                    div()
                        .debug_selector(|| "bottom-permissions-trigger".into())
                        .child(menu),
                )
        }
    }
    #[gpui_kit::test]
    fn permissions_popup_uses_single_level_track_without_writing_on_open(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|_| ComposerFooterPane {
                service: DesktopService::disconnected_for_ui_fixture(),
                snapshot: json!({"settings":{"interfaceFontSize":14},"tasks":[{"id":"permission-task","provider":"codex","status":"running","archived":false,"sandbox":"read-only"}],"agents":[]}).into(),
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
        let panel = visual
            .debug_bounds("composer-permission-picker")
            .expect("permission level track");
        assert!(panel.size.width >= px(280.) && panel.size.width <= px(370.));
        assert!(panel.top() >= px(0.) && panel.bottom() <= px(800.));
        visual.read(|cx| {
            let pane = retained.as_ref().unwrap().read(cx);
            assert_eq!(pane.snapshot["tasks"][0]["sandbox"], "read-only");
            assert!(
                !pane.saving && pane.save_generation == 0,
                "opening never sends permission writes"
            );
        });
    }
}
