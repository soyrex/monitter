//! Read-only, task-scoped changes drawer and native Git diff review.
use crate::design::{self, ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    base::{ElementExt as _, TextSelectionGroupId},
    component::{ActiveTheme as _, Disableable, Icon, Selectable, Sizable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[path = "git_diff_model.rs"]
mod model;
#[path = "git_diff_view.rs"]
mod view;
use model::{DiffDocument, DiffOptions, DiffProjection, DiffRow};
static STATUS_SESSION: AtomicU64 = AtomicU64::new(1);
fn fixture_bounds(d: Div, name: &'static str) -> Div {
    #[cfg(test)]
    {
        return d.debug_selector(move || name.into());
    }
    #[cfg(not(test))]
    {
        let _ = name;
        d
    }
}
fn field(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().into()
}
fn changed(v: &Value, k: &str) -> bool {
    v[k].as_str().is_some_and(|s| !s.trim().is_empty())
}
fn stat(row: &Value, key: &str) -> Option<u64> {
    row["stats"][key].as_u64()
}
fn file_rows(status: &Value) -> Vec<Value> {
    let mut result = vec![];
    for file in status["files"].as_array().into_iter().flatten() {
        for scope in ["staged", "unstaged", "untracked"] {
            let untracked = file["untracked"].as_bool().unwrap_or(false);
            if !match scope {
                "staged" => !untracked && changed(file, "indexStatus"),
                "unstaged" => !untracked && changed(file, "worktreeStatus"),
                _ => untracked,
            } {
                continue;
            }
            let key = if scope == "staged" {
                "stagedStats"
            } else {
                "unstagedStats"
            };
            let badge: String = if untracked {
                "A".into()
            } else {
                field(
                    file,
                    if scope == "staged" {
                        "indexStatus"
                    } else {
                        "worktreeStatus"
                    },
                )
            };
            result.push(json!({"path":file["path"],"originalPath":file["originalPath"],"scope":scope,"badge":badge,"stats":file[key],"newFile":untracked || field(file,"indexStatus") == "A" || badge == "A"}));
        }
    }
    result.sort_by(|a, b| {
        // Known churn first; generated files stay visible but below authored code.
        let generated =
            |r: &Value| model::classify_path(r["path"].as_str().unwrap_or("")).is_some();
        generated(a)
            .cmp(&generated(b))
            .then_with(|| {
                let churn = |r: &Value| {
                    stat(r, "additions")
                        .zip(stat(r, "deletions"))
                        .map(|(a, d)| a.saturating_add(d))
                };
                churn(b).cmp(&churn(a))
            })
            .then_with(|| field(a, "path").cmp(&field(b, "path")))
            .then_with(|| field(a, "scope").cmp(&field(b, "scope")))
    });
    result
}
fn path_parts(path: &str) -> (String, String) {
    path.rsplit_once('/')
        .map(|(p, n)| (format!("{p}/"), n.into()))
        .unwrap_or_else(|| (String::new(), path.into()))
}
fn reset_view(pane: &mut GitPane) {
    pane.options = DiffOptions::default();
    pane.generated_open = false;
    pane.file_open = true;
    pane.full_context = false;
    pane.rebuild_diff();
}

pub struct GitPane {
    service: DesktopService,
    snapshot: Value,
    task_id: Option<String>,
    generation: u64,
    status_request: u64,
    diff_request: u64,
    status: Option<Value>,
    files: Arc<Vec<Value>>,
    file_list: ListState,
    visible_files: usize,
    selected: Option<(String, String)>,
    diff: Option<Value>,
    document: Arc<DiffDocument>,
    projection: Arc<DiffProjection>,
    changes_only: bool,
    changes_limit: usize,
    diff_list: ListState,
    options: DiffOptions,
    generated_open: bool,
    file_open: bool,
    full_context: bool,
    diff_error: Option<String>,
    status_error: Option<String>,
    status_busy: bool,
    diff_busy: bool,
    compact_layout: bool,
    width_design: f32,
    selection_group: TextSelectionGroupId,
}
impl GitPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Self {
        Self {
            service,
            snapshot: json!({"settings":snapshot["settings"]}),
            task_id: None,
            generation: 0,
            status_request: 0,
            diff_request: 0,
            status: None,
            files: Arc::new(vec![]),
            file_list: ListState::new(0, ListAlignment::Top, px(120.)),
            visible_files: 80,
            selected: None,
            diff: None,
            document: Arc::new(DiffDocument::default()),
            projection: Arc::new(DiffProjection::default()),
            changes_only: false,
            changes_limit: 24,
            diff_list: ListState::new(0, ListAlignment::Top, px(120.)),
            options: DiffOptions::default(),
            generated_open: false,
            file_open: true,
            full_context: false,
            diff_error: None,
            status_error: None,
            status_busy: false,
            diff_busy: false,
            compact_layout: false,
            width_design: 900.,
            selection_group: TextSelectionGroupId::new(),
        }
    }
    pub fn set_context(&mut self, snapshot: &Value, task_id: &str, cx: &mut Context<Self>) {
        self.snapshot = json!({"settings":snapshot["settings"]});
        if self.task_id.as_deref() != Some(task_id) {
            self.task_id = Some(task_id.into());
            self.generation += 1;
            self.status_request += 1;
            self.diff_request += 1;
            self.status_busy = false;
            self.diff_busy = false;
            self.status = None;
            self.files = Arc::new(vec![]);
            self.file_list.reset(0);
            self.selected = None;
            self.diff = None;
            self.document = Arc::new(DiffDocument::default());
            reset_view(self);
            self.diff_error = None;
            self.status_error = None;
            self.changes_only = false;
            self.visible_files = 80;
            self.refresh_status(cx);
        }
        cx.notify();
    }
    fn rebuild_diff(&mut self) {
        self.projection = Arc::new(self.document.project(&self.options));
        self.diff_list.reset(self.projection.rows.len());
    }
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.task_id.clone() else {
            return;
        };
        if self.status_busy || self.diff_busy {
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
        let result = cx.background_executor().spawn(async move {
            service.invoke(
                "get_task_git_status",
                json!({"taskId":task,"detectorSession":session,"includeStats":true}),
            )
        });
        cx.spawn(async move |pane, cx| {
            let result = result.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation || pane.status_request != request {
                    return;
                }
                pane.status_busy = false;
                match result {
                    Ok(status) => {
                        pane.files = Arc::new(file_rows(&status));
                        pane.status = Some(status);
                        pane.file_list
                            .reset(pane.files.len().min(pane.visible_files));
                        let selected = pane.selected.clone().filter(|(p, s)| {
                            pane.files
                                .iter()
                                .any(|r| field(r, "path") == *p && field(r, "scope") == *s)
                        });
                        if pane.changes_only {
                            pane.load_changes(cx);
                        } else if let Some((p, s)) = selected {
                            pane.load_diff(p, s, cx);
                        } else if let Some(row) = pane.files.first() {
                            pane.select_file(field(row, "path"), field(row, "scope"), cx);
                        } else {
                            pane.selected = None;
                            pane.diff = None;
                            pane.document = Arc::new(DiffDocument::default());
                            pane.rebuild_diff();
                        }
                    }
                    Err(e) => pane.status_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn select_file(&mut self, path: String, scope: String, cx: &mut Context<Self>) {
        self.changes_only = false;
        self.selected = Some((path.clone(), scope.clone()));
        self.diff = None;
        self.document = Arc::new(DiffDocument::default());
        reset_view(self);
        self.diff_error = None;
        // Lock/generated content requires a deliberate second click to load.
        if model::classify_path(&path).is_none() {
            self.load_diff(path, scope, cx);
        } else {
            self.diff_request += 1;
            self.diff_busy = false;
        }
        cx.notify();
    }
    fn load_diff(&mut self, path: String, scope: String, cx: &mut Context<Self>) {
        let context = if self.full_context { 1000 } else { 3 };
        self.load_diff_context(path, scope, context, cx);
    }
    fn load_diff_context(
        &mut self,
        path: String,
        scope: String,
        context: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.task_id.clone() else {
            return;
        };
        self.diff_busy = true;
        self.diff_error = None;
        self.diff_request += 1;
        let request = self.diff_request;
        let generation = self.generation;
        let service = self.service.clone();
        let result = cx.background_executor().spawn(async move {
            service.invoke(
                "get_task_git_diff",
                json!({"taskId":task,"path":path,"scope":scope,"contextLines":context}),
            )
        });
        cx.spawn(async move |pane, cx| {
            let result = result.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation || pane.diff_request != request {
                    return;
                }
                pane.diff_busy = false;
                match result {
                    Ok(diff) => {
                        pane.full_context = context > 3 && context_acknowledged(&diff, context);
                        if context > 3 && !pane.full_context {
                            pane.diff_error = Some("This server cannot expand diff context. Update the backend to enable it.".into());
                        }
                        if pane.full_context { pane.options.expanded_folds = (0..1000).collect(); }
                        pane.document =
                            Arc::new(DiffDocument::parse(diff["text"].as_str().unwrap_or("")));
                        pane.diff = Some(diff);
                        pane.rebuild_diff();
                    }
                    Err(e) => pane.diff_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load_changes(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.task_id.clone() else {
            return;
        };
        self.diff_request += 1;
        let request = self.diff_request;
        let generation = self.generation;
        let files = self.files.clone();
        let limit = self.changes_limit;
        let service = self.service.clone();
        self.diff_busy = true;
        self.projection = Arc::new(DiffProjection::default());
        self.diff_list.reset(0);
        let result = cx.background_executor().spawn(async move {
            collect_changes(&files, limit, |path, scope| {
                service.invoke(
                    "get_task_git_diff",
                    json!({"taskId":task,"path":path,"scope":scope,"contextLines":0}),
                )
            })
        });
        cx.spawn(async move |pane, cx| {
            let projection = result.await;
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation || pane.diff_request != request {
                    return;
                }
                pane.diff_busy = false;
                pane.projection = Arc::new(projection);
                pane.diff_list.reset(pane.projection.rows.len());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn render_changes(
        &mut self,
        p: design::Palette,
        c: view::DiffColors,
        ratio: f32,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.projection.clone();
        let mut panel = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden();
        if self.diff_busy {
            return panel
                .child(div().p(px(8.)).child("Loading changes…"))
                .into_any_element();
        }
        panel = panel.child(
            list(self.diff_list.clone(), move |ix, w, cx| {
                match &rows.rows[ix] {
                    DiffRow::Code { .. } => view::render_code(
                        &rows.rows[ix],
                        ix,
                        false,
                        "git-whole-set",
                        c,
                        ratio,
                        w,
                        cx,
                    ),
                    DiffRow::Note(text) => div()
                        .w_full()
                        .min_w_0()
                        .px(px(8.))
                        .py(px(4.))
                        .whitespace_normal()
                        .text_size(px(10. * ratio))
                        .text_color(rgb(p.muted))
                        .child(text.clone())
                        .into_any_element(),
                    _ => div().into_any_element(),
                }
            })
            .w_full()
            .min_w_0()
            .flex_1()
            .min_h_0(),
        );
        if self.files.len() > self.changes_limit {
            panel = panel.child(
                Button::new("git-more-set-changes")
                    .compact()
                    .text()
                    .label(format!(
                        "Load more · {} changes remaining",
                        self.files.len() - self.changes_limit
                    ))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.changes_limit += 24;
                        pane.load_changes(cx);
                    })),
            );
        }
        panel.into_any_element()
    }
    fn option_changed(&mut self, cx: &mut Context<Self>) {
        self.rebuild_diff();
        cx.notify();
    }
    fn expand_context(&mut self, cx: &mut Context<Self>) {
        if let Some((p, s)) = self.selected.clone() {
            self.load_diff_context(p, s, 1000, cx);
        }
        cx.notify();
    }
}
/// Sequential reads bound provider pressure; per-file projection and total rows bound UI work.
fn collect_changes(
    files: &[Value],
    limit: usize,
    mut fetch: impl FnMut(&str, &str) -> Result<Value, String>,
) -> DiffProjection {
    let mut result = DiffProjection::default();
    let mut added = HashSet::new();
    for file in files.iter().take(limit) {
        if result.rows.len() >= 2400 {
            result.rows.push(DiffRow::Note("Changes view reached its 2400 row limit. Select individual files for the remaining edits.".into()));
            break;
        }
        let path = field(file, "path");
        let scope = field(file, "scope");
        if file["newFile"].as_bool().unwrap_or(false) {
            if added.insert(path.clone()) {
                result
                    .rows
                    .push(DiffRow::Note(format!("+ {path} · added file")));
            }
            continue;
        }
        result.rows.push(DiffRow::Note(format!("{path} · {scope}")));
        if model::classify_path(&path).is_some() {
            result.rows.push(DiffRow::Note(
                "Generated file · select file to load diff".into(),
            ));
            continue;
        }
        match fetch(&path, &scope) {
            Err(e) => result
                .rows
                .push(DiffRow::Note(format!("Diff unavailable: {e}"))),
            Ok(diff) => {
                if diff["binary"].as_bool().unwrap_or(false) {
                    result.rows.push(DiffRow::Note("Binary file".into()));
                    continue;
                }
                if !diff["repository"].as_bool().unwrap_or(false) {
                    result
                        .rows
                        .push(DiffRow::Note("Change no longer available".into()));
                    continue;
                }
                let doc = DiffDocument::parse(diff["text"].as_str().unwrap_or(""));
                let mut options = DiffOptions {
                    show_whitespace: true,
                    visible_hunks: usize::MAX,
                    row_limit: usize::MAX,
                    ..Default::default()
                };
                // Strip context before projection, including context from older servers.
                let mut changed_doc = doc;
                for h in &mut changed_doc.hunks {
                    h.lines.retain(|l| l.kind != model::LineKind::Context);
                }
                options.row_limit = 400.min(2400usize.saturating_sub(result.rows.len()));
                let projection = changed_doc.project(&options);
                result.rows.extend(
                    projection
                        .rows
                        .into_iter()
                        .filter(|r| matches!(r, DiffRow::Code { .. })),
                );
                if projection.hidden_rows > 0 || diff["truncated"].as_bool().unwrap_or(false) {
                    result.rows.push(DiffRow::Note(
                        "Diff truncated · select file for more".into(),
                    ));
                }
                if changed_doc.hunks.is_empty() {
                    result.rows.push(DiffRow::Note(
                        "No text edits · rename or mode change".into(),
                    ));
                }
            }
        }
    }
    result
}
fn context_acknowledged(diff: &Value, requested: u32) -> bool {
    diff["contextLines"]
        .as_u64()
        .is_some_and(|actual| actual >= u64::from(requested))
}
fn stats_view(add: Option<u64>, del: Option<u64>, colors: view::DiffColors, ratio: f32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_shrink_0()
        .font_family(design::FONT_MONO)
        .text_size(px(11. * ratio))
        .child(
            div()
                .text_color(rgb(colors.add))
                .child(add.map(|v| format!("+{v}")).unwrap_or_else(|| "—".into())),
        )
        .child(
            div()
                .text_color(rgb(colors.del))
                .child(del.map(|v| format!("−{v}")).unwrap_or_else(|| "—".into())),
        )
}
fn status_badge(badge: &str, c: view::DiffColors, ratio: f32) -> Div {
    let color = match badge {
        "A" | "?" => c.add,
        "D" => c.del,
        _ => c.muted,
    };
    div()
        .size(px(18. * ratio))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(color).opacity(0.4))
        .font_family(design::FONT_MONO)
        .text_size(px(10. * ratio))
        .text_color(rgb(color))
        .child(badge.to_owned())
}
fn churn_bar(add: u64, del: u64, c: view::DiffColors) -> Div {
    let mut bar = div().w_full().h(px(3.)).flex().gap(px(2.));
    let total = add.saturating_add(del).max(1) as f32;
    if add > 0 {
        bar = bar.child(
            div()
                .w(relative(add as f32 / total))
                .h_full()
                .rounded(px(2.))
                .bg(rgb(c.add)),
        );
    }
    if del > 0 {
        bar = bar.child(
            div()
                .w(relative(del as f32 / total))
                .h_full()
                .rounded(px(2.))
                .bg(rgb(c.del)),
        );
    }
    bar
}
impl Render for GitPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = design::palette(&self.snapshot["settings"], window.appearance());
        let c = view::DiffColors::new(p);
        let ratio = (cx.theme().font_size / px(14.)).clamp(0.57, 2.29);
        let ui = px(10. * ratio);
        let owner = cx.weak_entity();
        let measurement = canvas(
            move |bounds, window, cx| {
                let width = bounds.size.width / px(1.);
                let owner = owner.clone();
                window.defer(cx, move |_, cx| {
                    let _ = owner.update(cx, |pane, cx| {
                        let compact = width < 720.;
                        if compact != pane.compact_layout || (width - pane.width_design).abs() > 8.
                        {
                            pane.width_design = width;
                            pane.compact_layout = compact;
                            pane.diff_list.reset(pane.projection.rows.len());
                            cx.notify();
                        }
                    });
                });
            },
            |_, _, _, _| (),
        )
        .absolute()
        .size_full();
        let unique = self
            .files
            .iter()
            .map(|r| field(r, "path"))
            .collect::<HashSet<_>>()
            .len();
        let any_stats = self.files.iter().any(|r| stat(r, "additions").is_some());
        let truncated = self.status.as_ref().is_some_and(|status| status["truncated"].as_bool().unwrap_or(false));
        let partial_stats = self.files.iter().any(|r| stat(r, "additions").is_none());
        let (add, del) = self.files.iter().fold((0u64, 0u64), |(a, d), r| {
            (
                a.saturating_add(stat(r, "additions").unwrap_or(0)),
                d.saturating_add(stat(r, "deletions").unwrap_or(0)),
            )
        });
        let branch = self
            .status
            .as_ref()
            .and_then(|v| v["branch"].as_str())
            .unwrap_or("Detached HEAD")
            .to_owned();
        let status_bar = fixture_bounds(
            div().w_full().min_w_0().flex_shrink_0().flex().flex_col()
                .border_t_1().border_color(rgb(p.line))
                .child(fixture_bounds(
                    div().w_full().min_w_0().px(px(8.)).py(px(4.))
                        .flex().flex_wrap().items_center().gap(px(6.))
                        .text_size(px(9. * ratio))
                        .child(div().flex_1().min_w(px(60.)).truncate()
                            .font_family(design::FONT_MONO).text_color(rgb(p.muted))
                            .child(branch.clone()))
                        .child(div().flex_shrink_0().text_color(rgb(p.muted))
                            .child(format!("{unique}{} files", if truncated { "+" } else { "" })))
                        .when(partial_stats, |row| row.child(fixture_bounds(
                            div().text_color(rgb(p.muted)).child("· partial stats"), "git-status-partial")))
                        .when(truncated, |row| row.child(fixture_bounds(
                            div().text_color(rgb(p.muted)).child("· truncated results"), "git-status-truncated")))
                        .child(stats_view(any_stats.then_some(add), any_stats.then_some(del), c, ratio))
                        .child(Button::new("git-refresh").compact().xsmall().ghost()
                            .icon(Icon::new(IconName::RefreshCw).with_size(px(12.)))
                            .accessibility_label("Refresh Git status").tooltip("Refresh Git status")
                            .disabled(self.status_busy)
                            .on_click(cx.listener(|pane, _, _, cx| pane.refresh_status(cx)))),
                    "git-summary-header",
                ))
                .child(churn_bar(add, del, c)),
            "git-status-bar",
        ).id("git-status-bar").role(Role::Status)
            .aria_label(format!("Git working tree: {branch}; {unique}{} changed files{}{}",
                if truncated { "+" } else { "" }, if partial_stats { "; partial statistics" } else { "" },
                if truncated { "; truncated results" } else { "" }));
        let mut root = div()
            .id("git-changes-pane")
            .tab_group()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family(cx.theme().font_family.clone())
            .text_size(ui)
            .text_color(rgb(p.ink))
            .bg(design::surface_color(p.paper))
            .child(measurement)
            .child(fixture_bounds(
                div().w_full().min_w_0().flex_shrink_0().px(px(8.)).py(px(4.))
                    .flex().items_center().gap(px(5.))
                    .border_b_1().border_color(rgb(p.line))
                    .child(Button::new("git-files-mode").compact().xsmall().ghost()
                        .text_size(px(10.)).h(px(20.)).px(px(6.)).py_0()
                        .selected(!self.changes_only).disabled(self.diff_busy).label("Files")
                        .accessibility_label("Show changed files")
                        .debug_selector(|| "git-files-mode".into())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            if pane.changes_only {
                                pane.changes_only = false;
                                if let Some((path, scope)) = pane.selected.clone() {
                                    pane.select_file(path, scope, cx);
                                }
                                cx.notify();
                            }
                        })))
                    .child(Button::new("git-changes-only").compact().xsmall().ghost()
                        .text_size(px(10.)).h(px(20.)).px(px(6.)).py_0()
                        .selected(self.changes_only).disabled(self.diff_busy).label("Changes only")
                        .accessibility_label("Show changes only")
                        .debug_selector(|| "git-changes-only-mode".into())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            if !pane.changes_only {
                                pane.changes_only = true;
                                pane.changes_limit = 24;
                                pane.load_changes(cx);
                            }
                        }))),
                "git-mode-control",
            ));
        if let Some(e) = &self.status_error {
            root = root.child(
                div()
                    .p(px(10.))
                    .text_color(rgb(c.del))
                    .child(format!("Git status unavailable: {e}")),
            );
        }
        let notice = if self.task_id.is_none() {
            Some("Select a task to inspect its Git working tree.")
        } else if self.status_busy && self.status.is_none() {
            Some("Loading task Git status…")
        } else if self
            .status
            .as_ref()
            .is_some_and(|s| !s["repository"].as_bool().unwrap_or(false))
        {
            Some("This task folder is not a Git repository.")
        } else if self.status.is_none() {
            Some("Git status is unavailable.")
        } else {
            None
        };
        if let Some(notice) = notice {
            return root
                .child(div().flex_1().min_h_0().p(px(20.)).text_color(rgb(p.muted)).child(notice))
                .child(status_bar)
                .into_any_element();
        }
        let rows = self.files.clone();
        let selected = self.selected.clone();
        let owner = cx.weak_entity();
        let file_list = list(self.file_list.clone(), move |ix, _, cx| {
            let row = &rows[ix];
            let path = field(row, "path");
            let scope = field(row, "scope");
            let active = selected
                .as_ref()
                .is_some_and(|(a, b)| *a == path && *b == scope);
            let (dir, name) = path_parts(&path);
            let generated = model::classify_path(&path);
            let binary = row["stats"].is_object() && stat(row, "additions").is_none();
            let mut line = div()
                .id(SharedString::from(format!("git-file-{scope}-{path}")))
                .w_full()
                .min_w_0()
                .h(px(26. * ratio))
                .text_size(px(10. * ratio))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .when(active, |d| d.bg(rgb(p.accent).opacity(0.14)))
                .hover(|d| d.bg(rgb(p.soft)))
                .child(status_badge(&field(row, "badge"), c, ratio))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .flex()
                        .items_center()
                        .child(div().text_color(rgb(p.muted)).truncate().child(dir))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name),
                        ),
                )
                .child(
                    div()
                        .text_size(px(9. * ratio))
                        .text_color(rgb(p.muted))
                        .child(if scope == "staged" {
                            "staged"
                        } else if scope == "untracked" {
                            "new"
                        } else {
                            ""
                        }),
                );
            if let Some(reason) = generated {
                line = line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10. * ratio))
                        .text_color(rgb(p.muted))
                        .child(if reason == "Dependency lock file" {
                            "Lock file"
                        } else {
                            "Generated"
                        }),
                );
            } else if binary {
                line = line.child(
                    div()
                        .text_size(px(10. * ratio))
                        .text_color(rgb(p.muted))
                        .child("Binary"),
                );
            } else {
                line = line.child(stats_view(
                    stat(row, "additions"),
                    stat(row, "deletions"),
                    c,
                    ratio,
                ));
                if let Some((a, d)) = stat(row, "additions").zip(stat(row, "deletions")) {
                    let total = a.saturating_add(d).max(1);
                    let greens = ((a as f64 / total as f64) * 5.).round() as usize;
                    line = line.child(div().flex().gap(px(1.)).flex_shrink_0().children(
                        (0..5).map(move |i| {
                            div().size(px(4.)).rounded(px(1.)).bg(rgb(if i < greens {
                                c.add
                            } else {
                                c.del
                            })
                            .opacity(if a + d == 0 { 0.2 } else { 0.8 }))
                        }),
                    ));
                }
            }
            let key_owner = owner.clone();
            let key_path = path.clone();
            let key_scope = scope.clone();
            let owner = owner.clone();
            line = line
                .tab_index(ix as isize)
                .focus(|d| d.bg(rgb(p.accent).opacity(0.18)))
                .on_key_down(move |event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        let _ = key_owner.update(cx, |pane, cx| {
                            pane.select_file(key_path.clone(), key_scope.clone(), cx)
                        });
                        cx.stop_propagation();
                    }
                })
                .role(Role::Button)
                .aria_selected(active)
                .aria_label(format!("Review {scope} changes: {path}"))
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |pane, cx| {
                        pane.select_file(path.clone(), scope.clone(), cx)
                    });
                });
            #[cfg(test)]
            {
                line = line.debug_selector(move || format!("git-file-row-{ix}"));
            }
            line.into_any_element()
        })
        .flex_1()
        .min_h_0();
        let mut files = div().flex().flex_col().min_h_0().overflow_hidden().child(
            crate::transcript_edge_fade::TranscriptEdgeFade::for_list(
                file_list,
                self.file_list.clone(),
                px(16.),
            ),
        );
        if self.files.is_empty() {
            files = files.child(
                div()
                    .p(px(15.))
                    .text_color(rgb(p.muted))
                    .child("Working tree is clean."),
            );
        }
        if self.files.len() > self.visible_files {
            files = files.child(
                Button::new("git-more-files")
                    .w_full()
                    .compact()
                    .text()
                    .label(format!(
                        "{} more changes",
                        self.files.len() - self.visible_files
                    ))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.visible_files += 80;
                        pane.file_list
                            .reset(pane.files.len().min(pane.visible_files));
                        cx.notify();
                    })),
            );
        }
        if self
            .status
            .as_ref()
            .is_some_and(|s| s["truncated"].as_bool().unwrap_or(false))
        {
            files = files.child(
                div()
                    .p(px(8.))
                    .text_size(px(11. * ratio))
                    .text_color(rgb(p.muted))
                    .child("Some files or statistics were truncated by the backend."),
            );
        }
        let file_panel = if self.compact_layout {
            files
                .w_full()
                .h(relative(0.35))
                .min_h(px(100.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(p.line))
        } else {
            files
                .w(relative(0.34))
                .min_w(px(240.))
                .flex_shrink_0()
                .border_r_1()
                .border_color(rgb(p.line))
        };
        let diff = if self.changes_only {
            self.render_changes(p, c, ratio, window, cx)
        } else {
            self.render_diff(p, c, ratio, window, cx)
        };
        root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_hidden()
                .when(self.compact_layout, |d| d.flex_col())
                .when(!self.changes_only, |d| d.child(file_panel))
                .child(diff),
        )
        .child(status_bar)
        .text_selection_group(self.selection_group)
        .into_any_element()
    }
}
impl GitPane {
    fn render_diff(
        &mut self,
        p: design::Palette,
        c: view::DiffColors,
        ratio: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden();
        let Some((path, scope)) = self.selected.clone() else {
            return panel
                .child(
                    div()
                        .p(px(20.))
                        .text_color(rgb(p.muted))
                        .child("Select a changed file to inspect its diff."),
                )
                .into_any_element();
        };
        let (dir, name) = path_parts(&path);
        let generated = model::classify_path(&path);
        let selected_stats = self
            .files
            .iter()
            .find(|r| field(r, "path") == path && field(r, "scope") == scope);
        let parsed = self.diff.as_ref().is_some_and(|d| {
            d["repository"].as_bool().unwrap_or(false) && !d["binary"].as_bool().unwrap_or(false)
        });
        let additions = selected_stats
            .and_then(|r| stat(r, "additions"))
            .or_else(|| parsed.then_some(self.document.additions as u64));
        let deletions = selected_stats
            .and_then(|r| stat(r, "deletions"))
            .or_else(|| parsed.then_some(self.document.deletions as u64));
        let mut collapse = div()
            .id("git-file-collapse")
            .tab_index(0)
            .on_key_down(cx.listener(|pane, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    pane.file_open = !pane.file_open;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(if self.file_open {
                "Collapse file diff"
            } else {
                "Expand file diff"
            })
            .child(
                Icon::new(if self.file_open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(px(12.)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(p.muted))
                    .child(dir),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .on_click(cx.listener(|pane, _, _, cx| {
                pane.file_open = !pane.file_open;
                cx.notify();
            }));
        #[cfg(test)]
        {
            collapse = collapse.debug_selector(|| "git-file-collapse".into());
        }
        panel = panel.child(fixture_bounds(
            div()
                .flex_shrink_0()
                .w_full()
                .min_w_0()
                .px(px(10.))
                .py(px(8.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .bg(design::surface_color(p.panel))
                .border_b_1()
                .border_color(rgb(p.line))
                .text_size(px(11. * ratio))
                .child(collapse)
                .child(stats_view(additions, deletions, c, ratio))
                .child(
                    Button::new("git-unified")
                        .compact()
                        .text()
                        .selected(!self.options.split)
                        .label("Unified")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.options.split = false;
                            pane.option_changed(cx);
                        })),
                )
                .child(
                    Button::new("git-split")
                        .compact()
                        .text()
                        .selected(self.options.split)
                        .label("Split")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.options.split = true;
                            pane.option_changed(cx);
                        })),
                )
                .child(
                    Button::new("git-whitespace")
                        .compact()
                        .text()
                        .selected(self.options.show_whitespace)
                        .label(format!("WS {}", self.projection.hidden_whitespace))
                        .tooltip("Toggle whitespace-only edits")
                        .accessibility_label("Toggle whitespace-only edits")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.options.show_whitespace = !pane.options.show_whitespace;
                            pane.option_changed(cx);
                        })),
                )
                .child(
                    Button::new("git-copy-path")
                        .compact()
                        .text()
                        .icon(Icon::new(IconName::Copy).with_size(px(13.)))
                        .tooltip("Copy path")
                        .accessibility_label("Copy file path")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
                        }),
                )
                .child(
                    Button::new("git-copy-diff")
                        .compact()
                        .text()
                        .icon(Icon::new(IconName::FileText).with_size(px(13.)))
                        .tooltip("Copy full diff")
                        .accessibility_label("Copy full diff")
                        .disabled(self.diff.is_none())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            if let Some(text) = pane.diff.as_ref().and_then(|d| d["text"].as_str())
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                            }
                        })),
                ),
            "git-diff-toolbar",
        ));
        if !self.file_open {
            return panel.into_any_element();
        }
        if let Some(reason) = generated.filter(|_| !self.generated_open) {
            return panel
                .child(
                    div()
                        .p(px(20.))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(10.))
                        .text_color(rgb(p.muted))
                        .child(format!("{reason} · diff folded by default"))
                        .child(
                            Button::new("git-show-generated")
                                .compact()
                                .label("Load file diff")
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    pane.generated_open = true;
                                    if let Some((p, s)) = pane.selected.clone() {
                                        pane.load_diff(p, s, cx);
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element();
        }
        if self.diff_busy {
            return panel
                .child(
                    div()
                        .p(px(20.))
                        .text_color(rgb(p.muted))
                        .child("Loading diff…"),
                )
                .into_any_element();
        }
        if let Some(e) = &self.diff_error {
            panel = panel.child(
                div()
                    .p(px(20.))
                    .text_color(rgb(c.del))
                    .child(format!("Diff unavailable: {e}")),
            );
        }
        let Some(diff) = &self.diff else {
            return panel.into_any_element();
        };
        let metadata = diff["text"]
            .as_str()
            .unwrap_or("")
            .lines()
            .take(128)
            .take_while(|line| !line.starts_with("@@"))
            .filter(|line| {
                [
                    "old mode ",
                    "new mode ",
                    "new file mode ",
                    "deleted file mode ",
                    "rename from ",
                    "rename to ",
                    "copy from ",
                    "copy to ",
                    "similarity index ",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !metadata.is_empty() {
            panel = panel.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .font_family(design::FONT_MONO)
                    .text_size(px(11. * ratio))
                    .text_color(rgb(p.muted))
                    .children(metadata),
            );
        }
        if diff["binary"].as_bool().unwrap_or(false) {
            return panel
                .child(
                    div()
                        .p(px(20.))
                        .text_color(rgb(p.muted))
                        .child("Binary file · no text diff available."),
                )
                .into_any_element();
        }
        if !diff["repository"].as_bool().unwrap_or(false) {
            return panel
                .child(
                    div()
                        .p(px(20.))
                        .text_color(rgb(p.muted))
                        .child("This change is no longer available. Refresh to check again."),
                )
                .into_any_element();
        }
        if diff["truncated"].as_bool().unwrap_or(false) {
            panel = panel.child(
                div()
                    .p(px(8.))
                    .text_size(px(11. * ratio))
                    .text_color(rgb(p.muted))
                    .child("Diff truncated at the backend’s 512 KiB limit."),
            );
        }
        let projection = self.projection.clone();
        let document = self.document.clone();
        let split = self.options.split;
        let owner = cx.weak_entity();
        let full_context = self.full_context;
        let id = format!("git-diff-{}-{scope}", self.selected.as_ref().unwrap().0);
        let available = if self.compact_layout {
            self.width_design
        } else {
            self.width_design * 0.66
        };
        let content = list(self.diff_list.clone(), move |ix, w, cx| {
            match &projection.rows[ix] {
                DiffRow::Code { .. } => {
                    view::render_code(&projection.rows[ix], ix, split, &id, c, ratio, w, cx)
                }
                DiffRow::Hunk { index, header } => {
                    let mut block = div().w_full().flex().flex_col();
                    if *index > 0 && !full_context {
                        let previous = &document.hunks[index - 1];
                        let next = &document.hunks[*index];
                        let gap = next
                            .old_start
                            .saturating_sub(previous.old_start.saturating_add(previous.old_count));
                        if gap > 0 {
                            let owner = owner.clone();
                            let gap_row = div().w(px(available));
                            #[cfg(test)]
                            let gap_row = gap_row.debug_selector({
                                let index = *index;
                                move || format!("git-context-gap-{index}")
                            });
                            block = block.child(
                                gap_row.child(
                                    Button::new(SharedString::from(format!(
                                        "git-context-gap-{index}"
                                    )))
                                    .w_full()
                                    .compact()
                                    .text()
                                    .label(format!("{gap} unchanged lines · expand context"))
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = owner
                                                .update(cx, |pane, cx| pane.expand_context(cx));
                                        },
                                    ),
                                ),
                            );
                        }
                    }
                    block
                        .child(
                            div()
                                .w_full()
                                .min_h(px(20. * ratio))
                                .min_w_0()
                                .whitespace_normal()
                                .pl(px(8. * ratio))
                                .font_family(design::FONT_MONO)
                                .text_size(px(11. * ratio))
                                .text_color(rgb(c.signal))
                                .bg(rgb(c.signal).opacity(0.07))
                                .child(header.clone()),
                        )
                        .into_any_element()
                }
                DiffRow::Fold { id, count } => {
                    let owner = owner.clone();
                    let fold = *id;
                    Button::new(SharedString::from(format!("git-context-fold-{id}")))
                        .w(px(available))
                        .compact()
                        .text()
                        .label(format!("{count} unchanged lines"))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |pane, cx| {
                                pane.options.expanded_folds.insert(fold);
                                pane.option_changed(cx);
                            });
                        })
                        .into_any_element()
                }
                DiffRow::Note(text) => div()
                    .w_full()
                    .text_color(rgb(p.muted))
                    .child(text.clone())
                    .into_any_element(),
            }
        })
        .w_full()
        .min_w_0()
        .h_full();
        panel = panel.child(
            div()
                .id("git-diff-horizontal")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(crate::transcript_edge_fade::TranscriptEdgeFade::for_list(
                    content,
                    self.diff_list.clone(),
                    px(16.),
                )),
        );
        if self.document.hunks.is_empty() {
            panel=panel.child(div().p(px(20.)).text_color(rgb(p.muted)).child("No text hunks. The file may have only a rename, mode change, or no remaining differences."));
        }
        if self.projection.hidden_rows > 0 {
            panel = panel.child(
                Button::new("git-more-lines")
                    .compact()
                    .text()
                    .w_full()
                    .label(format!(
                        "Show more · {} remaining lines",
                        self.projection.hidden_rows
                    ))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.options.row_limit += 400;
                        pane.option_changed(cx);
                    })),
            );
        }
        if self.projection.hidden_hunks > 0 {
            panel = panel.child(
                Button::new("git-more-hunks")
                    .compact()
                    .text()
                    .w_full()
                    .label(format!(
                        "Show remaining {} hunks",
                        self.projection.hidden_hunks
                    ))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.options.visible_hunks += 2;
                        pane.option_changed(cx);
                    })),
            );
        }
        let _ = window;
        panel.into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn whole_set_skips_added_bodies_and_removes_all_context() {
        let files = file_rows(&json!({"files":[
            {"path":"new.rs","indexStatus":"A","worktreeStatus":"M"},
            {"path":"untracked.rs","untracked":true},
            {"path":"old.rs","indexStatus":"M","worktreeStatus":"M"}
        ]}));
        let mut calls = Vec::new();
        let projection = collect_changes(&files, 24, |path, scope| {
            assert_eq!(path, "old.rs");
            calls.push(scope.to_owned());
            Ok(json!({"repository":true,"text":"@@ -1,2 +1,2 @@\n unchanged secret\n-old\n+new\n"}))
        });
        assert_eq!(calls.len(), 2);
        assert!(calls.contains(&"staged".into()) && calls.contains(&"unstaged".into()));
        assert_eq!(
            projection
                .rows
                .iter()
                .filter(|r| matches!(r, DiffRow::Note(t) if t == "+ new.rs · added file"))
                .count(),
            1
        );
        for row in projection.rows {
            if let DiffRow::Code { kind, old, new } = row {
                assert_ne!(kind, model::LineKind::Context);
                assert!(
                    old.iter()
                        .chain(new.iter())
                        .all(|c| !c.text.contains("secret"))
                );
            }
        }
    }
    #[::core::prelude::v1::test]
    fn whole_set_bounds_reads_and_keeps_errors_and_truncation_visible() {
        let files = (0..30)
            .map(|i| json!({"path":format!("{i}.rs"),"scope":"unstaged"}))
            .collect::<Vec<_>>();
        let mut count = 0;
        let p = collect_changes(&files, 24, |_, _| {
            count += 1;
            Err("fixture error".into())
        });
        assert_eq!(count, 24);
        assert!(
            p.rows
                .iter()
                .any(|r| matches!(r, DiffRow::Note(t) if t.contains("fixture error")))
        );
        let p = collect_changes(&files[..1], 24, |_, _| {
            Ok(json!({"repository":true,"truncated":true,"text":"@@ -1 +1 @@\n-old\n+new\n"}))
        });
        assert!(
            p.rows
                .iter()
                .any(|r| matches!(r, DiffRow::Note(t) if t.contains("truncated")))
        );
    }
    #[::core::prelude::v1::test]
    fn expansion_requires_server_acknowledgement() {
        assert!(!context_acknowledged(&json!({"text":"legacy patch"}), 1000));
        assert!(!context_acknowledged(&json!({"contextLines":3}), 1000));
        assert!(context_acknowledged(&json!({"contextLines":1000}), 1000));
    }
    #[::core::prelude::v1::test]
    fn changes_are_sorted_by_churn_and_keep_scopes_and_unknown_counts() {
        let status = json!({"files":[{"path":"small.rs","indexStatus":"M","worktreeStatus":"M","stagedStats":{"additions":1,"deletions":0},"unstagedStats":{"additions":5,"deletions":1}},
            {"path":"large.rs","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":80,"deletions":20}},
            {"path":"Cargo.lock","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":10000,"deletions":0}},
            {"path":"binary.png","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":null,"deletions":null}}]});
        let rows = file_rows(&status);
        assert_eq!(rows.len(), 5);
        assert_eq!(field(&rows[0], "path"), "large.rs");
        assert_eq!(field(&rows[1], "scope"), "unstaged");
        assert_eq!(field(&rows[2], "scope"), "staged");
        assert_eq!(field(&rows[4], "path"), "Cargo.lock");
        assert!(stat(&rows[3], "additions").is_none());
    }
}
#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
#[path = "git_design_tests.rs"]
mod design_tests;

