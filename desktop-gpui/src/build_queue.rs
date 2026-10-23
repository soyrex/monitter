//! Read-only project build queue projection; the backend owns scheduling and results.
use crate::design::{Palette, ui_scaled_px as px};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, button::*, spinner::Spinner};
use gpui_kit::{prelude::*, *};
use serde_json::Value;

fn field(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_owned()
}
fn rows<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    snapshot[key].as_array().into_iter().flatten()
}
pub(super) fn project<'a>(snapshot: &'a Value, task_id: Option<&str>) -> Option<&'a Value> {
    let task = rows(snapshot, "tasks").find(|task| task["id"].as_str() == task_id)?;
    let project_id = task["projectId"].as_str()?;
    rows(snapshot, "projects").find(|project| {
        project["id"].as_str() == Some(project_id)
            && (project["buildManagementEnabled"] == true
                || rows(snapshot, "buildRuns")
                    .any(|run| run["projectId"] == project_id && run["status"] == "running"))
    })
}
pub(super) fn pending_count(snapshot: &Value, project_id: &str) -> usize {
    rows(snapshot, "buildRequests")
        .filter(|request| {
            request["projectId"] == project_id
                && matches!(
                    request["status"].as_str(),
                    Some("planned" | "ready" | "running")
                )
        })
        .count()
}
pub(super) fn stalled_runs<'a>(snapshot: &'a Value, project_id: &str) -> Vec<&'a Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    rows(snapshot, "buildRuns")
        .filter(|run| {
            run["projectId"] == project_id
                && run["status"] == "running"
                && run["leaseExpiresAt"]
                    .as_i64()
                    .is_some_and(|expires| expires < now)
        })
        .collect()
}
fn task_label(snapshot: &Value, task_id: &str) -> String {
    let Some(task) = rows(snapshot, "tasks").find(|task| task["id"] == task_id) else {
        return "Unavailable pane".into();
    };
    let agent = rows(snapshot, "agents")
        .find(|agent| agent["id"] == task["agentId"])
        .map(|agent| field(agent, "name"))
        .unwrap_or_else(|| "Agent".into());
    format!("{agent} · {}", field(task, "title"))
}
fn heading(title: &str, palette: Palette) -> Div {
    div()
        .pt(px(7.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .child(title.to_owned())
}
fn code(command: String, palette: Palette) -> Div {
    div()
        .px(px(6.))
        .py(px(4.))
        .rounded(px(4.))
        .bg(rgb(palette.code))
        .font_family(crate::design::FONT_MONO)
        .text_size(px(10.))
        .child(command)
}
fn status_label(status: &str) -> &str {
    match status {
        "running" => "Building",
        "succeeded" => "Succeeded",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        "ready" | "planned" => "Queued",
        _ => status,
    }
}
#[derive(IntoElement)]
struct StatusBadge {
    status: String,
    time: Option<String>,
    palette: Palette,
}
impl RenderOnce for StatusBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (color, icon) = match self.status.as_str() {
            "succeeded" => (rgb(self.palette.ok()).into(), IconName::Check),
            "failed" => (rgb(self.palette.err()).into(), IconName::X),
            "Needs attention" => (cx.theme().warning, IconName::TriangleAlert),
            "running" => (rgb(self.palette.accent).into(), IconName::LoaderCircle),
            "ready" | "planned" => (rgb(self.palette.warn()).into(), IconName::Clock),
            _ => (cx.theme().muted_foreground, IconName::Clock),
        };
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(3.))
            .child(
                div()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_full()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(color.opacity(0.13))
                    .when(self.status == "running", |badge| badge.border_1().border_color(color.opacity(0.35)))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .when(self.status == "running", |row| {
                        row.child(Spinner::new().xsmall().color(color))
                    })
                    .when(self.status != "running", |row| {
                        row.child(Icon::new(icon).size(px(11.)))
                    })
                    .child(status_label(&self.status).to_owned()),
            )
            .when_some(self.time, |row, time| {
                row.child(
                    div()
                        .font_family(crate::design::FONT_MONO)
                        .text_size(px(10.))
                        .text_color(rgb(self.palette.muted))
                        .child(time),
                )
            })
    }
}
fn status_badge(status: &str, record: &Value, palette: Palette) -> StatusBadge {
    let keys = if status == "running" || status == "Needs attention" {
        [
            "startedAt",
            "createdAt",
            "updatedAt",
            "finishedAt",
            "completedAt",
        ]
    } else {
        [
            "finishedAt",
            "completedAt",
            "updatedAt",
            "createdAt",
            "startedAt",
        ]
    };
    let time = keys
        .into_iter()
        .find_map(|key| record[key].as_i64().filter(|time| *time > 0))
        .and_then(crate::timestamps::local_datetime)
        .map(|date| timestamp_label(date, chrono::Local::now()));
    StatusBadge {
        status: status.to_owned(),
        time,
        palette,
    }
}
fn timestamp_label(
    date: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    date.format(if date.date_naive() == now.date_naive() {
        "%-I:%M %p"
    } else {
        "%b %-d, %Y, %-I:%M %p"
    })
    .to_string()
}
fn run_status(run: &Value, now: i64) -> &str {
    if run["status"] == "running"
        && run["leaseExpiresAt"]
            .as_i64()
            .is_some_and(|expires| expires < now)
    {
        "Needs attention"
    } else {
        run["status"].as_str().unwrap_or("")
    }
}
#[derive(IntoElement)]
struct BuildDetails {
    id: String,
    body: Div,
}
impl RenderOnce for BuildDetails {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            SharedString::from(format!("build-details-open-{}", self.id)),
            cx,
            |_, _| false,
        );
        let open = *state.read(cx);
        let selector = format!("build-details-toggle-{}", self.id);
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(5.))
            .child(
                Button::new(SharedString::from(format!("build-details-{}", self.id)))
                    .ghost()
                    .compact()
                    .h(px(28.))
                    .label(if open { "Hide details" } else { "Details" })
                    .accessibility_label(if open {
                        "Hide build details"
                    } else {
                        "Show build details"
                    })
                    .child(
                        div().debug_selector(move || selector.clone()).child(
                            Icon::new(if open {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(12.)),
                        ),
                    )
                    .on_click(move |_, _, cx| {
                        state.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        })
                    }),
            )
            .when(open, |this| this.child(self.body))
    }
}
fn detail_line(label: &str, value: String, palette: Palette) -> Div {
    div()
        .min_w_0()
        .whitespace_normal()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(div().text_size(px(11.)).child(value))
}
fn summary_preview(summary: &str) -> String {
    summary.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn details(record: &Value, id: String, palette: Palette, extra: Vec<String>) -> Div {
    let detail_selector = format!("build-details-body-{id}");
    let mut body = div()
        .debug_selector(move || detail_selector.clone())
        .min_w_0()
        .whitespace_normal()
        .w_full()
        .p(px(10.))
        .rounded(px(8.))
        .bg(crate::design::surface_color(palette.panel))
        .grid()
        .grid_cols(2)
        .gap(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(palette.muted))
                        .child("Command"),
                )
                .child(code(field(record, "command"), palette)),
        )
        .child(detail_line("Working folder", field(record, "cwd"), palette))
        .child(detail_line(
            "Build resource",
            field(record, "targetKey"),
            palette,
        ));
    for title in extra {
        body = body.child(detail_line("Included request", title, palette));
    }
    if let Some(exit) = record["exitCode"].as_i64() {
        body = body.child(detail_line("Exit code", exit.to_string(), palette));
    }
    let summary = field(record, "summary");
    if !summary.is_empty() {
        // Keep the complete backend report, including line breaks and indentation.
        body = body.child(div().col_span(2).child(detail_line(
            "Build report",
            summary.clone(),
            palette,
        )));
    }
    let summary_selector = format!("build-summary-{id}");
    let exit_selector = format!("build-exit-{id}");
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(5.))
        .when(!summary.is_empty(), |row| {
            row.child(
                div()
                    .debug_selector(move || summary_selector.clone())
                    .text_size(px(11.))
                    .text_color(rgb(palette.muted))
                    .line_clamp(2)
                    .child(summary_preview(&summary)),
            )
        })
        .when_some(record["exitCode"].as_i64(), |row, exit| {
            row.child(
                div()
                    .debug_selector(move || exit_selector.clone())
                    .flex()
                    .items_start()
                    .child(code(format!("Exit {exit}"), palette)),
            )
        })
        .child(BuildDetails { id, body })
}
fn task_metadata(snapshot: &Value, task: &str, count: usize, palette: Palette) -> Div {
    div().font_family(crate::design::FONT_MONO)
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .text_color(rgb(palette.muted))
        .child(div().min_w_0().truncate().child(task_label(snapshot, task)))
        .when(count > 1, |row| {
            row.child(
                div()
                    .flex_shrink_0()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(8.))
                    .bg(crate::design::surface_color(palette.panel))
                    .child(format!("{count} combined")),
            )
        })
}

