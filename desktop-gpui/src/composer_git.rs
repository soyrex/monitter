//! Retained task-scoped Git summary displayed immediately above the composer.
//! This is a read-only projection of the backend Git status command.
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, Sizable,
        button::{Button, ButtonCustomVariant, ButtonVariants},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[path = "sidebar_hover.rs"]
mod rail_hover;

const GIT_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;
const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
static NEXT_DETECTOR_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub enum ComposerGitEvent {
    OpenGit { task_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextKey {
    task_id: String,
    host_id: String,
    cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskMarker {
    updated_at: Option<i64>,
    status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusRequest {
    context: ContextKey,
    generation: u64,
    sequence: u64,
}

/// Pure scheduling state. Native revisions do not identify a Git working tree;
/// own-task activity only marks it dirty, without superseding a useful read.
#[derive(Default)]
struct RefreshState {
    context: Option<ContextKey>,
    marker: Option<TaskMarker>,
    generation: u64,
    sequence: u64,
    visible: bool,
    dirty: bool,
    urgent: bool,
    in_flight: Option<StatusRequest>,
    last_started: Option<Duration>,
}

impl RefreshState {
    fn observe(&mut self, next: Option<(ContextKey, TaskMarker)>) -> bool {
        let (context, marker) = match next {
            Some((context, marker)) => (Some(context), Some(marker)),
            None => (None, None),
        };
        let scope_changed = self.context != context;
        if scope_changed {
            self.generation = self.generation.wrapping_add(1);
            self.context = context;
            self.marker = marker;
            self.last_started = None;
            self.dirty = self.context.is_some();
            self.urgent = self.dirty;
            // Retain the old in-flight slot until its worker finishes. Scope
            // changes fence its result without creating concurrent detectors.
        } else if self.marker != marker {
            self.urgent |= self.marker.as_ref().map(|marker| &marker.status)
                != marker.as_ref().map(|marker| &marker.status);
            self.marker = marker;
            self.dirty = self.context.is_some();
        }
        scope_changed
    }

    fn set_visible(&mut self, visible: bool) {
        if visible && !self.visible {
            self.request_urgent();
        }
        self.visible = visible;
    }

    fn request_urgent(&mut self) {
        if self.context.is_some() {
            self.dirty = true;
            self.urgent = true;
        }
    }

    fn deadline(&self, now: Duration) -> Option<Duration> {
        if !self.visible || !self.dirty || self.context.is_none() || self.in_flight.is_some() {
            return None;
        }
        Some(if self.urgent {
            now
        } else {
            self.last_started
                .map_or(now, |last| now.max(last + STATUS_REFRESH_INTERVAL))
        })
    }

    fn begin(&mut self, now: Duration) -> Option<StatusRequest> {
        if self.deadline(now)? > now {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let request = StatusRequest {
            context: self.context.clone()?,
            generation: self.generation,
            sequence: self.sequence,
        };
        self.in_flight = Some(request.clone());
        self.last_started = Some(now);
        self.dirty = false;
        self.urgent = false;
        Some(request)
    }

    fn complete(&mut self, request: &StatusRequest) -> bool {
        if self.in_flight.as_ref() != Some(request) {
            return false;
        }
        self.in_flight = None;
        self.generation == request.generation && self.context.as_ref() == Some(&request.context)
    }
}

fn task_context(snapshot: &Value, task_id: &str) -> Option<(ContextKey, TaskMarker)> {
    let task = snapshot["tasks"]
        .as_array()?
        .iter()
        .find(|task| task["id"].as_str() == Some(task_id))?;
    let cwd = task
        .get("cwd")?
        .as_str()
        .filter(|cwd| !cwd.trim().is_empty())?;
    Some((
        ContextKey {
            task_id: task_id.to_owned(),
            host_id: task["hostId"].as_str().unwrap_or_default().to_owned(),
            cwd: cwd.to_owned(),
        },
        TaskMarker {
            updated_at: task["updatedAt"].as_i64(),
            status: task["status"].as_str().unwrap_or_default().to_owned(),
        },
    ))
}

fn status_summary(status: &Value) -> Option<String> {
    if status["repository"].as_bool() != Some(true) {
        return None;
    }
    let files = status["files"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let suffix = if status["truncated"].as_bool() == Some(true) {
        "+"
    } else {
        ""
    };
    let count = files.len();
    Some(format!(
        "{count}{suffix} {}",
        if count == 1 {
            "file changed"
        } else {
            "files changed"
        }
    ))
}

/// Counts only reported text statistics; absent/binary/untracked data is not zero.
#[derive(Default, Debug, PartialEq, Eq)]
struct LineTotals {
    additions: u64,
    deletions: u64,
    known: bool,
    partial: bool,
}
fn line_totals(status: &Value) -> LineTotals {
    let mut totals = LineTotals::default();
    let files = status["files"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    totals.partial = status["truncated"].as_bool() == Some(true);
    totals.known = files.is_empty() && !totals.partial;
    for file in files {
        if file["untracked"].as_bool() == Some(true) {
            totals.partial = true;
            continue;
        }
        for (state, key) in [
            ("indexStatus", "stagedStats"),
            ("worktreeStatus", "unstagedStats"),
        ] {
            if !file[state].as_str().is_some_and(|v| !v.trim().is_empty()) {
                continue;
            }
            match (
                file[key]["additions"].as_u64(),
                file[key]["deletions"].as_u64(),
            ) {
                (Some(add), Some(del)) => {
                    let (a, overflow_a) = totals.additions.overflowing_add(add);
                    let (d, overflow_d) = totals.deletions.overflowing_add(del);
                    totals.additions = if overflow_a { u64::MAX } else { a };
                    totals.deletions = if overflow_d { u64::MAX } else { d };
                    totals.known = true;
                    totals.partial |= overflow_a || overflow_d;
                }
                _ => totals.partial = true,
            }
        }
    }
    totals
}
fn grouped_count(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            result.push(',');
        }
        result.push(digit);
    }
    result
}
fn change_bar(status: &Value, palette: crate::design::Palette, ratio: f32, reveal: f32) -> Div {
    let totals = line_totals(status);
    let add = if palette.dark { 0x34d399 } else { 0x087f5b };
    let del = if palette.dark { 0xff5c6c } else { 0xc73545 };
    let rail_group: SharedString = "composer-git-change-bar".into();
    let add_idle = mix_rgb(
        palette.panel,
        palette.muted,
        if palette.dark { 0.72 } else { 0.58 },
    );
    let del_idle = mix_rgb(
        palette.panel,
        palette.muted,
        if palette.dark { 0.44 } else { 0.34 },
    );
    let files = status["files"].as_array().map_or(0, Vec::len);
    let count = format!(
        "{}{} {}",
        grouped_count(files as u64),
        if status["truncated"] == true { "+" } else { "" },
        if files == 1 { "file" } else { "files" }
    );
    let branch = status["branch"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Detached HEAD");
    let row = div()
        .w_full()
        .px(px(9.))
        .py(px(4.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(7.))
        .child(
            div()
                .flex_1()
                .min_w(px(65. * ratio))
                .flex()
                .items_center()
                .gap(px(7.))
                .child(
                    Icon::new(IconName::GitBranch)
                        .size(px(10. * ratio))
                        .flex_shrink_0()
                        .text_color(rgb(palette.muted)),
                )
                .child(
                    div()
                        .debug_selector(|| "composer-git-branch".into())
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .font_family(crate::design::FONT_MONO)
                        .text_size(px(8.5 * ratio))
                        .text_color(rgb(palette.muted))
                        .child(branch.to_owned()),
                )
                .when(status["worktree"] == true, |row| {
                    row.child(
                        Icon::new(IconName::FolderGit2)
                            .size(px(10. * ratio))
                            .text_color(rgb(palette.muted)),
                    )
                }),
        )
        .child(
            div()
                .debug_selector(|| "composer-git-stats".into())
                .flex_shrink_0()
                .min_w_0()
                .max_w_full()
                .ml_auto()
                .flex()
                .flex_wrap()
                .justify_end()
                .items_center()
                .gap(px(6.))
                .font_family(crate::design::FONT_MONO)
                .text_size(px(8.5 * ratio))
                .whitespace_nowrap()
                .child(div().text_color(rgb(palette.muted)).child(count))
                .child(div().text_color(rgb(palette.muted)).child("·"))
                .child(
                    div()
                        .text_color(rgb(if totals.known { add } else { palette.muted }))
                        .child(if totals.known {
                            format!("+{}", grouped_count(totals.additions))
                        } else {
                            "+—".into()
                        }),
                )
                .child(
                    div()
                        .text_color(rgb(if totals.known { del } else { palette.muted }))
                        .child(if totals.known {
                            format!("−{}", grouped_count(totals.deletions))
                        } else {
                            "−—".into()
                        }),
                ),
        );
    let mut rail = div()
        .debug_selector(|| "composer-git-change-rail".into())
        .w_full()
        .absolute()
        .bottom_0()
        .left_0()
        .h(px(2. * reveal))
        .overflow_hidden()
        .flex()
        .bg(rgb(palette.line));
    let total = totals.additions as f64 + totals.deletions as f64;
    if total > 0. {
        if totals.additions > 0 {
            rail = rail.child(
                div()
                    .debug_selector(|| "composer-git-added-rail".into())
                    .h_full()
                    .flex_shrink_0()
                    .w(relative((totals.additions as f64 / total) as f32))
                    .bg(rgb(add_idle))
                    .group_hover(rail_group.clone(), move |style| style.bg(rgb(add))),
            );
        }
        if totals.deletions > 0 {
            rail = rail.child(
                div()
                    .debug_selector(|| "composer-git-deleted-rail".into())
                    .h_full()
                    .flex_1()
                    .bg(rgb(del_idle))
                    .group_hover(rail_group.clone(), move |style| style.bg(rgb(del))),
            );
        }
    }
    div()
        .relative()
        .group(rail_group)
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .child(row)
        .child(rail)
}

fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn detail_status_summary(status: &Value) -> Option<String> {
    if status["repository"].as_bool() != Some(true) {
        return None;
    }
    let files = status["files"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if files.is_empty() {
        return Some("Working tree clean".into());
    }
    let untracked = |file: &&Value| file["untracked"].as_bool() == Some(true);
    let staged = files
        .iter()
        .filter(|file| {
            !untracked(file)
                && file["indexStatus"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty())
        })
        .count();
    let changed = files
        .iter()
        .filter(|file| {
            !untracked(file)
                && file["worktreeStatus"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty())
        })
        .count();
    let untracked_count = files.iter().filter(untracked).count();
    let suffix = if status["truncated"].as_bool() == Some(true) {
        "+"
    } else {
        ""
    };
    Some(format!(
        "{staged} staged · {changed} changed · {untracked_count} untracked{suffix}"
    ))
}

fn start_status_worker(
    service: DesktopService,
    task_id: String,
) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
    let (sender, receiver) = oneshot::channel();
    let detector_session = format!(
        "gpui-composer-git-{}",
        NEXT_DETECTOR_SESSION.fetch_add(1, Ordering::Relaxed)
    );
    std::thread::Builder::new()
        .name("monitter-composer-git-status".into())
        .stack_size(GIT_WORKER_STACK_SIZE)
        .spawn(move || {
            // Preserve cheap/silent non-repository detection. Only a confirmed
            // repository requests the existing bounded aggregate line statistics.
            let result = service
                .invoke(
                    "get_task_git_status",
                    json!({"taskId":task_id,"detectorSession":detector_session}),
                )
                .map(|status| {
                    if status["repository"] == true {
                        service
                            .invoke(
                                "get_task_git_status",
                                json!({"taskId":task_id,"includeStats":true}),
                            )
                            .unwrap_or(status)
                    } else {
                        status
                    }
                });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("Could not start Git status worker: {error}"))?;
    Ok(receiver)
}

fn workspace_status_ready(service: &DesktopService) -> bool {
    service.workspace_commands_ready()
}

pub struct ComposerGitStrip {
    service: DesktopService,
    snapshot: Value,
    refresh: RefreshState,
    clock: Instant,
    refresh_timer: Option<Task<()>>,
    refresh_deadline: Option<Duration>,
    status: Option<Value>,
    error: Option<String>,
    composed_palette: Option<crate::design::Palette>,
    working: bool,
    active: bool,
}

impl EventEmitter<ComposerGitEvent> for ComposerGitStrip {}

impl ComposerGitStrip {
    /// Reuse the task-scoped status cache for Details; this view never starts
    /// a second detector or takes Git mutation authority.
    pub fn render_detail_summary(
        &self,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let status = self.status.as_ref()?;
        let summary = detail_status_summary(status)?;
        let task_id = self.refresh.context.as_ref()?.task_id.clone();
        let branch = status["branch"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Detached HEAD");
        let ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let background = mix_rgb(palette.sidebar, palette.ink, 0.03);
        let hover = mix_rgb(palette.sidebar, palette.accent, 0.07);
        let border = mix_rgb(palette.sidebar, palette.line, 0.65);
        let label = format!("Open Git changes: {branch}, {summary}");
        Some(
            Button::new("details-git-open")
                .w_full()
                .h_auto()
                .min_h(px(0.))
                .px(px(11.))
                .py(px(10.))
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(border))
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(crate::design::surface_color(background).into())
                        .foreground(rgb(palette.ink).into())
                        .hover(crate::design::surface_color(hover).into())
                        .active(crate::design::surface_color(hover).into()),
                )
                .accessibility_label(label)
                .tooltip("Open Git changes")
                .on_click(cx.listener(move |strip, _, _, cx| {
                    strip.refresh_now(cx);
                    cx.emit(ComposerGitEvent::OpenGit {
                        task_id: task_id.clone(),
                    })
                }))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(px(15.))
                                .text_color(rgb(palette.accent)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .text_size(px(9. * ratio))
                                        .font_family(crate::design::FONT_MONO)
                                        .text_color(rgb(palette.muted))
                                        .child("GIT BRANCH"),
                                )
                                .child(
                                    div()
                                        .text_size(px(12. * ratio))
                                        .font_family(crate::design::FONT_MONO)
                                        .truncate()
                                        .child(branch.to_owned()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10. * ratio))
                                        .text_color(rgb(palette.muted))
                                        .child(summary),
                                ),
                        )
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .size(px(13.))
                                .text_color(rgb(palette.muted)),
                        ),
                )
                .into_any_element(),
        )
    }

    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        revision: Option<&str>,
        task_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut strip = Self {
            service,
            snapshot: json!({}),
            refresh: RefreshState::default(),
            clock: Instant::now(),
            refresh_timer: None,
            refresh_deadline: None,
            status: None,
            error: None,
            composed_palette: None,
            working: false,
            active: false,
        };
        strip.set_context(snapshot, revision, task_id, cx);
        strip
    }

    /// Relevant task activity requests a bounded refresh. Unrelated native
    /// revisions neither start reads nor discard a same-scope result.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        _revision: Option<&str>,
        task_id: &str,
        cx: &mut Context<Self>,
    ) {
        // This strip needs appearance only; copying the entire transcript on
        // each native revision stalls the UI thread for large conversations.
        let appearance_changed = self.snapshot["settings"] != snapshot["settings"];
        if appearance_changed {
            self.snapshot = json!({"settings": snapshot["settings"]});
        }
        let scope_changed = self.refresh.observe(task_context(snapshot, task_id));
        if scope_changed {
            self.status = None;
            self.error = None;
        }
        self.drive_refresh(cx);
        if self.refresh.visible && (appearance_changed || scope_changed) {
            cx.notify();
        }
    }

    /// Hidden panes retain their cache and dirtiness without starting workers.
    /// A previously dispatched worker may finish; revealing always reads again.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let revealed = visible && !self.refresh.visible;
        self.refresh.set_visible(visible);
        self.drive_refresh(cx);
        if revealed {
            cx.notify();
        }
    }

    /// Explicit Git actions bypass the ordinary one-second activity budget,
    /// coalescing behind an existing read rather than creating another worker.
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self.refresh.request_urgent();
        self.drive_refresh(cx);
    }

    /// Retained panes exist before the backend becomes authoritative. Keep the
    /// pending refresh intact until startup authorizes workspace commands, then
    /// dispatch it through the normal bounded scheduler.
    pub(crate) fn startup_service_ready(&mut self, cx: &mut Context<Self>) {
        if !workspace_status_ready(&self.service) {
            return;
        }
        self.refresh.request_urgent();
        self.drive_refresh(cx);
        if self.refresh.visible {
            cx.notify();
        }
    }

    fn drive_refresh(&mut self, cx: &mut Context<Self>) {
        // Git status is a workspace command. A retained chat pane can become
        // visible during startup, before those commands are authorized. Do not
        // consume its dirty marker with the expected connecting error: the
        // owning ChatPane explicitly resumes this scheduler at readiness.
        if !workspace_status_ready(&self.service) {
            return;
        }
        let now = self.clock.elapsed();
        let Some(deadline) = self.refresh.deadline(now) else {
            self.refresh_timer = None;
            self.refresh_deadline = None;
            return;
        };
        if deadline > now {
            if self.refresh_deadline != Some(deadline) {
                self.refresh_deadline = Some(deadline);
                self.refresh_timer = Some(cx.spawn(async move |strip, cx| {
                    cx.background_executor()
                        .timer(deadline.saturating_sub(now))
                        .await;
                    let _ = strip.update(cx, |strip, cx| {
                        strip.refresh_timer = None;
                        strip.refresh_deadline = None;
                        strip.drive_refresh(cx);
                    });
                }));
            }
            return;
        }
        self.refresh_timer = None;
        self.refresh_deadline = None;
        let Some(request) = self.refresh.begin(now) else {
            return;
        };
        let receiver =
            match start_status_worker(self.service.clone(), request.context.task_id.clone()) {
                Ok(receiver) => receiver,
                Err(error) => {
                    self.refresh.complete(&request);
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            };
        cx.spawn(async move |strip, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Git status worker ended without a result".into()));
            let _ = strip.update(cx, |strip, cx| {
                if strip.refresh.complete(&request) {
                    match result {
                        Ok(status) => {
                            strip.status = Some(status);
                            strip.error = None;
                        }
                        Err(error) => strip.error = Some(error),
                    }
                    if strip.refresh.visible {
                        cx.notify();
                    }
                }
                strip.drive_refresh(cx);
            });
        })
        .detach();
    }

    /// Whether a real repository status is available for the composer seam.
    pub fn has_visible_strip(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status["repository"].as_bool() == Some(true))
    }

    /// Use the owning transcript's fully composed appearance. Repeated parent
    /// renders with the same palette do not schedule another render.
    pub fn set_composed_palette(
        &mut self,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self.composed_palette.is_some_and(|current| {
            current.dark == palette.dark
                && current.paper == palette.paper
                && current.sidebar == palette.sidebar
                && current.panel == palette.panel
                && current.card == palette.card
                && current.ink == palette.ink
                && current.muted == palette.muted
                && current.line == palette.line
                && current.soft == palette.soft
                && current.code == palette.code
                && current.accent == palette.accent
        });
        if !unchanged {
            self.composed_palette = Some(palette);
            cx.notify();
        }
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            cx.notify();
        }
    }

    pub fn set_working(&mut self, working: bool, cx: &mut Context<Self>) {
        if self.working != working {
            self.working = working;
            cx.notify();
        }
    }
}

