//! Revision-checked history requests, never blocking the UI thread.
use super::*;
impl chat::ChatPane {
    /// Presentation-only seed for the cached-to-live handoff. This deliberately
    /// leaves latest_history_loaded and pagination/request ownership untouched:
    /// the real latest page still refreshes asynchronously from the backend.
    pub(super) fn seed_startup_messages(&mut self, mut messages: Vec<Value>, cx: &mut Context<Self>) {
        if self.latest_history_loaded || messages.is_empty() { return; }
        let task_id = self.task_id();
        let state = self.transcripts.entry(task_id).or_default();
        // Metadata can already contain a system notice or a newer streamed
        // row. Preserve those authoritative rows while filling missing bodies.
        messages.retain(|message| !state.messages.iter().any(|known| known["id"] == message["id"]));
        if messages.is_empty() { return; }
        self.startup_seed_ids.extend(messages.iter().map(|message| field(message, "id")));
        state.merge_compact(messages);
        self.invalidate_transcript_list(cx);
    }

    /// Warm the latest page as soon as a pane is materialized, including while
    /// its workspace is in the background. This is separate from the
    /// older-page cursor: `None` remains the request cursor even if a live
    /// delta has already placed newer rows in the transcript.
    pub(super) fn ensure_latest_history(&mut self, cx: &mut Context<Self>) {
        if self.latest_history_loaded
            || self.latest_history_inflight
            || self.history_warmup_pending
            || self.history_page_inflight
        {
            return;
        }
        let Some(task_id) = self.selected.clone() else {
            return;
        };
        let Some(service) = self.service.clone() else {
            return;
        };
        let state = self.transcripts.entry(task_id.clone()).or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.error = None;
        self.latest_history_inflight = true;
        self.history_page_inflight = true;
        Self::request_history_page(task_id, service, None, cx);
    }

    pub(super) fn load_earlier(&mut self, cx: &mut Context<Self>) {
        // An Unknown/empty boundary can mean that native metadata arrived
        // without message coverage. Establish the latest-page boundary first;
        // never infer that absent rows mean a complete empty transcript.
        if !self.latest_history_loaded {
            self.ensure_latest_history(cx);
            return;
        }
        if self.history_page_inflight {
            return;
        }
        let Some(task_id) = self.selected.clone() else {
            return;
        };
        let Some(service) = self.service.clone() else {
            return;
        };
        let state = self.transcripts.entry(task_id.clone()).or_default();
        if state.loading || state.complete() {
            return;
        }
        state.loading = true;
        state.error = None;
        let before_id = state.before_id();
        self.history_page_inflight = true;
        Self::request_history_page(task_id, service, before_id, cx);
    }

    pub(super) fn set_history_warmup_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.history_warmup_pending = pending;
        if !pending && self.dock_visible && !self.latest_history_loaded {
            self.ensure_latest_history(cx);
        }
    }

    pub(super) fn accept_warmed_history(&mut self, page: &TaskMessagesPage, cx: &mut Context<Self>) {
        self.history_warmup_pending = false;
        if self.latest_history_loaded || self.latest_history_inflight || self.history_page_inflight {
            return;
        }
        let Ok(page_value) = serde_json::to_value(page) else { return; };
        let startup_ids = std::mem::take(&mut self.startup_seed_ids);
        let task_id = self.task_id();
        let state = self.transcripts.entry(task_id).or_default();
        state.loading = false;
        state.error = None;
        state.merge_startup_page(&page_value, &startup_ids);
        self.latest_history_loaded = true;
        let known_messages = state.messages.iter().cloned().collect::<Vec<_>>();
        self.composer_footer.update(cx, |footer, cx| {
            footer.set_context_messages(known_messages, cx)
        });
        cx.notify();
    }

    fn request_history_page(
        task_id: String,
        service: DesktopService,
        before_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let request_task = task_id.clone();
        let task = cx.background_executor().spawn(async move {
            // Startup only needs a viewport-sized page. Explicit older-history
            // demand can fetch further ahead without delaying first paint.
            let limit = if before_id.is_some() { 100 } else { 20 };
            service.read_task_messages(&request_task, before_id.as_deref(), Some(limit))
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |_, cx| {
                cx.emit(chat::ChatEvent::HistoryPage {
                    task_id,
                    page: result,
                })
            });
        })
        .detach();
        cx.notify();
    }
}
