//! Revision-checked history requests, never blocking the UI thread.
use super::*;
impl chat::ChatPane {
    pub(super) fn load_earlier(&mut self, cx: &mut Context<Self>) {
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
