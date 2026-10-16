//! Read-only provider allowance, run context, and process metrics views.
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{
    component::{ActiveTheme as _, Disableable, Icon, Sizable, button::*},
    assets::IconName,
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};

const USAGE_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn number(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
fn format_amount(value: f64, unit: &str) -> String {
    let amount = format_number(value);
    match unit {
        "usd" => format!("${amount}"),
        "credits" => format!("{amount} credits"),
        "requests" => format!("{amount} requests"),
        "tokens" => format!("{amount} tokens"),
        _ => amount,
    }
}
fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{} MB", (bytes / (1024.0 * 1024.0)).round() as u64)
    }
}
fn format_duration_ms(ms: f64) -> String {
    let seconds = ms / 1000.0;
    if seconds >= 3600.0 {
        format!("{:.1} h", seconds / 3600.0)
    } else if seconds >= 60.0 {
        format!("{:.1} min", seconds / 60.0)
    } else {
        format!("{seconds:.1} s")
    }
}
fn date(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_i64)
        .map(crate::timestamps::datetime_label)
        .unwrap_or_else(|| "not reported".into())
}
fn label(value: &Value, key: &str, fallback: &str) -> String {
    let result = text(value, key);
    if result.trim().is_empty() {
        fallback.into()
    } else {
        result
    }
}

fn start_usage_overview_worker(
    service: DesktopService,
    policy: &'static str,
) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
    let (sender, receiver) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("monitter-usage-overview".into())
        .stack_size(USAGE_WORKER_STACK_SIZE)
        .spawn(move || {
            let _ = sender.send(service.invoke("get_usage_overview", json!({"policy": policy})));
        })
        .map_err(|error| format!("Could not start usage overview worker: {error}"))?;
    drop(worker);
    Ok(receiver)
}

pub struct UsagePane {
    service: DesktopService,
    snapshot: Value,
    overview: Option<Value>,
    metrics: Option<Value>,
    usage_busy: bool,
    metrics_busy: bool,
    usage_error: Option<String>,
    metrics_error: Option<String>,
}

