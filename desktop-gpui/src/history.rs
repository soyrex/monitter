//! Revision-checked history requests, never blocking the UI thread.
use super::*;
impl chat::ChatPane {
    /// Fetch the latest page once a pane is visible. This is separate from the
    /// older-page cursor: `None` remains the request cursor even if a live
    /// delta has already placed newer rows in the transcript.
    pub(super) fn ensure_latest_history(&mut self, cx: &mut Context<Self>) {
        if !self.dock_visible
            || self.latest_history_loaded
            || self.latest_history_inflight
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

    fn request_history_page(
        task_id: String,
        service: DesktopService,
        before_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let request_task = task_id.clone();
        let task = cx.background_executor().spawn(async move {
            service.read_task_messages(&request_task, before_id.as_deref(), Some(100))
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
