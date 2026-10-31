//! Read-only, task-scoped Codex observations. No provider actions originate here.
use crate::design::{Palette, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::{
    AnyElement, Context, Pixels, UniformListScrollHandle, Window, div, prelude::*, rgb,
    uniform_list,
};
#[cfg(test)]
use gpui_kit::{Entity, FocusHandle, Render, TestAppContext};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::HashSet, rc::Rc, time::Duration};

const PAGE_LIMIT: usize = 100;
const MAX_EVENTS: usize = 10_000;

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn metadata(event: &Value) -> Option<Value> {
    if text(event, "kind") != "codexObservation" {
        return None;
    }
    let detail = match &event["detail"] {
        Value::String(detail) => serde_json::from_str(detail).ok()?,
        Value::Object(_) => event["detail"].clone(),
        _ => return None,
    };
    (detail["schemaVersion"].as_u64() == Some(1)
        && detail["source"] == "codexAppServer"
        && detail["eventType"].as_str().is_some_and(|s| !s.is_empty()))
    .then_some(detail)
}
fn merge(existing: &[Value], incoming: &[Value], task: &str) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut events: Vec<_> = incoming
        .iter()
        .chain(existing)
        .filter(|event| {
            text(event, "taskId") == task
                && !text(event, "id").is_empty()
                && event["createdAt"].as_i64().is_some()
                && metadata(event).is_some()
                && seen.insert(text(event, "id"))
        })
        .cloned()
        .collect();
    events.sort_by(|a, b| {
        b["createdAt"]
            .as_i64()
            .cmp(&a["createdAt"].as_i64())
            .then_with(|| text(b, "id").cmp(&text(a, "id")))
    });
    events.truncate(MAX_EVENTS);
    events
}
fn title(detail: &Value) -> String {
    let kind = text(detail, "eventType");
    let label = match kind.as_str() {
        "turnStarted" => "Turn started",
        "turnCompleted" => "Turn completed",
        "itemStarted" => "Item started",
        "itemCompleted" => "Item completed",
        "inputRequested" => "Input requested",
        "approvalRequested" => "Approval requested",
        _ => &kind,
    };
    let item = readable(&text(detail, "itemType"));
    if item.is_empty() {
        label.to_owned()
    } else {
        format!("{label} · {item}")
    }
}
/// Turn protocol camelCase identifiers into compact readable labels.
fn readable(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_uppercase() && !result.is_empty() {
            result.push(' ');
        }
        result.extend(ch.to_lowercase());
    }
    result
}
/// A non-overlapping head cannot be joined to retained history: its missing
/// middle must be recovered through the new head's pagination cursor.
fn reconcile_page(
    existing: &[Value],
    incoming: &[Value],
    task: &str,
    before: Option<i64>,
    previous_cursor: Option<i64>,
    next_cursor: Option<i64>,
    loaded_older: bool,
) -> (Vec<Value>, Option<i64>, bool) {
    let incoming = merge(&[], incoming, task);
    let overlaps = incoming.iter().any(|new| {
        existing
            .iter()
            .any(|old| text(new, "id") == text(old, "id"))
    });
    let reset = before.is_none() && !overlaps;
    let events = merge(if reset { &[] } else { existing }, &incoming, task);
    let cursor = if before.is_none() && loaded_older && !reset {
        previous_cursor
    } else {
        next_cursor
    };
    (
        events,
        cursor,
        if reset {
            false
        } else {
            loaded_older || before.is_some()
        },
    )
}
fn font(snapshot: &Value, size: f32) -> Pixels {
    let scale = snapshot["settings"]["interfaceFontSize"]
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(14.)
        .clamp(8., 32.) as f32
        / 14.;
    px(size * scale)
}

