//! Pane-local, read-only overview of authoritative work already in progress.
use super::*;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::{ThemeStyled as _, popover::Popover};

#[derive(Default)]
pub(super) struct WatcherState {
    pub enabled: bool,
    pub(super) interject_open: bool,
    interjection: Option<InterjectionExchange>,
    scroll: ScrollHandle,
    finished_expanded: bool,
    finished_plan_expanded: bool,
    pub(super) transitioning: bool,
    pub(super) fade_in: bool,
    pub(super) transition_serial: u64,
    transition_task: Option<Task<()>>,
}

#[derive(Clone, Debug)]
struct InterjectionExchange {
    text: String,
    wire_text: String,
    known_message_ids: Vec<String>,
    assistant_baseline: Vec<(String, String, String)>,
    known_queue_ids: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct InterjectionProjection {
    status: String,
    response_id: Option<String>,
    response_text: Option<String>,
}

impl InterjectionExchange {
    fn capture(text: String, wire_text: String, snapshot: &Value, messages: &[Rc<Value>]) -> Self {
        Self {
            text,
            wire_text,
            known_message_ids: messages
                .iter()
                .map(|message| field(message, "id"))
                .collect(),
            assistant_baseline: messages
                .iter()
                .filter(|message| field(message, "role") == "assistant")
                .map(|message| {
                    (
                        field(message, "id"),
                        field(message, "text"),
                        field(message, "streamStatus"),
                    )
                })
                .collect(),
            known_queue_ids: values(snapshot, "queuedMessages")
                .map(|message| field(message, "id"))
                .collect(),
        }
    }

