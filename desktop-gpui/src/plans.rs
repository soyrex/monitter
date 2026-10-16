//! Read-only task-owned work plans shown in the task details sidebar.
use crate::design::{self, Palette};
use crate::design::ui_scaled_px as px;
use gpui_kit::component::{Icon, Sizable, button::*};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use serde_json::Value;
use std::collections::HashSet;

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
        "completed" => if palette.dark {0x34d399} else {0x147d59},
        "blocked" => if palette.dark {0xff5c6c} else {0xb82c3e},
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
    current_only: bool,
}

impl WorkPlanPane {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: Value::Null,
            task_id: String::new(),
            palette: design::palette(&Value::Null, WindowAppearance::Light),
            font_ratio: 1.,
            history_expanded: false,
            current_only: false,
        }
    }

    pub fn set_current_only(&mut self, current_only: bool, cx: &mut Context<Self>) {
        if self.current_only != current_only {
            self.current_only = current_only;
            cx.notify();
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

    fn render_plan(&self, plan: &Value, _cx: &mut Context<Self>) -> AnyElement {
        render_plan_card(plan, self.palette, self.font_ratio, false)
    }

}

/// A retained native card keeps disclosure local to this mounted plan. The
/// public renderer also serves the sidebar and design preview.
#[derive(IntoElement)]
struct PlanCardView {
    plan: Value,
    palette: Palette,
    font_ratio: f32,
    popup: bool,
    history: bool,
}

pub(crate) fn render_plan_card(plan: &Value, palette: Palette, font_ratio: f32, popup: bool) -> AnyElement {
    PlanCardView { plan: plan.clone(), palette, font_ratio, popup, history: false }.into_any_element()
}

impl RenderOnce for PlanCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variant = if self.history { "history" } else if self.popup { "popup" } else { "sidebar" };
        let key = SharedString::from(format!("work-plan-card-{variant}-{}", text(&self.plan, "id")));
        let holder = window.use_keyed_state(key, cx, |_, cx| cx.new(|_| PlanCard {
            plan: Value::Null, palette: self.palette, font_ratio: self.font_ratio,
            popup: self.popup, history: self.history, expanded: HashSet::new(), summary_expanded: false,
        }));
        let card = holder.read(cx).clone();
        card.update(cx, |card, cx| {
            if card.plan != self.plan || card.palette != self.palette
                || card.font_ratio != self.font_ratio || card.popup != self.popup || card.history != self.history {
                card.plan = self.plan;
                card.palette = self.palette;
                card.font_ratio = self.font_ratio;
                card.popup = self.popup;
                card.history = self.history;
                cx.notify();
            }
        });
        card
    }
}

struct PlanCard {
    plan: Value,
    palette: Palette,
    font_ratio: f32,
    popup: bool,
    history: bool,
    expanded: HashSet<String>,
    summary_expanded: bool,
}

