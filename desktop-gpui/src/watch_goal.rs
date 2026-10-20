//! Watch presentation of the retained native goal reader and current work plan.
use super::*;

#[derive(Default, PartialEq, Eq, Debug)]
pub(super) struct Progress {
    plan: Option<String>,
    completed: usize,
    total: usize,
    current: Option<String>,
    update: Option<String>,
}
impl Progress {
    pub(super) fn from_snapshot(snapshot: &Value, task_id: &str) -> Self {
        let rows = |key: &str| snapshot[key].as_array().into_iter().flatten();
        let plan = rows("workPlans")
            .filter(|plan| plan["taskId"] == task_id && plan["status"] == "active")
            .max_by_key(|plan| plan["createdAt"].as_i64().unwrap_or_default());
        let items: Vec<_> = plan
            .into_iter()
            .flat_map(|plan| plan["items"].as_array().into_iter().flatten())
            .collect();
        let current = items
            .iter()
            .find(|item| item["status"] == "in_progress")
            .or_else(|| items.iter().find(|item| item["status"] == "blocked"))
            .map(|item| text(item, "title"));
        let start = snapshot["nativeMessageContext"][task_id]["latestUserAt"]
            .as_i64()
            .or_else(|| {
                rows("messages")
                    .filter(|m| m["taskId"] == task_id && m["role"] == "user")
                    .filter_map(|m| m["createdAt"].as_i64())
                    .max()
            });
        let update = rows("events")
            .filter(|event| {
                event["taskId"] == task_id
                    && event["kind"] == "agent_progress"
                    && start.is_some_and(|start| {
                        event["createdAt"].as_i64().is_some_and(|at| at >= start)
                    })
            })
            .max_by_key(|event| event["createdAt"].as_i64().unwrap_or_default())
            .map(|event| text(event, "title"))
            .filter(|s| !s.trim().is_empty());
        Self {
            plan: plan.map(|plan| text(plan, "title")),
            completed: items
                .iter()
                .filter(|item| item["status"] == "completed")
                .count(),
            total: items.len(),
            current,
            update,
        }
    }
}

pub(super) fn read_delay(
    watch: bool,
    goal: Option<&Value>,
    running: bool,
    failures: u32,
) -> Option<Duration> {
    if watch && failures == 0 {
        Some(Duration::from_secs(60))
    } else {
        next_goal_read(goal, running, failures)
    }
}
fn status_label(status: &str) -> &'static str {
    match status {
        "active" => "active",
        "paused" => "paused",
        "blocked" => "blocked",
        "complete" => "complete",
        "usageLimited" => "usage limited",
        "budgetLimited" => "budget reached",
        _ => "unknown",
    }
}
fn compact_token_count(value: u64) -> String {
    let (divisor, suffix) = if value >= 1_000_000 {
        (1_000_000., "m")
    } else if value >= 1_000 {
        (1_000., "k")
    } else {
        return value.to_string();
    };
    let scaled = value as f64 / divisor;
    if scaled >= 10. || scaled.fract() < 0.05 {
        format!("{scaled:.0}{suffix}")
    } else {
        format!("{scaled:.1}{suffix}")
    }
}
fn accounting(goal: &Value) -> Option<String> {
    let used = goal["tokensUsed"].as_u64()?;
    Some(match goal["tokenBudget"].as_u64().filter(|budget| *budget > 0) {
        Some(budget) => format!(
            "{} / {} tokens",
            compact_token_count(used),
            compact_token_count(budget)
        ),
        None => format!("{} tokens", compact_token_count(used)),
    })
}

