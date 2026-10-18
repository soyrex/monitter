//! Native approval/input and durable queue controls. All decisions go to Service.
use super::*;
use crate::design::ui_scaled_px as px;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::{
    assets::IconName,
    component::{Icon, Sizable},
};
#[path = "interaction_input.rs"]
mod input;

fn bounded_request_detail(detail: &str) -> (String, bool) {
    const DETAIL_LIMIT: usize = 4_096;
    let structured = serde_json::from_str::<Value>(detail.trim()).is_ok();
    let normalized: String = detail
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let transport = structured
        || [
            "agentthreadid",
            "receiverthreadids",
            "connectorpermissionmetadata",
            "permissionmetadata",
        ]
        .iter()
        .any(|field| normalized.contains(field));
    let mut chars = detail.chars();
    let bounded: String = chars.by_ref().take(DETAIL_LIMIT).collect();
    let truncated = chars.next().is_some();
    let needs_disclosure = transport || truncated;
    let detail = if truncated {
        format!("{bounded}\n… details truncated")
    } else {
        bounded
    };
    (detail, needs_disclosure)
}

impl chat::ChatPane {
    pub(super) fn submit_questions(&mut self, request: &Value, cx: &mut Context<Self>) {
        if self.busy || !self.user_question_is_current(request) {
            return;
        }
        let id = field(request, "id");
        let text = self
            .answers
            .iter()
            .filter(|((request, _), _)| request == &id)
            .map(|((_, question), state)| (question.clone(), state.read(cx).value().to_string()))
            .collect();
        let choices = self
            .question_choices
            .iter()
            .filter(|((request, _), _)| request == &id)
            .map(|((_, question), selected)| (question.clone(), selected.clone()))
            .collect();
        match input::questions_response(&request["input"], &text, &choices) {
            Ok(response) => self.user_question_command(
                "resolve_input",
                json!({"approvalId":id,"response":response}),
                cx,
            ),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn submit_form(&mut self, request: &Value, cx: &mut Context<Self>) {
        if self.busy || !self.user_question_is_current(request) {
            return;
        }
        let id = field(request, "id");
        let text = self
            .answers
            .iter()
            .filter(|((request_id, _), _)| request_id == &id)
            .map(|((_, name), state)| (name.clone(), state.read(cx).value().to_string()))
            .collect();
        let choices = self
            .form_choices
            .iter()
            .filter(|((request_id, _), _)| request_id == &id)
            .map(|((_, name), value)| (name.clone(), value.clone()))
            .collect();
        match input::response(&request["input"]["schema"], &text, &choices) {
            Ok(response) => self.user_question_command(
                "resolve_input",
                json!({"approvalId": id, "response": response}),
                cx,
            ),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn user_question_request(request: &Value) -> bool {
        request["input"].is_object()
            && crate::approval_prompt::mcp_approval_choices(request).is_empty()
    }

    pub(super) fn pending_user_questions(&self) -> Vec<Value> {
        let mut requests: Vec<_> = rows(&self.snapshot, "approvalRequests")
            .into_iter()
            .filter(|request| {
                field(request, "taskId") == self.task_id()
                    && request["status"] == "pending"
                    && Self::user_question_request(request)
                    && !self.resolved_user_questions.contains(&field(request, "id"))
            })
            .collect();
        requests.sort_by_key(|request| request["createdAt"].as_i64().unwrap_or_default());
        requests
    }

    pub(super) fn has_pending_user_question(&self) -> bool {
        !self.pending_user_questions().is_empty()
    }

    fn user_question_is_current(&self, request: &Value) -> bool {
        self.pending_user_questions()
            .iter()
            .any(|pending| pending["id"] == request["id"] && pending["input"] == request["input"])
    }

    fn user_question_command(
        &mut self,
        command: &'static str,
        args: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = args["approvalId"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        if self.busy
            || !self
                .pending_user_questions()
                .iter()
                .any(|request| request["id"].as_str() == Some(id))
        {
            return;
        }
        #[cfg(test)]
        if self.is_performance_fixture() {
            self.user_question_commands.push((command, args));
            self.busy = true;
            cx.notify();
            return;
        }
        self.command(command, args, cx);
    }

    pub(super) fn cancel_user_question(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.busy
            && self
                .pending_user_questions()
                .iter()
                .any(|request| field(request, "id") == id)
        {
            self.user_question_command(
                "resolve_approval",
                json!({"approvalId":id,"decision":"deny"}),
                cx,
            );
        }
    }

    pub(super) fn accept_user_question_response(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .pending_user_questions()
            .iter()
            .any(|request| field(request, "id") == id)
        {
            self.resolved_user_questions.insert(id.to_owned());
            self.question_choices
                .retain(|(request, _), _| request != id);
            cx.notify();
        }
    }

    pub(super) fn render_user_questions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let form = self.render_interaction_surface(true, window, cx);
        let requests = self.pending_user_questions();
        let next = self
            .interaction_request_id
            .as_ref()
            .filter(|id| requests.iter().any(|request| field(request, "id") == **id))
            .cloned()
            .or_else(|| requests.first().map(|request| field(request, "id")));
        if next != self.user_question_focus_id {
            let was_open = self.user_question_focus_id.is_some();
            if self.dock_visible
                && !self.approval_popup_open
                && !crate::preferences_modal::workspace_blocked(cx)
                && (self.focus_handle(cx).contains_focused(window, cx)
                    || self.user_question_focus.contains_focused(window, cx)
                    || self.composer.read(cx).focus_handle(cx).is_focused(window))
            {
                // A hidden pane or modal may postpone this transition. Keep it
                // pending until this pane owns focus, without stealing it from
                // another pane when snapshots publish in the background.
                self.user_question_focus_id = next.clone();
                let focus = next
                    .as_ref()
                    .and_then(|id| {
                        let request = requests
                            .iter()
                            .find(|request| field(request, "id") == *id)?;
                        let field_id = match request["input"]["kind"].as_str() {
                            Some("questions") => {
                                request["input"]["questions"].as_array()?.first()?["id"].as_str()?
                            }
                            Some("form") => request["input"]["schema"]["properties"]
                                .as_object()?
                                .keys()
                                .find(|name| {
                                    self.answers.contains_key(&(id.clone(), (*name).clone()))
                                })?,
                            _ => return None,
                        };
                        self.answers
                            .get(&(id.clone(), field_id.to_owned()))
                            .map(|answer| answer.read(cx).focus_handle(cx))
                    })
                    .unwrap_or_else(|| {
                        if next.is_some() {
                            self.user_question_focus.clone()
                        } else if self.user_questions_in_watch() {
                            self.focus_handle(cx)
                        } else {
                            self.composer.read(cx).focus_handle(cx)
                        }
                    });
                if next.is_some() || was_open {
                    window.defer(cx, move |window, cx| window.focus(&focus, cx));
                }
            }
        }
        form
    }

    pub(super) fn render_interactions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_interaction_surface(false, window, cx)
    }

    fn render_interaction_surface(
        &mut self,
        user_questions: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings::palette(&self.snapshot["settings"], window.appearance());
        let Some(task_id) = self.selected.clone() else {
            return div().into_any_element();
        };
        let selector = if user_questions {
            "user-question-form"
        } else {
            "task-interactions"
        };
        let mut panel = div()
            .id(selector)
            .debug_selector(move || selector.into())
            .when(user_questions, |panel| {
                panel
                    .track_focus(&self.user_question_focus)
                    .max_h(window.viewport_size().height * 0.65)
                    .overflow_y_scroll()
            })
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(6.));
        let mut requests: Vec<_> = rows(&self.snapshot, "approvalRequests")
            .into_iter()
            .filter(|request| {
                field(request, "taskId") == task_id
                    && field(request, "status") == "pending"
                    && Self::user_question_request(request) == user_questions
                    && !self.resolved_user_questions.contains(&field(request, "id"))
            })
            .collect();
        requests.sort_by_key(|request| request["createdAt"].as_i64().unwrap_or_default());
        let count = requests.len();
        let selected = self
            .interaction_request_id
            .as_ref()
            .and_then(|id| {
                requests
                    .iter()
                    .position(|request| field(request, "id") == *id)
            })
            .unwrap_or(0);
        if user_questions {
            self.interaction_request_id =
                requests.get(selected).map(|request| field(request, "id"));
        }
        if count > 1 {
            let previous = field(&requests[(selected + count - 1) % count], "id");
            let next = field(&requests[(selected + 1) % count], "id");
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new(if user_questions {
                            "previous-user-question"
                        } else {
                            "previous-pending-approval"
                        })
                        .debug_selector(move || {
                            if user_questions {
                                "previous-user-question"
                            } else {
                                "previous-pending-approval"
                            }
                            .into()
                        })
                        .label("Previous")
                        .accessibility_label(if user_questions {
                            "Previous question request"
                        } else {
                            "Previous approval"
                        })
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.interaction_request_id = Some(previous.clone());
                            cx.notify();
                        })),
                    )
                    .child(format!(
                        "{} of {} awaiting your response",
                        selected + 1,
                        count
                    ))
                    .child(
                        Button::new(if user_questions {
                            "next-user-question"
                        } else {
                            "next-pending-approval"
                        })
                        .debug_selector(move || {
                            if user_questions {
                                "next-user-question"
                            } else {
                                "next-pending-approval"
                            }
                            .into()
                        })
                        .label("Next")
                        .accessibility_label(if user_questions {
                            "Next question request"
                        } else {
                            "Next approval"
                        })
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.interaction_request_id = Some(next.clone());
                            cx.notify();
                        })),
                    ),
            );
        }
        for request in requests.into_iter().skip(selected).take(1) {
            let id = field(&request, "id");
            let mcp_approval = !crate::approval_prompt::mcp_approval_choices(&request).is_empty();
            if request["input"].is_null() || mcp_approval {
                if !self.approval_popup_open {
                    panel = panel.child(
                        Button::new("review-pending-approval")
                            .compact()
                            .label("Review approval")
                            .on_click(cx.listener(move |pane, _, window, cx| {
                                pane.show_approval_prompt(id.clone(), window, cx);
                            })),
                    );
                }
                continue;
            }
            let pending = field(&request, "status") == "pending";
            let input = request.get("input").filter(|input| !input.is_null());
            let summary = field(&request, "summary");
            let summary = if summary.trim().is_empty() {
                field(&request, "tool")
            } else {
                summary
            };
            let (bounded_summary, summary_is_raw) = bounded_request_detail(&summary);
            let summary_needs_disclosure = summary_is_raw || summary.chars().count() > 240;
            let summary_preview = if summary_is_raw {
                "Review the requested action".to_owned()
            } else {
                let preview: String = summary.chars().take(240).collect();
                if summary.chars().count() > 240 {
                    format!("{preview}…")
                } else {
                    preview
                }
            };
            let detail = field(&request, "detail");
            let (bounded_detail, detail_needs_disclosure) = bounded_request_detail(&detail);
            let mut hidden_details = Vec::new();
            if summary_needs_disclosure {
                hidden_details.push(format!("Request summary:\n{bounded_summary}"));
            }
            if detail_needs_disclosure {
                hidden_details.push(format!("Request detail:\n{bounded_detail}"));
            }
            let mut card = div()
                .id(SharedString::from(format!("input-body-{id}")))
                .debug_selector({
                    let id = id.clone();
                    move || format!("input-body-{id}")
                })
                .max_h(px(220.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(summary_preview),
                );
            let mut actions = div().flex().flex_wrap().gap_2().flex_shrink_0();
            if !detail.trim().is_empty() && !detail_needs_disclosure {
                card = card.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(bounded_detail),
                );
            }
            if !hidden_details.is_empty() {
                let hidden_details = hidden_details.join("\n\n");
                card = card.child(
                    Accordion::new(format!("approval-request-details-{id}"))
                        .bordered(false)
                        .item(|item| {
                            item.title("Request details").child(
                                div()
                                    .id(SharedString::from(format!("request-details-{id}")))
                                    .max_h(px(160.))
                                    .overflow_y_scroll()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette.muted))
                                    .child(hidden_details),
                            )
                        })
                        .into_any_element(),
                );
            }
            if pending {
                if let Some(input) = input {
                    if field(input, "kind") == "questions" {
                        for question in rows(input, "questions") {
                            let question_id = field(&question, "id");
                            let key = (id.clone(), question_id.clone());
                            let secret = question["isSecret"] == true;
                            let multiple = question["selectionMode"] == "multiple";
                            let state = self
                                .answers
                                .entry(key)
                                .or_insert_with(|| {
                                    cx.new(|cx| {
                                        InputState::new(window, cx)
                                            .placeholder("Your answer")
                                            .masked(secret)
                                    })
                                })
                                .clone();
                            card = card.child(format!(
                                "{} {}",
                                field(&question, "header"),
                                field(&question, "question")
                            ));
                            for (index, option) in
                                rows(&question, "options").into_iter().enumerate()
                            {
                                let label = field(&option, "label");
                                let state = state.clone();
                                card = card.child(
                                    Button::new(SharedString::from(format!(
                                        "answer-{id}-{question_id}-{index}"
                                    )))
                                    .debug_selector({
                                        let id = id.clone();
                                        let question_id = question_id.clone();
                                        move || format!("answer-{id}-{question_id}-{index}")
                                    })
                                    .toggled(
                                        multiple
                                            && self
                                                .question_choices
                                                .get(&(id.clone(), question_id.clone()))
                                                .is_some_and(|choices| choices.contains(&label)),
                                    )
                                    .label(format!("{} {}", label, field(&option, "description")))
                                    .disabled(self.busy)
                                    .on_click(cx.listener({
                                        let key = (id.clone(), question_id.clone());
                                        move |pane, _, window, cx| {
                                            if pane.busy {
                                                return;
                                            }
                                            if multiple {
                                                let selected = pane
                                                    .question_choices
                                                    .entry(key.clone())
                                                    .or_default();
                                                if let Some(index) = selected
                                                    .iter()
                                                    .position(|choice| choice == &label)
                                                {
                                                    selected.remove(index);
                                                } else {
                                                    selected.push(label.clone());
                                                }
                                                cx.notify();
                                            } else {
                                                state.update(cx, |state, cx| {
                                                    state.set_value(label.clone(), window, cx)
                                                });
                                            }
                                        }
                                    })),
                                );
                            }
                            card = card.child(Input::new(&state).disabled(self.busy));
                        }
                        let request = request.clone();
                        actions = actions.child(
                            Button::new(SharedString::from(format!("submit-{id}")))
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("submit-{id}")
                                })
                                .label("Submit answers")
                                .icon(Icon::new(IconName::Check))
                                .primary()
                                .h(px(44.))
                                .disabled(self.busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.submit_questions(&request, cx)
                                })),
                        );
                    } else if field(input, "kind") == "form" {
                        match input::fields(&input["schema"]) {
                            Err(error) => card = card.child(error),
                            Ok(fields) => {
                                if fields.is_empty() {
                                    card = card.child("No fields are required. Submit response continues this request; Cancel declines it.");
                                }
                                for (name, schema) in fields {
                                    let key = (id.clone(), name.clone());
                                    let title = schema["title"].as_str().unwrap_or(name);
                                    let required = input["schema"]["required"]
                                        .as_array()
                                        .is_some_and(|names| {
                                            names.iter().any(|value| value.as_str() == Some(name))
                                        });
                                    card = card.child(format!(
                                        "{title}{}",
                                        if required {
                                            " (required)"
                                        } else {
                                            " (optional)"
                                        }
                                    ));
                                    if let Some(description) = schema["description"].as_str() {
                                        card = card.child(description.to_owned());
                                    }
                                    let options =
                                        schema["enum"].as_array().cloned().or_else(|| {
                                            (schema["type"] == "boolean")
                                                .then(|| vec![json!(true), json!(false)])
                                        });
                                    if let Some(options) = options {
                                        for (index, option) in options.into_iter().enumerate() {
                                            let selected =
                                                self.form_choices.get(&key) == Some(&option);
                                            let label = option
                                                .as_bool()
                                                .map(|value| {
                                                    if value {
                                                        "Yes".to_owned()
                                                    } else {
                                                        "No".to_owned()
                                                    }
                                                })
                                                .or_else(|| option.as_str().map(str::to_owned))
                                                .unwrap_or_else(|| option.to_string());
                                            let key = key.clone();
                                            card = card.child(
                                                Button::new(SharedString::from(format!(
                                                    "form-{id}-{name}-{index}"
                                                )))
                                                .debug_selector({
                                                    let selector =
                                                        format!("form-{id}-{name}-{index}");
                                                    move || selector.clone()
                                                })
                                                .label(format!(
                                                    "{}{}",
                                                    if selected { "✓ " } else { "" },
                                                    if label.is_empty() {
                                                        "(empty)"
                                                    } else {
                                                        &label
                                                    }
                                                ))
                                                .disabled(self.busy)
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.form_choices
                                                        .insert(key.clone(), option.clone());
                                                    cx.notify();
                                                })),
                                            );
                                        }
                                        let key = key.clone();
                                        card = card.child(
                                            Button::new(SharedString::from(format!(
                                                "clear-form-{id}-{name}"
                                            )))
                                            .label("Clear selection")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.form_choices.remove(&key);
                                                cx.notify();
                                            })),
                                        );
                                    } else {
                                        let state = self
                                            .answers
                                            .entry(key)
                                            .or_insert_with(|| {
                                                cx.new(|cx| {
                                                    InputState::new(window, cx)
                                                        .placeholder("Enter value")
                                                })
                                            })
                                            .clone();
                                        card = card.child(Input::new(&state).disabled(self.busy));
                                    }
                                }
                                let request = request.clone();
                                actions = actions.child(
                                    Button::new(SharedString::from(format!("submit-form-{id}")))
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("submit-form-{id}")
                                        })
                                        .label("Submit response")
                                        .icon(Icon::new(IconName::Check))
                                        .primary()
                                        .h(px(44.))
                                        .disabled(self.busy)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.submit_form(&request, cx)
                                        })),
                                );
                            }
                        }
                    } else if field(input, "kind") == "url" {
                        match input::safe_url(&field(input, "url")) {
                            Err(error) => card = card.child(error),
                            Ok(url) => {
                                card = card.child("Check this address before opening it. Opening does not submit a response.").child(url.clone());
                                card = card.child(
                                    Button::new(SharedString::from(format!("open-input-{id}")))
                                        .label("Open browser step")
                                        .disabled(self.busy)
                                        .on_click(move |_, _, cx| cx.open_url(&url)),
                                );
                                let id = id.clone();
                                actions = actions.child(
                                    Button::new(SharedString::from(format!("complete-input-{id}")))
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("complete-input-{id}")
                                        })
                                        .label("I have completed this step")
                                        .icon(Icon::new(IconName::Check))
                                        .primary()
                                        .h(px(44.))
                                        .disabled(self.busy)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.user_question_command(
                                                "resolve_input",
                                                json!({"approvalId": id, "response": true}),
                                                cx,
                                            )
                                        })),
                                );
                            }
                        }
                    } else {
                        card = card.child(format!("Unsupported input kind: {}. Deny this request and ask for another way.", field(input, "kind")));
                    }
                } else {
                    let mut decisions = vec![("approve_once", "Approve once")];
                    if field(&request, "sessionScope") == "file_changes" {
                        card = card.child(div().text_sm().child(
                            "Session approval allows subsequent file edits in this session.",
                        ));
                        decisions.push(("approve_session", "Allow all edits this session"));
                    }
                    if request["rememberable"] == true {
                        card = card.child(div().text_sm().child(
                            "A saved rule allows this exact action in the same agent, host and workspace scope until revoked.",
                        ));
                        decisions.push(("approve_always", "Always allow exact action"));
                    }
                    for (decision, label) in decisions {
                        let approval_id = id.clone();
                        card = card.child(
                            Button::new(SharedString::from(format!("{decision}-{id}")))
                                .label(label)
                                .disabled(self.busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.command(
                                        "resolve_approval",
                                        json!({"approvalId":approval_id,"decision":decision}),
                                        cx,
                                    )
                                })),
                        );
                    }
                }
                actions = actions.child(
                    Button::new(SharedString::from(format!("deny-{id}")))
                        .label(if user_questions { "Cancel" } else { "Deny" })
                        .debug_selector({
                            let id = id.clone();
                            move || format!("cancel-input-{id}")
                        })
                        .icon(Icon::new(IconName::X))
                        .danger()
                        .h(px(44.))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if user_questions {
                                view.cancel_user_question(&id, cx);
                            } else {
                                view.command(
                                    "resolve_approval",
                                    json!({"approvalId":id,"decision":"deny"}),
                                    cx,
                                );
                            }
                        })),
                );
            } else {
                card = card.child(
                    div()
                        .text_sm()
                        .child(format!("Decision: {}", field(&request, "decision"))),
                );
            }
            panel =
                panel.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(palette.accent))
                        .bg(crate::design::modal_surface_color(palette.panel))
                        .font_family(cx.theme().font_family.clone())
                        .text_size(cx.theme().font_size)
                        .flex()
                        .flex_col()
                        .gap_3()
                        .flex_shrink_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(18.))
                                        .text_color(rgb(palette.accent)),
                                )
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Your input is needed"),
                                ),
                        )
                        .when(!user_questions, |panel| {
                            panel.child(div().text_sm().text_color(rgb(palette.muted)).child(
                                format!(
                                    "{} · {} · {} risk",
                                    field(&request, "provider"),
                                    field(&request, "tool"),
                                    field(&request, "risk")
                                ),
                            ))
                        })
                        .child(card)
                        .child(actions.pt_3().border_t_1().border_color(rgb(palette.line))),
                );
        }
        if user_questions {
            if let Some(error) = &self.error {
                panel = panel.child(div().text_color(rgb(0xdc6262)).child(error.clone()));
            }
            return panel.into_any_element();
        }
        // Saved rules are managed in the pane's Approvals view; they are not
        // active requests and must not occupy the conversation/composer area.
        let queued: Vec<_> = rows(&self.snapshot, "queuedMessages")
            .into_iter()
            .filter(|message| field(message, "taskId") == task_id)
            .collect();
        if let Some(id) = self.queue_edit.clone() {
            let message = queued.iter().find(|message| field(message, "id") == id);
            let editable = message.is_some_and(|message| {
                field(message, "status") != "sending"
                    && field(message, "origin") != "channel-agent-mention"
            });
            let has_attachments =
                message.is_some_and(|message| !rows(message, "attachmentIds").is_empty());
            let text = self.queue_editor.read(cx).value().to_string();
            let mut editor = div()
                .px(px(10.))
                .py(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(palette.line))
                .bg(crate::design::surface_color(palette.card))
                .text_size(px(11.))
                .text_color(rgb(palette.ink))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child("Edit queued message")
                .child(
                    Textarea::new(&self.queue_editor)
                        .h(px(80.))
                        .disabled(self.busy),
                );
            if !editable {
                editor = editor
                    .child("This message is no longer editable. Your edit has not been sent.");
            }
            panel = panel.child(
                editor.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .debug_selector(|| "queue-edit-actions".into())
                        .child(
                            Button::new("save-queue-edit")
                                .compact()
                                .xsmall()
                                .label("Save")
                                .accessibility_label("Save queued edit")
                                .debug_selector(|| "save-queue-edit".into())
                                .disabled(
                                    self.busy
                                        || !editable
                                        || (text.trim().is_empty() && !has_attachments),
                                )
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    let text = view.queue_editor.read(cx).value().to_string();
                                    view.command(
                                        "edit_queued_message",
                                        json!({"id":id,"text":text}),
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("discard-queue-edit")
                                .compact()
                                .xsmall()
                                .ghost()
                                .label("Cancel")
                                .accessibility_label("Cancel queued edit")
                                .debug_selector(|| "discard-queue-edit".into())
                                .disabled(self.busy)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.queue_edit = None;
                                    cx.notify();
                                })),
                        ),
                ),
            );
        }
        for message in queued {
            let id = field(&message, "id");
            let editable = field(&message, "status") != "sending"
                && field(&message, "origin") != "channel-agent-mention";
            let sending = field(&message, "status") == "sending";
            let text = field(&message, "text");
            let edit_id = id.clone();
            let row_selector = format!("queued-message-{id}");
            let edit_selector = format!("edit-queue-{id}");
            let remove_selector = format!("remove-queue-{id}");
            let attachments = rows(&message, "attachmentIds").len();
            let error = field(&message, "error");
            let status = if sending && field(&message, "origin") == "steering" {
                "Steering".to_owned()
            } else if field(&message, "status") == "queued" {
                "Queued".to_owned()
            } else {
                format!("Queue · {}", field(&message, "status"))
            };
            let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("queued-message-{id}")))
                    .debug_selector(move || row_selector.clone())
                    .role(Role::Group)
                    .aria_label(format!("{status}: {text}"))
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(crate::design::surface_color(palette.card))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .text_size(px(11.))
                    .text_color(rgb(palette.ink))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .min_w_0()
                            .child(
                                Icon::new(IconName::Clock)
                                    .size(px(12.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgb(palette.muted))
                                    .child(status),
                            )
                            .when(attachments > 0, |row| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("{attachments} attached")),
                                )
                            })
                            .child(
                                Button::new(SharedString::from(format!("edit-queue-{id}")))
                                    .compact()
                                    .xsmall()
                                    .ghost()
                                    .w(px(24.))
                                    .min_w(px(24.))
                                    .h(px(24.))
                                    .p_0()
                                    .flex_shrink_0()
                                    .icon(IconName::Pencil)
                                    .accessibility_label("Edit queued message")
                                    .tooltip("Edit queued message")
                                    .debug_selector(move || edit_selector.clone())
                                    .disabled(self.busy || !editable || self.queue_edit.is_some())
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.queue_edit = Some(edit_id.clone());
                                        view.queue_editor.update(cx, |state, cx| {
                                            state.set_value(text.clone(), window, cx)
                                        });
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("remove-queue-{id}")))
                                    .compact()
                                    .xsmall()
                                    .ghost()
                                    .w(px(24.))
                                    .min_w(px(24.))
                                    .h(px(24.))
                                    .p_0()
                                    .flex_shrink_0()
                                    .icon(IconName::X)
                                    .accessibility_label("Remove unsent message")
                                    .tooltip("Remove unsent message")
                                    .debug_selector(move || remove_selector.clone())
                                    .disabled(self.busy || sending)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.command("cancel_queued_message", json!({"id":id}), cx)
                                    })),
                            ),
                    )
                    .child(div().min_w_0().truncate().child(preview))
                    .when(!error.is_empty(), |row| {
                        row.child(div().min_w_0().text_color(rgb(palette.muted)).child(error))
                    }),
            );
        }
        panel.into_any_element()
    }
}
