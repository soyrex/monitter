//! A retained transcript sibling, independent of native composer invalidation.
//!
//! Preparation belongs to committed transcript/context changes. This entity
//! only renders the already-prepared virtual list, including when a streaming
//! message, disclosure, focus or viewport resize invalidates its descendants.
use super::*;
use gpui_kit::base::{ElementExt as _, TextSelectionGroupId};

/// Paint/layout geometry is independent of committed transcript content.
/// Visible virtual rows sample this retained scalar after each pane reflow.
#[derive(Clone, Copy, Default)]
pub(super) struct TranscriptGeometry {
    width: f32,
}
impl TranscriptGeometry {
    pub(super) fn from_viewport_width(width: f32) -> Self {
        Self {
            width: width
                - (space::SECTION
                    + if minimap_preference::enabled() {
                        5.
                    } else {
                        0.
                    })
                    * 2.,
        }
    }
    pub(super) fn bubble_fraction(self) -> f32 {
        if self.width <= 420. { 1. } else { 0.9 }
    }
    pub(super) fn attachment_width(self) -> f32 {
        self.width.min(design::CHAT_MAX_WIDTH).max(120.) * self.bubble_fraction()
    }
    pub(super) fn attachment_layout(
        self,
        preferred: message_attachments::AttachmentLayout,
    ) -> message_attachments::AttachmentLayout {
        use message_attachments::AttachmentLayout;
        if self.attachment_width() < 320. && preferred == AttachmentLayout::PortraitLeft {
            AttachmentLayout::LandscapeBottom
        } else {
            preferred
        }
    }
}

#[derive(Clone)]
pub(super) struct TranscriptPreparationKey {
    snapshot: std::rc::Weak<Value>,
    pub(super) minimap_enabled: bool,
    rows: Option<Rc<im::Vector<TimelineRow>>>,
    selected: Option<String>,
    palette: design::Palette,
    interface_family: SharedString,
    appearance: WindowAppearance,
    typography: Option<String>,
    scale: u16,
    width: f32,
    companion_width: f32,
    companion_visible: bool,
    compact_detail: bool,
    history_error: Option<String>,
    loading: bool,
    complete: bool,
    history_top_reached: bool,
    refreshing: bool,
    date: chrono::NaiveDate,
}
impl TranscriptPreparationKey {
    pub(super) fn capture(
        pane: &ChatPane,
        palette: design::Palette,
        interface_family: SharedString,
        appearance: WindowAppearance,
    ) -> Self {
        let state = pane
            .selected
            .as_ref()
            .and_then(|id| pane.transcripts.get(id));
        Self {
            snapshot: Rc::downgrade(&pane.snapshot),
            minimap_enabled: minimap_preference::enabled(),
            rows: state.map(|state| state.rows.clone()),
            selected: pane.selected.clone(),
            palette,
            interface_family,
            appearance,
            typography: pane.typography.clone(),
            scale: design::interface_scale(),
            width: pane.pane_width_design,
            companion_width: pane.companion_width,
            companion_visible: pane.companion_visible,
            compact_detail: pane.compact_detail,
            history_error: state.and_then(|state| state.error.clone()),
            loading: state.is_some_and(|state| state.loading),
            complete: state.is_some_and(TranscriptState::complete),
            history_top_reached: state.is_some_and(|state| {
                state.history_top_reached
                    && state.list.logical_scroll_top().item_ix == 0
                    && state.list.is_scrolled_to_end() == Some(false)
            }),
            refreshing: pane.refreshing,
            date: chrono::Local::now().date_naive(),
        }
    }
    /// The caller proved this publication leaves all transcript dependencies
    /// unchanged. Preserve a pre-existing invalidation from a hidden pane's
    /// substantive update; only retarget a key matching its current context.
    pub(super) fn advance_unaffected_snapshot(&mut self, previous: &Rc<Value>, next: &Rc<Value>) {
        if std::rc::Weak::ptr_eq(&self.snapshot, &Rc::downgrade(previous)) {
            self.snapshot = Rc::downgrade(next);
        }
    }
    /// Text-only substitution preserves every prepared wrapper and grouping.
    /// Do not erase an already-pending structural invalidation.
    pub(super) fn advance_stream_rows(
        &mut self,
        previous: &Rc<im::Vector<TimelineRow>>,
        next: &Rc<im::Vector<TimelineRow>>,
    ) {
        if self
            .rows
            .as_ref()
            .is_some_and(|rows| Rc::ptr_eq(rows, previous))
        {
            self.rows = Some(next.clone());
        }
    }
}
impl PartialEq for TranscriptPreparationKey {
    fn eq(&self, other: &Self) -> bool {
        self.minimap_enabled == other.minimap_enabled
            && std::rc::Weak::ptr_eq(&self.snapshot, &other.snapshot)
            && match (&self.rows, &other.rows) {
                (Some(left), Some(right)) => Rc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
            && self.selected == other.selected
            && self.palette == other.palette
            && self.interface_family == other.interface_family
            && self.appearance == other.appearance
            && self.typography == other.typography
            && self.scale == other.scale
            // Scalar widths drive visible-row layout through TranscriptGeometry.
            && self.companion_visible == other.companion_visible
            && self.compact_detail == other.compact_detail
            && self.history_error == other.history_error
            && self.loading == other.loading
            && self.complete == other.complete
            && self.history_top_reached == other.history_top_reached
            && self.refreshing == other.refreshing
            && self.date == other.date
    }
}

#[cfg(feature = "performance-profiling")]
const PREPARATION_REASONS: [&str; 20] = [
    "initial",
    "missingViewport",
    "snapshot",
    "rows",
    "selection",
    "palette",
    "interfaceFamily",
    "appearance",
    "typography",
    "scale",
    "width",
    "companionWidth",
    "companionVisible",
    "compactDetail",
    "historyError",
    "historyLoading",
    "historyComplete",
    "historyTop",
    "refreshing",
    "date",
];
#[cfg(feature = "performance-profiling")]
static PREPARATION_REASON_COUNTS: [std::sync::atomic::AtomicU64; 20] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 20];

