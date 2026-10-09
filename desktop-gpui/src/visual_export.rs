//! Opt-in development export of this GPUI window's rendered frame.
//!
//! This uses GPUI's renderer output, not a desktop screenshot API. The module is
//! gated by the parent crate's `visual-export` development feature.

use gpui_kit::{AnyWindowHandle, App, AppContext, KeyBinding};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

gpui_kit::actions!(monitter_visual_export, [ExportPreviewFrame]);

/// Local development capture, available only in the opt-in export build.
/// Keeps repeatable visual inspection independent of MCP inline image limits.
pub fn install(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-alt-g", ExportPreviewFrame, None)]);
    cx.on_action(|_: &ExportPreviewFrame, cx| {
        if let Some(window) = cx.active_window() {
            // Action dispatch temporarily removes the active Window from App.
            // Re-enter only after dispatch returns it to the window registry.
            cx.defer(move |cx| {
                schedule(window, cx);
                cx.refresh_windows();
            });
        }
    });
}

/// Schedule a one-shot PNG export when `MONITTER_GPUI_EXPORT_FRAME=1`.
///
/// Capture occurs two seconds after the window's next completed frame. The
/// image is saved below the fork's `artifacts/gpui/visual` directory.
pub fn schedule(window: AnyWindowHandle, cx: &mut App) {
    if std::env::var_os("MONITTER_GPUI_EXPORT_FRAME").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }

    let capture_window = window;
    window
        .update(cx, |_, window, _| {
            window.on_next_frame(move |_, cx| {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;

                    let frame =
                        cx.update_window(capture_window, |_, window, _| window.render_to_image());
                    match frame {
                        Ok(Ok(frame)) => {
                            let path = output_path();
                            cx.background_executor()
                                .spawn(async move {
                                    if let Some(parent) = path.parent() {
                                        if let Err(error) = std::fs::create_dir_all(parent) {
                                            eprintln!(
                                                "GPUI frame export failed for {}: {error}",
                                                path.display()
                                            );
                                            return;
                                        }
                                    }
                                    match frame.save(&path) {
                                        Ok(()) => {
                                            eprintln!("GPUI frame exported: {}", path.display())
                                        }
                                        Err(error) => eprintln!(
                                            "GPUI frame export failed for {}: {error}",
                                            path.display()
                                        ),
                                    }
                                })
                                .detach();
                        }
                        Ok(Err(error)) => {
                            eprintln!("GPUI frame export failed during rendering: {error}");
                        }
                        Err(error) => {
                            eprintln!("GPUI frame export failed; window unavailable: {error}");
                        }
                    }
                })
                .detach();
            });
        })
        .unwrap_or_else(|error| eprintln!("GPUI frame export could not be scheduled: {error}"));
}

fn output_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("artifacts/gpui/visual")
        .join(format!("gpui-frame-{timestamp}.png"))
}
