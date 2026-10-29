use super::{ContextKey, RefreshState, TaskMarker, task_context, workspace_status_ready};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::json;
use std::time::Duration;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn context(
    task: &str,
    host: &str,
    cwd: &str,
    updated: i64,
    status: &str,
) -> Option<(ContextKey, TaskMarker)> {
    Some((
        ContextKey {
            task_id: task.into(),
            host_id: host.into(),
            cwd: cwd.into(),
        },
        TaskMarker {
            updated_at: Some(updated),
            status: status.into(),
        },
    ))
}

fn running(updated: i64) -> Option<(ContextKey, TaskMarker)> {
    context("task", "host", "/workspace", updated, "running")
}

#[::core::prelude::v1::test]
fn unrelated_native_revisions_neither_refresh_nor_fence_same_scope_result() {
    let mut snapshot = json!({"tasks":[
        {"id":"task","hostId":"host","cwd":"/workspace","updatedAt":1,"status":"running"},
        {"id":"other","hostId":"host","cwd":"/other","updatedAt":1,"status":"running"}
    ]});
    let mut state = RefreshState::default();
    state.observe(task_context(&snapshot, "task"));
    state.set_visible(true);
    let request = state.begin(ms(0)).unwrap();
    for revision in 1..=100 {
        snapshot["tasks"][1]["updatedAt"] = json!(revision);
        snapshot["revision"] = json!(format!("epoch:{revision}"));
        assert!(!state.observe(task_context(&snapshot, "task")));
        assert!(state.begin(ms(revision as u64)).is_none());
    }
    assert!(
        state.complete(&request),
        "same working tree remains valid through unrelated revisions"
    );
    assert!(state.deadline(ms(10_000)).is_none());
    assert_eq!(state.sequence, 1);
}

#[::core::prelude::v1::test]
fn continuous_task_activity_is_bounded_and_final_marker_gets_trailing_read() {
    let mut state = RefreshState::default();
    state.observe(running(0));
    state.set_visible(true);
    let first = state.begin(ms(0)).unwrap();
    assert!(state.complete(&first));
    for update in 1..=99 {
        state.observe(running(update));
        let now = ms(update as u64 * 10);
        assert_eq!(state.deadline(now), Some(ms(1_000)));
        assert!(state.begin(now).is_none());
    }
    let second = state.begin(ms(1_000)).unwrap();
    for update in 100..=199 {
        state.observe(running(update));
        assert!(
            state.begin(ms(update as u64 * 10)).is_none(),
            "one request remains in flight"
        );
    }
    assert!(state.complete(&second));
    assert_eq!(state.deadline(ms(1_999)), Some(ms(2_000)));
    let trailing = state.begin(ms(2_000)).unwrap();
    assert_eq!(state.marker.as_ref().unwrap().updated_at, Some(199));
    assert!(state.complete(&trailing));
    assert!(
        state.deadline(ms(10_000)).is_none(),
        "unchanged task does not poll"
    );
    assert_eq!(state.sequence, 3);
}

#[::core::prelude::v1::test]
fn completion_and_explicit_git_actions_bypass_budget_but_coalesce_in_flight() {
    let mut state = RefreshState::default();
    state.observe(running(1));
    state.set_visible(true);
    let first = state.begin(ms(0)).unwrap();
    state.observe(context("task", "host", "/workspace", 2, "idle"));
    for _ in 0..100 {
        state.request_urgent();
        assert!(state.begin(ms(10)).is_none());
    }
    assert!(state.complete(&first));
    let completion = state.begin(ms(10)).unwrap();
    assert!(state.complete(&completion));
    assert!(state.deadline(ms(11)).is_none());
    state.request_urgent();
    let explicit = state.begin(ms(11)).unwrap();
    assert!(state.complete(&explicit));
    assert!(state.deadline(ms(100_000)).is_none());
    assert_eq!(state.sequence, 3);
}

#[::core::prelude::v1::test]
fn scope_changes_fence_task_host_cwd_and_aba_without_overlapping_workers() {
    for next in [
        context("another-task", "host", "/workspace", 1, "running"),
        context("task", "another-host", "/workspace", 1, "running"),
        context("task", "host", "/another-cwd", 1, "running"),
    ] {
        let mut state = RefreshState::default();
        state.observe(running(1));
        state.set_visible(true);
        let old = state.begin(ms(0)).unwrap();
        assert!(state.observe(next.clone()));
        assert!(state.begin(ms(1)).is_none());
        assert!(!state.complete(&old));
        let current = state.begin(ms(1)).unwrap();
        assert_eq!(Some(current.context.clone()), next.map(|(key, _)| key));
        assert!(
            !state.complete(&old),
            "late duplicate completion cannot release the current slot"
        );
        assert!(state.in_flight.is_some());
        assert!(state.complete(&current));
    }
    let mut state = RefreshState::default();
    state.observe(running(1));
    state.set_visible(true);
    let old = state.begin(ms(0)).unwrap();
    state.observe(context("other", "host", "/workspace", 1, "running"));
    state.observe(running(1));
    assert!(
        !state.complete(&old),
        "returning to the original scope does not revive an old request"
    );
    assert!(state.begin(ms(1)).is_some());
}

