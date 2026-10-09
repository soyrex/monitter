//! Read-only, task-scoped Git status and diff inspection.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, button::*, text::TextView},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

static STATUS_SESSION: AtomicU64 = AtomicU64::new(1);

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn changed(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

fn display_path(path: &str) -> String {
    let mut chars = path.chars();
    let head: String = chars.by_ref().take(42).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn literal_diff_markdown(source: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in source.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    let newline = if source.ends_with('\n') { "" } else { "\n" };
    format!("{fence}\n{source}{newline}{fence}")
}

fn file_rows(status: &Value) -> Vec<Value> {
    let mut result = Vec::new();
    for (scope, label) in [
        ("staged", "Staged"),
        ("unstaged", "Changes"),
        ("untracked", "Untracked"),
    ] {
        let group: Vec<_> = rows(status, "files")
            .into_iter()
            .filter(|file| match scope {
                "staged" => {
                    changed(file, "indexStatus") && !file["untracked"].as_bool().unwrap_or(false)
                }
                "unstaged" => {
                    changed(file, "worktreeStatus") && !file["untracked"].as_bool().unwrap_or(false)
                }
                _ => file["untracked"].as_bool().unwrap_or(false),
            })
            .collect();
        if !group.is_empty() {
            result.push(json!({"kind":"heading","label":label,"count":group.len()}));
            result.extend(group.into_iter().map(|file| json!({
                "kind":"file", "path":file["path"], "originalPath":file["originalPath"],
                "scope":scope, "indexStatus":file["indexStatus"],"worktreeStatus":file["worktreeStatus"],
                "untracked":file["untracked"]
            })));
        }
    }
    result
}

pub struct GitPane {
    service: DesktopService,
    snapshot: Value,
    task_id: Option<String>,
    generation: u64,
    status_request: u64,
    diff_request: u64,
    status: Option<Value>,
    files: Vec<Value>,
    file_list: ListState,
    selected: Option<(String, String)>,
    diff: Option<Value>,
    diff_error: Option<String>,
    status_error: Option<String>,
    status_busy: bool,
    diff_busy: bool,
    compact_layout: bool,
}

impl GitPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            service,
            snapshot: snapshot.clone(),
            task_id: None,
            generation: 0,
            status_request: 0,
            diff_request: 0,
            status: None,
            files: vec![],
            file_list: ListState::new(0, ListAlignment::Top, px(120.)),
            selected: None,
            diff: None,
            diff_error: None,
            status_error: None,
            status_busy: false,
            diff_busy: false,
            // Assume a normal desktop-width pane until its own first canvas
            // measurement arrives; narrow dock splits switch after layout.
            compact_layout: false,
        }
    }

    pub fn set_context(&mut self, snapshot: &Value, task_id: &str, cx: &mut Context<Self>) {
        let changed_task = self.task_id.as_deref() != Some(task_id);
        self.snapshot = snapshot.clone();
        if changed_task {
            self.task_id = Some(task_id.to_owned());
            self.generation += 1;
            self.status_request += 1;
            self.diff_request += 1;
            self.status_busy = false;
            self.diff_busy = false;
            self.status = None;
            self.files.clear();
            self.file_list.reset(0);
            self.selected = None;
            self.diff = None;
            self.status_error = None;
            self.diff_error = None;
            self.refresh_status(cx);
        }
        cx.notify();
    }

    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let Some(task_id) = self.task_id.clone() else {
            return;
        };
        if self.status_busy {
            return;
        }
        self.status_busy = true;
        self.status_error = None;
        self.status_request += 1;
        let request = self.status_request;
        let generation = self.generation;
        let session = format!(
            "gpui-git-{}",
            STATUS_SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            service.invoke(
                "get_task_git_status",
                json!({"taskId":task_id,"detectorSession":session}),
            )
        });
        cx.spawn(async move |pane, cx| {
            let result = task.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation || pane.status_request != request {
                    return;
                }
                pane.status_busy = false;
                match result {
                    Ok(status) => {
                        pane.status = Some(status.clone());
                        pane.files = file_rows(&status);
                        pane.file_list.reset(pane.files.len());
                        if let Some((path, scope)) = pane.selected.clone() {
                            let still_valid = pane.files.iter().any(|f| {
                                field(f, "kind") == "file"
                                    && field(f, "path") == path
                                    && field(f, "scope") == scope
                            });
                            if !still_valid {
                                pane.selected = None;
                                pane.diff = None;
                                pane.diff_error = None;
                            } else {
                                pane.load_diff(path, scope, cx);
                            }
                        }
                    }
                    Err(error) => {
                        pane.status_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_file(&mut self, path: String, scope: String, cx: &mut Context<Self>) {
        self.selected = Some((path.clone(), scope.clone()));
        self.diff = None;
        self.diff_error = None;
        self.load_diff(path, scope, cx);
        cx.notify();
    }

    fn set_compact_layout(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact_layout != compact {
            self.compact_layout = compact;
            cx.notify();
        }
    }

    fn load_diff(&mut self, path: String, scope: String, cx: &mut Context<Self>) {
        let Some(task_id) = self.task_id.clone() else {
            return;
        };
        self.diff_busy = true;
        self.diff_error = None;
        self.diff_request += 1;
        let request = self.diff_request;
        let generation = self.generation;
        let service = self.service.clone();
        let task = cx.background_executor().spawn(async move {
            service.invoke(
                "get_task_git_diff",
                json!({"taskId":task_id,"path":path,"scope":scope}),
            )
        });
        cx.spawn(async move |pane, cx| {
            let result = task.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation || pane.diff_request != request {
                    return;
                }
                pane.diff_busy = false;
                match result {
                    Ok(diff) => pane.diff = Some(diff),
                    Err(error) => pane.diff_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for GitPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = crate::design::palette(&self.snapshot["settings"], window.appearance());
        let interface_scale = crate::design::interface_scale();
        let interface_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let measurement_owner = cx.weak_entity();
        let measurement = canvas(
            move |bounds, window, cx| {
                if bounds.size.width > px(0.) {
                    let width_design =
                        bounds.size.width / crate::design::ui_px(1., interface_scale);
                    let compact = width_design < 720.;
                    let owner = measurement_owner.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = owner.update(cx, |pane, cx| pane.set_compact_layout(compact, cx));
                    });
                }
            },
            |_, _, _, _| (),
        )
        .absolute()
        .size_full();
        let mut root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .gap_0()
            .bg(crate::design::surface_color(palette.paper))
            .text_color(rgb(palette.ink))
            .text_size(px(11. * interface_ratio))
            .line_height(relative(1.3))
            .child(measurement);
        let status = self.status.clone();
        let branch = status
            .as_ref()
            .and_then(|v| v["branch"].as_str())
            .unwrap_or("Git")
            .to_owned();
        root = root.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_2()
                .border_b_1()
                .border_color(rgb(palette.line))
                .child(div().font_weight(FontWeight::MEDIUM).child(branch))
                .child(
                    Button::new("git-refresh")
                        .compact()
                        .icon(Icon::new(IconName::RefreshCw).with_size(px(14.)))
                        .accessibility_label("Refresh Git status")
                        .tooltip(if self.status_busy {
                            "Refreshing…"
                        } else {
                            "Refresh"
                        })
                        .disabled(self.status_busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.refresh_status(cx))),
                ),
        );
        if let Some(error) = &self.status_error {
            root = root.child(
                div()
                    .text_color(rgb(0xdc2626))
                    .child(format!("Git status unavailable: {error}")),
            );
        }
        if self.task_id.is_none() {
            return root.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Select a task to inspect its Git working tree."),
            );
        }
        if self.status_busy && self.status.is_none() {
            return root.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Loading task Git status…"),
            );
        }
        let Some(status) = self.status.clone() else {
            return root.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Git status is unavailable."),
            );
        };
        if !status["repository"].as_bool().unwrap_or(false) {
            return root.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("This task folder is not currently a Git repository."),
            );
        }
        let root_path = field(&status, "root");
        let mut body = div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .gap_0()
            .overflow_hidden();
        if self.compact_layout {
            body = body.flex_col();
        }
        let files = self.files.clone();
        let selected = self.selected.clone();
        let pane = cx.weak_entity();
        let file_list = list(self.file_list.clone(), move |index, _, app| {
            let row = &files[index];
            if field(row, "kind") == "heading" {
                return div()
                    .px(px(4.))
                    .pt(px(10.))
                    .pb(px(3.))
                    .text_size(px(10. * interface_ratio))
                    .line_height(relative(1.2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{}  {}",
                        field(row, "label"),
                        row["count"].as_u64().unwrap_or(0)
                    ))
                    .into_any_element();
            }
            let path = field(row, "path");
            let scope = field(row, "scope");
            let active = selected
                .as_ref()
                .is_some_and(|(p, s)| p == &path && s == &scope);
            let label = display_path(&path);
            let pane = pane.clone();
            Button::new(SharedString::from(format!("git-file-{scope}-{path}")))
                .accessibility_label(format!("Review {scope} changes: {path}"))
                .compact()
                .w_full()
                .h(px(25.))
                .px(px(4.))
                .py(px(3.))
                .gap(px(5.))
                .justify_start()
                .rounded(px(4.))
                .border_0()
                .custom(
                    ButtonCustomVariant::new(app)
                        .color(crate::design::surface_color(if active { palette.soft } else { palette.paper }).into())
                        .foreground(rgb(palette.ink).into())
                        .hover(crate::design::surface_color(palette.soft).into())
                        .active(crate::design::surface_color(palette.soft).into()),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(Icon::new(IconName::FileText).with_size(px(12.)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11. * interface_ratio))
                                .line_height(relative(1.3))
                                .child(label),
                        ),
                )
                .tooltip(path.clone())
                .on_click(move |_, _, app| {
                    let _ = pane.update(app, |pane, cx| {
                        pane.select_file(path.clone(), scope.clone(), cx)
                    });
                })
                .into_any_element()
        })
        .flex_1();
        let mut file_panel = div()
            .flex()
            .flex_col()
            .gap_0()
            .p(px(6.))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(10. * interface_ratio))
                    .line_height(relative(1.2))
                    .text_color(rgb(palette.muted))
                    .child(root_path),
            )
            .child(file_list);
        if self.compact_layout {
            file_panel = file_panel
                .w_full()
                .h(relative(0.4))
                .min_h(px(90.))
                .flex_none()
                .border_b_1()
                .border_color(rgb(palette.line));
        } else {
            file_panel = file_panel
                .w(relative(0.32))
                .min_w(px(180.))
                .flex_none()
                .border_r_1()
                .border_color(rgb(palette.line));
        }
        if self.files.is_empty() {
            file_panel = file_panel.child(
                div()
                    .p(px(10.))
                    .text_size(px(11. * interface_ratio))
                    .line_height(relative(1.3))
                    .text_color(rgb(palette.muted))
                    .child("Working tree is clean."),
            );
        }
        if status["truncated"].as_bool().unwrap_or(false) {
            file_panel = file_panel.child(
                div()
                    .p(px(10.))
                    .text_size(px(11. * interface_ratio))
                    .text_color(rgb(palette.muted))
                    .child("File list truncated."),
            );
        }
        body = body.child(file_panel);

        let mut diff_panel = div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap_0()
            .overflow_hidden();
        match (&self.selected, &self.diff_error, &self.diff, self.diff_busy) {
            (None, _, _, _) => {
                diff_panel = diff_panel.child(
                    div()
                        .p(px(10.))
                        .text_size(px(11. * interface_ratio))
                        .line_height(relative(1.3))
                        .text_color(rgb(palette.muted))
                        .child("Select a changed file to inspect its diff."),
                )
            }
            (_, _, _, true) => {
                diff_panel = diff_panel.child(
                    div()
                        .p(px(10.))
                        .text_size(px(11. * interface_ratio))
                        .text_color(rgb(palette.muted))
                        .child("Loading diff…"),
                )
            }
            (_, Some(error), _, _) => {
                diff_panel = diff_panel.child(
                    div()
                        .p(px(10.))
                        .text_size(px(11. * interface_ratio))
                        .text_color(rgb(0xdc2626))
                        .child(format!("Diff unavailable: {error}")),
                )
            }
            (_, _, Some(diff), _) if diff["binary"].as_bool().unwrap_or(false) => {
                diff_panel = diff_panel.child(
                    div()
                        .p(px(10.))
                        .text_size(px(11. * interface_ratio))
                        .text_color(rgb(palette.muted))
                        .child("This file is binary and cannot be shown as a text diff."),
                )
            }
            (_, _, Some(diff), _) if diff["repository"].as_bool().unwrap_or(false) => {
                if diff["truncated"].as_bool().unwrap_or(false) {
                    diff_panel = diff_panel.child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child("Diff truncated by the backend (512 KiB limit)."),
                    );
                }
                let text = literal_diff_markdown(diff["text"].as_str().unwrap_or(""));
                let id = self
                    .selected
                    .as_ref()
                    .map(|(path, scope)| format!("git-diff-{scope}-{path}"))
                    .unwrap_or_else(|| "git-diff".into());
                diff_panel = diff_panel.child(
                    div()
                        .id(SharedString::from(format!("git-diff-scroll-{id}")))
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .text_size(px(11. * interface_ratio))
                        .line_height(relative(1.3))
                        .child(
                            TextView::markdown(SharedString::from(id), text)
                                .selectable(true)
                                .scrollable(false),
                        ),
                );
            }
            _ => diff_panel = diff_panel.child(
                div()
                    .p(px(10.))
                    .text_size(px(11. * interface_ratio))
                    .text_color(rgb(palette.muted))
                    .child(
                        "The repository or change is no longer available. Refresh to check again.",
                    ),
            ),
        }
        root.child(body.child(diff_panel))
    }
}

#[cfg(test)]
mod tests {
    use super::literal_diff_markdown;

    #[::core::prelude::v1::test]
    fn diff_text_is_fenced_without_interpreting_markdown_or_embedded_fences() {
        let source =
            "# heading\n- item\n[link](https://example.test)\n```rust\nfn main() {}\n```\n";
        let rendered = literal_diff_markdown(source);
        assert!(rendered.starts_with("````\n"));
        assert!(rendered.ends_with("\n````"));
        assert!(rendered.contains(source));
        assert!(!rendered.contains("````\nfn main"));
    }
}