impl Render for PlanCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.history { return self.render_history(cx); }
        let plan = &self.plan;
        let palette = self.palette;
        // Work-plan metadata stays compact independently of large chat fonts.
        let font_ratio = self.font_ratio.min(1.);
        let popup = self.popup;
        let type_size = |size: f32| px(size * font_ratio);
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
        let finished = !items.is_empty() && completed == items.len();
        let current_status = if active { "Active" } else if finished { "Complete" } else { "Closed" };
        let plan_status = if active { "active" } else if finished { "completed" } else { "closed" };
        let mut card = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .whitespace_normal()
            .p(px(10.))
            .border_1()
            .border_color(rgb(palette.line))
            .rounded(px(design::radius::CARD))
            .when(!popup, |card| card.bg(crate::design::surface_color(palette.panel)))
            .flex()
            .flex_col()
            .gap(px(5.))
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
                            .bg(crate::design::surface_color(mix_rgb(palette.panel, status_color(plan_status, palette), 0.12)))
                            .text_color(rgb(status_color(plan_status, palette)))
                            .font_family(design::FONT_MONO)
                            .text_size(type_size(9.))
                            .child(current_status),
                    ),
            )
            .child(
                div()
                    .mt(px(1.))
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
            .gap(px(4.))
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
                        "completed" => if palette.dark {0x34d399} else {0x147d59},
                        "in_progress" => palette.accent,
                        "blocked" => if palette.dark {0xff5c6c} else {0xb82c3e},
                        "skipped" => mix_rgb(palette.panel, palette.muted, 0.55),
                        _ => mix_rgb(palette.panel, palette.muted, 0.19),
                    }))
                    .aria_label(format!("{}: {}", text(item, "title"), status_label(&status))),
            );
        }
        card = card.child(progress);

        for (index, item) in items.into_iter().enumerate() {
            let item_id = item.get("id").and_then(Value::as_str)
                .map(str::to_owned).unwrap_or_else(|| index.to_string());
            let expanded = self.expanded.contains(&item_id);
            let status = text(item, "status");
            let mut body = div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .debug_selector(move || format!("work-plan-title-{index}"))
                        .text_size(type_size(11.))
                        .line_height(type_size(15.))
                        .text_color(rgb(if matches!(status.as_str(), "completed" | "skipped" | "pending") {
                            palette.muted
                        } else {
                            palette.ink
                        }))
                        .child(text(item, "title")),
                );
            let note = text(item, "note");
            if expanded && !note.is_empty() {
                body = body.child(
                    div()
                        .debug_selector(move || format!("work-plan-note-{index}"))
                        .text_size(type_size(10.))
                        .line_height(type_size(14.))
                        .text_color(rgb(palette.muted))
                        .child(note.clone()),
                );
            }
            card = card.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .border_t_1()
                    .border_color(rgb(mix_rgb(palette.line, palette.panel, 0.45)))
                    .pt(px(5.))
                    .child(Icon::new(match status.as_str() {
                        "completed" => IconName::Check,
                        "in_progress" => IconName::CircleDot,
                        "blocked" => IconName::TriangleAlert,
                        "skipped" => IconName::Minus,
                        _ => IconName::Circle,
                    }).size(px(13. * font_ratio)).text_color(rgb(status_color(&status, palette))))
                    .id(("work-plan-step", index))
                    .debug_selector(move || format!("work-plan-step-{index}"))
                    .role(gpui_kit::accesskit::Role::ListItem)
                    .aria_label(format!("{}: {}", text(item, "title"), status_label(&status)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .child(body),
                    )
                    .when(!note.is_empty(), |row| row.child(
                        Button::new(("work-plan-item-info", index))
                            .compact().xsmall().ghost()
                            .h(px(18. * font_ratio)).px(px(4.)).py_0()
                            .text_size(type_size(9.))
                            .label("Info")
                            .accessibility_label(format!("{} details for {}", if expanded { "Hide" } else { "Show" }, text(item, "title")))
                            .tooltip(if expanded { "Hide item details" } else { "Show item details" })
                            .toggled(expanded)
                            .debug_selector(move || format!("work-plan-info-{index}"))
                            .on_click(cx.listener(move |card, _, _, cx| {
                                if !card.expanded.remove(&item_id) { card.expanded.insert(item_id.clone()); }
                                cx.notify();
                            })),
                    ))
            );
        }

        let summary = text(plan, "summary");
        if !summary.is_empty() {
            let expanded = self.summary_expanded;
            card = card.child(
                div().flex().flex_col().gap(px(4.))
                    .child(Button::new("work-plan-summary-info")
                        .compact().xsmall().ghost().h(px(20. * font_ratio))
                        .px(px(4.)).py_0().text_size(type_size(9.))
                        .label("Outcome · Info")
                        .accessibility_label(if expanded { "Hide work-plan outcome details" } else { "Show work-plan outcome details" })
                        .toggled(expanded)
                        .debug_selector(|| "work-plan-summary-info".into())
                        .on_click(cx.listener(|card, _, _, cx| {
                            card.summary_expanded = !card.summary_expanded;
                            cx.notify();
                        })))
                    .when(expanded, |row| row.child(
                        div().debug_selector(|| "work-plan-summary".into())
                            .p(px(6.)).rounded(px(5.))
                            .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.accent, 0.07)))
                            .text_size(type_size(10.)).child(summary),
                    )),
            );
        }

        card.into_any_element()
    }
}

