//! Read-only task-owned work plans shown in the task details sidebar.
use crate::design::{self, Palette};
use gpui_kit::{prelude::*, *};
use serde_json::Value;

const FONT_RATIO_MIN: f32 = 8. / 14.;
const FONT_RATIO_MAX: f32 = 32. / 14.;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn rows<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn number(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn status_label(status: &str) -> &'static str {
    match status {
        "in_progress" => "In progress",
        "completed" => "Done",
        "blocked" => "Blocked",
        "skipped" => "Skipped",
        "closed" => "Closed",
        "active" => "Active",
        _ => "Pending",
    }
}

fn status_color(status: &str, palette: Palette) -> u32 {
    match status {
        "in_progress" | "active" => palette.accent,
        "completed" => 0x3eaa7a,
        "blocked" => 0xc96058,
        "skipped" | "closed" => palette.muted,
        _ => palette.muted,
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

fn progress_summary(completed: usize, total: usize, in_progress: usize, blocked: usize, skipped: usize) -> String {
    format!(
        "{completed} of {total} done{}{}{}",
        if in_progress > 0 { format!(" · {in_progress} in progress") } else { String::new() },
        if blocked > 0 { format!(" · {blocked} blocked") } else { String::new() },
        if skipped > 0 { format!(" · {skipped} skipped") } else { String::new() },
    )
}

/// Retained plan panel. Its state is limited to disclosure; all displayed plan
/// data comes from the backend-owned workspace snapshot.
pub struct WorkPlanPane {
    snapshot: Value,
    task_id: String,
    palette: Palette,
    font_ratio: f32,
    history_expanded: bool,
}

impl WorkPlanPane {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: Value::Null,
            task_id: String::new(),
            palette: design::palette(&Value::Null, WindowAppearance::Light),
            font_ratio: 1.,
            history_expanded: false,
        }
    }

    pub fn set_context(
        &mut self,
        snapshot: &Value,
        task_id: &str,
        palette: Palette,
        font_ratio: f32,
        cx: &mut Context<Self>,
    ) {
        if self.task_id != task_id {
            self.history_expanded = false;
        }
        let ratio = if font_ratio.is_finite() {
            font_ratio.clamp(FONT_RATIO_MIN, FONT_RATIO_MAX)
        } else {
            1.
        };
        if self.snapshot != *snapshot
            || self.task_id != task_id
            || self.palette != palette
            || self.font_ratio != ratio
        {
            self.snapshot = snapshot.clone();
            self.task_id = task_id.to_owned();
            self.palette = palette;
            self.font_ratio = ratio;
            cx.notify();
        }
    }

    fn render_plan(&self, plan: &Value, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let type_size = |size: f32| px(size * self.font_ratio);
        let items = rows(plan, "items");
        let completed = items
            .iter()
            .filter(|item| text(item, "status") == "completed")
            .count();
        let in_progress = items
            .iter()
            .filter(|item| text(item, "status") == "in_progress")
            .count();
        let blocked = items
            .iter()
            .filter(|item| text(item, "status") == "blocked")
            .count();
        let skipped = items
            .iter()
            .filter(|item| text(item, "status") == "skipped")
            .count();
        let active = text(plan, "status") == "active";
        let current_status = if active { "Active" } else { "Closed" };
        let mut card = div()
            .w_full()
            .p(px(11.))
            .border_1()
            .border_color(rgb(palette.line))
            .rounded(px(7.))
            .bg(crate::design::surface_color(palette.panel))
            .flex()
            .flex_col()
            .gap(px(7.))
            .text_color(rgb(palette.ink))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .text_size(type_size(10.))
                    .font_family(design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child("WORK PLAN")
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(3.))
                            .rounded_full()
                            .bg(crate::design::surface_color(mix_rgb(palette.panel, status_color(if active { "active" } else { "closed" }, palette), 0.12)))
                            .text_color(rgb(status_color(if active { "active" } else { "closed" }, palette)))
                            .font_family(design::FONT_MONO)
                            .text_size(type_size(9.))
                            .child(current_status),
                    ),
            )
            .child(
                div()
                    .mt(px(2.))
                    .text_size(type_size(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text(plan, "title")),
            )
            .child(
                div()
                    .text_size(type_size(10.))
                    .text_color(rgb(palette.muted))
                    .child(progress_summary(completed, items.len(), in_progress, blocked, skipped)),
            );

        let mut progress = div()
            .id("work-plan-progress")
            .w_full()
            .h(px(5.))
            .my(px(2.))
            .overflow_hidden()
            .flex()
            .gap(px(2.))
            .rounded_full()
            .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.muted, 0.15)))
            .role(gpui_kit::accesskit::Role::ProgressIndicator)
            .aria_label("Completed work-plan items")
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(items.len() as f64)
            .aria_numeric_value(completed as f64);
        for (index, item) in items.iter().enumerate() {
            let status = text(item, "status");
            progress = progress.child(
                div()
                    .id(format!("work-plan-progress-item-{index}"))
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(5.))
                    .bg(rgb(match status.as_str() {
                        "completed" => 0x3eaa7a,
                        "in_progress" => palette.accent,
                        "blocked" => 0xc96058,
                        "skipped" => mix_rgb(palette.panel, palette.muted, 0.55),
                        _ => mix_rgb(palette.panel, palette.muted, 0.19),
                    }))
                    .aria_label(format!("{}: {}", text(item, "title"), status_label(&status))),
            );
        }
        card = card.child(progress);

        for item in items {
            let status = text(item, "status");
            let mut body = div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(type_size(11.))
                        .text_color(rgb(if matches!(status.as_str(), "completed" | "skipped") {
                            palette.muted
                        } else {
                            palette.ink
                        }))
                        .child(text(item, "title")),
                );
            let note = text(item, "note");
            if !note.is_empty() {
                body = body.child(
                    div()
                        .text_size(type_size(10.))
                        .text_color(rgb(palette.muted))
                        .child(note),
                );
            }
            card = card.child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(rgb(mix_rgb(palette.line, palette.panel, 0.45)))
                    .pt(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .child(body),
                    )
                    .child(
                        div()
                            .max_w(px(65.))
                            .text_size(type_size(9.))
                            .font_family(design::FONT_MONO)
                            .text_color(rgb(status_color(&status, palette)))
                            .child(status_label(&status)),
                    ),
            );
        }

        let summary = text(plan, "summary");
        if !summary.is_empty() {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .p(px(8.))
                    .rounded(px(5.))
                    .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.accent, 0.07)))
                    .child(
                        div()
                            .text_size(type_size(9.))
                            .font_family(design::FONT_MONO)
                            .text_color(rgb(palette.muted))
                            .child("OUTCOME"),
                    )
                    .child(div().text_size(type_size(10.)).child(summary)),
            );
        }

        let _ = cx;
        card.into_any_element()
    }
}