impl Render for ComposerGitStrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rail_hover, rail_reveal) = rail_hover::state(
            &format!("composer-git-rail-{}", cx.entity_id()), false, window, cx,
        );
        let palette = self.composed_palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let Some(status) = self.status.as_ref() else {
            if let Some(error) = self.error.as_deref() {
                return div()
                    .w_full()
                    .px(px(9.))
                    .py(px(6.))
                    .text_size(px(10.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(0xdc2626))
                    .child(format!("Git status unavailable: {error}"));
            }
            return div().w_full();
        };
        let Some(summary) = status_summary(status) else {
            return div().w_full();
        };
        let branch = status["branch"]
            .as_str()
            .filter(|branch| !branch.trim().is_empty())
            .unwrap_or("Detached HEAD")
            .to_owned();
        let interface_font_size = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32;
        let ratio = interface_font_size / 14.;
        let text_size = px(10. * ratio);
        // Darken the previous resting fill by another 40%; mirror with white in light themes.
        // Keep surface opacity and foreground text independent of the tint.
        let background = if self.working {
            crate::surface_blend::working_composer_color(palette.card, palette.dark)
        } else {
            crate::surface_blend::theme_tint_color(
                mix_rgb(palette.panel, palette.ink, 0.08),
                palette.dark,
                0.52,
            )
        };
        let background = if self.active {
            crate::surface_blend::theme_tint_color(background, palette.dark, 0.10)
        } else {
            background
        };
        let hover_background = if self.working {
            mix_rgb(background, palette.accent, 0.10)
        } else {
            mix_rgb(palette.panel, palette.accent, 0.18)
        };
        let accessible_label = format!(
            "Open Git changes: {branch}, {summary}{}",
            if status["worktree"] == true {
                ", linked worktree"
            } else {
                ""
            }
        );
        let Some(context) = self.refresh.context.as_ref() else {
            return div().w_full();
        };
        let task_id = context.task_id.clone();
        let mut root = div().w_full().child(
            div()
                .w_full()
                .border_1()
                .border_color(crate::design::composer_border_color(palette))
                .rounded(px(7.))
                .rounded_bl(px(0.))
                .rounded_br(px(0.))
                .mb(-px(1.))
                .overflow_hidden()
                .child(
                    Button::new("composer-git-open")
                        .debug_selector(|| "composer-git-open".into())
                        .w_full()
                        .h_auto()
                        .min_h(px(0.))
                        .px(px(0.))
                        .py(px(0.))
                        .border_0()
                        .rounded(px(6.))
                        .rounded_bl(px(0.))
                        .rounded_br(px(0.))
                        .line_height(relative(1.2))
                        .text_size(text_size)
                        .font_family(cx.theme().font_family.clone())
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(crate::surface_blend::git_surface_color(background).into())
                                .foreground(rgb(palette.muted).into())
                                .hover(
                                    crate::surface_blend::git_surface_color(hover_background)
                                        .into(),
                                )
                                .active(
                                    crate::surface_blend::git_surface_color(hover_background)
                                        .into(),
                                ),
                        )
                        .accessibility_label(accessible_label)
                        .tooltip({
                            let totals = line_totals(status);
                            format!(
                                "Open Git changes · {branch}{}{}",
                                if status["worktree"] == true {
                                    " · linked worktree"
                                } else {
                                    ""
                                },
                                if totals.partial {
                                    " · line totals exclude unavailable/binary/untracked changes"
                                } else {
                                    ""
                                }
                            )
                        })
                        .on_click(cx.listener(move |strip, _, _, cx| {
                            strip.refresh_now(cx);
                            cx.emit(ComposerGitEvent::OpenGit {
                                task_id: task_id.clone(),
                            });
                        }))
                        .child(
                            change_bar(status, palette, ratio, rail_reveal)
                                .id("composer-git-rail-hover")
                                .on_hover(cx.listener(move |_, hovered, _, cx| {
                                    rail_hover::set(&rail_hover, *hovered, cx);
                                })),
                        ),
                ),
        );
        if let Some(error) = self.error.as_deref() {
            root = root.child(
                div()
                    .px_2()
                    .pt_1()
                    .text_size(px(10.))
                    .text_color(rgb(0xdc2626))
                    .child(format!("Git status refresh failed: {error}")),
            );
        }
        root
    }
}