#[cfg(feature = "performance-profiling")]
pub(super) fn preparation_reason_counts() -> Vec<(&'static str, u64)> {
    use std::sync::atomic::Ordering::Relaxed;
    PREPARATION_REASONS
        .iter()
        .zip(&PREPARATION_REASON_COUNTS)
        .map(|(reason, count)| (*reason, count.load(Relaxed)))
        .collect()
}

#[cfg(feature = "performance-profiling")]
impl TranscriptPreparationKey {
    /// Only runs after the preparation gate opens. Reports dependency changes
    /// without message, draft, task, provider or source payload information.
    pub(super) fn record_invalidation(&self, previous: Option<&Self>, missing_viewport: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut flags = [false; 20];
        flags[0] = previous.is_none();
        flags[1] = missing_viewport;
        if let Some(old) = previous {
            flags[2] = !std::rc::Weak::ptr_eq(&old.snapshot, &self.snapshot);
            flags[3] = match (&old.rows, &self.rows) {
                (Some(left), Some(right)) => !Rc::ptr_eq(left, right),
                (None, None) => false,
                _ => true,
            };
            flags[4] = old.selected != self.selected;
            flags[5] = old.palette != self.palette;
            flags[6] = old.interface_family != self.interface_family;
            flags[7] = old.appearance != self.appearance;
            flags[8] = old.typography != self.typography;
            flags[9] = old.scale != self.scale;
            // Width changes are observations, not preparation causes.
            flags[10] = false;
            flags[11] = false;
            flags[12] = old.companion_visible != self.companion_visible;
            flags[13] = old.compact_detail != self.compact_detail;
            flags[14] = old.history_error != self.history_error;
            flags[15] = old.loading != self.loading;
            flags[16] = old.complete != self.complete;
            flags[17] = old.history_top_reached != self.history_top_reached;
            flags[18] = old.refreshing != self.refreshing;
            flags[19] = old.date != self.date;
        }
        for (changed, count) in flags.iter().zip(&PREPARATION_REASON_COUNTS) {
            if *changed {
                count.fetch_add(1, Relaxed);
            }
        }
        if std::env::var_os("MONITTER_GPUI_TRACE_TRANSCRIPT_PREPARATION").is_some() {
            let reasons: Vec<_> = flags
                .iter()
                .zip(PREPARATION_REASONS)
                .filter_map(|(changed, reason)| changed.then_some(reason))
                .collect();
            eprintln!(
                "[transcript-preparation] reasons={reasons:?} width={:?}->{:.3} companionWidth={:?}->{:.3} scale={:?}->{} compact={:?}->{} historyTop={:?}->{}",
                previous.map(|old| old.width),
                self.width,
                previous.map(|old| old.companion_width),
                self.companion_width,
                previous.map(|old| old.scale),
                self.scale,
                previous.map(|old| old.compact_detail),
                self.compact_detail,
                previous.map(|old| old.history_top_reached),
                self.history_top_reached
            );
        }
    }
}

