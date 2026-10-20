//! Finite, opt-in numeric performance observations from GPUI's own profiler.
//!
//! This does not generate input, request frames, inspect content, or contact a
//! backend. Input latency ends at platform submission, not physical display.
use gpui_kit::{AnyWindowHandle, App, AppContext};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn seconds(name: &str, default: u64, maximum: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
        .clamp(1, maximum)
}

macro_rules! summary {
    ($histogram:expr, $scale:expr) => {{
        let histogram = &$histogram;
        if histogram.is_empty() {
            json!({"count": 0})
        } else {
            json!({
                "count": histogram.len(),
                "min": histogram.min() as f64 / $scale,
                "mean": histogram.mean() / $scale,
                "p50": histogram.value_at_quantile(0.5) as f64 / $scale,
                "p95": histogram.value_at_quantile(0.95) as f64 / $scale,
                "p99": histogram.value_at_quantile(0.99) as f64 / $scale,
                "max": histogram.max() as f64 / $scale,
            })
        }
    }};
}

/// Starts a single observation only when an explicit output path is supplied.
/// Waiting and writing are off the UI thread. Two small histogram snapshots
/// bracket the observation; no recurring task or repaint remains afterward.
pub fn schedule(window: AnyWindowHandle, cx: &mut App) {
    let Some(path) = std::env::var_os("MONITTER_GPUI_PERF_OUTPUT").map(PathBuf::from) else {
        return;
    };
    let warmup = seconds("MONITTER_GPUI_PERF_WARMUP_SECONDS", 5, 300);
    let duration = seconds("MONITTER_GPUI_PERF_SECONDS", 30, 600);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(warmup)).await;
        let Ok((start_input, start_frames)) = cx.update_window(window, |_, window, _| {
            crate::performance_scenario::begin_window_observation(window);
            (window.input_latency_snapshot(), window.frame_duration_snapshot())
        }) else { return; };
        let start_transcript = crate::chat::transcript_preparation_counts();
        let start_chrome = crate::chat::chrome_render_counts();
        let start_reasons = crate::chat::transcript_preparation_reason_counts();
        let start_outline = crate::native_pane_highlight::performance_counts();
        let start_browser = crate::browser::performance_counts();
        let start_webview = gpui_wry::performance_counts();
        let start_dock = crate::docking::visibility_performance_counts();
        let started = Instant::now();
        let started_at = SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|time| time.as_millis() as u64);
        cx.background_executor().timer(Duration::from_secs(duration)).await;
        let Ok((mut input, mut frames, window_conditions)) = cx.update_window(window, |_, window, _| {
            (window.input_latency_snapshot(), window.frame_duration_snapshot(),
             crate::performance_scenario::finish_window_observation(window))
        }) else { return; };
        let subtraction = [
            input.latency_histogram.subtract(&start_input.latency_histogram),
            input.events_per_frame_histogram.subtract(&start_input.events_per_frame_histogram),
            frames.draw_duration_histogram.subtract(&start_frames.draw_duration_histogram),
            frames.dirty_to_present_histogram.subtract(&start_frames.dirty_to_present_histogram),
            frames.present_interval_histogram.subtract(&start_frames.present_interval_histogram),
        ];
        if subtraction.iter().any(Result::is_err) {
            eprintln!("GPUI performance observation discarded: histogram reset during measurement");
            return;
        }
        let measured_seconds = started.elapsed().as_secs_f64();
        let finished_at = SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|time| time.as_millis() as u64);
        let transcript = crate::chat::transcript_preparation_counts();
        let chrome = crate::chat::chrome_render_counts();
        let preparation_reasons: serde_json::Map<String, serde_json::Value> =
            crate::chat::transcript_preparation_reason_counts().into_iter().zip(start_reasons)
                .map(|((reason, count), (_, start))| (reason.to_owned(), json!(count.saturating_sub(start))))
                .collect();
        let outline = crate::native_pane_highlight::performance_counts();
        let browser = crate::browser::performance_counts();
        let webview = gpui_wry::performance_counts();
        let dock = crate::docking::visibility_performance_counts();
        let output = json!({
            "schemaVersion": 1,
            "pid": std::process::id(),
            "windowConditions": window_conditions,
            "debugAssertions": cfg!(debug_assertions),
            "visualExport": cfg!(feature = "visual-export"),
            "profilerEnabled": true,
            "warmupSeconds": warmup,
            "requestedSeconds": duration,
            "measuredSeconds": measured_seconds,
            "measurementStartedAtUnixMs": started_at,
            "measurementFinishedAtUnixMs": finished_at,
            "inputToPlatformSubmitMs": summary!(input.latency_histogram, 1_000_000.0),
            "inputEventsPerFrame": summary!(input.events_per_frame_histogram, 1.0),
            "midDrawInputEventsExcluded": input.mid_draw_events_dropped.saturating_sub(start_input.mid_draw_events_dropped),
            "drawMs": summary!(frames.draw_duration_histogram, 1_000_000.0),
            "dirtyToPlatformSubmitMs": summary!(frames.dirty_to_present_histogram, 1_000_000.0),
            "animationFrameIntervalMs": summary!(frames.present_interval_histogram, 1_000_000.0),
            "transcriptPreparations": transcript.0.saturating_sub(start_transcript.0),
            "transcriptRowsPrepared": transcript.1.saturating_sub(start_transcript.1),
            "chatChromeRenders": chrome.0.saturating_sub(start_chrome.0),
            "detailBodyConstructions": chrome.1.saturating_sub(start_chrome.1),
            "transcriptPreparationReasons": preparation_reasons,
            "outlineShows": outline.shows.saturating_sub(start_outline.shows),
            "outlineHides": outline.hides.saturating_sub(start_outline.hides),
            "outlineGeometryPaints": outline.geometry_paints.saturating_sub(start_outline.geometry_paints),
            "browser": {
                "showRequests": browser.show_requests.saturating_sub(start_browser.show_requests),
                "hideRequests": browser.hide_requests.saturating_sub(start_browser.hide_requests),
                "nativeShows": browser.successful_show_transitions.saturating_sub(start_browser.successful_show_transitions),
                "nativeHides": browser.successful_hide_transitions.saturating_sub(start_browser.successful_hide_transitions),
                "wrapperShows": webview.show_transitions.saturating_sub(start_webview.show_transitions),
                "wrapperHides": webview.hide_transitions.saturating_sub(start_webview.hide_transitions),
                "boundsWrites": webview.bounds_writes.saturating_sub(start_webview.bounds_writes),
                "dockPaintedObservations": dock.painted_pane_observations.saturating_sub(start_dock.painted_pane_observations),
                "dockMissingVisibleObservations": dock.missing_visible_pane_observations.saturating_sub(start_dock.missing_visible_pane_observations),
            },
            "limitations": [
                "Input timing starts at GPUI dispatch, excludes OS event queue wait and physical display scanout.",
                "Each input-latency sample represents a presented frame, measured from its oldest pending input event.",
                "Workload and window visibility are controlled by the observer, not this collector.",
                "Profiler instrumentation is enabled; compare builds with the same feature and workload.",
            ],
        });
        cx.background_executor().spawn(async move {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                // Avoid overwriting an earlier observation or unrelated file.
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
                serde_json::to_writer_pretty(&mut file, &output)?;
                Ok(())
            })();
            match result {
                Ok(()) => eprintln!("GPUI performance observation saved: {}", path.display()),
                Err(error) => eprintln!("GPUI performance observation failed: {error}"),
            }
        }).detach();
    }).detach();
}