/// Cached miniature for a recorded tool patch. Never manufactures a patch from
/// a filename/count record; callers keep non-diff details in their literal form.
#[derive(Clone)]
pub(crate) struct InlineDiff {
    projection: Arc<DiffProjection>,
}
impl InlineDiff {
    pub(crate) fn prepare(source: &str) -> Option<Self> {
        let mut doc = DiffDocument::parse(source);
        if doc.hunks.is_empty() || doc.additions + doc.deletions == 0 {
            return None;
        }
        let hunk = &mut doc.hunks[0];
        let change = hunk
            .lines
            .iter()
            .position(|line| line.kind != model::LineKind::Context)?;
        let start = change.saturating_sub(2);
        let end = (start + 6).min(hunk.lines.len());
        hunk.lines = hunk.lines[start..end].to_vec();
        let mut projection = doc.project(&DiffOptions {
            visible_hunks: 1,
            row_limit: 7,
            show_whitespace: true,
            ..Default::default()
        });
        projection
            .rows
            .retain(|r| matches!(r, DiffRow::Code { .. }));
        projection.rows.truncate(6);
        Some(Self {
            projection: Arc::new(projection),
        })
    }
    pub(crate) fn render(&self, id: &str, window: &mut Window, cx: &mut App) -> AnyElement {
        let p = design::palette(&json!({}), window.appearance());
        let colors = view::DiffColors::new(p);
        let ratio = (cx.theme().font_size / px(14.)).clamp(0.57, 2.29);
        let mut body = div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(p.line))
            .flex()
            .flex_col();
        for (i, row) in self.projection.rows.iter().enumerate() {
            body = body.child(view::render_code(
                row, i, false, id, colors, ratio, window, cx,
            ));
        }
        body.into_any_element()
    }
}