/// Deferred input work remains bound to the task that produced the event.
/// Re-check after yielding because selection can change before the callback.
fn defer_for_task<T: 'static>(
    owner: WeakEntity<T>,
    task_id: String,
    current_task: fn(&T) -> Option<&str>,
    action: impl FnOnce(&mut T, &mut Context<T>) + 'static,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = owner.update(cx, |view, cx| {
            if current_task(view) == Some(task_id.as_str()) {
                action(view, cx);
            }
        });
    });
}

pub(super) struct TranscriptViewport {
    task_id: String,
    selection_group: TextSelectionGroupId,
    owner: Option<WeakEntity<ChatPane>>,
    load_earlier: Rc<dyn Fn(&str, &mut App)>,
    load_unmeasured_history: Rc<dyn Fn(&str, &mut App)>,
    #[cfg(test)]
    render_count: usize,
    list: Option<Entity<StableTranscriptList>>,
    error: Option<String>,
    palette: design::Palette,
    geometry: Rc<std::cell::Cell<TranscriptGeometry>>,
    minimap_visible: bool,
    minimap_exit: Option<std::time::Instant>,
    minimap_jump: Option<minimap::MarkerJump>,
    pub(super) minimap_emphasis: minimap::Emphasis,
    pub(super) minimap_bottom_inset: Pixels,
}
impl TranscriptViewport {
    pub(super) fn new(
        owner: WeakEntity<ChatPane>,
        task_id: String,
        list: Option<Entity<StableTranscriptList>>,
        error: Option<String>,
        palette: design::Palette,
        geometry: Rc<std::cell::Cell<TranscriptGeometry>>,
    ) -> Self {
        let chat_owner = owner.clone();
        let retry_owner = owner.clone();
        let load_earlier = Rc::new(move |task_id: &str, cx: &mut App| {
            let _ = retry_owner.update(cx, |pane, cx| {
                if pane.selected.as_deref() == Some(task_id) {
                    pane.load_earlier(cx);
                }
            });
        });
        let load_unmeasured_history = Rc::new(move |task_id: &str, cx: &mut App| {
            defer_for_task(
                owner.clone(),
                task_id.to_owned(),
                |pane| pane.selected.as_deref(),
                |pane, cx| {
                    let Some(state) = pane.transcripts.get(&pane.task_id()) else {
                        return;
                    };
                    if pane.dock_visible
                        && state.list.is_scrolled_to_end().is_none()
                        && !state.messages.is_empty()
                        && !state.loading
                        && !state.complete()
                        && state.error.is_none()
                    {
                        pane.load_earlier(cx);
                    }
                },
                cx,
            );
        });
        Self {
            task_id,
            selection_group: TextSelectionGroupId::new(),
            owner: Some(chat_owner),
            load_earlier,
            load_unmeasured_history,
            list,
            error,
            palette,
            geometry,
            minimap_visible: false,
            minimap_exit: None,
            minimap_jump: None,
            minimap_emphasis: minimap::Emphasis::default(),
            minimap_bottom_inset: gpui_kit::px(0.),
            #[cfg(test)]
            render_count: 0,
        }
    }
    pub(super) fn disable_minimap(&mut self, cx: &mut Context<Self>) {
        self.minimap_visible = false;
        self.minimap_exit = None;
        self.minimap_jump = None;
        self.minimap_emphasis = minimap::Emphasis::default();
        self.minimap_bottom_inset = gpui_kit::px(0.);
        if let Some(list) = &self.list {
            // End a minimap drag whose hit target disappears with this toggle.
            list.read(cx).state.scrollbar_drag_ended();
        }
        cx.notify();
    }
    pub(super) fn cancel_minimap_jump(&mut self) {
        self.minimap_jump = None;
    }
    pub(super) fn set_composer_hidden(&mut self, hidden: Pixels, cx: &mut Context<Self>) {
        if minimap_preference::enabled() && self.minimap_bottom_inset != hidden {
            self.minimap_bottom_inset = hidden;
            cx.notify();
        }
    }
    pub(super) fn minimap_marker_jump(&mut self, row: usize, cx: &mut Context<Self>) {
        if !minimap_preference::enabled() {
            return;
        }
        self.minimap_navigated(false, cx);
        let Some(list) = &self.list else {
            return;
        };
        let state = list.read(cx).state.clone();
        state.set_follow_mode(FollowMode::Normal);
        if crate::native_motion::motion_allowed() {
            self.minimap_jump = Some(minimap::MarkerJump::new(
                row.min(state.item_count().saturating_sub(1)),
            ));
        } else {
            state.scroll_to(gpui_kit::ListOffset {
                item_ix: row,
                offset_in_item: gpui_kit::px(0.),
            });
        }
        self.repaint_scroll(cx);
    }
    fn advance_minimap_jump(&mut self, cx: &mut Context<Self>) {
        if !minimap_preference::enabled() {
            self.minimap_jump = None;
            return;
        }
        let Some(list) = &self.list else {
            self.minimap_jump = None;
            return;
        };
        let state = list.read(cx).state.clone();
        if self
            .minimap_jump
            .as_mut()
            .is_some_and(|jump| jump.advance(&state))
        {
            self.minimap_navigated(false, cx);
        } else {
            self.repaint_scroll(cx);
        }
    }
    pub(super) fn minimap_navigated(&mut self, at_bottom: bool, cx: &mut Context<Self>) {
        if !minimap_preference::enabled() {
            return;
        }
        self.cancel_minimap_jump();
        let Some(owner) = self.owner.clone() else {
            self.repaint_scroll(cx);
            return;
        };
        let task_id = self.task_id.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |pane, cx| {
                if !minimap_preference::enabled()
                    || pane.selected.as_deref() != Some(task_id.as_str())
                {
                    return;
                }
                if at_bottom {
                    pane.finish_bottom_jump(cx);
                    return;
                }
                pane.bottom_jump = None;
                if let Some(state) = pane.transcripts.get_mut(&task_id) {
                    state.list.set_follow_mode(FollowMode::Normal);
                    pane.composer_scroll_distance = crate::composer_scroll::remaining_tail_distance(
                        state.list.max_offset_for_scrollbar().y,
                        state.list.scroll_px_offset_for_scrollbar().y,
                        gpui_kit::px(0.),
                    );
                }
                let earlier = pane.transcripts.get(&task_id).is_some_and(|state| {
                    state.list.logical_scroll_top().item_ix <= 12
                        && !state.loading
                        && !state.complete()
                        && state.error.is_none()
                });
                if earlier {
                    pane.load_earlier(cx);
                }
                pane.invalidate_transcript_list(cx);
                cx.notify();
            });
        });
        self.repaint_scroll(cx);
    }
    pub(super) fn repaint_scroll(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = &self.list {
            list.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }
    pub(super) fn set_content(
        &mut self,
        task_id: String,
        list: Option<Entity<StableTranscriptList>>,
        error: Option<String>,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) {
        if self.task_id != task_id {
            self.selection_group = TextSelectionGroupId::new();
            self.minimap_visible = false;
            self.minimap_exit = None;
            self.minimap_jump = None;
            self.minimap_emphasis = minimap::Emphasis::default();
        }
        self.task_id = task_id;
        self.list = list;
        self.error = error;
        self.palette = palette;
        cx.notify();
    }
}
impl Render for TranscriptViewport {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        let minimap_enabled = minimap_preference::enabled();
        if minimap_enabled && self.minimap_jump.is_some() {
            let owner = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = owner.update(cx, |view, cx| view.advance_minimap_jump(cx));
            });
            window.request_animation_frame();
        }
        let load_unmeasured_history = self.load_unmeasured_history.clone();
        let scroll_task_id = self.task_id.clone();
        let geometry = self.geometry.clone();
        let repaint = cx.weak_entity();
        let mut transcript = div()
            .id("transcript")
            .relative()
            // Run before the cached list's prepaint, using this frame's allocation.
            .child(
                canvas(
                    move |bounds, _, _| {
                        let width_design = bounds.size.width / design::ui_scaled_px(1.);
                        geometry.set(TranscriptGeometry::from_viewport_width(width_design));
                    },
                    |_, _, _, _| (),
                )
                .absolute()
                .inset_0(),
            )
            // A cached view renders as a layout root. Explicitly fill its
            // assigned bounds; flex_1 alone has no parent flex layout here.
            .size_full()
            .px(px(space::SECTION))
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(space::SECTION))
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                if minimap_enabled {
                    let repaint = repaint.clone();
                    cx.defer(move |cx| {
                        let _ = repaint.update(cx, |view, cx| {
                            view.cancel_minimap_jump();
                            cx.notify();
                        });
                    });
                }
                // An empty scroll extent does not prove older history should
                // load. Only an upward user gesture can fetch the earlier page.
                if event.delta.pixel_delta(px(20.)).y <= gpui_kit::px(0.) {
                    return;
                }
                load_unmeasured_history(&scroll_task_id, cx);
            });
        #[cfg(test)]
        {
            transcript = transcript.debug_selector(|| "retained-transcript-viewport".into());
        }
        if let Some(list) = self.list.as_ref() {
            transcript =
                transcript.child(list.clone().cached(StyleRefinement::default().size_full()));
            if minimap_enabled {
                let retained = list.read(cx);
                let visible = (retained.state.item_count() > 0
                    && !retained.state.is_following_tail())
                    || retained.state.is_scrollbar_dragging();
                if visible {
                    self.minimap_exit = None;
                } else if self.minimap_visible && crate::native_motion::motion_allowed() {
                    self.minimap_exit = Some(std::time::Instant::now());
                }
                self.minimap_visible = visible;
                let exit = self
                    .minimap_exit
                    .map(|start| (start.elapsed().as_secs_f32() / 0.15).min(1.));
                if exit == Some(1.) {
                    self.minimap_exit = None;
                }
                if visible || self.minimap_exit.is_some() {
                    let emphasis = self.minimap_emphasis.advance(std::time::Instant::now());
                    transcript = transcript.child(minimap::render(
                        retained.state.clone(),
                        retained.minimap.clone(),
                        retained.rows.clone(),
                        self.geometry.get(),
                        self.palette,
                        cx.weak_entity(),
                        window,
                        cx,
                        exit,
                        emphasis,
                        self.minimap_bottom_inset,
                    ));
                    if self.minimap_emphasis.animating() {
                        window.request_animation_frame();
                        cx.notify();
                    }
                    if self.minimap_exit.is_some() {
                        window.request_animation_frame();
                        cx.notify();
                    }
                }
            }
        } else {
            transcript = transcript.child(transcript_column(div().child(
                "Select a chat or create one from Agents. Saved CLI authentication is used only when you send a message.",
            )));
        }
        transcript.text_selection_group(self.selection_group)
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn bubble_and_attachment_modes_track_live_width_thresholds() {
        use message_attachments::AttachmentLayout;
        let _minimap = minimap_preference::set_for_ui_fixture(false);
        let wide = TranscriptGeometry::from_viewport_width(461.);
        let narrow = TranscriptGeometry::from_viewport_width(460.);
        assert_eq!(wide.bubble_fraction(), 0.9);
        assert_eq!(narrow.bubble_fraction(), 1.);
        let portrait = TranscriptGeometry::from_viewport_width(360.);
        let landscape = TranscriptGeometry::from_viewport_width(359.);
        assert_eq!(
            portrait.attachment_layout(AttachmentLayout::PortraitLeft),
            AttachmentLayout::PortraitLeft
        );
        assert_eq!(
            landscape.attachment_layout(AttachmentLayout::PortraitLeft),
            AttachmentLayout::LandscapeBottom
        );
        assert_eq!(
            landscape.attachment_layout(AttachmentLayout::Cards),
            AttachmentLayout::Cards
        );
        assert!(landscape.attachment_width() < portrait.attachment_width());
        assert_eq!(
            TranscriptGeometry::from_viewport_width(1_000.).attachment_width(),
            design::CHAT_MAX_WIDTH * 0.9
        );
    }
}
#[cfg(all(test, feature = "visual-export"))]
#[path = "chat_render_isolation_tests.rs"]
mod tests;

#[cfg(all(test, feature = "visual-export"))]
#[path = "chat_minimap_tests.rs"]
mod minimap_tests;

#[cfg(all(test, feature = "visual-export"))]
#[path = "chat_selection_tests.rs"]
mod pane_selection_tests;
