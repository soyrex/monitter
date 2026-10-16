//! Cache-only usage refresh scheduling, independent of rendering and providers.
use std::time::Duration;

const NORMAL_REFRESH_INTERVAL: Duration = Duration::from_secs(65);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UsageRequest {
    task_id: String,
    generation: u64,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct UsageRefresh {
    task_id: Option<String>,
    status: Option<String>,
    generation: u64,
    sequence: u64,
    in_flight: Option<UsageRequest>,
    urgent: bool,
    last_refresh: Option<Duration>,
}

impl UsageRefresh {
    /// Timestamp-only streaming activity does not change these dependencies.
    pub(super) fn observe(&mut self, task_id: &str, status: Option<&str>) {
        if self.task_id.as_deref() != Some(task_id) {
            self.task_id = Some(task_id.to_owned());
            self.generation = self.generation.wrapping_add(1);
            self.urgent = true;
        }
        if self.status.as_deref() != status {
            self.status = status.map(str::to_owned);
            self.urgent = true;
        }
    }

    pub(super) fn request_urgent(&mut self) {
        self.urgent = true;
    }

    pub(super) fn begin(&mut self, now: Duration) -> Option<UsageRequest> {
        if self.in_flight.is_some()
            || (!self.urgent
                && self
                    .last_refresh
                    .is_some_and(|last| now.saturating_sub(last) < NORMAL_REFRESH_INTERVAL))
        {
            return None;
        }
        let request = UsageRequest {
            task_id: self.task_id.clone()?,
            generation: self.generation,
            sequence: self.sequence.wrapping_add(1),
        };
        self.sequence = request.sequence;
        self.urgent = false;
        self.in_flight = Some(request.clone());
        Some(request)
    }

    /// Both success and failure consume the normal read budget. A status
    /// transition recorded during the read remains urgent for one follow-up.
    pub(super) fn complete(&mut self, request: &UsageRequest, now: Duration) -> bool {
        if self.in_flight.as_ref() != Some(request) {
            return false;
        }
        self.in_flight = None;
        let accepted = self.generation == request.generation
            && self.task_id.as_deref() == Some(request.task_id.as_str());
        if accepted {
            self.last_refresh = Some(now);
        }
        accepted
    }

    /// A shared cache publication supersedes old results, while their worker
    /// still owns the in-flight slot until it actually completes.
    pub(super) fn publish(&mut self, now: Duration) {
        self.generation = self.generation.wrapping_add(1);
        self.last_refresh = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn one_hundred_stream_markers_respect_normal_sixty_five_second_budget() {
        let mut refresh = UsageRefresh::default();
        refresh.observe("task", Some("running"));
        let first = refresh.begin(ms(0)).unwrap();
        assert!(refresh.complete(&first, ms(1)));
        for timestamp in 1..=100 {
            // updatedAt changes in the containing snapshot; selected task and
            // status remain the same inputs used by the actual footer.
            refresh.observe("task", Some("running"));
            assert!(refresh.begin(ms(timestamp * 100)).is_none());
        }
        assert!(refresh.begin(ms(65_000)).is_none());
        let next = refresh.begin(ms(65_001)).unwrap();
        assert!(refresh.complete(&next, ms(65_010)));
        assert!(refresh.begin(ms(65_011)).is_none());
        assert_eq!(refresh.sequence, 2);
    }

    #[test]
    fn completion_during_a_read_coalesces_to_one_prompt_trailing_refresh() {
        let mut refresh = UsageRefresh::default();
        refresh.observe("task", Some("running"));
        let first = refresh.begin(ms(0)).unwrap();
        refresh.observe("task", Some("idle"));
        for _ in 0..100 {
            refresh.observe("task", Some("idle"));
            assert!(refresh.begin(ms(10)).is_none());
        }
        assert!(refresh.complete(&first, ms(11)));
        let trailing = refresh.begin(ms(11)).unwrap();
        assert!(refresh.complete(&trailing, ms(12)));
        assert!(refresh.begin(ms(13)).is_none());
        assert_eq!(refresh.sequence, 2);
    }

    #[test]
    fn task_switch_and_return_fence_old_results_without_concurrent_reads() {
        let mut refresh = UsageRefresh::default();
        refresh.observe("old", Some("idle"));
        let old = refresh.begin(ms(0)).unwrap();
        refresh.observe("new", Some("idle"));
        refresh.observe("old", Some("idle"));
        assert!(refresh.begin(ms(1)).is_none());
        assert!(!refresh.complete(&old, ms(2)));
        let current = refresh.begin(ms(2)).unwrap();
        assert!(!refresh.complete(&old, ms(3)));
        assert!(refresh.begin(ms(3)).is_none());
        assert!(refresh.complete(&current, ms(4)));
        assert!(refresh.begin(ms(5)).is_none());
    }

    #[test]
    fn cache_publication_supersedes_old_result_and_holds_actual_worker_slot() {
        let mut refresh = UsageRefresh::default();
        refresh.observe("task", Some("running"));
        let old = refresh.begin(ms(0)).unwrap();
        refresh.publish(ms(10));
        assert!(refresh.begin(ms(11)).is_none());
        assert!(!refresh.complete(&old, ms(12)));
        assert!(refresh.begin(ms(65_009)).is_none());
        assert!(refresh.begin(ms(65_010)).is_some());
    }

    #[test]
    fn publication_does_not_lose_completion_or_explicit_refresh_queued_in_flight() {
        let mut refresh = UsageRefresh::default();
        refresh.observe("task", Some("running"));
        let old = refresh.begin(ms(0)).unwrap();
        refresh.observe("task", Some("idle"));
        refresh.publish(ms(1));
        assert!(!refresh.complete(&old, ms(2)));
        let completion = refresh.begin(ms(2)).unwrap();
        for _ in 0..100 {
            refresh.request_urgent();
        }
        assert!(refresh.begin(ms(3)).is_none());
        assert!(refresh.complete(&completion, ms(4)));
        let explicit = refresh.begin(ms(4)).unwrap();
        assert!(refresh.complete(&explicit, ms(5)));
        assert!(refresh.begin(ms(6)).is_none());
        assert_eq!(refresh.sequence, 3);
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn native_footer_cache_publication_survives_old_read_and_stream_markers(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::super::ComposerFooterPane;
        use gpui_kit::AppContext as _;
        use monitter_lib::experimental_desktop::DesktopService;
        use serde_json::json;
        cx.update(gpui_kit::init);
        // Archived prevents model-catalog reads; the disconnected service can
        // perform no provider calls, profile access or backend mutation.
        let snapshot = json!({"settings":{},"agents":[],"tasks":[{
            "id":"task","provider":"claude","archived":true,"updatedAt":0,"status":"running"
        }]});
        let published = json!({"runs":[{"id":"newer-published-cache"}]});
        let (pane, visual) = cx.add_window_view(|window, cx| {
            let mut pane = ComposerFooterPane::new(
                DesktopService::disconnected_for_ui_fixture(),
                &snapshot,
                "task",
                window,
                cx,
            );
            assert_eq!(pane.usage_refresh.sequence, 1);
            pane.set_usage_overview(&published, cx);
            assert!(
                pane.usage_refresh.in_flight.is_some(),
                "publication cannot release an actual worker slot"
            );
            pane
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                assert_eq!(pane.usage_overview.as_ref(), Some(&published));
                assert!(
                    pane.usage_error.is_none(),
                    "superseded failed read cannot replace published cache"
                );
                for marker in 1..=100 {
                    let mut updated = snapshot.clone();
                    updated["tasks"][0]["updatedAt"] = json!(marker);
                    pane.set_snapshot(&updated, "task", cx);
                }
                assert_eq!(pane.usage_refresh.sequence, 1);
                assert!(!pane.usage_loading);
                let mut completed = snapshot.clone();
                completed["tasks"][0]["updatedAt"] = json!(101);
                completed["tasks"][0]["status"] = json!("idle");
                pane.set_snapshot(&completed, "task", cx);
                assert_eq!(
                    pane.usage_refresh.sequence, 2,
                    "completion bypasses the normal budget"
                );
            })
        });
    }
}
