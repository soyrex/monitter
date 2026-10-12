//! Native approval/input and durable queue controls. All decisions go to Service.
use super::*;
use gpui_kit::component::ActiveTheme;
use crate::design::ui_scaled_px as px;
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
        let request_id = field(request, "id");
        let mut answers = serde_json::Map::new();
        for question in rows(&request["input"], "questions") {
            let question_id = field(&question, "id");
            let value = self
                .answers
                .get(&(request_id.clone(), question_id.clone()))
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default();
            if value.trim().is_empty() {
                self.error = Some("Please answer each question.".into());
                cx.notify();
                return;
            }
            if value.len() > 16_384 {
                self.error = Some("An answer exceeds the 16,384-byte limit.".into());
                cx.notify();
                return;
            }
            answers.insert(question_id, json!({"answers": [value.trim()]}));
        }
        self.command(
            "resolve_input",
            json!({"approvalId": request_id, "response": {"answers": answers}}),
            cx,
        );
    }

    fn submit_form(&mut self, request: &Value, cx: &mut Context<Self>) {
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
            Ok(response) => self.command(
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

    pub(super) fn render_interactions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = settings::palette(&self.snapshot["settings"], window.appearance());
        let Some(task_id) = self.selected.clone() else {
            return div().into_any_element();
        };
        let mut panel = div().id("task-interactions").flex().flex_col().gap_3();
        let mut requests: Vec<_> = rows(&self.snapshot, "approvalRequests")
            .into_iter()
            .filter(|request| {
                field(request, "taskId") == task_id && field(request, "status") == "pending"
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
        self.interaction_request_id = requests.get(selected).map(|request| field(request, "id"));
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
                        Button::new("previous-pending-approval")
                            .label("Previous")
                            .accessibility_label("Previous approval")
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
                        Button::new("next-pending-approval")
                            .label("Next")
                            .accessibility_label("Next approval")
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
                let approval_id = id.clone();
                let native_mcp_supported = self.mcp_approval_supported;
                let owner = cx.weak_entity();
                let decision = Rc::new(
                    move |decision: &'static str, _: &mut Window, cx: &mut App| {
                        let _ = owner.update(cx, |pane, cx| {
                            if mcp_approval && !native_mcp_supported && decision == "approve_once" {
                                pane.command("resolve_input", json!({"approvalId": approval_id, "response":{}}), cx);
                            } else {
                                pane.command(
                                    if mcp_approval && decision != "deny" { "resolve_mcp_approval" } else { "resolve_approval" },
                                    json!({"approvalId": approval_id, "decision": decision}), cx);
                            }
                        });
                    },
                );
                let owner = cx.weak_entity();
                let toggle = Rc::new(move |_: &mut Window, cx: &mut App| {
                    let _ = owner.update(cx, |pane, cx| {
                        pane.approval_details_expanded = !pane.approval_details_expanded;
                        cx.notify();
                    });
                });
                let task = rows(&self.snapshot, "tasks")
                    .into_iter()
                    .find(|task| field(task, "id") == task_id)
                    .unwrap_or(Value::Null);
                panel = panel.child(crate::approval_prompt::render_approval_prompt(
                    &request,
                    &task,
                    &self.snapshot,
                    palette,
                    cx.theme().font_family.clone(),
                    cx.theme().font_size,
                    self.busy,
                    self.approval_details_expanded,
                    self.mcp_approval_supported,
                    decision,
                    toggle,
                    window,
                    cx,
                ));
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
                                    .label(format!("{} {}", label, field(&option, "description")))
                                    .disabled(self.busy)
                                    .on_click(
                                        move |_, window, cx| {
                                            state.update(cx, |state, cx| {
                                                state.set_value(label.clone(), window, cx)
                                            })
                                        },
                                    ),
                                );
                            }
                            card = card.child(Input::new(&state).disabled(self.busy));
                        }
                        let request = request.clone();
                        actions = actions.child(
                            Button::new(SharedString::from(format!("submit-{id}")))
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
                                    card = card.child("No fields are required. Submit response continues this tool request; Deny declines it.");
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
                                        .label("I have completed this step")
                                        .icon(Icon::new(IconName::Check))
                                        .primary()
                                        .h(px(44.))
                                        .disabled(self.busy)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.command(
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
                        .label("Deny")
                        .icon(Icon::new(IconName::X))
                        .danger()
                        .h(px(44.))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.command(
                                "resolve_approval",
                                json!({"approvalId":id,"decision":"deny"}),
                                cx,
                            )
                        })),
                );
            } else {
                card = card.child(
                    div()
                        .text_sm()
                        .child(format!("Decision: {}", field(&request, "decision"))),
                );
            }
            panel = panel.child(
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
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} · {} · {} risk",
                                field(&request, "provider"),
                                field(&request, "tool"),
                                field(&request, "risk")
                            )),
                    )
                    .child(card)
                    .child(actions.pt_3().border_t_1().border_color(rgb(palette.line))),
            );
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
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
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
                editor
                    .child(
                        Button::new("save-queue-edit")
                            .label("Save queued edit")
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
                            .label("Cancel edit")
                            .disabled(self.busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.queue_edit = None;
                                cx.notify();
                            })),
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
            panel = panel.child(
                div()
                    .p_3()
                    .bg(crate::design::surface_color(palette.card))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!("Queue · {}", field(&message, "status")))
                    .child(text.clone())
                    .child(format!(
                        "{} attachment(s) {}",
                        rows(&message, "attachmentIds").len(),
                        field(&message, "error")
                    ))
                    .child(
                        Button::new(SharedString::from(format!("edit-queue-{id}")))
                            .label("Edit")
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
                            .label("Remove unsent message")
                            .disabled(self.busy || sending)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.command("cancel_queued_message", json!({"id":id}), cx)
                            })),
                    ),
            );
        }
        panel.into_any_element()
    }
}