pub(crate) fn content(snapshot: &Value, project_id: &str, palette: Palette) -> Div {
    let mut body = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .text_size(px(12.))
        .text_color(rgb(palette.ink));
    let requests: Vec<_> = rows(snapshot, "buildRequests")
        .filter(|row| row["projectId"] == project_id)
        .collect();
    let mut runs: Vec<_> = rows(snapshot, "buildRuns")
        .filter(|row| row["projectId"] == project_id)
        .collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run["createdAt"].as_i64().unwrap_or(0)));
    if requests.is_empty() && runs.is_empty() {
        return body.child(div().text_color(rgb(palette.muted)).child("No builds requested yet. Agents will post requests here and combine compatible ready changes."));
    }
    for (active, title) in [(true, "Building"), (false, "Recent builds")] {
        let group: Vec<_> = runs
            .iter()
            .filter(|run| (run["status"] == "running") == active)
            .take(if active { usize::MAX } else { 15 })
            .collect();
        if group.is_empty() {
            continue;
        }
        body = body.child(heading(title, palette));
        for run in group {
            let now = chrono::Utc::now().timestamp_millis();
            let status = run_status(run, now);
            let stalled = status == "Needs attention";
            let members: Vec<_> = run["requestIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let titles: Vec<_> = members
                .iter()
                .filter_map(|request_id| {
                    requests
                        .iter()
                        .find(|request| request["id"] == *request_id)
                        .map(|request| field(request, "title"))
                        .filter(|title| !title.is_empty())
                })
                .collect();
            let title = titles.first().cloned().unwrap_or_else(|| "Build".into());
            let selector = format!("build-run-{}", field(run, "id"));
            let mut row = div()
                .debug_selector(move || selector.clone())
                .min_w_0()
                .py(px(10.))
                .border_b_1()
                .border_color(rgb(palette.line))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_start()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(140.))
                                .whitespace_normal()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        )
                        .child(status_badge(status, run, palette)),
                )
                .child(task_metadata(
                    snapshot,
                    &field(run, "ownerTaskId"),
                    members.len(),
                    palette,
                ));
            if stalled {
                row = row.child(
                    div()
                        .text_size(px(11.))
                        .child("Heartbeat expired. Build slot remains reserved."),
                );
            }
            row = row.child(details(
                run,
                field(run, "id"),
                palette,
                if titles.len() > 1 { titles } else { Vec::new() },
            ));
            body = body.child(row);
        }
    }
    let waiting: Vec<_> = requests
        .iter()
        .filter(|request| matches!(request["status"].as_str(), Some("planned" | "ready")))
        .collect();
    if !waiting.is_empty() {
        body = body.child(heading("Waiting requests", palette));
    }
    for request in waiting {
        let selector = format!("build-request-{}", field(request, "id"));
        body = body.child(
            div()
                .debug_selector(move || selector.clone())
                .min_w_0()
                .py(px(10.))
                .border_b_1()
                .border_color(rgb(palette.line))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_start()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(140.))
                                .whitespace_normal()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(field(request, "title")),
                        )
                        .child(status_badge(
                            request["status"].as_str().unwrap_or(""),
                            request,
                            palette,
                        )),
                )
                .child(task_metadata(
                    snapshot,
                    &field(request, "taskId"),
                    1,
                    palette,
                ))
                .child(details(request, field(request, "id"), palette, Vec::new())),
        );
    }
    let mut recent: Vec<_> = requests
        .iter()
        .filter(|request| {
            request["buildRunId"].is_null()
                && matches!(
                    request["status"].as_str(),
                    Some("cancelled" | "failed" | "succeeded")
                )
        })
        .collect();
    recent.sort_by_key(|request| std::cmp::Reverse(request["updatedAt"].as_i64().unwrap_or(0)));
    if !recent.is_empty() {
        body = body.child(heading("Recent requests", palette));
    }
    for request in recent.into_iter().take(15) {
        body = body.child(
            div()
                .min_w_0()
                .py(px(5.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(field(request, "title")),
                        )
                        .child(status_badge(
                            request["status"].as_str().unwrap_or(""),
                            request,
                            palette,
                        )),
                )
                .child(task_metadata(
                    snapshot,
                    &field(request, "taskId"),
                    1,
                    palette,
                ))
                .child(details(request, field(request, "id"), palette, Vec::new())),
        );
    }
    body
}
#[cfg(test)]
mod tests {
    use super::{
        content, pending_count, project, run_status, stalled_runs, status_badge, status_label,
        timestamp_label,
    };
    use gpui_kit::prelude::*;
    use gpui_kit::{
        AppContext as _, Context, IntoElement, Render, TestAppContext, Window, WindowAppearance,
        div,
    };
    use serde_json::json;
    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn design_system_queue_renders_active_and_waiting_rows_without_other_project_data(cx: &mut TestAppContext) {
        struct Fixture;
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let snapshot = json!({"buildRuns":[{"id":"active","projectId":"p","status":"running","command":"cargo test","requestIds":[],"leaseExpiresAt":i64::MAX}],"buildRequests":[{"id":"waiting","projectId":"p","status":"ready","command":"npm run build","title":"New layout"},{"id":"private","projectId":"other","status":"ready","command":"private"}]});
                div().w(gpui_kit::px(600.)).child(content(
                    &snapshot,
                    "p",
                    crate::design::palette(&json!({}), WindowAppearance::Dark),
                ))
            }
        }
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| Fixture);
            gpui_kit::component::Root::new(view, window, cx)
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let active = visual
            .debug_bounds("build-run-active")
            .expect("active build visible");
        let waiting = visual
            .debug_bounds("build-request-waiting")
            .expect("waiting request visible");
        assert!(waiting.top() >= active.bottom());
        assert!(visual.debug_bounds("build-request-private").is_none());
    }
    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn design_system_details_disclosure_mounts_report_and_retains_open_state(cx: &mut TestAppContext) {
        struct Fixture;
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let snapshot = json!({"buildRuns":[{"id":"report","projectId":"p","status":"failed","exitCode":7,"summary":"First line\n  indented line\n\nlast line","command":"cargo test","requestIds":[]} ]});
                div().w(gpui_kit::px(600.)).child(content(
                    &snapshot,
                    "p",
                    crate::design::palette(&json!({}), WindowAppearance::Dark),
                ))
            }
        }
        cx.update(gpui_kit::init);
        let (_, mut visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| Fixture);
            gpui_kit::component::Root::new(view, window, cx)
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("build-summary-report").is_some());
        assert!(
            visual.debug_bounds("build-exit-report").is_some(),
            "exit code visible before disclosure"
        );
        assert!(visual.debug_bounds("build-details-body-report").is_none());
        let toggle = visual
            .debug_bounds("build-details-toggle-report")
            .expect("compact disclosure");
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("build-details-body-report").is_some());
        // A subsequent paint must retain keyed disclosure state.
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("build-details-body-report").is_some());
        let toggle = visual.debug_bounds("build-details-toggle-report").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("build-details-body-report").is_none());
        assert!(visual.debug_bounds("build-summary-report").is_some());
    }
    #[test]
    fn design_system_status_projection_never_calls_expired_reservation_building() {
        assert_eq!(
            run_status(&json!({"status":"running","leaseExpiresAt":99}), 100),
            "Needs attention"
        );
        assert_eq!(
            run_status(&json!({"status":"running","leaseExpiresAt":100}), 100),
            "running"
        );
        assert_eq!(run_status(&json!({"status":"running"}), 100), "running");
        assert_eq!(
            run_status(&json!({"status":"failed","leaseExpiresAt":99}), 100),
            "failed"
        );
        assert_eq!(status_label("running"), "Building");
        assert_eq!(status_label("Needs attention"), "Needs attention");
    }
    #[test]
    fn design_system_timestamps_use_local_day_and_preserve_started_time_for_expired_runs() {
        let now = crate::timestamps::local_datetime(1_760_000_000_000).unwrap();
        assert!(!timestamp_label(now, now).contains(','));
        assert!(timestamp_label(now - chrono::Duration::days(1), now).contains(','));
        let palette = crate::design::palette(&json!({}), WindowAppearance::Dark);
        let record = json!({"startedAt":1_760_000_000_000_i64,"updatedAt":1_760_100_000_000_i64});
        assert_eq!(
            status_badge("running", &record, palette).time,
            status_badge("Needs attention", &record, palette).time
        );
        assert!(status_badge("failed", &json!({}), palette).time.is_none());
    }
    #[test]
    fn design_system_summary_preview_is_one_paragraph_without_changing_full_report() {
        let report = "First line\n  indented line\n\nlast line";
        assert_eq!(super::summary_preview(report), "First line indented line last line");
        assert_eq!(report, "First line\n  indented line\n\nlast line");
    }
    #[test]
    fn design_system_queue_is_project_scoped_and_requires_opt_in() {
        let mut snapshot = json!({"tasks":[{"id":"a","projectId":"p"},{"id":"b","projectId":"q"}],"projects":[{"id":"p","buildManagementEnabled":true},{"id":"q","buildManagementEnabled":false}],"buildRequests":[{"projectId":"p","status":"ready"},{"projectId":"p","status":"succeeded"},{"projectId":"q","status":"running"}]});
        assert!(project(&snapshot, Some("a")).is_some());
        assert!(project(&snapshot, Some("b")).is_none());
        assert!(project(&snapshot, None).is_none());
        assert_eq!(pending_count(&snapshot, "p"), 1);
        snapshot["projects"][0]["buildManagementEnabled"] = json!(false);
        assert!(project(&snapshot, Some("a")).is_none());
        snapshot["buildRuns"] =
            json!([{"id":"run","projectId":"p","status":"running","leaseExpiresAt":0}]);
        assert!(
            project(&snapshot, Some("a")).is_some(),
            "disabled project retains active build recovery access"
        );
        assert_eq!(stalled_runs(&snapshot, "p").len(), 1);
        assert!(stalled_runs(&snapshot, "q").is_empty());
    }
}