#[cfg(test)]
#[path = "composer_git_refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
mod tests {
    use super::{detail_status_summary, status_summary};
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn git_strip_summary_uses_tauri_file_count_and_truncation() {
        let status = json!({"repository":true,"truncated":true,"files":[
            {"indexStatus":"M","worktreeStatus":"M","untracked":false},
            {"indexStatus":"","worktreeStatus":"?","untracked":true},
            {"indexStatus":"A","worktreeStatus":"","untracked":false}
        ]});
        assert_eq!(status_summary(&status).as_deref(), Some("3+ files changed"));
    }

    #[::core::prelude::v1::test]
    fn clean_and_non_repository_states_are_distinct() {
        assert_eq!(
            status_summary(&json!({"repository":true,"files":[]})).as_deref(),
            Some("0 files changed")
        );
        assert_eq!(status_summary(&json!({"repository":false})), None);
    }

    #[::core::prelude::v1::test]
    fn details_counts_staged_and_unstaged_independently_excluding_untracked() {
        let status = json!({"repository":true,"truncated":true,"files":[
            {"indexStatus":"M","worktreeStatus":"M","untracked":false},
            {"indexStatus":"?","worktreeStatus":"?","untracked":true},
            {"indexStatus":"A","worktreeStatus":" ","untracked":false},
            {"indexStatus":" ","worktreeStatus":"D","untracked":false}
        ]});
        assert_eq!(
            detail_status_summary(&status).as_deref(),
            Some("2 staged · 2 changed · 1 untracked+")
        );
        assert_eq!(
            detail_status_summary(&json!({"repository":true,"files":[]})).as_deref(),
            Some("Working tree clean")
        );
        assert_eq!(detail_status_summary(&json!({"repository":false})), None);
    }
}

#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
#[path = "composer_git_design_tests.rs"]
mod design_tests;