pub struct PaneEvents {
    service: DesktopService,
    snapshot: Rc<Value>,
    task_id: String,
    generation: u64,
    request: u64,
    visibility_generation: u64,
    visible: bool,
    loading: bool,
    loaded: bool,
    error: Option<String>,
    pending_before: Option<i64>,
    next_before: Option<i64>,
    loaded_older: bool,
    events: Rc<Vec<Value>>,
    selected: Option<String>,
    scroll: UniformListScrollHandle,
}
impl PaneEvents {
    pub fn new(
        service: DesktopService,
        snapshot: Rc<Value>,
        task_id: &str,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            service,
            snapshot,
            task_id: task_id.into(),
            generation: 0,
            request: 0,
            visibility_generation: 0,
            visible: false,
            loading: false,
            loaded: false,
            error: None,
            pending_before: None,
            next_before: None,
            loaded_older: false,
            events: Rc::new(Vec::new()),
            selected: None,
            scroll: UniformListScrollHandle::new(),
        }
    }
    pub fn set_context(&mut self, snapshot: Rc<Value>, task_id: &str, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        if self.task_id != task_id {
            self.task_id = task_id.into();
            self.generation = self.generation.wrapping_add(1);
            self.request = self.request.wrapping_add(1);
            self.events = Rc::new(Vec::new());
            self.selected = None;
            self.loaded = false;
            self.loading = false;
            self.error = None;
            self.next_before = None;
            self.pending_before = None;
            self.loaded_older = false;
            self.scroll = UniformListScrollHandle::new();
            if self.visible {
                self.start_page(None, cx);
            }
            cx.notify();
        }
    }
    /// Parent must call this when the sidebar, tab, or dock visibility changes.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.visibility_generation = self.visibility_generation.wrapping_add(1);
        if !visible {
            return;
        }
        self.start_page(None, cx);
        let token = self.visibility_generation;
        cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep = pane
                    .update(cx, |pane, cx| {
                        if !pane.visible || pane.visibility_generation != token {
                            return false;
                        }
                        // Errors remain visible until an explicit retry.
                        if pane.error.is_none() {
                            pane.start_page(None, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    fn start_page(&mut self, before: Option<i64>, cx: &mut Context<Self>) {
        if self.loading || self.task_id.is_empty() {
            return;
        }
        self.loading = true;
        self.error = None;
        self.pending_before = before;
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let generation = self.generation;
        let task = self.task_id.clone();
        let worker_task = task.clone();
        let service = self.service.clone();
        let (sender, receiver) = oneshot::channel();
        let started = std::thread::Builder::new()
            .name("monitter-captured-events".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut args = json!({"taskId":worker_task,"limit":PAGE_LIMIT});
                if let Some(before) = before {
                    args["before"] = json!(before);
                }
                let _ = sender.send(service.invoke("get_task_captured_events", args));
            });
        if let Err(error) = started {
            self.loading = false;
            self.error = Some(format!("Could not start Events reader: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Events reader ended without a result".into()));
            let _ = pane.update(cx, |pane, cx| {
                if pane.task_id != task || pane.generation != generation || pane.request != request
                {
                    return;
                }
                pane.loading = false;
                match result {
                    Ok(page) if page["events"].is_array() => {
                        let incoming: Vec<_> = page["events"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .take(PAGE_LIMIT)
                            .cloned()
                            .collect();
                        let (events, cursor, loaded_older) = reconcile_page(
                            &pane.events,
                            &incoming,
                            &task,
                            before,
                            pane.next_before,
                            page["nextBefore"].as_i64(),
                            pane.loaded_older,
                        );
                        pane.events = Rc::new(events);
                        pane.next_before = cursor;
                        pane.loaded_older = loaded_older;
                        pane.loaded = true;
                    }
                    Ok(_) => pane.error = Some("Events reader returned an invalid page".into()),
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn render_body(
        &mut self,
        palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = div()
            .id("captured-events")
            .w_full()
            .flex_1()
            .px(px(14.))
            .py(px(10.))
            .min_h_0()
            .flex()
            .flex_col()
            .text_size(font(&self.snapshot, 12.))
            .text_color(rgb(palette.ink))
            .gap(px(8.));
        #[cfg(test)]
        {
            body = body.debug_selector(|| "captured-events-body".into());
        }
        body = body.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(format!("{} captured events", self.events.len()))
                .child(
                    div()
                        .id("refresh-captured-events")
                        .cursor_pointer()
                        .text_color(rgb(palette.accent))
                        .child(if self.loading {
                            "Loading…"
                        } else {
                            "Refresh"
                        })
                        .on_click(cx.listener(|pane, _, _, cx| pane.start_page(None, cx))),
                ),
        );
        if let Some(error) = &self.error {
            body = body
                .child(div().text_color(rgb(palette.muted)).child(error.clone()))
                .child(
                    div()
                        .id("retry-captured-events")
                        .cursor_pointer()
                        .text_color(rgb(palette.accent))
                        .child("Retry")
                        .on_click(
                            cx.listener(|pane, _, _, cx| pane.start_page(pane.pending_before, cx)),
                        ),
                );
        }
        if self.events.is_empty() && self.error.is_none() {
            body = body.child(div().text_color(rgb(palette.muted)).child(if self.loading {
                "Loading captured events…"
            } else if self.task_id.is_empty() {
                "Select a task to inspect captured events."
            } else if self.loaded {
                "No captured Codex events yet."
            } else {
                "Loading captured events…"
            }));
        } else {
            let events = self.events.clone();
            let selected = self.selected.clone();
            let weak = cx.weak_entity();
            let row_height = font(&self.snapshot, 12.) * 3.8;
            let list = uniform_list("captured-event-list", events.len(), move |range, _, _| {
                range
                    .map(|index| {
                        let event = &events[index];
                        let detail = metadata(event).unwrap_or(Value::Null);
                        let id = text(event, "id");
                        let weak = weak.clone();
                        let time = crate::timestamps::local_datetime(
                            event["createdAt"].as_i64().unwrap_or_default(),
                        )
                        .map(|d| d.format("%H:%M:%S").to_string())
                        .unwrap_or_default();
                        let outcome = text(&detail, "outcome");
                        div()
                            .id(index)
                            .h(row_height)
                            .w_full()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .border_b_1()
                            .border_color(rgb(palette.line))
                            .px(px(6.))
                            .cursor_pointer()
                            .when(selected.as_ref() == Some(&id), |row| {
                                row.bg(rgb(palette.soft))
                            })
                            .child(div().truncate().child(title(&detail)))
                            .child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .child(format!("{time}  {outcome}")),
                            )
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |pane, cx| {
                                    pane.selected = if pane.selected.as_ref() == Some(&id) {
                                        None
                                    } else {
                                        Some(id.clone())
                                    };
                                    cx.notify();
                                });
                            })
                            .into_any_element()
                    })
                    .collect()
            })
            .w_full()
            .h_full()
            .track_scroll(&self.scroll);
            let viewport = div()
                .id("captured-events-viewport")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(list);
            #[cfg(test)]
            let viewport = viewport.debug_selector(|| "captured-events-viewport".into());
            body = body.child(viewport);
        }
        if let Some(event) = self
            .events
            .iter()
            .find(|e| self.selected.as_ref() == Some(&text(e, "id")))
        {
            let detail = metadata(event).unwrap_or(Value::Null);
            let mut expanded = div()
                .id("captured-event-detail")
                .max_h(px(180.))
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .overflow_y_scroll()
                .border_t_1()
                .border_color(rgb(palette.line))
                .pt(px(6.))
                .flex()
                .flex_col()
                .gap(px(4.));
            #[cfg(test)]
            let mut expanded = expanded.debug_selector(|| "captured-event-detail".into());
            for key in [
                "eventType",
                "threadId",
                "turnId",
                "itemId",
                "requestId",
                "itemType",
                "outcome",
                "exitCode",
                "changeCount",
            ] {
                if let Some(value) = detail.get(key).filter(|v| !v.is_null()) {
                    let value = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    expanded = expanded.child(
                        div()
                            .flex_none()
                            .whitespace_normal()
                            .child(format!("{key}: {value}")),
                    );
                }
            }
            body = body.child(expanded);
        }
        if self.next_before.is_some() && self.events.len() < MAX_EVENTS {
            body = body.child(
                div()
                    .id("older-captured-events")
                    .cursor_pointer()
                    .text_color(rgb(palette.accent))
                    .child("Load older")
                    .on_click(cx.listener(|pane, _, _, cx| pane.start_page(pane.next_before, cx))),
            );
        } else if self.events.len() >= MAX_EVENTS {
            body = body.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Showing the newest 10,000 captured events."),
            );
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str, task: &str, time: i64) -> Value {
        json!({"id":id,"taskId":task,"createdAt":time,"kind":"codexObservation",
            "detail":json!({"schemaVersion":1,"source":"codexAppServer","eventType":"turnStarted"}).to_string()})
    }
    #[::core::prelude::v1::test]
    fn captured_event_merge_deduplicates_and_orders_pages() {
        let events = merge(
            &[event("a", "t", 1)],
            &[event("b", "t", 2), event("a", "t", 1)],
            "t",
        );
        assert_eq!(
            events.iter().map(|v| text(v, "id")).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }
    #[::core::prelude::v1::test]
    fn captured_event_merge_enforces_task_isolation() {
        assert!(merge(&[event("a", "old", 1)], &[event("b", "other", 2)], "new").is_empty());
    }
    #[::core::prelude::v1::test]
    fn captured_event_invalid_metadata_is_not_rendered() {
        let mut invalid = event("a", "t", 1);
        invalid["detail"] = json!("{broken");
        let mut wrong_schema = event("b", "t", 2);
        wrong_schema["detail"] =
            json!({"schemaVersion":2,"source":"codexAppServer","eventType":"turnStarted"});
        assert!(merge(&[], &[invalid, wrong_schema], "t").is_empty());
    }
    #[::core::prelude::v1::test]
    fn captured_event_head_gap_resets_history_and_cursor() {
        let old = vec![event("a", "t", 1), event("b", "t", 2)];
        let head = vec![event("z", "t", 200)];
        let (events, cursor, older) =
            reconcile_page(&old, &head, "t", None, Some(1), Some(200), true);
        assert_eq!(events.len(), 1);
        assert_eq!(text(&events[0], "id"), "z");
        assert_eq!(cursor, Some(200));
        assert!(!older);
        let (events, cursor, older) = reconcile_page(
            &old,
            &[event("b", "t", 2), event("c", "t", 3)],
            "t",
            None,
            Some(1),
            Some(2),
            true,
        );
        assert_eq!(events.len(), 3);
        assert_eq!(cursor, Some(1));
        assert!(older);
    }
    #[::core::prelude::v1::test]
    fn captured_event_friendly_labels_match_protocol() {
        assert_eq!(
            title(&json!({"eventType":"itemCompleted","itemType":"commandExecution"})),
            "Item completed · command execution"
        );
        assert_eq!(
            title(&json!({"eventType":"approvalRequested"})),
            "Approval requested"
        );
    }
    struct BoundedEvents {
        pane: Entity<PaneEvents>,
        focus: [FocusHandle; 5],
        motion: Rc<std::cell::RefCell<crate::sidebar_tabs_motion::SidebarTabsMotion>>,
    }
    impl Render for BoundedEvents {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette = crate::design::palette(&Value::Null, window.appearance());
            let body = self
                .pane
                .update(cx, |pane, cx| pane.render_body(palette, window, cx));
            let frame = crate::companion_sidebar::render_detail_frame(
                "events-fixture",
                crate::companion_sidebar::DetailTab::Events,
                None,
                body,
                palette,
                &json!({}),
                &self.focus,
                self.motion.clone(),
                window,
                cx,
                |_, _, _| {},
            );
            div()
                .w(gpui_kit::px(300.))
                .h(gpui_kit::px(420.))
                .child(frame)
        }
    }
    #[gpui_kit::test]
    fn captured_event_native_list_has_bounded_viewport(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane =
                cx.new(|cx| {
                    let mut pane = PaneEvents::new(
                        DesktopService::disconnected_for_ui_fixture(),
                        Rc::new(json!({})),
                        "t",
                        cx,
                    );
                    pane.events = Rc::new(
                        (0..100)
                            .map(|n| event(&format!("event-{n}"), "t", n))
                            .collect(),
                    );
                    let mut events = (*pane.events).clone();
                    events[0]["detail"] = json!({
                    "schemaVersion":1,"source":"codexAppServer","eventType":"itemCompleted",
                    "threadId":"long-thread-id-".repeat(12),"turnId":"long-turn-id-".repeat(12),
                    "itemId":"long-item-id-".repeat(12),"requestId":"long-request-id-".repeat(12),
                    "itemType":"commandExecution","outcome":"completed","exitCode":0,"changeCount":4
                }).to_string().into();
                    pane.events = Rc::new(events);
                    pane.loaded = true;
                    pane.selected = Some("event-0".into());
                    pane
                });
            let view = cx.new(|cx| BoundedEvents {
                pane,
                focus: std::array::from_fn(|_| cx.focus_handle()),
                motion: Default::default(),
            });
            gpui_kit::component::Root::new(view, window, cx)
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let viewport = visual
            .debug_bounds("captured-events-viewport")
            .expect("virtual list viewport");
        let details = visual
            .debug_bounds("captured-event-detail")
            .expect("selected metadata");
        let body = visual
            .debug_bounds("captured-events-body")
            .expect("Events flex body");
        let diagnostic = format!("body={body:?}, viewport={viewport:?}, detail={details:?}");
        assert!(viewport.size.height.as_f32() > 0., "{diagnostic}");
        assert!(viewport.size.height.as_f32() < 420., "{diagnostic}");
        assert!(viewport.right().as_f32() <= 300., "{diagnostic}");
        assert!(details.bottom().as_f32() <= 420., "{diagnostic}");
        assert!(details.origin.y >= viewport.bottom(), "{diagnostic}");
    }
}