impl UsagePane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self {
            service,
            snapshot: json!({"settings": snapshot["settings"]}),
            overview: None,
            metrics: None,
            usage_busy: false,
            metrics_busy: false,
            usage_error: None,
            metrics_error: None,
        };
        pane.refresh_usage("if-stale", cx);
        pane.refresh_metrics(cx);
        pane
    }

    pub fn set_snapshot(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        self.snapshot = json!({"settings": snapshot["settings"]});
        cx.notify();
    }

    fn refresh_usage(&mut self, policy: &'static str, cx: &mut Context<Self>) {
        if self.usage_busy {
            return;
        }
        self.usage_busy = true;
        self.usage_error = None;
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            match start_usage_overview_worker(service, policy) {
                Ok(receiver) => receiver.await.unwrap_or_else(|_| {
                    Err("Usage overview worker exited without returning a result.".into())
                }),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.usage_busy = false;
                match result {
                    Ok(value) => pane.overview = Some(value),
                    Err(error) => pane.usage_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_metrics(&mut self, cx: &mut Context<Self>) {
        if self.metrics_busy {
            return;
        }
        self.metrics_busy = true;
        self.metrics_error = None;
        let service = self.service.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { service.invoke("get_process_metrics", json!({})).and_then(crate::gui_process_metrics::include_gui) });
        cx.spawn(async move |pane, cx| {
            let result = worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.metrics_busy = false;
                match result {
                    Ok(value) => pane.metrics = Some(value),
                    Err(error) => pane.metrics_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_context(&self) -> AnyElement {
        let tasks = rows(&self.snapshot, "tasks");
        let messages = rows(&self.snapshot, "messages");
        let runs = self
            .overview
            .as_ref()
            .map(|value| rows(value, "recentRuns"))
            .unwrap_or_default();
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Task context windows");
        if tasks.is_empty() {
            section = section.child("No tasks are available.");
        }
        for task in tasks.iter().rev().take(24) {
            let task_id = text(task, "id");
            let model = text(task, "model");
            let cleared_at = self.snapshot["nativeMessageContext"][&task_id]["contextClearedAt"].as_i64().or_else(|| messages
                .iter()
                .filter(|message| {
                    text(message, "taskId") == task_id
                        && text(message, "role") == "system"
                        && text(message, "text") == "Context Cleared"
                })
                .filter_map(|message| message.get("createdAt").and_then(Value::as_i64))
                .max());
            let run = runs
                .iter()
                .filter(|run| {
                    text(run, "taskId") == task_id
                        && text(run, "configuredModel").trim() == model.trim()
                        && run
                            .get("context")
                            .is_some_and(|context| context.is_object())
                        && run["context"]["used"].as_f64().is_some()
                        && run["context"]["size"].as_f64().is_some()
                        && cleared_at
                            .is_none_or(|at| run["startedAt"].as_i64().unwrap_or_default() >= at)
                })
                .max_by_key(|run| run["startedAt"].as_i64().unwrap_or_default());
            let line = if let Some(run) = run {
                let used = number(&run["context"], "used").unwrap_or_default();
                let size = number(&run["context"], "size").unwrap_or_default();
                let percent = if size > 0.0 {
                    Some((used / size * 100.0).clamp(0.0, 100.0))
                } else {
                    None
                };
                format!(
                    "{} / {} tokens · {}% · {}",
                    format_number(used),
                    format_number(size),
                    percent
                        .map(|n| format_number(n))
                        .unwrap_or_else(|| "—".into()),
                    label(run, "configuredModel", "Model not reported")
                )
            } else if cleared_at.is_some() {
                "Context unavailable after clear; waiting for a new run report.".into()
            } else {
                "Context window not reported by this task's model.".into()
            };
            section = section.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(crate::design::surface_color(0x192333))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(task, "title", "Untitled task"))
                    .child(div().text_sm().child(line)),
            );
        }
        section.into_any_element()
    }

    fn render_provider_source(
        &self,
        source: &Value,
        palette: crate::design::Palette,
    ) -> AnyElement {
        let state = text(source, "state");
        let stale = source["staleAfter"]
            .as_i64()
            .is_some_and(|at| at < chrono_now_ms());
        let mut card = div()
            .p_3()
            .rounded_md()
            .bg(crate::design::surface_color(palette.panel))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(format!(
                        "{} · {}",
                        label(source, "provider", "Provider"),
                        label(source, "accountLabel", "account")
                    ))
                    .child(if stale {
                        "stale".to_owned()
                    } else {
                        state.clone()
                    }),
            );
        if let Some(plan) = source
            .get("planType")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            card = card.child(format!("Plan: {plan}"));
        }
        card = card.child(format!(
            "Source: {} · fetched {}",
            label(source, "source", "unknown"),
            date(source, "fetchedAt")
        ));
        if let Some(error) = source
            .get("error")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            card = card.child(div().text_color(rgb(0xf87171)).child(error.to_owned()));
        }
        if rows(source, "windows").is_empty() && rows(source, "balances").is_empty() {
            card = card.child("No allowance values reported.");
        }
        for window in rows(source, "windows") {
            let used = number(&window, "used");
            let limit = number(&window, "limit");
            let percent = number(&window, "usedPercent");
            let values = match (used, limit) {
                (Some(used), Some(limit)) => format!(
                    "{} / {}",
                    format_amount(used, &text(&window, "unit")),
                    format_amount(limit, &text(&window, "unit"))
                ),
                (Some(used), None) => format_amount(used, &text(&window, "unit")),
                _ => "value not reported".into(),
            };
            let ratio = percent
                .map(|p| format!(" · {}% used", format_number(p.clamp(0.0, 100.0))))
                .unwrap_or_default();
            let reset = window
                .get("resetsAt")
                .and_then(Value::as_i64)
                .map(|at| format!(" · resets {}", crate::timestamps::datetime_label(at)))
                .unwrap_or_default();
            card = card.child(format!(
                "{}: {values}{ratio}{reset}",
                label(&window, "label", "Allowance")
            ));
        }
        for balance in rows(source, "balances") {
            let remaining = number(&balance, "remaining")
                .map(|n| format_amount(n, &text(&balance, "unit")))
                .unwrap_or_else(|| "not reported".into());
            let limit = number(&balance, "limit")
                .map(|n| format!(" / {}", format_amount(n, &text(&balance, "unit"))))
                .unwrap_or_default();
            card = card.child(format!(
                "{} remaining: {remaining}{limit}",
                label(&balance, "label", "Balance")
            ));
        }
        card.into_any_element()
    }

    fn render_processes(&self, palette: crate::design::Palette) -> AnyElement {
        let Some(sample) = self.metrics.as_ref() else {
            return div()
                .child(if self.metrics_busy {
                    "Reading process metrics…"
                } else {
                    "No process sample is available."
                })
                .into_any_element();
        };
        let mut processes = rows(sample, "processes");
        processes.sort_by(|a, b| {
            b["residentMemoryBytes"]
                .as_u64()
                .cmp(&a["residentMemoryBytes"].as_u64())
                .then_with(|| b["cpuTimeMs"].as_u64().cmp(&a["cpuTimeMs"].as_u64()))
                .then_with(|| a["pid"].as_i64().cmp(&b["pid"].as_i64()))
        });
        let mut section = div().flex().flex_col().gap_2().child(format!(
            "Process sample · {} · {} CPU time · {} RAM · {} processes",
            date(sample, "sampledAt"),
            format_duration_ms(number(sample, "cpuTimeMs").unwrap_or_default()),
            format_bytes(number(sample, "residentMemoryBytes").unwrap_or_default()),
            processes.len()
        ));
        if processes.is_empty() {
            section = section.child("No process rows were reported.");
        }
        for process in processes {
            section = section.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(crate::design::surface_color(palette.panel))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(format!(
                        "{} · PID {} · parent {}",
                        label(&process, "name", "Process"),
                        process["pid"].as_i64().unwrap_or_default(),
                        process["parentPid"].as_i64().unwrap_or_default()
                    ))
                    .child(format!(
                        "{} RAM · {} CPU time · started {}",
                        format_bytes(number(&process, "residentMemoryBytes").unwrap_or_default()),
                        format_duration_ms(number(&process, "cpuTimeMs").unwrap_or_default()),
                        date(&process, "startedAt")
                    ))
                    .child(label(&process, "chatTitle", "No associated chat"))
                    .child(div().text_xs().child(label(
                        &process,
                        "commandLine",
                        "Command details unavailable",
                    ))),
            );
        }
        section.into_any_element()
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

impl Render for UsagePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = crate::settings::palette(&self.snapshot["settings"], window.appearance());
        let mut content = div()
            .id("native-usage-scroll")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Usage and resources")
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("usage-refresh")
                                    .label(if self.usage_busy {
                                        "Refreshing allowances…"
                                    } else {
                                        "Refresh allowances"
                                    })
                                    .disabled(self.usage_busy)
                                    .on_click(cx.listener(|pane, _, _, cx| {
                                        pane.refresh_usage("refresh", cx)
                                    })),
                            )
                            .child(
                                Button::new("metrics-refresh")
                                    .label(if self.metrics_busy {
                                        "Refreshing processes…"
                                    } else {
                                        "Refresh processes"
                                    })
                                    .disabled(self.metrics_busy)
                                    .on_click(
                                        cx.listener(|pane, _, _, cx| pane.refresh_metrics(cx)),
                                    ),
                            ),
                    ),
            );
        if let Some(error) = &self.usage_error {
            content = content.child(
                div()
                    .text_color(rgb(0xf87171))
                    .child(format!("Allowance refresh failed: {error}")),
            );
        }
        content = content.child(self.render_context());
        content = content.child("Provider allowances");
        if let Some(overview) = &self.overview {
            content = content.child(format!(
                "Run history captured since {} · updated {}",
                date(overview, "capturedSince"),
                date(overview, "generatedAt")
            ));
            for source in rows(overview, "subscriptions") {
                content = content.child(self.render_provider_source(&source, palette));
            }
            content = content.child("Recorded usage totals");
            for total in rows(overview, "providerTotals") {
                let tokens = &total["tokens"];
                let cost = number(&total, "costUsd")
                    .map(|v| format!("${v:.4}"))
                    .unwrap_or_else(|| "cost not reported".into());
                content = content.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(crate::design::surface_color(palette.panel))
                        .child(format!(
                            "{} · {} runs ({} final)",
                            label(&total, "provider", "Provider"),
                            total["runs"].as_i64().unwrap_or_default(),
                            total["finalRuns"].as_i64().unwrap_or_default()
                        ))
                        .child(format!(
                            "{} tokens · {} input · {} output · {}",
                            tokens["total"].as_i64().unwrap_or_default(),
                            tokens["input"].as_i64().unwrap_or_default(),
                            tokens["output"].as_i64().unwrap_or_default(),
                            cost
                        )),
                );
            }
        } else if !self.usage_busy && self.usage_error.is_none() {
            content = content.child("Allowance data has not been loaded.");
        }
        content = content.child("Process metrics");
        if let Some(error) = &self.metrics_error {
            content = content.child(
                div()
                    .text_color(rgb(0xf87171))
                    .child(format!("Process metrics failed: {error}")),
            );
        }
        content.child(self.render_processes(palette))
    }
}