    fn project(
        &self,
        snapshot: &Value,
        task_id: &str,
        messages: &[Rc<Value>],
    ) -> InterjectionProjection {
        let queued = values(snapshot, "queuedMessages")
            .filter(|message| {
                message["taskId"] == task_id
                    && field(message, "text") == self.wire_text
                    && !self.known_queue_ids.contains(&field(message, "id"))
            })
            .last();
        let acknowledged = messages.iter().any(|message| {
            field(message, "role") == "user"
                && field(message, "senderAgentId").is_empty()
                && field(message, "text") == self.wire_text
                && !self.known_message_ids.contains(&field(message, "id"))
        });
        let response = acknowledged.then(|| {
            messages.iter().rev().find(|message| {
                if field(message, "role") != "assistant" {
                    return false;
                }
                let id = field(message, "id");
                let text = field(message, "text");
                let stream_status = field(message, "streamStatus");
                self.assistant_baseline.iter().all(|baseline| {
                    baseline.0 != id || baseline.1 != text || baseline.2 != stream_status
                })
            })
        }).flatten();
        let (response_id, response_text) = response
            .map(|message| (Some(field(message, "id")), Some(field(message, "text"))))
            .unwrap_or_default();
        let status = if let Some(message) = response {
            if field(message, "streamStatus") == "streaming" {
                "Receiving reply…".into()
            } else {
                "Reply received".into()
            }
        } else if acknowledged {
            "Steered · waiting for reply…".into()
        } else if let Some(queued) = queued {
            match field(queued, "status").as_str() {
                "sending" if field(queued, "origin") == "steering" => {
                    "Steering into the current turn…".into()
                }
                "queued" => "Steer unavailable · queued for the next turn".into(),
                "error" => queued["error"]
                    .as_str()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or("The steer could not be sent.")
                    .to_owned(),
                _ => "Sending steer…".into(),
            }
        } else {
            "Sending steer…".into()
        };
        InterjectionProjection {
            status,
            response_id,
            response_text,
        }
    }
}

fn values<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    snapshot[key].as_array().into_iter().flatten()
}
fn at(value: &Value, key: &str) -> i64 {
    value[key].as_i64().unwrap_or_default()
}
fn active(status: &str) -> bool {
    matches!(
        status,
        "running" | "queued" | "pending" | "starting" | "in_progress"
    )
}
fn status_label(status: &str) -> &str {
    match status {
        "running" | "in_progress" => "Running",
        "completed" | "succeeded" => "Completed",
        "error" | "failed" => "Failed",
        "interrupted" | "cancelled" => "Stopped",
        "queued" | "pending" | "starting" => "Queued",
        "blocked" => "Blocked",
        "idle" => "Idle",
        _ => "Unknown",
    }
}
fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
fn event_time(event: &Value) -> String {
    crate::timestamps::local_datetime(at(event, "createdAt"))
        .map(|time| time.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}
fn agent_duration(record: &Value) -> String {
    let started = at(record, "createdAt");
    let ended = if active(&field(record, "status")) {
        chrono::Utc::now().timestamp_millis()
    } else {
        at(record, "updatedAt").max(started)
    };
    let seconds = ended.saturating_sub(started).max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
/// Elapsed run time of the watched task itself, matching the subagent rows.
fn task_duration(task: &Value) -> String {
    let started = at(task, "createdAt");
    let ended = if active(&field(task, "status")) {
        chrono::Utc::now().timestamp_millis()
    } else {
        at(task, "updatedAt").max(started)
    };
    let seconds = ended.saturating_sub(started).max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A pane-relative slot is shared by every Watch column in the same window.
/// It keeps plans aligned while retaining enough room for their useful detail.
fn watcher_plan_height(window: &Window) -> Pixels {
    (window.viewport_size().height * 0.36).clamp(px(248.), px(380.))
}

#[derive(Debug)]
struct WatcherProjection {
    plan: Option<Value>,
    /// True only while this chat has a plan the agent is still working through.
    /// A finished plan never occupies the plan card; it stays a disclosure below
    /// the task summary so the surface keeps describing work in progress.
    plan_active: bool,
    finished_plan: Option<Value>,
    title: String,
    provider: String,
    model: String,
    duration: String,
    request: Option<String>,
    agents: Vec<Value>,
    running_agents: usize,
    status: String,
    progress: Option<String>,
    latest_tool: Option<String>,
    activity: Vec<Value>,
    attention: usize,
}

impl WatcherProjection {
    fn new(
        snapshot: &Value,
        task_id: &str,
        records: &[Value],
        messages: &[Rc<Value>],
    ) -> Self {
        let task = values(snapshot, "tasks")
            .find(|task| task["id"] == task_id)
            .cloned();
        let status = task
            .as_ref()
            .map(|task| field(task, "status"))
            .unwrap_or_default();
        let user_at = if snapshot.get("nativeMessageContext").is_some() {
            snapshot["nativeMessageContext"][task_id]["latestUserAt"].as_i64()
        } else {
            values(snapshot, "messages")
                .filter(|row| row["taskId"] == task_id && row["role"] == "user")
                .map(|row| at(row, "createdAt"))
                .max()
        };
        let plan = values(snapshot, "workPlans")
            .filter(|plan| plan["taskId"] == task_id)
            .max_by_key(|plan| {
                (
                    plan["status"] == "active",
                    at(plan, "updatedAt"),
                    at(plan, "createdAt"),
                )
            })
            .cloned();
        let plan_active = plan
            .as_ref()
            .is_some_and(|plan| plan["status"] == "active");
        let finished_plan = values(snapshot, "workPlans")
            .filter(|plan| plan["taskId"] == task_id && plan["status"] != "active")
            .max_by_key(|plan| (at(plan, "updatedAt"), at(plan, "createdAt")))
            .cloned();
        // The summary describes what this chat is doing now, so it is the latest
        // request the owner sent. Delegated and steered peer traffic is not it.
        let request = messages
            .iter()
            .filter(|message| {
                field(message, "role") == "user"
                    && field(message, "senderAgentId").is_empty()
                    && !field(message, "text").trim().is_empty()
            })
            .max_by_key(|message| at(message, "createdAt"))
            .map(|message| {
                field(message, "text")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|text| !text.is_empty());
        // Keep the plan's team visible across turns, including results. Always
        // retain still-active children even if they started before this plan.
        let since = plan
            .as_ref()
            .map(|plan| at(plan, "createdAt"))
            .unwrap_or(user_at.unwrap_or_default());
        let mut agents: Vec<_> = records
            .iter()
            .filter(|record| {
                active(&field(record, "status"))
                    || at(record, "updatedAt").max(at(record, "createdAt")) >= since
            })
            .cloned()
            .collect();
        agents.sort_by_key(|record| {
            (
                !active(&field(record, "status")),
                at(record, "createdAt"),
                field(record, "id"),
            )
        });
        let running_agents = agents
            .iter()
            .filter(|record| active(&field(record, "status")))
            .count();
        let latest = |kinds: &[&str]| {
            values(snapshot, "events")
                .filter(|event| {
                    event["taskId"] == task_id
                        && user_at.is_some_and(|start| at(event, "createdAt") >= start)
                        && kinds.contains(&event["kind"].as_str().unwrap_or_default())
                })
                .max_by_key(|event| at(event, "createdAt"))
                .and_then(|event| event["title"].as_str())
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
        };
        let mut activity: Vec<_> = values(snapshot, "events")
            .filter(|event| {
                event["taskId"] == task_id
                    && user_at.is_some_and(|start| at(event, "createdAt") >= start)
            })
            .cloned()
            .collect();
        activity.sort_by_key(|event| std::cmp::Reverse(at(event, "createdAt")));
        activity.truncate(8);
        let attention = values(snapshot, "approvalRequests")
            .filter(|request| request["taskId"] == task_id && request["status"] == "pending")
            .count();
        Self {
            plan,
            plan_active,
            finished_plan,
            title: task
                .as_ref()
                .map(|task| field(task, "title"))
                .unwrap_or_default(),
            provider: task
                .as_ref()
                .map(|task| field(task, "provider"))
                .unwrap_or_default(),
            model: task
                .as_ref()
                .map(|task| field(task, "model"))
                .unwrap_or_default(),
            duration: task.as_ref().map(task_duration).unwrap_or_default(),
            request,
            agents,
            running_agents,
            status,
            progress: latest(&["agent_progress"]),
            latest_tool: latest(&["tool", "computer"]),
            activity,
            attention,
        }
    }
}

impl ChatPane {
    pub(super) fn toggle_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watcher.transitioning {
            return;
        }
        let target = !self.watcher.enabled;
        if !target {
            self.watcher.interject_open = false;
        }
        self.watcher.transitioning = true;
        self.watcher.fade_in = false;
        self.watcher.transition_serial = self.watcher.transition_serial.wrapping_add(1);
        if target {
            window.focus(&self.focus, cx);
            self.slash.update(cx, |slash, cx| {
                slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx);
            });
        } else if self.pending_tool_approval().is_some() {
            self.approval_popup_open = true;
            window.focus(&self.approval_focus, cx);
        } else {
            window.focus(&self.composer.read(cx).focus_handle(cx), cx);
        }
        self.watcher.transition_task = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            if pane
                .update(cx, |pane, cx| {
                    pane.watcher.enabled = target;
                    pane.watcher.fade_in = true;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = pane.update(cx, |pane, cx| {
                pane.watcher.transitioning = false;
                pane.watcher.fade_in = false;
                pane.watcher.transition_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn send_watcher_interjection(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.has_pending_user_question() || self.is_performance_fixture() || self.attachments.read(cx).pending() {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        let attachment_ids = self.attachments.read(cx).ids();
        if text.trim().is_empty() && attachment_ids.is_empty() {
            return;
        }
        let wire_text = chat_slash::literal_prompt(&text);
        let messages = self
            .transcripts
            .get(&self.task_id())
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.watcher.interjection = Some(InterjectionExchange::capture(
            text,
            wire_text,
            &self.snapshot,
            &messages,
        ));
        self.slash.update(cx, |slash, cx| {
            slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx);
        });
        self.send_with_delivery_mode(Some("steer"), false, cx);
        cx.notify();
    }

    /// Read-only summary of the watched task, shown in the plan slot whenever
    /// this chat has no plan in progress. A finished plan stays reachable below
    /// it as a disclosure so nothing already delivered is hidden away.
    fn render_watcher_task_summary(
        &mut self,
        state: &WatcherProjection,
        palette: design::Palette,
        ratio: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = if state.title.trim().is_empty() {
            "Untitled task".to_owned()
        } else {
            state.title.clone()
        };
        let status = status_label(&state.status);
        let mut meta = vec![status.to_owned()];
        if !state.duration.is_empty() {
            meta.push(state.duration.clone());
        }
        if !state.provider.is_empty() {
            meta.push(state.provider.clone());
        }
        if !state.model.is_empty() {
            meta.push(state.model.clone());
        }
        let status_color = if matches!(state.status.as_str(), "error" | "failed" | "blocked") {
            0xe2645a
        } else if active(&state.status) {
            palette.accent
        } else {
            palette.muted
        };
        let request = state.request.clone().unwrap_or_else(|| {
            "No request has been sent in this chat yet.".to_owned()
        });
        let mut surface = div()
            .id("watcher-task-summary")
            .debug_selector(|| "watcher-task-summary".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette.line))
            .bg(design::surface_color(palette.panel))
            .text_color(rgb(palette.ink))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(10. * ratio))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("CURRENT TASK"),
                            )
                            .child(
                                div()
                                    .text_size(px(14. * ratio))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .px(px(7.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .bg(design::surface_color(palette.card))
                            .child(div().size(px(5.)).rounded_full().bg(rgb(status_color)))
                            .child(
                                div()
                                    .text_size(px(10. * ratio))
                                    .text_color(rgb(palette.muted))
                                    .child(status.to_owned()),
                            ),
                    ),
            )
            .child(
                div()
                    .font_family(design::FONT_MONO)
                    .text_size(px(10. * ratio))
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .child(meta.join(" · ")),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(11. * ratio))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("CURRENT REQUEST"),
                    )
                    .child(
                        div()
                            .id("watcher-task-request")
                            .debug_selector(|| "watcher-task-request".into())
                            .min_w_0()
                            .whitespace_normal()
                            .line_clamp(5)
                            .text_size(px(12. * ratio))
                            .text_color(rgb(if state.request.is_some() {
                                palette.ink
                            } else {
                                palette.muted
                            }))
                            .child(request),
                    ),
            );
        if let Some(plan) = state.finished_plan.clone() {
            let expanded = self.watcher.finished_plan_expanded;
            let items = values(&plan, "items").count();
            let done = values(&plan, "items")
                .filter(|item| item["status"] == "completed")
                .count();
            let plan_title = field(&plan, "title");
            let heading = if plan_title.trim().is_empty() {
                "Last plan".to_owned()
            } else {
                format!("Last plan · {plan_title}")
            };
            surface = surface.child(
                div()
                    .id("watcher-last-plan")
                    .debug_selector(|| "watcher-last-plan".into())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pt(px(4.))
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .child(
                        Button::new("watcher-last-plan-toggle")
                            .compact()
                            .xsmall()
                            .ghost()
                            .justify_start()
                            .h(px(22.))
                            .px(px(2.))
                            .icon(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(10.)),
                            )
                            .label(format!("{heading} · {done}/{items} steps"))
                            .debug_selector(|| "watcher-last-plan-toggle".into())
                            .accessibility_label(if expanded {
                                "Hide the last finished work plan"
                            } else {
                                "Show the last finished work plan"
                            })
                            .on_click(cx.listener(|pane, _, _, cx| {
                                pane.watcher.finished_plan_expanded =
                                    !pane.watcher.finished_plan_expanded;
                                cx.notify();
                            })),
                    )
                    .when(expanded, |section| {
                        section.child(
                            div()
                                .id("watcher-last-plan-card")
                                .debug_selector(|| "watcher-last-plan-card".into())
                                .w_full()
                                .min_w_0()
                                .child(crate::plans::render_plan_card(
                                    &plan, palette, ratio, true,
                                )),
                        )
                    }),
            );
        }
        surface.into_any_element()
    }

    fn render_watcher_interject_menu(
        &mut self,
        palette: design::Palette,
        ratio: f32,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = self.task_id();
        let messages = self
            .transcripts
            .get(&task_id)
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let exchange = self.watcher.interjection.clone();
        let projection = exchange
            .as_ref()
            .map(|exchange| exchange.project(&self.snapshot, &task_id, &messages));
        let response = projection.as_ref().and_then(|projection| {
            projection
                .response_id
                .as_ref()
                .zip(projection.response_text.as_ref())
        });
        let response_body = response.map(|(id, text)| {
            self.streaming_bodies
                .get(id)
                .cloned()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| {
                    gpui_kit::component::text::TextView::markdown(
                        SharedString::from(format!("watcher-interject-response-{id}")),
                        text.clone(),
                    )
                    .selectable(true)
                    .scrollable(false)
                    .into_any_element()
                })
        });
        let attachment_count = self.attachments.read(cx).ids().len();
        let draft_empty = self.composer.read(cx).value().trim().is_empty();
        let pending_attachment = self.attachments.read(cx).pending();
        let can_send = !self.busy && !pending_attachment && (!draft_empty || attachment_count > 0);
        let error = self.error.clone();
        let font_family = cx.theme().font_family.clone();
        let danger = cx.theme().danger;
        div()
            .id("watcher-interject-menu")
            .debug_selector(|| "watcher-interject-menu".into())
            .w(width)
            .max_h(px(430.))
            .popover_style(cx)
            .overflow_hidden()
            .font_family(font_family.clone())
            .text_color(rgb(palette.ink))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12. * ratio))
                            .child(
                                Icon::new(IconName::MessageSquare)
                                    .with_size(px(13.))
                                    .text_color(rgb(palette.accent)),
                            )
                            .child("Interject"),
                    )
                    .child(
                        div()
                            .text_size(px(10. * ratio))
                            .text_color(rgb(palette.muted))
                            .child("Watch stays open"),
                    ),
            )
            .when_some(exchange, |surface, exchange| {
                surface.child(
                    div()
                        .id("watcher-interject-exchange")
                        .debug_selector(|| "watcher-interject-exchange".into())
                        .max_h(px(250.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(9.))
                        .px(px(12.))
                        .py(px(11.))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .child(
                                    div()
                                        .max_w(px(310.))
                                        .px(px(10.))
                                        .py(px(8.))
                                        .rounded(px(10.))
                                        .bg(design::surface_color(mix_rgb(
                                            palette.panel,
                                            palette.accent,
                                            if palette.dark { 0.22 } else { 0.12 },
                                        )))
                                        .text_size(px(12. * ratio))
                                        .child(exchange.text),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_start()
                                .child(
                                    div()
                                        .id("watcher-interject-reply")
                                        .debug_selector(|| "watcher-interject-reply".into())
                                        .max_w(px(326.))
                                        .px(px(10.))
                                        .py(px(8.))
                                        .rounded(px(10.))
                                        .border_1()
                                        .border_color(rgb(palette.line))
                                        .bg(design::surface_color(palette.paper))
                                        .text_size(px(12. * ratio))
                                        .when_some(response_body, |bubble, body| {
                                            bubble.child(body)
                                        })
                                        .when(response.is_none(), |bubble| {
                                            bubble.child(
                                                div()
                                                    .text_color(rgb(palette.muted))
                                                    .child(
                                                        projection
                                                            .as_ref()
                                                            .map(|projection| {
                                                                projection.status.clone()
                                                            })
                                                            .unwrap_or_else(|| {
                                                                "Sending steer…".into()
                                                            }),
                                                    ),
                                            )
                                        }),
                                ),
                        )
                        .when(response.is_some(), |exchange| {
                            exchange.child(
                                div()
                                    .pl(px(2.))
                                    .text_size(px(10. * ratio))
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        projection
                                            .as_ref()
                                            .map(|projection| projection.status.clone())
                                            .unwrap_or_default(),
                                    ),
                            )
                        }),
                )
            })
            .when_some(error, |surface, error| {
                surface.child(
                    div()
                        .px(px(12.))
                        .pt(px(9.))
                        .text_size(px(10. * ratio))
                        .text_color(danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .px(px(11.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .child(
                        div()
                            .id("watcher-interject-composer")
                            .debug_selector(|| "watcher-interject-composer".into())
                            .flex()
                            .items_end()
                            .gap(px(8.))
                            .p(px(7.))
                            .rounded(px(9.))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .bg(design::surface_color(palette.card))
                            .child(
                                Textarea::new(&self.composer)
                                    .small()
                                    .appearance(false)
                                    .bordered(false)
                                    .font_family(font_family)
                                    .text_size(px(12. * ratio))
                                    .min_h(px(30.))
                                    .max_h(px(108.))
                                    .flex_1()
                                    .disabled(self.busy),
                            )
                            .child(
                                Button::new("watcher-interject-send")
                                    .debug_selector(|| "watcher-interject-send".into())
                                    .compact()
                                    .outline()
                                    .size(px(30.))
                                    .icon(Icon::new(if self.busy {
                                        IconName::LoaderCircle
                                    } else {
                                        IconName::CornerDownLeft
                                    }))
                                    .accessibility_label(if self.busy {
                                        "Sending steer"
                                    } else {
                                        "Send steer"
                                    })
                                    .tooltip("Send steer")
                                    .disabled(!can_send)
                                    .on_click(cx.listener(|pane, _, _, cx| {
                                        pane.send_watcher_interjection(cx)
                                    })),
                            ),
                    )
                    .when(attachment_count > 0, |footer| {
                        footer.child(
                            div()
                                .pt(px(5.))
                                .text_size(px(10. * ratio))
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{attachment_count} attachment{} included",
                                    if attachment_count == 1 { "" } else { "s" }
                                )),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_watcher_agent_row(
        &self,
        index: usize,
        record: &Value,
        palette: design::Palette,
        ratio: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = field(record, "id");
        let title = field(record, "title");
        let name = field(record, "agentName");
        let status = field(record, "status");
        let activity = field(record, "activity");
        let duration = agent_duration(record);
        let child_id = field(record, "taskId");
        let can_open = !child_id.is_empty()
            && values(&self.snapshot, "tasks").any(|task| task["id"] == child_id);
        let color = if active(&status) {
            palette.accent
        } else if matches!(status.as_str(), "error" | "failed" | "blocked") {
            0xe2645a
        } else {
            palette.muted
        };
        let mut row = div()
            .id(SharedString::from(format!("watcher-agent-{id}")))
            .debug_selector(move || format!("watcher-agent-row-{index}").into())
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(5.))
            .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
            .child(
                div().flex_shrink_0().child(
                    div()
                        .text_size(px(11. * ratio))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(if title.is_empty() { name.clone() } else { title.clone() }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10. * ratio))
                    .text_color(rgb(palette.muted))
                    .child(if activity.is_empty() {
                        status_label(&status).to_owned()
                    } else {
                        activity.clone()
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(design::FONT_MONO)
                    .text_size(px(9. * ratio))
                    .text_color(rgb(palette.muted))
                    .child(duration),
            );
        if can_open {
            let modal_task = child_id.clone();
            row = row.child(
                Button::new(SharedString::from(format!("watcher-open-{id}")))
                    .compact()
                    .xsmall()
                    .ghost()
                    .size(px(18.))
                    .p_0()
                    .icon(Icon::new(IconName::Info).size(px(11.)))
                    .accessibility_label(format!("Show work plan for {title}"))
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.open_peer_workplan(modal_task.clone(), window, cx);
                    })),
            );
        }
        row.into_any_element()
    }

    pub(super) fn render_watcher_toggle(
        &self,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let watching = self.watcher.enabled;
        div()
            .id("chat-watch-mode-toggle")
            .debug_selector(|| "chat-watch-mode-toggle".into())
            .flex()
            .items_center()
            .gap(px(0.))
            .p(px(2.))
            .rounded(px(5.))
            .bg(design::surface_color(palette.panel))
            .border_1()
            .border_color(rgb(palette.line))
            .child(
                Button::new("chat-mode-chat")
                    .compact()
                    .xsmall()
                    .ghost()
                    .px(px(6.))
                    .h(px(20.))
                    .icon(Icon::new(IconName::MessageSquare).size(px(11.)))
                    .label("Chat")
                    .selected(!watching)
                    .accessibility_label("Show chat")
                    .on_click(cx.listener(|pane, _, window, cx| {
                        if pane.watcher.enabled {
                            pane.toggle_watcher(window, cx);
                        }
                    })),
            )
            .child(
                Button::new("chat-watcher-toggle")
                    .compact()
                    .xsmall()
                    .ghost()
                    .px(px(6.))
                    .h(px(20.))
                    .icon(
                        Icon::default()
                            .data(gpui_kit::assets::__private::Eye.1)
                            .with_size(px(11.)),
                    )
                    .label("Watch")
                    .selected(watching)
                    .debug_selector(|| "chat-watcher-toggle".into())
                    .accessibility_label("Show workplan and subagents")
                    .tooltip("Watch workplan, subagents and activity")
                    .on_click(cx.listener(|pane, _, window, cx| {
                        if !pane.watcher.enabled {
                            pane.toggle_watcher(window, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_watcher(
        &mut self,
        palette: design::Palette,
        user_question_form: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let records = self.companion.read(cx).records();
        let messages: Vec<Rc<Value>> = self
            .transcripts
            .get(&self.task_id())
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let state = WatcherProjection::new(&self.snapshot, &self.task_id(), &records, &messages);
        let ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let label = |text: String| {
            div()
                .min_w_0()
                .text_size(px(11. * ratio))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(text)
        };
        let mut body = div()
            .id("watcher-body")
            .debug_selector(|| "watcher-body".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(20.));
        if let Some(error) = self
            .error
            .as_deref()
            .and_then(connection_recovery::visible_error)
        {
            body = body.child(
                div()
                    .text_size(px(12. * ratio))
                    .text_color(rgb(0xe2645a))
                    .child(format!("Connection: {error}")),
            );
        }
        let question_count = self.pending_user_questions().len();
        let permission_count = values(&self.snapshot, "approvalRequests")
            .filter(|request| request["taskId"] == self.task_id()
                && request["status"] == "pending"
                && (request["input"].is_null()
                    || !crate::approval_prompt::mcp_approval_choices(request).is_empty()))
            .count();
        if question_count > 0 {
            body = body.children(user_question_form);
        }
        if permission_count > 0 {
            body = body.child(
                Button::new("watcher-attention")
                    .compact()
                    .small()
                    .w_full()
                    .child(format!(
                        "{} request{} need your attention · Review in chat",
                        permission_count,
                        if permission_count == 1 { "" } else { "s" }
                    ))
                    .debug_selector(|| "watcher-attention".into())
                    .accessibility_label("Review pending requests in chat")
                    .on_click(cx.listener(|pane, _, window, cx| pane.toggle_watcher(window, cx))),
            );
        }
        let mut now = div()
            .id("watcher-now")
            .debug_selector(|| "watcher-now".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .h(px(78.))
            .overflow_hidden();
        let live_line = div()
            .id("watcher-live-line")
            .debug_selector(|| "watcher-live-line".into())
            .w_full()
            .h(px(2.))
            .relative()
            .overflow_hidden()
            .rounded_full()
            .bg(rgb(if state.status == "running" {
                palette.accent
            } else {
                palette.line
            }));
        let live_line = if state.status == "running" && !cx.reduce_motion() {
            live_line
                .with_animation(
                    "watcher-live-line-slide",
                    Animation::new(Duration::from_millis(1800))
                        .repeat_synced()
                        .with_max_fps(30.),
                    move |line, progress| {
                        let clear = rgba(palette.accent << 8);
                        let bright = rgb(palette.accent);
                        line.bg(rgba((palette.accent << 8) | 0x28)).child(
                            div().absolute().top_0().h_full()
                                .left(relative(progress * 1.6 - 0.6))
                                .w(relative(0.6)).flex()
                                .child(div().flex_1().h_full().bg(linear_gradient(
                                    90., linear_color_stop(clear, 0.), linear_color_stop(bright, 1.),
                                )))
                                .child(div().flex_1().h_full().bg(linear_gradient(
                                    90., linear_color_stop(bright, 0.), linear_color_stop(clear, 1.),
                                ))),
                        )
                    },
                )
                .into_any_element()
        } else {
            live_line.into_any_element()
        };
        now = now
            .child(
                div()
                    .id("watcher-latest-update")
                    .debug_selector(|| "watcher-latest-update".into())
                    .min_w_0()
                    .text_size(px(14. * ratio))
                    .font_weight(FontWeight::MEDIUM)
                    .line_height(px(20. * ratio))
                    .line_clamp(3)
                    .child(state.progress.clone().unwrap_or_else(|| {
                        if state.status != "running" {
                            "No active update".into()
                        } else if state.plan_active {
                            "Working through the current plan".into()
                        } else {
                            "Working on the current request".into()
                        }
                    })),
            )
            .child(live_line);
        // Without a plan in progress the surface describes the task itself, so
        // the plan slot never shows an empty or finished plan card over live work.
        let plan_region: AnyElement = if state.plan_active {
            crate::plans::render_plan_popup_filling(&self.snapshot, &self.task_id(), palette, ratio)
        } else {
            self.render_watcher_task_summary(&state, palette, ratio, cx)
        };
        body = body.child(now).child(
            div()
                .id("watcher-plan")
                .debug_selector(|| "watcher-plan".into())
                .w_full()
                .min_w_0()
                .h(watcher_plan_height(window))
                .child(plan_region),
        );
        let finished_agents = state
            .agents
            .iter()
            .filter(|record| !active(&field(record, "status")))
            .collect::<Vec<_>>();
        let active_agents = state
            .agents
            .iter()
            .filter(|record| active(&field(record, "status")))
            .collect::<Vec<_>>();
        let mut team = div()
            .id("watcher-agents")
            .debug_selector(|| "watcher-agents".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label("SUBAGENTS".into()))
                    .child(
                        div()
                            .font_family(design::FONT_MONO)
                            .text_size(px(10. * ratio))
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} running · {} done",
                                state.running_agents, finished_agents.len()
                            )),
                    ),
            );
        let mut agent_cards = div()
            .id("watcher-agent-cards")
            .debug_selector(|| "watcher-agent-cards".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.));
        if state.agents.is_empty() {
            agent_cards = agent_cards.child(
                div()
                    .text_size(px(12. * ratio))
                    .text_color(rgb(palette.muted))
                    .child("No subagents for this work yet."),
            );
        }
        for (index, record) in active_agents.iter().enumerate() {
            agent_cards = agent_cards.child(self.render_watcher_agent_row(
                index, record, palette, ratio, cx,
            ));
        }
        if !finished_agents.is_empty() {
            let expanded = self.watcher.finished_expanded;
            agent_cards = agent_cards.child(
                Button::new("watcher-finished-agents")
                    .debug_selector(|| "watcher-finished-agents".into())
                    .compact()
                    .xsmall()
                    .text()
                    .justify_start()
                    .h(px(22.))
                    .p_0()
                    .opacity(0.5)
                    .label(format!("> {} finished", finished_agents.len()))
                    .accessibility_label(if expanded {
                        "Hide finished subagents"
                    } else {
                        "Show finished subagents"
                    })
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.watcher.finished_expanded = !pane.watcher.finished_expanded;
                        cx.notify();
                    })),
            );
            if expanded {
                for (offset, record) in finished_agents.iter().enumerate() {
                    agent_cards = agent_cards.child(self.render_watcher_agent_row(
                        active_agents.len() + offset,
                        record,
                        palette,
                        ratio,
                        cx,
                    ));
                }
            }
        }
        body = body.child(team.child(agent_cards));
        let mut activity_feed = div()
            .id("watcher-activity")
            .debug_selector(|| "watcher-activity".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(label("ACTIVITY".into())),
            );
        if state.activity.is_empty() {
            activity_feed = activity_feed.child(
                div()
                    .text_size(px(12. * ratio))
                    .text_color(rgb(palette.muted))
                    .child("No activity recorded for this turn yet."),
            );
        }
        for (index, event) in state.activity.iter().enumerate() {
            let kind = field(event, "kind");
            let title = field(event, "title");
            let time = event_time(event);
            activity_feed = activity_feed.child(
                div()
                    .id(("watcher-activity-row", index))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(58.))
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .font_family(design::FONT_MONO)
                            .text_size(px(10. * ratio))
                            .text_color(rgb(palette.muted))
                            .child(time),
                    )
                    .child(
                        div()
                            .w(px(72.))
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .font_family(design::FONT_MONO)
                            .text_size(px(10. * ratio))
                            .text_color(rgb(if matches!(kind.as_str(), "tool" | "computer") {
                                palette.accent
                            } else {
                                palette.muted
                            }))
                            .child(kind.replace('_', " ")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11. * ratio))
                            .text_color(rgb(palette.ink))
                            .truncate()
                            .child(if title.is_empty() {
                                "Activity update".to_owned()
                            } else {
                                title
                            }),
                    ),
            );
        }
        body = body.child(activity_feed);
        let interject_width = (window.viewport_size().width - px(24.)).min(px(390.));
        let interject_open = self.watcher.interject_open && question_count == 0;
        let interject_focus = self.composer.read(cx).focus_handle(cx);
        let owner = cx.weak_entity();
        let content_owner = owner.clone();
        let interject = Popover::new("watcher-interject-popover")
            .appearance(false)
            .anchor(Anchor::BottomRight)
            .offset(px(7.))
            .open(interject_open)
            .overlay_closable(true)
            .track_focus(&interject_focus)
            .trigger(
                Button::new("watcher-back")
                    .disabled(question_count > 0)
                    .compact()
                    .xsmall()
                    .outline()
                    .icon(Icon::new(IconName::MessageSquare))
                    .label("Interject")
                    .debug_selector(|| "watcher-back".into())
                    .accessibility_label("Interject without leaving Watch")
                    .tooltip("Send a steer without leaving Watch"),
            )
            .on_open_change(move |open, _, cx| {
                let _ = owner.update(cx, |pane, cx| {
                    pane.watcher.interject_open = *open;
                    if *open {
                        pane.slash.update(cx, |slash, cx| {
                            slash.handle_action(
                                crate::slash_palette::PaletteAction::Dismiss,
                                cx,
                            );
                        });
                    }
                    cx.notify();
                });
            })
            .content(move |_, _, cx| {
                content_owner
                    .update(cx, |pane, cx| {
                        pane.render_watcher_interject_menu(
                            palette,
                            ratio,
                            interject_width,
                            cx,
                        )
                    })
                    .unwrap_or_else(|_| div().into_any_element())
            });
        body = body.child(
            div()
            .id("watcher-interject-dock")
            .debug_selector(|| "watcher-interject-dock".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_end()
            .child(interject),
        );
        div()
            .id("chat-watcher")
            .debug_selector(|| "chat-watcher".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .overflow_hidden()
            .bg(design::surface_color(mix_rgb(
                palette.paper,
                palette.panel,
                if palette.dark { 0.28 } else { 0.55 },
            )))
            .text_color(rgb(palette.ink))
            .font_family(cx.theme().font_family.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pane, _, _, cx| cx.emit(ChatEvent::Focused(pane.task_id()))),
            )
            .child(crate::transcript_edge_fade::TranscriptEdgeFade::for_scroll(
                div()
                    .id("watcher-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.watcher.scroll)
                    .child(
                        div()
                            .id("watcher-content")
                            .debug_selector(|| "watcher-content".into())
                            .w_full()
                            .flex()
                            .justify_center()
                            .px(px(44.))
                            .py(px(48.))
                            .child(div().w_full().max_w(px(760.)).min_w_0().child(body)),
                    ),
                self.watcher.scroll.clone(),
                px(24.),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn watcher_scopes_plan_team_and_activity_and_keeps_completion() {
        let mut snapshot = json!({"tasks":[{"id":"a","status":"running"}],
            "nativeMessageContext":{"a":{"latestUserAt":100}},
            "workPlans":[{"id":"old","taskId":"a","status":"closed","updatedAt":300,"createdAt":1},
                {"id":"current","taskId":"a","status":"active","updatedAt":150,"createdAt":90},
                {"id":"foreign","taskId":"b","status":"active","updatedAt":400}],
            "events":[{"taskId":"a","kind":"agent_progress","title":"Old update","createdAt":1},
                {"taskId":"a","kind":"agent_progress","title":"Checking tests","createdAt":120},
                {"taskId":"b","kind":"tool","title":"Unrelated tool","createdAt":200},
                {"taskId":"a","kind":"tool","title":"cargo test","createdAt":125}],
            "approvalRequests":[{"taskId":"a","status":"pending"},{"taskId":"b","status":"pending"}]});
        let records = vec![
            json!({"id":"stale","status":"completed","createdAt":1,"updatedAt":80}),
            json!({"id":"live","status":"running","createdAt":1}),
            json!({"id":"done","status":"completed","createdAt":92,"updatedAt":130}),
        ];
        let state = WatcherProjection::new(&snapshot, "a", &records, &[]);
        assert_eq!(state.plan.unwrap()["id"], "current");
        assert!(state.plan_active, "the active plan owns the plan card");
        assert_eq!(state.agents.len(), 2);
        assert_eq!(state.agents[0]["id"], "live");
        assert_eq!(state.running_agents, 1);
        assert_eq!(state.progress.as_deref(), Some("Checking tests"));
        assert_eq!(state.latest_tool.as_deref(), Some("cargo test"));
        assert_eq!(state.attention, 1);
        snapshot["tasks"][0]["status"] = json!("idle");
        snapshot["workPlans"][1]["status"] = json!("closed");
        snapshot["workPlans"][1]["updatedAt"] = json!(400);
        let state = WatcherProjection::new(&snapshot, "a", &records, &[]);
        assert_eq!(state.plan.unwrap()["id"], "current");
        assert_eq!(state.status, "idle");
        assert!(
            !state.plan_active,
            "a finished plan hands the card to the task summary"
        );
    }

    #[::core::prelude::v1::test]
    fn watcher_summary_replaces_the_plan_when_none_is_active() {
        let mut snapshot = json!({"tasks":[{"id":"a","title":"Ship the release","status":"idle",
                "provider":"codex","model":"gpt-5-codex","createdAt":1_000,"updatedAt":4_000}],
            "nativeMessageContext":{"a":{"latestUserAt":100}},
            "workPlans":[{"id":"older","taskId":"a","title":"Older plan","status":"closed",
                "createdAt":1,"updatedAt":300},
                {"id":"latest","taskId":"a","title":"Cutover plan","status":"closed",
                "createdAt":10,"updatedAt":900,"items":[
                    {"id":"one","status":"completed"},{"id":"two","status":"completed"},
                    {"id":"three","status":"pending"}]}],
            "events":[],"approvalRequests":[]});
        let messages = vec![
            Rc::new(json!({"id":"first","taskId":"a","role":"user","text":"Old request",
                "createdAt":10})),
            Rc::new(json!({"id":"second","taskId":"a","role":"user",
                "text":"Finish the cutover and ship the release notes","createdAt":40})),
            Rc::new(json!({"id":"peer","taskId":"a","role":"user","senderAgentId":"builder",
                "text":"Delegated note", "createdAt":60})),
            Rc::new(json!({"id":"answer","taskId":"a","role":"assistant","text":"Done",
                "createdAt":80})),
        ];
        let state = WatcherProjection::new(&snapshot, "a", &[], &messages);
        assert!(
            !state.plan_active,
            "a closed plan never occupies the plan card"
        );
        assert_eq!(
            state.finished_plan.clone().unwrap()["id"],
            "latest",
            "the most recently finished plan stays available"
        );
        assert_eq!(state.title, "Ship the release");
        assert_eq!(state.provider, "codex");
        assert_eq!(state.model, "gpt-5-codex");
        assert_eq!(state.duration, "0:03");
        assert_eq!(
            state.request.as_deref(),
            Some("Finish the cutover and ship the release notes"),
            "the summary reads the owner's latest request, not peer traffic"
        );

        // Republishing the plan as active hands the slot back to the plan card.
        snapshot["workPlans"][1]["status"] = json!("active");
        let active = WatcherProjection::new(&snapshot, "a", &[], &messages);
        assert!(active.plan_active);
        assert_eq!(
            active.finished_plan.unwrap()["id"],
            "older",
            "only non-active plans remain eligible as the last plan"
        );

        // No plan at all still summarises the task, and whitespace collapses.
        snapshot["workPlans"] = json!([]);
        let bare = WatcherProjection::new(&snapshot, "a", &[], &[]);
        assert!(!bare.plan_active);
        assert!(bare.finished_plan.is_none());
        assert!(bare.request.is_none(), "no request means no summary request");
        let wrapped = WatcherProjection::new(
            &snapshot,
            "a",
            &[],
            &[Rc::new(json!({"id":"w","role":"user","text":"line one\n\n  line two  ",
                "createdAt":5}))],
        );
        assert_eq!(wrapped.request.as_deref(), Some("line one line two"));
    }

    #[::core::prelude::v1::test]
    fn watcher_activity_time_includes_seconds() {
        let rendered = event_time(&json!({"createdAt": 1_700_000_000_000_i64}));
        assert_eq!(rendered.matches(':').count(), 2, "{rendered}");
    }

    #[::core::prelude::v1::test]
    fn watcher_interject_correlates_ack_with_new_or_updated_assistant_reply() {
        let baseline = vec![Rc::new(json!({
            "id":"assistant","taskId":"watch","role":"assistant",
            "text":"Working","streamStatus":"streaming"
        }))];
        let snapshot = json!({"queuedMessages":[]});
        let exchange = InterjectionExchange::capture(
            "Check the edge case".into(),
            "Check the edge case".into(),
            &snapshot,
            &baseline,
        );
        let sending = json!({"queuedMessages":[{
            "id":"steer","taskId":"watch","text":"Check the edge case",
            "status":"sending","origin":"steering"
        }]});
        assert_eq!(
            exchange.project(&sending, "watch", &baseline).status,
            "Steering into the current turn…"
        );

        let peer_only = vec![
            baseline[0].clone(),
            Rc::new(json!({
                "id":"peer","taskId":"watch","role":"user",
                "senderAgentId":"peer-agent","text":"Check the edge case"
            })),
        ];
        assert!(
            exchange
                .project(&json!({"queuedMessages":[]}), "watch", &peer_only)
                .response_id
                .is_none(),
            "a peer message with the same text is not the steer acknowledgement"
        );

        let replied = vec![
            Rc::new(json!({
                "id":"assistant","taskId":"watch","role":"assistant",
                "text":"Working\nI checked it.","streamStatus":"streaming"
            })),
            Rc::new(json!({
                "id":"accepted","taskId":"watch","role":"user",
                "text":"Check the edge case"
            })),
        ];
        let projection = exchange.project(&json!({"queuedMessages":[]}), "watch", &replied);
        assert_eq!(projection.response_id.as_deref(), Some("assistant"));
        assert_eq!(
            projection.response_text.as_deref(),
            Some("Working\nI checked it.")
        );
        assert_eq!(projection.status, "Receiving reply…");

        let fallback = json!({"queuedMessages":[{
            "id":"steer","taskId":"watch","text":"Check the edge case",
            "status":"queued","origin":"steering"
        }]});
        assert_eq!(
            exchange.project(&fallback, "watch", &baseline).status,
            "Steer unavailable · queued for the next turn"
        );
    }
}

#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
mod native_tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::{TestAppContext, VisualTestContext};
    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }
    #[gpui_kit::test]
    fn watcher_interject_popover_keeps_the_watch_surface_and_retained_draft(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            let mut snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14},
                "tasks":[{"id":"watch","title":"Long workplan","agentId":"agent","provider":"codex","status":"running","queue":[]}],
                "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                "nativeMessageContext":{"watch":{"latestUserAt":10}},
                "workPlans":[{"id":"plan","taskId":"watch","title":"Ship the change","status":"active","createdAt":10,"updatedAt":10,
                    "items":(0..3).map(|i| json!({"id":format!("step-{i}"),"title":format!("Work item {i}"),"status":"pending"})).collect::<Vec<_>>() }],
                "subagentSessions":[
                    {"id":"child-1","parentTaskId":"watch","agentPath":"architecture","title":"Saved Layout Architecture","status":"running","createdAt":10,"updatedAt":10},
                    {"id":"child-2","parentTaskId":"watch","agentPath":"model","title":"Saved Layout Model","status":"running","createdAt":11,"updatedAt":11},
                    {"id":"child-3","parentTaskId":"watch","agentPath":"preview","title":"Saved Layout Preview","status":"running","createdAt":12,"updatedAt":12},
                    {"id":"child-4","parentTaskId":"watch","agentPath":"state","title":"Watcher State Map","status":"running","createdAt":13,"updatedAt":13}],
                "messages":[],"events":[{"id":"update","taskId":"watch","kind":"agent_progress","title":"Checking the implementation","createdAt":11}],
                "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
            let mut retained = None;
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    ChatPane::performance_fixture(
                        "watch".into(),
                        Rc::new(snapshot.clone()),
                        window,
                        cx,
                    )
                });
                pane.update(cx, |pane, cx| {
                    assert!(pane.restore_draft("Keep this draft", window, cx));
                });
                retained = Some(pane.clone());
                gpui_kit::component::Root::new(pane, window, cx)
            });
            let pane = retained.unwrap();
            for width in [320., 800.] {
                snapshot["tasks"][0]["status"] = json!("running");
                snapshot["workPlans"][0]["status"] = json!("active");
                snapshot["approvalRequests"] = json!([]);
                for session in snapshot["subagentSessions"].as_array_mut().unwrap() {
                    session["status"] = json!("running");
                }
                pane.update(visual, |pane, cx| {
                    pane.watcher.finished_expanded = false;
                    pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx)
                });
                visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(600.)));
                draw(visual);
                let toggle = visual
                    .debug_bounds("chat-watcher-toggle")
                    .expect("header toggle");
                visual.simulate_click(toggle.center(), Default::default());
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                let root = visual
                    .debug_bounds("chat-watcher")
                    .expect("Watcher mounted");
                let header = visual
                    .debug_bounds("chat-pane-header")
                    .expect("mode switch header remains mounted");
                assert!(
                    (root.size.width - gpui_kit::px(width)).abs() <= gpui_kit::px(1.),
                    "Watcher owns the pane body"
                );
                assert!((root.bottom() - gpui_kit::px(600.)).abs() <= gpui_kit::px(1.));
                assert!(root.top() >= header.bottom() - gpui_kit::px(1.));
                assert!(
                    visual.debug_bounds("send").is_none(),
                    "chat composer is hidden in Watch mode"
                );
                assert!(
                    visual.debug_bounds("working-plan-toggle").is_none(),
                    "plan access moved out of status pill"
                );
                assert!(visual.debug_bounds("watcher-now").is_some());
                let update = visual
                    .debug_bounds("watcher-latest-update")
                    .expect("the update remains visible without a heading");
                let line = visual
                    .debug_bounds("watcher-live-line")
                    .expect("the live line remains visible");
                let plan = visual
                    .debug_bounds("watcher-plan")
                    .expect("watch plan slot");
                assert!(
                    line.top() >= update.bottom() - gpui_kit::px(1.),
                    "the pulsing line follows the update: {line:?} after {update:?}"
                );
                assert!(
                    plan.top() >= line.bottom(),
                    "the reserved plan slot starts after the update: {plan:?} after {line:?}"
                );
                assert!(visual.debug_bounds("watcher-live-bar").is_none());
                assert!(visual.debug_bounds("watcher-agents").is_some());
                assert!(visual.debug_bounds("watcher-activity").is_some());
                let body = visual.debug_bounds("watcher-body").expect("inset body");
                assert!(
                    body.left() >= root.left() + gpui_kit::px(43.),
                    "Watch content keeps the added outer inset: {body:?} in {root:?}"
                );
                assert!(visual.debug_bounds("watcher-agent-cards").is_some());
                if width >= 800. {
                    let cards = [
                        visual
                            .debug_bounds("watcher-agent-row-0")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-1")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-2")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-3")
                            .expect("agent card"),
                    ];
                    assert!(cards[1].top() >= cards[0].bottom());
                    assert!(cards[2].top() >= cards[1].bottom());
                    assert!(cards[3].top() >= cards[2].bottom());
                }
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        assert!(!pane.composer_keyboard_available(window, cx));
                    })
                });
                pane.read_with(visual, |pane, cx| {
                    assert_eq!(
                        pane.composer.read(cx).value().to_string(),
                        "Keep this draft"
                    )
                });
                // Completion arrives through the same sync path as a real native publication.
                snapshot["tasks"][0]["status"] = json!("idle");
                snapshot["workPlans"][0]["status"] = json!("closed");
                for session in snapshot["subagentSessions"].as_array_mut().unwrap() {
                    session["status"] = json!("completed");
                }
                pane.update(visual, |pane, cx| {
                    pane.sync(Rc::new(snapshot.clone()), Some("done".into()), cx)
                });
                draw(visual);
                pane.read_with(visual, |pane, cx| {
                    assert!(pane.watcher.enabled, "completion stays visible");
                    let messages: Vec<Rc<Value>> = pane
                        .transcripts
                        .get(&pane.task_id())
                        .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    let state = WatcherProjection::new(
                        &pane.snapshot,
                        "watch",
                        &pane.companion.read(cx).records(),
                        &messages,
                    );
                    assert_eq!(state.status, "idle");
                    assert_eq!(state.plan.unwrap()["status"], "closed");
                    assert!(
                        !state.plan_active,
                        "a closed plan no longer occupies the plan card"
                    );
                    assert_eq!(state.finished_plan.unwrap()["id"], "plan");
                    assert_eq!(state.running_agents, 0);
                });
                let mut finished = visual
                    .debug_bounds("watcher-finished-agents")
                    .expect("completed agents are disclosed by a left-aligned link");
                if finished.bottom() > root.bottom() {
                    visual.simulate_event(gpui_kit::ScrollWheelEvent {
                        position: root.center(),
                        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-600.),
                        )),
                        ..Default::default()
                    });
                    draw(visual);
                    finished = visual
                        .debug_bounds("watcher-finished-agents")
                        .expect("completed-agent disclosure stays mounted after scrolling");
                }
                visual.simulate_click(finished.center(), Default::default());
                draw(visual);
                let first_finished = visual
                    .debug_bounds("watcher-agent-row-0")
                    .or_else(|| {
                        visual.simulate_event(gpui_kit::ScrollWheelEvent {
                            position: finished.center(),
                            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                gpui_kit::px(0.),
                                gpui_kit::px(-600.),
                            )),
                            ..Default::default()
                        });
                        draw(visual);
                        visual.debug_bounds("watcher-agent-row-0")
                    })
                    .expect("finished agents render after their disclosure");
                assert!(
                    first_finished.top() >= finished.bottom() - gpui_kit::px(1.),
                    "finished rows stay below their disclosure: {first_finished:?} after {finished:?}"
                );
                let mut back = visual.debug_bounds("watcher-back").unwrap();
                if back.bottom() > root.bottom() {
                    visual.simulate_event(gpui_kit::ScrollWheelEvent {
                        position: root.center(),
                        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-1_200.),
                        )),
                        ..Default::default()
                    });
                    draw(visual);
                    back = visual
                        .debug_bounds("watcher-back")
                        .expect("Interject trigger stays mounted after scrolling");
                }
                visual.simulate_click(back.center(), Default::default());
                draw(visual);
                let menu = visual
                    .debug_bounds("watcher-interject-menu")
                    .expect("anchored interject menu");
                let composer = visual
                    .debug_bounds("watcher-interject-composer")
                    .expect("interject composer");
                assert!(visual.debug_bounds("chat-watcher").is_some());
                assert!(menu.bottom() <= back.top() + gpui_kit::px(1.));
                assert!(composer.left() >= menu.left() && composer.right() <= menu.right());
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        assert!(pane.watcher.enabled);
                        assert!(pane.watcher.interject_open);
                        assert!(pane.composer_keyboard_available(window, cx));
                    })
                });
                pane.read_with(visual, |pane, cx| {
                    assert_eq!(
                        pane.composer.read(cx).value().to_string(),
                        "Keep this draft"
                    )
                });
                visual.simulate_keystrokes("escape");
                draw(visual);
                assert!(visual.debug_bounds("watcher-interject-menu").is_none());
                assert!(
                    visual.debug_bounds("chat-watcher").is_some(),
                    "first Escape dismisses only the interject menu"
                );
                visual.simulate_keystrokes("escape");
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                assert!(
                    visual.debug_bounds("chat-watcher").is_none(),
                    "Escape returns to chat"
                );
                snapshot["approvalRequests"] = json!([{"id":"request","taskId":"watch","status":"pending","title":"Run tests","createdAt":20}]);
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.sync(Rc::new(snapshot.clone()), Some("request".into()), cx);
                        pane.toggle_watcher(window, cx);
                    })
                });
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                assert!(
                    visual.debug_bounds("watcher-attention").is_some(),
                    "pending request stays visible"
                );
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.review_notification_request("request".into(), window, cx);
                        assert!(pane.approval_popup_open);
                    })
                });
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                pane.read_with(visual, |pane, _| {
                    assert!(!pane.watcher.enabled, "notification must reveal request UI");
                });
                snapshot["approvalRequests"] = json!([]);
                pane.update(visual, |pane, cx| {
                    pane.sync(Rc::new(snapshot.clone()), Some("clear".into()), cx)
                });
                draw(visual);
            }
        }
    }

    #[gpui_kit::test]
    fn watcher_task_summary_stands_in_for_the_plan_when_none_is_active(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            let mut snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14},
                "tasks":[{"id":"watch","title":"Ship the release","agentId":"agent","provider":"codex",
                    "model":"gpt-5-codex","status":"running","createdAt":10,"updatedAt":20,"queue":[]}],
                "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                "nativeMessageContext":{"watch":{"latestUserAt":10}},
                "workPlans":[{"id":"plan","taskId":"watch","title":"Cutover plan","status":"closed",
                    "createdAt":10,"updatedAt":10,"items":[
                        {"id":"one","title":"Land the migration","status":"completed"},
                        {"id":"two","title":"Publish the release notes","status":"pending"}]}],
                "subagentSessions":[],
                "messages":[{"id":"request","taskId":"watch","role":"user",
                    "text":"Finish the cutover and ship the release notes","createdAt":12,
                    "streamStatus":"complete"}],
                "events":[{"id":"update","taskId":"watch","kind":"agent_progress",
                    "title":"Checking the implementation","createdAt":11}],
                "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
            let mut retained = None;
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    ChatPane::performance_fixture(
                        "watch".into(),
                        Rc::new(snapshot.clone()),
                        window,
                        cx,
                    )
                });
                pane.update(cx, |pane, cx| {
                    pane.reconcile_history(
                        snapshot["messages"].as_array().unwrap().clone(),
                        true,
                        cx,
                    )
                });
                retained = Some(pane.clone());
                gpui_kit::component::Root::new(pane, window, cx)
            });
            let pane = retained.unwrap();
            visual.simulate_resize(size(gpui_kit::px(900.), gpui_kit::px(1000.)));
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx)
            });
            draw(visual);
            let toggle = visual
                .debug_bounds("chat-watcher-toggle")
                .expect("header toggle");
            visual.simulate_click(toggle.center(), Default::default());
            visual
                .executor()
                .advance_clock(std::time::Duration::from_millis(601));
            draw(visual);

            let plan_slot = visual.debug_bounds("watcher-plan").expect("plan slot");
            let summary = visual
                .debug_bounds("watcher-task-summary")
                .expect("the task summary stands in for the plan card");
            assert!(
                summary.top() >= plan_slot.top() && summary.bottom() <= plan_slot.bottom(),
                "the summary occupies the plan slot: {summary:?} in {plan_slot:?}"
            );
            let request = visual
                .debug_bounds("watcher-task-request")
                .expect("the current request is summarised");
            let disclosure = visual
                .debug_bounds("watcher-last-plan-toggle")
                .expect("the finished plan stays reachable");
            assert!(
                disclosure.top() >= request.bottom() - gpui_kit::px(1.)
                    && disclosure.bottom() <= summary.bottom() + gpui_kit::px(1.),
                "the finished plan sits below the request and inside the summary: \
                 {disclosure:?} in {request:?} in {summary:?}"
            );
            assert!(
                visual.debug_bounds("watcher-last-plan-card").is_none(),
                "the finished plan starts collapsed"
            );

            visual.simulate_click(disclosure.center(), Default::default());
            draw(visual);
            let card = visual
                .debug_bounds("watcher-last-plan-card")
                .expect("the finished plan opens in place");
            assert!(
                card.top() >= disclosure.top(),
                "the opened plan renders below its disclosure: {card:?} in {disclosure:?}"
            );
            pane.read_with(visual, |pane, _| {
                assert!(pane.watcher.finished_plan_expanded);
                assert!(pane.watcher.enabled, "Watch stays mounted");
            });

            // Reopening the plan hands the slot back to the plan card.
            snapshot["workPlans"][0]["status"] = json!("active");
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("reopened".into()), cx)
            });
            draw(visual);
            assert!(visual.debug_bounds("watcher-plan").is_some());
            assert!(
                visual.debug_bounds("watcher-task-summary").is_none(),
                "an active plan owns the card again"
            );
            assert!(visual.debug_bounds("watcher-last-plan-toggle").is_none());
            snapshot["workPlans"][0]["status"] = json!("closed");
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("closed".into()), cx)
            });
            draw(visual);
            assert!(
                visual.debug_bounds("watcher-task-summary").is_some(),
                "closing the plan restores the summary"
            );
            assert!(
                visual.debug_bounds("watcher-last-plan-card").is_some(),
                "the disclosure keeps the state the watcher already reached"
            );
        }
    }
}