impl PlanCard {
    fn render_history(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let type_size = |size: f32| px(size * self.font_ratio.min(1.));
        let items = rows(&self.plan, "items");
        let completed = items.iter().filter(|item| text(item, "status") == "completed").count();
        let summary = text(&self.plan, "summary");
        let expanded = self.summary_expanded;
        div().w_full().min_w_0().whitespace_normal().p(px(6.)).rounded(px(5.))
            .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.muted, 0.05)))
            .flex().flex_col().gap(px(2.))
            .child(div().text_size(type_size(10.)).font_weight(FontWeight::SEMIBOLD)
                .child(text(&self.plan, "title")))
            .child(div().text_size(type_size(9.)).text_color(rgb(palette.muted))
                .child(format!("{completed} / {} done · {}", items.len(), status_label(&text(&self.plan, "status")))))
            .when(!summary.is_empty(), |row| row.child(
                Button::new("work-plan-history-info").compact().xsmall().ghost()
                    .h(px(18.)).px(px(4.)).py_0().text_size(type_size(9.)).label("Outcome · Info")
                    .accessibility_label(if expanded { "Hide earlier work-plan outcome" } else { "Show earlier work-plan outcome" })
                    .toggled(expanded)
                    .debug_selector(|| "work-plan-history-info".into())
                    .on_click(cx.listener(|card, _, _, cx| {
                        card.summary_expanded = !card.summary_expanded;
                        cx.notify();
                    })),
            ))
            .when(expanded && !summary.is_empty(), |row| row.child(
                div().debug_selector(|| "work-plan-history-summary".into())
                    .text_size(type_size(9.)).text_color(rgb(palette.muted)).child(summary),
            ))
            .into_any_element()
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
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(9.))
            .text_color(rgb(palette.ink));
        if let Some(current) = current {
            root = root.child(self.render_plan(current, cx));
            if history_count > 0 && !self.current_only {
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
                                .toggled(expanded)
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
        PlanCardView { plan: plan.clone(), palette: self.palette, font_ratio: self.font_ratio,
            popup: false, history: true }.into_any_element()
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod design_system_tests {
    use super::*;
    use ::core::prelude::v1::test;
    use gpui_kit::{TestAppContext, WindowAppearance};
    use serde_json::json;

    #[gpui_kit::test]
    fn design_system_plan_rows_keep_state_icons_and_labels_inside_narrow_cards(cx: &mut TestAppContext) {
        struct Fixture;
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let plan = json!({"status":"active","title":"Verify the native design system", "items":[
                    {"title":"Review reference", "status":"completed"},
                    {"title":"Implement a long step that wraps safely inside a narrow pane", "status":"in_progress"},
                    {"title":"Verify result", "status":"pending"},
                    {"title":"Unavailable prerequisite", "status":"blocked"},
                    {"title":"Optional follow-up", "status":"skipped"}]});
                div().w(gpui_kit::px(320.)).child(render_plan_card(&plan,
                    design::palette(&Value::Null, WindowAppearance::Dark), 1., false))
            }
        }
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| Fixture);
            gpui_kit::component::Root::new(view, window, cx)
        });
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let mut bottom = gpui_kit::px(0.);
        for selector in ["work-plan-step-0", "work-plan-step-1", "work-plan-step-2", "work-plan-step-3", "work-plan-step-4"] {
            let bounds = visual.debug_bounds(selector).expect("every real state renders");
            assert!(bounds.top() >= bottom);
            assert!(bounds.right() <= gpui_kit::px(320.));
            assert!(bounds.size.height >= gpui_kit::px(13.));
            bottom = bounds.bottom();
        }
    }

    #[gpui_kit::test]
    fn compact_sidebar_plan_info_discloses_stored_note_and_outcome(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            let (_, visual) = cx.add_window_view(move |window, cx| {
                let view = cx.new(WorkPlanPane::new);
                let plan = json!({"workPlans":[{"id":"sidebar-plan", "taskId":"chat", "status":"active",
                    "title":"Native plan", "summary":"Authoritative stored outcome.", "items":[
                        {"id":"build", "title":"Build", "status":"in_progress", "note":"Full stored response with all detail preserved."},
                        {"id":"verify", "title":"Verify", "status":"pending"}]}]});
                view.update(cx, |view, cx| view.set_context(&plan, "chat",
                    design::palette(&Value::Null, appearance), 32. / 14., cx));
                gpui_kit::component::Root::new(view, window, cx)
            });
            visual.simulate_resize(size(gpui_kit::px(300.), gpui_kit::px(600.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("work-plan-note-0").is_none());
            assert!(visual.debug_bounds("work-plan-summary").is_none());
            assert!(visual.debug_bounds("work-plan-info-1").is_none(), "no empty disclosure");
            let title = visual.debug_bounds("work-plan-title-0").unwrap();
            assert!(title.size.height <= design::ui_scaled_px(15.) + gpui_kit::px(1.), "compact 11px title with scaled 15px line height: {title:?}");
            let row = visual.debug_bounds("work-plan-step-0").unwrap();
            // 18px Info control + 5px top padding + border, allowing pixel rounding.
            assert!(row.size.height <= design::ui_scaled_px(24.) + gpui_kit::px(1.), "compact collapsed row: {row:?}");
            let info = visual.debug_bounds("work-plan-info-0").unwrap();
            visual.simulate_click(info.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("work-plan-note-0").is_some());
            let info = visual.debug_bounds("work-plan-info-0").unwrap();
            visual.simulate_click(info.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("work-plan-note-0").is_none());
            let summary = visual.debug_bounds("work-plan-summary-info").unwrap();
            visual.simulate_click(summary.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("work-plan-summary").is_some());
            let summary = visual.debug_bounds("work-plan-summary-info").unwrap();
            visual.simulate_click(summary.center(), Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            assert!(visual.debug_bounds("work-plan-summary").is_none());
        }
    }

}
