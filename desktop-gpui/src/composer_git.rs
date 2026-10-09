//! Retained task-scoped Git summary displayed immediately above the composer.
//! This is a read-only projection of the backend Git status command.
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{
        Disableable, Icon, Sizable,
        button::{Button, ButtonCustomVariant, ButtonVariants},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

const GIT_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;
static NEXT_DETECTOR_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub enum ComposerGitEvent {
    OpenGit { task_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextKey {
    task_id: String,
    cwd: String,
    revision: Option<String>,
}

fn task_cwd(snapshot: &Value, task_id: &str) -> Option<String> {
    snapshot["tasks"]
        .as_array()?
        .iter()
        .find(|task| task["id"].as_str() == Some(task_id))?
        .get("cwd")?
        .as_str()
        .filter(|cwd| !cwd.trim().is_empty())
        .map(str::to_owned)
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
            let result = service.invoke(
                "get_task_git_status",
                json!({"taskId":task_id,"detectorSession":detector_session}),
            );
            let _ = sender.send(result);
        })
        .map_err(|error| format!("Could not start Git status worker: {error}"))?;
    Ok(receiver)
}

pub struct ComposerGitStrip {
    service: DesktopService,
    snapshot: Value,
    context: Option<ContextKey>,
    generation: u64,
    request: u64,
    busy: bool,
    status: Option<Value>,
    error: Option<String>,
    composed_palette: Option<crate::design::Palette>,
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
        let task_id = self.context.as_ref()?.task_id.clone();
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
                .on_click(cx.listener(move |_, _, _, cx| {
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
            snapshot: snapshot.clone(),
            context: None,
            generation: 0,
            request: 0,
            busy: false,
            status: None,
            error: None,
            composed_palette: None,
        };
        strip.set_context(snapshot, revision, task_id, cx);
        strip
    }

    /// Refreshes only when task, saved working directory, or authoritative
    /// snapshot revision changes. Late results are discarded on any mismatch.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        revision: Option<&str>,
        task_id: &str,
        cx: &mut Context<Self>,
    ) {
        // This strip needs appearance only; copying the entire transcript on
        // each native revision stalls the UI thread for large conversations.
        self.snapshot = json!({"settings": snapshot["settings"]});
        let Some(cwd) = task_cwd(snapshot, task_id) else {
            self.generation = self.generation.wrapping_add(1);
            self.request = self.request.wrapping_add(1);
            self.context = None;
            self.status = None;
            self.error = None;
            self.busy = false;
            cx.notify();
            return;
        };
        let next = ContextKey {
            task_id: task_id.to_owned(),
            cwd,
            revision: revision.map(str::to_owned),
        };
        if self.context.as_ref() == Some(&next) {
            return;
        }
        let same_task_location = self
            .context
            .as_ref()
            .is_some_and(|previous| previous.task_id == next.task_id && previous.cwd == next.cwd);
        self.context = Some(next.clone());
        self.generation = self.generation.wrapping_add(1);
        self.request = self.request.wrapping_add(1);
        self.busy = false;
        if !same_task_location {
            self.status = None;
            self.error = None;
        }
        let generation = self.generation;
        let request = self.request;
        let receiver = match start_status_worker(self.service.clone(), task_id.to_owned()) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.busy = true;
        cx.spawn(async move |strip, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Git status worker ended without a result".into()));
            let _ = strip.update(cx, |strip, cx| {
                if strip.generation != generation
                    || strip.request != request
                    || strip.context.as_ref() != Some(&next)
                {
                    return;
                }
                strip.busy = false;
                match result {
                    Ok(status) => {
                        strip.status = Some(status);
                        strip.error = None;
                    }
                    Err(error) => strip.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
}

impl Render for ComposerGitStrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let worktree = status["worktree"].as_bool() == Some(true);
        let interface_font_size = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32;
        let text_size = px(10. * interface_font_size / 14.);
        // A 20% black multiply tint scales the resting surface RGB by 0.8.
        // Keep surface opacity and foreground text independent of the tint.
        let background = mix_rgb(mix_rgb(palette.panel, palette.ink, 0.08), 0x000000, 0.20);
        let hover_background = mix_rgb(palette.panel, palette.accent, 0.18);
        let accessible_label = format!("Open Git changes: {branch}, {summary}");
        let Some(context) = self.context.as_ref() else {
            return div().w_full();
        };
        let task_id = context.task_id.clone();
        let mut root = div().w_full().child(
            Button::new("composer-git-open")
                .w_full()
                .h_auto()
                .min_h(px(0.))
                .px(px(9.))
                .py(px(6.))
                .border_1()
                .rounded(px(7.))
                .rounded_bl(px(0.))
                .rounded_br(px(0.))
                .mb(-px(1.))
                .line_height(relative(1.2))
                .text_size(text_size)
                .font_family(crate::design::FONT_MONO)
                .border_color(rgb(palette.line))
                .border_blend_mode(crate::surface_blend::separator_mode())
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(crate::design::surface_color(background).into())
                        .foreground(rgb(palette.muted).into())
                        .hover(crate::design::surface_color(hover_background).into())
                        .active(crate::design::surface_color(hover_background).into()),
                )
                .accessibility_label(accessible_label)
                .tooltip("Open Git changes")
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ComposerGitEvent::OpenGit {
                        task_id: task_id.clone(),
                    });
                }))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_size(text_size)
                        .font_family(crate::design::FONT_MONO)
                        .line_height(relative(1.2))
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(px(13.))
                                .text_color(rgb(palette.accent)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(branch),
                        )
                        .when(worktree, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap(px(4.))
                                    .ml(px(2.))
                                    .pl(px(7.))
                                    .border_l_1()
                                    .border_color(rgb(palette.line))
                .border_blend_mode(crate::surface_blend::separator_mode())
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        Icon::new(IconName::FolderGit2)
                                            .size(px(12.))
                                            .text_color(rgb(palette.muted)),
                                    )
                                    .child("Worktree"),
                            )
                        })
                        .child(
                            div()
                                .flex_shrink_0()
                                .ml_auto()
                                .whitespace_nowrap()
                                .text_color(rgb(palette.muted))
                                .child(summary),
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