pub(super) fn render(view: &TaskActivityPane, palette: design::Palette, ratio: f32) -> AnyElement {
    let Some(goal) = view.goal.as_ref() else {
        // Hermes supplies a native progress note rather than a Codex Goal object.
        return if view.goal_note.is_empty() {
            div().into_any_element()
        } else {
            div()
                .id("watcher-goal-note")
                .debug_selector(|| "watcher-goal-note".into())
                .text_size(px(12. * ratio))
                .text_color(rgb(palette.muted))
                .child(format!("Last goal update · {}", view.goal_note))
                .into_any_element()
        };
    };
    let status = status_label(goal["status"].as_str().unwrap_or_default());
    let active = status == "active";
    let usage = accounting(goal);
    let mut pill = div()
        .id("watcher-goal-pill")
        .debug_selector(|| "watcher-goal-pill".into())
        .max_w_full()
        .min_w_0()
        .h(px(26. * ratio))
        .px(px(10. * ratio))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .border_1()
        .border_color(rgb(palette.line))
        .bg(crate::surface_blend::floating_control_background(palette.panel, true))
        .text_size(px(11. * ratio))
        .line_height(px(14. * ratio))
        .text_color(rgb(if active { palette.accent } else { palette.muted }))
        .child(Icon::default().data(icon_assets::Goal.1).with_size(px(13.)))
        .child(
            div()
                .id("watcher-goal-label")
                .debug_selector(|| "watcher-goal-label".into())
                .font_weight(FontWeight::SEMIBOLD)
                .child("Goal"),
        );
    if let Some(usage) = usage {
        pill = pill.child(
            div()
                .id("watcher-goal-usage")
                .debug_selector(|| "watcher-goal-usage".into())
                .min_w_0()
                .truncate()
                .font_family(design::FONT_MONO)
                .text_color(rgb(palette.muted))
                .child(usage),
        );
    }
    pill = pill.child(
        div()
            .id("watcher-goal-status")
            .debug_selector(|| "watcher-goal-status".into())
            .flex_shrink_0()
            .child(status.to_owned()),
    );
    pill.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Progress, accounting, compact_token_count, read_delay, status_label};
    use serde_json::json;
    use std::time::Duration;
    #[test]
    fn watch_goal_progress_is_task_scoped_and_does_not_count_skipped_as_complete() {
        let snapshot = json!({"workPlans":[
            {"taskId":"other","status":"active","createdAt":9,"title":"Other","items":[]},
            {"taskId":"t","status":"closed","createdAt":8,"items":[]},
            {"taskId":"t","status":"active","createdAt":1,"title":"Current","items":[
                {"status":"completed"},{"status":"skipped"},{"status":"in_progress","title":"Verifying UI"}]}],
            "nativeMessageContext":{"t":{"latestUserAt":20}},"events":[
                {"taskId":"t","kind":"agent_progress","createdAt":10,"title":"Old update"},
                {"taskId":"other","kind":"agent_progress","createdAt":30,"title":"Wrong task"}]});
        let progress = Progress::from_snapshot(&snapshot, "t");
        assert_eq!(progress.plan.as_deref(), Some("Current"));
        assert_eq!((progress.completed, progress.total), (1, 3));
        assert_eq!(progress.current.as_deref(), Some("Verifying UI"));
        assert!(progress.update.is_none());
    }
    #[test]
    fn watch_goal_resources_are_usage_not_objective_percentage() {
        assert_eq!(
            accounting(&json!({"tokensUsed":1200,"tokenBudget":1000,"timeUsedSeconds":125}))
                .unwrap(),
            "1.2k / 1k tokens"
        );
        assert!(accounting(&json!({"status":"active"})).is_none());
        assert_eq!(compact_token_count(124000), "124k");
        assert_eq!(compact_token_count(2000000), "2m");
        assert_eq!(status_label("complete"), "complete");
        assert_eq!(status_label("budgetLimited"), "budget reached");
    }
    #[test]
    fn watch_goal_reads_discover_new_goals_and_stop_when_watch_closes() {
        assert_eq!(
            read_delay(true, None, false, 0),
            Some(Duration::from_secs(60))
        );
        assert_eq!(read_delay(false, None, true, 0), None);
        assert_eq!(
            read_delay(true, None, true, 3),
            Some(Duration::from_secs(240))
        );
    }
}

#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
mod native_tests {
    use crate::chat::ChatPane;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, size};
    use serde_json::json;
    use std::rc::Rc;
    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }
    #[gpui_kit::test]
    fn watcher_activity_goal_pill_updates_without_replacing_plan_and_clears_on_task_change(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let snapshot = json!({"settings":{"theme":"dark"},
            "tasks":[{"id":"watch","title":"Goal work","agentId":"agent","provider":"codex","status":"running","queue":[]},
                {"id":"other","title":"Other task","agentId":"agent","provider":"codex","status":"idle","queue":[]}],
            "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
            "workPlans":[{"id":"p","taskId":"watch","status":"active","title":"Deliver change","items":[
                {"id":"a","title":"Implement","status":"completed"},{"id":"b","title":"Verify","status":"in_progress"}]}],
            "subagentSessions":[],"messages":[],"events":[],"approvalRequests":[],"hosts":[],"channels":[]});
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| {
                ChatPane::performance_fixture("watch".into(), Rc::new(snapshot.clone()), window, cx)
            });
            pane.update(cx, |pane, _| pane.watcher.enabled = true);
            retained = Some(pane.clone());
            gpui_kit::component::Root::new(pane, window, cx)
        });
        visual.simulate_resize(size(gpui_kit::px(320.), gpui_kit::px(900.)));
        draw(visual);
        draw(visual);
        assert!(visual.debug_bounds("watcher-goal-pill").is_none());
        let pane = retained.unwrap();
        let activity = pane.read_with(visual, |pane, _| pane.activity.clone());
        activity.update(visual, |view, cx| {
            view.goal = Some(json!({"objective":"Complete the requested desktop update","status":"active","tokensUsed":124000,"tokenBudget":2000000,"timeUsedSeconds":65}));
            cx.notify();
        });
        draw(visual);
        draw(visual);
        for selector in [
            "watcher-goal-pill",
            "watcher-goal-label",
            "watcher-goal-status",
            "watcher-goal-usage",
        ] {
            let bounds = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("Missing {selector}"));
            assert!(
                bounds.origin.x >= gpui_kit::px(0.) && bounds.right() <= gpui_kit::px(320.),
                "{selector} must fit a narrow pane"
            );
        }
        assert!(visual.debug_bounds("watcher-goal").is_none());
        assert!(visual.debug_bounds("watcher-goal-objective").is_none());
        assert!(visual.debug_bounds("watcher-card-previous").is_none());
        assert!(visual.debug_bounds("watcher-card-next").is_none());
        assert!(visual.debug_bounds("watcher-plan").is_some());
        activity.update(visual, |view, cx| {
            view.goal.as_mut().unwrap()["status"] = json!("paused");
            view.watch_progress.current = Some("Awaiting review".into());
            cx.notify();
        });
        draw(visual);
        draw(visual);
        assert!(
            visual.debug_bounds("watcher-goal-pill").is_some(),
            "A paused goal remains visible as the compact pill"
        );
        assert!(visual.debug_bounds("watcher-plan").is_some());
        activity.update(visual, |view, cx| view.set_context(&snapshot, "other", cx));
        draw(visual);
        draw(visual);
        assert!(
            visual.debug_bounds("watcher-goal-pill").is_none(),
            "A goal must never leak across task identities"
        );
    }
}