impl Render for WorkPlanPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let type_size = |size: f32| px(size * self.font_ratio);
        let mut plans: Vec<&Value> = rows(&self.snapshot, "workPlans")
            .into_iter()
            .filter(|plan| text(plan, "taskId") == self.task_id)
            .collect();
        plans.sort_by(|left, right| {
            number(right, "updatedAt")
                .cmp(&number(left, "updatedAt"))
                .then_with(|| number(right, "createdAt").cmp(&number(left, "createdAt")))
                .then_with(|| text(right, "id").cmp(&text(left, "id")))
        });
        let current_index = plans
            .iter()
            .position(|plan| text(plan, "status") == "active")
            .unwrap_or(0);
        let current = if plans.is_empty() {
            None
        } else {
            Some(plans.remove(current_index))
        };
        let history = plans;
        let history_count = history.len();

        let mut root = div()
            .id("task-work-plans")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(9.))
            .text_color(rgb(palette.ink));
        if let Some(current) = current {
            root = root.child(self.render_plan(current, cx));
            if history_count > 0 {
                let expanded = self.history_expanded;
                let toggle = cx.entity();
                root = root.child(
                    div()
                        .w_full()
                        .border_t_1()
                        .border_color(rgb(palette.line))
                        .pt(px(8.))
                        .child(
                            Button::new("work-plan-history-toggle")
                                .text()
                                .compact()
                                .xsmall()
                                .w_full()
                                .h_auto()
                                .min_h(px(0.))
                                .p_0()
                                .justify_start()
                                .text_size(type_size(10.))
                                .text_color(rgb(palette.muted))
                                .label(if expanded {
                                    format!("Earlier plans · {history_count} · Hide")
                                } else {
                                    format!("Earlier plans · {history_count}")
                                })
                                .accessibility_label(if expanded {
                                    format!("Hide {history_count} earlier work plans")
                                } else {
                                    format!("Show {history_count} earlier work plans")
                                })
                                .aria_expanded(expanded)
                                .on_click(move |_, _, cx| {
                                    let _ = toggle.update(cx, |pane, cx| {
                                        pane.history_expanded = !pane.history_expanded;
                                        cx.notify();
                                    });
                                }),
                        ),
                );
                if expanded {
                    for plan in history {
                        root = root.child(self.render_history(plan));
                    }
                }
            }
        } else {
            root = root.child(
                div()
                    .text_size(type_size(10.))
                    .text_color(rgb(palette.muted))
                    .child("No work plan published for this chat yet."),
            );
        }
        root
    }
}

impl WorkPlanPane {
    fn render_history(&self, plan: &Value) -> AnyElement {
        let palette = self.palette;
        let type_size = |size: f32| px(size * self.font_ratio);
        let items = rows(plan, "items");
        let completed = items
            .iter()
            .filter(|item| text(item, "status") == "completed")
            .count();
        let status = text(plan, "status");
        let mut row = div()
            .w_full()
            .p(px(6.))
            .rounded(px(5.))
            .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.muted, 0.05)))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .text_size(type_size(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text(plan, "title")),
            )
            .child(
                div()
                    .text_size(type_size(9.))
                    .text_color(rgb(palette.muted))
                    .child(format!("{completed} / {} done · {}", items.len(), status_label(&status))),
            );
        let summary = text(plan, "summary");
        if !summary.is_empty() {
            row = row.child(
                div()
                    .text_size(type_size(9.))
                    .text_color(rgb(palette.muted))
                    .child(summary),
            );
        }
        row.into_any_element()
    }
}
