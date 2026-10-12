//! A retained transcript sibling, independent of native composer invalidation.
//!
//! Preparation belongs to committed transcript/context changes. This entity
//! only renders the already-prepared virtual list, including when a streaming
//! message, disclosure, focus or viewport resize invalidates its descendants.
use super::*;

#[derive(Clone)]
pub(super) struct TranscriptPreparationKey {
    snapshot: std::rc::Weak<Value>,
    rows: Option<Rc<Vec<TimelineRow>>>,
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
    disclosure_generation: u64,
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
            disclosure_generation: pane.response_disclosure_generation,
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
    pub(super) fn advance_stream_rows(&mut self, previous: &Rc<Vec<TimelineRow>>, next: &Rc<Vec<TimelineRow>>) {
        if self.rows.as_ref().is_some_and(|rows| Rc::ptr_eq(rows, previous)) {
            self.rows = Some(next.clone());
        }
    }

}
impl PartialEq for TranscriptPreparationKey {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Weak::ptr_eq(&self.snapshot, &other.snapshot)
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
            && self.width == other.width
            && self.companion_width == other.companion_width
            && self.companion_visible == other.companion_visible
            && self.compact_detail == other.compact_detail
            && self.history_error == other.history_error
            && self.loading == other.loading
            && self.complete == other.complete
            && self.history_top_reached == other.history_top_reached
            && self.refreshing == other.refreshing
            && self.disclosure_generation == other.disclosure_generation
            && self.date == other.date
    }
}

#[cfg(feature = "performance-profiling")]
const PREPARATION_REASONS: [&str; 21] = [
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
    "disclosure",
    "date",
];
#[cfg(feature = "performance-profiling")]
static PREPARATION_REASON_COUNTS: [std::sync::atomic::AtomicU64; 21] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 21];

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
        let mut flags = [false; 21];
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
            flags[10] = old.width != self.width;
            flags[11] = old.companion_width != self.companion_width;
            flags[12] = old.companion_visible != self.companion_visible;
            flags[13] = old.compact_detail != self.compact_detail;
            flags[14] = old.history_error != self.history_error;
            flags[15] = old.loading != self.loading;
            flags[16] = old.complete != self.complete;
            flags[17] = old.history_top_reached != self.history_top_reached;
            flags[18] = old.refreshing != self.refreshing;
            flags[19] = old.disclosure_generation != self.disclosure_generation;
            flags[20] = old.date != self.date;
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
    load_earlier: Rc<dyn Fn(&str, &mut App)>,
    load_unmeasured_history: Rc<dyn Fn(&str, &mut App)>,
    #[cfg(test)]
    render_count: usize,
    list: Option<Entity<StableTranscriptList>>,
    error: Option<String>,
    palette: design::Palette,
}
impl TranscriptViewport {
    pub(super) fn new(
        owner: WeakEntity<ChatPane>,
        task_id: String,
        list: Option<Entity<StableTranscriptList>>,
        error: Option<String>,
        palette: design::Palette,
    ) -> Self {
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
                    if state.list.is_scrolled_to_end().is_none()
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
            load_earlier,
            load_unmeasured_history,
            list,
            error,
            palette,
            #[cfg(test)]
            render_count: 0,
        }
    }
    pub(super) fn set_content(
        &mut self,
        task_id: String,
        list: Option<Entity<StableTranscriptList>>,
        error: Option<String>,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) {
        self.task_id = task_id;
        self.list = list;
        self.error = error;
        self.palette = palette;
        cx.notify();
    }
}
impl Render for TranscriptViewport {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        let load_unmeasured_history = self.load_unmeasured_history.clone();
        let scroll_task_id = self.task_id.clone();
        let mut transcript = div()
            .id("transcript")
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
                // An empty scroll extent does not prove older history should
                // load. Only an upward user gesture can fetch the earlier page.
                if event.delta.pixel_delta(px(20.)).y <= gpui_kit::px(0.) {
                    return;
                }
                load_unmeasured_history(&scroll_task_id, cx);
            });
        if let Some(error) = self.error.clone() {
            let load_earlier = self.load_earlier.clone();
            let retry_task_id = self.task_id.clone();
            transcript = transcript.child(transcript_column(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(self.palette.muted))
                    .child(error)
                    .child(
                        Button::new("retry-earlier-history")
                            .compact()
                            .label("Retry history")
                            .on_click(move |_, _, cx| {
                                load_earlier(&retry_task_id, cx);
                            }),
                    ),
            ));
        }
        if let Some(list) = self.list.as_ref() {
            transcript =
                transcript.child(list.clone().cached(StyleRefinement::default().size_full()));
        } else {
            transcript = transcript.child(transcript_column(div().child(
                "Select a chat or create one from Agents. Saved CLI authentication is used only when you send a message.",
            )));
        }
        transcript
    }
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "chat_render_isolation_tests.rs"]
mod tests;