#[::core::prelude::v1::test]
fn hidden_panes_schedule_nothing_and_reveal_refreshes_latest_scope_immediately() {
    let mut state = RefreshState::default();
    state.observe(running(1));
    state.request_urgent();
    assert!(state.deadline(ms(0)).is_none());
    state.set_visible(true);
    let first = state.begin(ms(0)).unwrap();
    assert!(state.complete(&first));
    state.observe(running(2));
    assert_eq!(state.deadline(ms(100)), Some(ms(1_000)));
    state.set_visible(false);
    for marker in 3..100 {
        state.observe(running(marker));
        assert!(state.begin(ms(100)).is_none());
    }
    state.set_visible(true);
    let revealed = state.begin(ms(100)).unwrap();
    assert_eq!(state.marker.as_ref().unwrap().updated_at, Some(99));
    state.set_visible(false);
    assert!(
        state.complete(&revealed),
        "already-dispatched same-scope result may fill the hidden cache"
    );
    assert!(state.deadline(ms(200)).is_none());
    state.set_visible(true);
    assert!(
        state.begin(ms(200)).is_some(),
        "reveal refreshes even when the old read completed hidden"
    );
}

#[::core::prelude::v1::test]
fn startup_readiness_deferral_keeps_the_initial_git_refresh_pending() {
    let service = DesktopService::pending();
    let mut state = RefreshState::default();
    state.observe(running(1));
    state.set_visible(true);

    // The startup gate may defer dispatch, but must not consume the initial
    // dirty marker as a failed request. Once commands are authorized, the
    // retained strip starts exactly one urgent read.
    assert!(!workspace_status_ready(&service));
    assert_eq!(state.deadline(ms(0)), Some(ms(0)));
    assert!(state.in_flight.is_none());
    service.enable_workspace_commands();
    assert!(workspace_status_ready(&service));
    let ready_request = state.begin(ms(0)).expect("read starts after readiness");
    assert!(state.complete(&ready_request));
    assert!(state.deadline(ms(10_000)).is_none());
}

#[::core::prelude::v1::test]
fn missing_task_or_empty_cwd_fences_result_and_stops_refresh_until_scope_returns() {
    let mut state = RefreshState::default();
    state.observe(running(1));
    state.set_visible(true);
    let old = state.begin(ms(0)).unwrap();
    assert!(state.observe(None));
    assert!(!state.complete(&old));
    assert!(state.deadline(ms(10_000)).is_none());
    assert!(task_context(&json!({"tasks":[{"id":"task","cwd":"  "}]}), "task").is_none());
    assert!(task_context(&json!({"tasks":[]}), "task").is_none());
    state.observe(running(2));
    assert!(state.begin(ms(10_001)).is_some());
}

#[cfg(feature = "visual-export")]
#[gpui_kit::test]
fn native_git_strip_keeps_hidden_context_without_dispatching_workers(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Window, div};
    use monitter_lib::experimental_desktop::DesktopService;
    struct Host(Entity<super::ComposerGitStrip>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let _ = &self.0;
            div()
        }
    }
    cx.update(gpui_kit::init);
    let mut strip = None;
    let snapshot = json!({"settings":{},"tasks":[{"id":"task","hostId":"host","cwd":"/workspace","updatedAt":0,"status":"running"}]});
    let (_, visual) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| {
            super::ComposerGitStrip::new(
                DesktopService::disconnected_for_ui_fixture(),
                &snapshot,
                Some("epoch:0"),
                "task",
                window,
                cx,
            )
        });
        strip = Some(entity.clone());
        Host(entity)
    });
    let strip = strip.unwrap();
    visual.update(|_, cx| {
        strip.update(cx, |strip, cx| {
            for revision in 1..=100 {
                let mut changed = snapshot.clone();
                changed["tasks"][0]["updatedAt"] = json!(revision);
                strip.set_context(&changed, Some(&format!("epoch:{revision}")), "task", cx);
            }
            assert_eq!(strip.refresh.sequence, 0);
            assert!(strip.refresh_timer.is_none());
            assert!(strip.status.is_none());
            // Visibility transitions are covered by RefreshState unit tests.
            // This native fixture verifies hidden behavior without creating a real
            // OS worker that can wake the deterministic GPUI scheduler off-thread.
            strip.refresh_now(cx);
            assert_eq!(strip.refresh.sequence, 0);
            assert!(strip.refresh_timer.is_none());
        })
    });
}