/// Detailed resource charts share the sidebar's exact live sample stream.
/// Docking owns placement; this retained view owns only disclosure state.
pub(crate) struct AppResourcePane {
    widget: Entity<crate::sidebar_usage::SidebarUsageWidget>,
    expanded: std::collections::HashSet<String>,
    initialized_groups: std::collections::HashSet<String>,
    scroll: ScrollHandle,
    resource_revision: (i64, Option<String>, Option<crate::design::Palette>),
    _observation: Subscription,
}
impl AppResourcePane {
    pub(crate) fn new(widget: Entity<crate::sidebar_usage::SidebarUsageWidget>, cx: &mut Context<Self>) -> Self {
        let resource_revision = widget.read(cx).resource_revision();
        let observation = cx.observe(&widget, |pane, widget, cx| {
            let revision = widget.read(cx).resource_revision();
            if pane.resource_revision != revision {
                pane.resource_revision = revision;
                cx.notify();
            }
        });
        Self { widget, expanded: Default::default(), initialized_groups: Default::default(), scroll: ScrollHandle::new(), resource_revision, _observation: observation }
    }
}
impl Render for AppResourcePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let widget = self.widget.read(cx);
        for key in widget.resource_default_expanded_keys() {
            if self.initialized_groups.insert(key.clone()) {
                self.expanded.insert(key);
            }
        }
        let palette = widget.resource_palette(window);
        let weak = cx.weak_entity();
        let toggle: crate::sidebar_usage::ResourceGroupToggle = std::rc::Rc::new(move |key, _, cx| {
            let _ = weak.update(cx, |pane, cx| {
                if !pane.expanded.remove(&key) { pane.expanded.insert(key); }
                cx.notify();
            });
        });
        div().size_full().min_w_0().flex().flex_col()
            .bg(crate::surface_blend::background(crate::surface_blend::Area::Workspace, palette.paper))
            .text_color(rgb(palette.ink))
            .child(div().h(px(widget.resource_header_height())).flex_shrink_0()
                .flex().items_center().gap(px(crate::design::space::INSET))
                .px(px(crate::design::space::GROUP)).border_b_1().border_color(rgb(palette.line)).border_blend_mode(crate::surface_blend::separator_mode())
                .font_family(cx.theme().font_family.clone())
                .child(Icon::new(IconName::Activity).with_size(px(20.)))
                .child(div().min_w_0().truncate().text_size(px(13.2)).font_weight(FontWeight::BOLD)
                    .child("App resource usage")))
            .child(div().id("app-resource-usage-scroll").flex_1().min_h_0().min_w_0().overflow_y_scroll()
                .track_scroll(&self.scroll)
                .font_family(crate::design::FONT_SANS).text_size(px(12.)).p(px(12.))
                .child(widget.resource_content(palette, &self.expanded, toggle)))
    }
}