#[cfg(feature = "visual-export")]
pub(super) fn open_preview_if_requested(cx: &mut App) -> bool {
    let Ok(mode) = std::env::var("MONITTER_GPUI_GIT_DESIGN_PREVIEW") else {
        return false;
    };
    use gpui_kit::component::{Theme, ThemeMode};
    let light = mode == "light";
    let _ = monitter_gpui_desktop::fonts::register(cx);
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_family = design::FONT_SANS.into();
        theme.font_size = px(14.);
    });
    let (window,_) = gpui_kit::open_window(WindowOptions{
        window_bounds:Some(WindowBounds::Windowed(Bounds::centered(None,size(gpui_kit::px(1200.),gpui_kit::px(780.)),cx))),
        titlebar:Some(TitlebarOptions{title:Some("Synthetic Git design preview · disconnected".into()),..Default::default()}),..Default::default()
    },cx,move|window,cx|{
        window.on_window_should_close(cx,|_,cx|{cx.quit();true});
        let pane=cx.new(|cx|{
            let mut pane=GitPane::new(DesktopService::disconnected_for_ui_fixture(),&json!({"settings":{"theme":if light{"light"}else{"dark"}}}),window,cx);
            let status=json!({"repository":true,"branch":"synthetic-design","files":[
                {"path":"src/store.rs","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":186,"deletions":42}},
                {"path":"src/collaboration.rs","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":64,"deletions":21}},
                {"path":"docs/WORK-PLAN.md","indexStatus":" ","worktreeStatus":"A","unstagedStats":{"additions":23,"deletions":0}},
                {"path":"src/legacy_usage.rs","indexStatus":" ","worktreeStatus":"D","unstagedStats":{"additions":0,"deletions":118}},
                {"path":"Cargo.lock","indexStatus":" ","worktreeStatus":"M","unstagedStats":{"additions":4210,"deletions":1}}]});
            let diff=json!({"repository":true,"text":"diff --git a/src/store.rs b/src/store.rs\n--- a/src/store.rs\n+++ b/src/store.rs\n@@ -168,7 +168,8 @@ impl Store\n     }\n \n-    pub fn remove_task_usage(&mut self, id: &str) {\n+    pub fn remove_task_usage(&mut self, task_id: &str) {\n         let removed = self.staged.remove(task_id);\n-        self.usage_samples.retain(|s| s.task_id != task_id);\n+        self.tombstones.push(Tombstone::usage(task_id));\n+        self.save_update(Update::UsageRemoved { task_id: task_id.into() });\n         removed\n     }\n@@ -214,4 +215,5 @@ fn usage_overview\n     let history = self.history.lock();\n-    history.iter().map(|h| h.total()).sum()\n+    let committed = self.db.usage_samples()?;\n+    committed.iter().map(|s| s.total()).sum()\n }\n \n"});
            pane.task_id=Some("synthetic-only".into());pane.files=Arc::new(file_rows(&status));pane.file_list.reset(pane.files.len());pane.status=Some(status);
            pane.selected=Some(("src/store.rs".into(),"unstaged".into()));pane.document=Arc::new(DiffDocument::parse(diff["text"].as_str().unwrap()));pane.diff=Some(diff);pane.rebuild_diff();pane
        });cx.new(|cx|gpui_kit::component::Root::new(pane,window,cx))
    }).expect("isolated synthetic Git design preview");
    crate::visual_export::install(cx);
    crate::visual_export::schedule(window, cx);
    cx.activate(true);
    true
}
