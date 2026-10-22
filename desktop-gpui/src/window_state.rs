//! Device-local native window placement. Never changes the shared workspace profile.
use crate::workspace_window_state::{MAX_WINDOWS, WindowId, WorkspaceWindowState};
use gpui_kit::*;
use serde_json::{Value, json};
const FILE: &str = "window-state.json";
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_NATIVE_BYTES: usize = 16 * 1024;
fn path() -> Option<std::path::PathBuf> {
    #[cfg(feature = "performance-profiling")]
    if crate::workspace_window_preview::enabled() {
        return None;
    }
    // Native fixtures must never read or overwrite the user's window placement.
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    monitter_lib::experimental_desktop::profile_dir()
        .ok()
        .map(|p| p.join(FILE))
}
fn cached() -> &'static std::sync::Mutex<Option<Value>> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<Option<Value>>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        use std::io::Read;
        let loaded = path()
            .and_then(|p| std::fs::File::open(p).ok())
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take((MAX_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .ok()?;
                if bytes.len() > MAX_FILE_BYTES {
                    return None;
                }
                let value: Value = serde_json::from_slice(&bytes).ok()?;
                valid_root(&value).then_some(value)
            });
        std::sync::Mutex::new(loaded)
    })
}
fn valid_root(value: &Value) -> bool {
    value.is_object()
        && value.get("windows").is_none_or(|windows| {
            windows.as_object().is_some_and(|windows| {
                windows.len() < MAX_WINDOWS
                    && windows.iter().all(|(key, placement)| {
                        WindowId::try_from(key.clone()).is_ok_and(|id| id != WindowId::main())
                            && placement.is_object()
                    })
            })
        })
}
fn read() -> Option<Value> {
    cached().lock().ok()?.clone()
}
/// A replaceable snapshot and a nonblocking wakeup keep disk latency out of
/// native event callbacks. The worker never holds the mailbox during I/O.
#[derive(Default)]
struct PendingWrite {
    value: Option<Value>,
    acknowledgements: Vec<futures::channel::oneshot::Sender<()>>,
}
struct Writer {
    pending: std::sync::Arc<std::sync::Mutex<PendingWrite>>,
    wake: std::sync::mpsc::SyncSender<()>,
}
fn writer() -> &'static Writer {
    static WRITER: std::sync::OnceLock<Writer> = std::sync::OnceLock::new();
    WRITER.get_or_init(|| {
        let pending = std::sync::Arc::new(std::sync::Mutex::new(PendingWrite::default()));
        let worker_pending = pending.clone();
        let (wake, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while receiver.recv().is_ok() {
                let batch = {
                    let Ok(mut pending) = worker_pending.lock() else {
                        break;
                    };
                    std::mem::take(&mut *pending)
                };
                if let Some(value) = batch.value {
                    if let Some(path) = path() {
                        let result = (|| -> std::io::Result<()> {
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            let bytes = serde_json::to_vec(&value)?;
                            if bytes.len() > MAX_FILE_BYTES {
                                return Err(std::io::Error::other(
                                    "Window state exceeds size limit",
                                ));
                            }
                            let temporary = path.with_extension("json.tmp");
                            std::fs::write(&temporary, bytes)?;
                            std::fs::rename(temporary, path)
                        })();
                        if let Err(error) = result {
                            eprintln!("Could not persist native window placement: {error}");
                        }
                    }
                }
                for acknowledgement in batch.acknowledgements {
                    let _ = acknowledgement.send(());
                }
            }
        });
        Writer { pending, wake }
    })
}
/// Called under the cache mutex so concurrent placement changes retain order.
/// The mailbox contains only the latest complete snapshot, never a backlog.
fn enqueue(value: Value, _flush: bool) {
    let writer = writer();
    if let Ok(mut pending) = writer.pending.lock() {
        pending.value = Some(value);
    }
    let _ = writer.wake.try_send(());
}
/// Quit awaits this barrier asynchronously after capturing all window state.
/// A full wake channel already guarantees the worker will drain the mailbox.
pub fn flush_pending() -> futures::channel::oneshot::Receiver<()> {
    let (send, receive) = futures::channel::oneshot::channel();
    let writer = writer();
    if let Ok(mut pending) = writer.pending.lock() {
        pending.acknowledgements.push(send);
    }
    let _ = writer.wake.try_send(());
    receive
}
fn update(mutator: impl FnOnce(&mut Value), flush: bool) -> Result<(), String> {
    let mut cache = cached()
        .lock()
        .map_err(|_| "Window state cache is unavailable")?;
    let mut value = cache.clone().unwrap_or_else(|| json!({"version":1}));
    mutator(&mut value);
    if !valid_root(&value) {
        return Err("Invalid or excessive window placements".into());
    }
    let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Window state exceeds size limit".into());
    }
    if cache.as_ref() != Some(&value) || flush {
        *cache = Some(value.clone());
        enqueue(value, flush);
    }
    Ok(())
}
fn placement<'a>(root: &'a Value, id: &WindowId) -> Option<&'a Value> {
    if id == &WindowId::main() {
        Some(root)
    } else {
        root.get("windows")?.get(id.as_str())
    }
}
fn set_placement(root: &mut Value, id: &WindowId, value: Value) {
    if id == &WindowId::main() {
        // Keep the version-1 root placement layout readable by older releases.
        if let Some(fields) = value.as_object() {
            for (key, value) in fields {
                root[key] = value.clone();
            }
        }
    } else {
        if !root.get("windows").is_some_and(Value::is_object) {
            root["windows"] = json!({});
        }
        root["windows"][id.as_str()] = value;
    }
}
/// The initial native host follows the persisted primary role.
pub fn primary_window_id() -> WindowId {
    // Fixtures share no production placement/profile and start with main.
    #[cfg(test)]
    return WindowId::main();
    #[cfg(not(test))]
    load_workspace_windows()
        .map(|state| state.primary_window_id().clone())
        .unwrap_or_else(WindowId::main)
}
pub fn load_workspace_windows() -> Option<WorkspaceWindowState> {
    let root = read()?;
    let bytes = serde_json::to_vec(root.get("workspaceWindows")?).ok()?;
    WorkspaceWindowState::decode(&bytes).ok()
}
pub fn save_workspace_windows(state: &WorkspaceWindowState, flush: bool) -> Result<(), String> {
    let value: Value = serde_json::from_slice(&state.encode()?).map_err(|e| e.to_string())?;
    update(
        |root| {
            root["workspaceWindows"] = value;
            // Closing/reconciling windows also drops their stale native placements.
            if let Some(placements) = root.get_mut("windows").and_then(Value::as_object_mut) {
                placements.retain(|key, _| {
                    state
                        .windows()
                        .iter()
                        .any(|w| w.window_id().as_str() == key)
                });
            }
        },
        flush,
    )
}
pub fn remove_placement_for(id: &WindowId) -> Result<(), String> {
    if id == &WindowId::main() {
        return Err("Primary placement cannot be removed".into());
    }
    update(
        |root| {
            if let Some(windows) = root.get_mut("windows").and_then(Value::as_object_mut) {
                windows.remove(id.as_str());
            }
        },
        false,
    )
}
#[derive(Clone, Copy)]
enum DeferredWindowMode {
    Windowed,
    Maximized,
    Fullscreen,
}
impl DeferredWindowMode {
    fn from_saved(saved: Option<&Value>) -> Self {
        match saved.and_then(|saved| saved["state"].as_str()) {
            Some("fullscreen") => Self::Fullscreen,
            Some("maximized") => Self::Maximized,
            _ => Self::Windowed,
        }
    }
    fn matches(self, window: &Window) -> bool {
        match self {
            Self::Fullscreen => window.is_fullscreen(),
            Self::Maximized => !window.is_fullscreen() && window.is_maximized(),
            Self::Windowed => !window.is_fullscreen() && !window.is_maximized(),
        }
    }
}
#[derive(Clone)]
struct HiddenNativeRestore {
    saved: Option<Value>,
    mode: DeferredWindowMode,
    revealed_at: Option<std::time::Instant>,
}
thread_local! {
    // Native window identity, rather than the currently selected workspace,
    // keeps placement protection scoped to the one initially hidden host.
    static HIDDEN_NATIVE_RESTORES: std::cell::RefCell<std::collections::HashMap<gpui_kit::WindowId, HiddenNativeRestore>> = Default::default();
}

/// Register placement/native-state restoration without touching AppKit while
/// the initial styled scene and session-lock authority are still preparing.
/// Pair with `initial_hidden` and call `finish_hidden_restore` after reveal.
pub fn restore_native_hidden(window: &Window, _cx: &App) {
    let saved = read().and_then(|root| placement(&root, &primary_window_id()).cloned());
    let mode = DeferredWindowMode::from_saved(saved.as_ref());
    HIDDEN_NATIVE_RESTORES.with(|pending| {
        pending
            .borrow_mut()
            .entry(window.window_handle().window_id())
            .or_insert(HiddenNativeRestore {
                saved,
                mode,
                revealed_at: None,
            });
    });
}

/// Apply the saved native mode only after the fully styled scene has been
/// drawn and `activate_window` requested. GPUI queues fullscreen/zoom changes
/// on the foreground executor; transient placement callbacks stay protected.
pub fn finish_hidden_restore(window: &Window, cx: &App) {
    let restore = HIDDEN_NATIVE_RESTORES.with(|pending| {
        let mut pending = pending.borrow_mut();
        let restore = pending.get_mut(&window.window_handle().window_id())?;
        if restore.revealed_at.is_some() {
            return None;
        }
        restore.revealed_at = Some(std::time::Instant::now());
        Some(restore.clone())
    });
    let Some(restore) = restore else {
        return;
    };
    if let Some(saved) = restore.saved.as_ref() {
        restore_native_placement(saved, window, cx);
    }
    // Native archive restoration is synchronous and may already restore zoom.
    // Check its resulting mode before issuing any toggle (never toggle twice).
    match restore.mode {
        DeferredWindowMode::Fullscreen if !window.is_fullscreen() => window.toggle_fullscreen(),
        DeferredWindowMode::Maximized if !window.is_maximized() && !window.is_fullscreen() => {
            window.zoom_window()
        }
        _ => {}
    }
}

fn hidden_restore_blocks_save(window: &Window) -> bool {
    HIDDEN_NATIVE_RESTORES.with(|pending| {
        let mut pending = pending.borrow_mut();
        let key = window.window_handle().window_id();
        let Some(restore) = pending.get(&key) else {
            return false;
        };
        let Some(revealed_at) = restore.revealed_at else {
            return true;
        };
        // A native transition normally completes within a few frames. Release
        // protection on the first matching mode, or after a bounded grace period
        // so an OS-denied transition cannot suppress future user placements.
        if !restore.mode.matches(window)
            && revealed_at.elapsed() < std::time::Duration::from_secs(5)
        {
            return true;
        }
        pending.remove(&key);
        false
    })
}

pub fn restore_native(window: &Window, cx: &App) {
    restore_native_for(&primary_window_id(), window, cx);
}
pub fn restore_native_for(id: &WindowId, window: &Window, cx: &App) {
    let Some(root) = read() else {
        return;
    };
    let Some(saved) = placement(&root, id) else {
        return;
    };
    restore_native_placement(saved, window, cx);
}
fn restore_native_placement(saved: &Value, window: &Window, cx: &App) {
    if saved["state"] == "fullscreen"
        || saved["displays"] != displays(cx)
        || saved["nativeStateBounds"] != saved["bounds"]
        || saved["nativeStateWindowMode"] != saved["state"]
    {
        return;
    }
    // Native restoration can bypass GPUI placement; reject offscreen geometry.
    let Some(bounds) = decode(&saved["bounds"]) else {
        return;
    };
    if !cx
        .displays()
        .iter()
        .any(|d| contains(d.visible_bounds(), bounds))
    {
        return;
    }
    if let Some(bytes) = saved["nativeState"]
        .as_array()
        .filter(|a| a.len() <= MAX_NATIVE_BYTES)
        .and_then(|a| {
            a.iter()
                .map(|n| n.as_u64().filter(|n| *n <= 255).map(|n| n as u8))
                .collect::<Option<Vec<_>>>()
        })
    {
        window.restore_native_window_state(&bytes);
    }
}
pub fn save_native(window: &Window, cx: &App) {
    save_native_for(&primary_window_id(), window, cx);
}
pub fn save_native_for(id: &WindowId, window: &Window, cx: &App) {
    save_inner(id, window, cx, true);
}
fn encode(b: Bounds<Pixels>) -> Value {
    json!([
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height)
    ])
}
fn decode(v: &Value) -> Option<Bounds<Pixels>> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut n = [0_f32; 4];
    for (i, v) in a.iter().enumerate() {
        n[i] = v.as_f64()? as f32;
        if !n[i].is_finite() {
            return None;
        }
    }
    if n[2] < 100. || n[3] < 100. {
        return None;
    }
    Some(Bounds::new(
        point(px(n[0]), px(n[1])),
        size(px(n[2]), px(n[3])),
    ))
}
fn displays(cx: &App) -> Value {
    let mut rows: Vec<Value> = cx.displays().iter().map(|d| json!({
        "uuid": d.uuid().ok().map(|u|u.to_string()), "bounds": encode(d.bounds()), "visible": encode(d.visible_bounds())
    })).collect();
    rows.sort_by_key(|v| v["uuid"].to_string());
    Value::Array(rows)
}
fn contains(visible: Bounds<Pixels>, bounds: Bounds<Pixels>) -> bool {
    bounds.origin.x >= visible.origin.x
        && bounds.origin.y >= visible.origin.y
        && bounds.right() <= visible.right()
        && bounds.bottom() <= visible.bottom()
}
fn clamp(mut bounds: Bounds<Pixels>, visible: Bounds<Pixels>) -> Bounds<Pixels> {
    bounds.size.width = bounds.size.width.min(visible.size.width);
    bounds.size.height = bounds.size.height.min(visible.size.height);
    bounds.origin.x = bounds
        .origin
        .x
        .max(visible.origin.x)
        .min(visible.right() - bounds.size.width);
    bounds.origin.y = bounds
        .origin
        .y
        .max(visible.origin.y)
        .min(visible.bottom() - bounds.size.height);
    bounds
}
pub fn display_id(cx: &App) -> Option<DisplayId> {
    display_id_for(&primary_window_id(), cx)
}
pub fn display_id_for(id: &WindowId, cx: &App) -> Option<DisplayId> {
    let root = read()?;
    let saved = placement(&root, id)?;
    let uuid = saved["displayUuid"].as_str()?;
    cx.displays()
        .iter()
        .find(|d| d.uuid().ok().map(|u| u.to_string()).as_deref() == Some(uuid))
        .map(|d| d.id())
}
pub fn initial(cx: &App) -> Option<WindowBounds> {
    initial_for(&primary_window_id(), cx)
}
/// Use the same clamped saved restore geometry without asking GPUI to queue
/// fullscreen/zoom during Window::new, before the initial hidden scene is ready.
pub fn initial_hidden(cx: &App) -> Option<WindowBounds> {
    initial(cx).map(|bounds| match bounds {
        WindowBounds::Windowed(bounds)
        | WindowBounds::Maximized(bounds)
        | WindowBounds::Fullscreen(bounds) => WindowBounds::Windowed(bounds),
    })
}
pub fn initial_for(id: &WindowId, cx: &App) -> Option<WindowBounds> {
    let root = read()?;
    let saved = placement(&root, id)?;
    let mut b = decode(&saved["restoreBounds"]).or_else(|| decode(&saved["bounds"]))?;
    let ds = cx.displays();
    let target = ds
        .iter()
        .find(|d| {
            saved["displayUuid"]
                .as_str()
                .is_some_and(|uuid| d.uuid().ok().map(|u| u.to_string()).as_deref() == Some(uuid))
        })
        .cloned()
        .or_else(|| ds.iter().find(|d| contains(d.visible_bounds(), b)).cloned())
        .or_else(|| cx.primary_display())?;
    // Also clamp when display IDs are unchanged: a corrupt/offscreen frame is
    // never allowed to hide a restored native workspace window.
    b = clamp(b, target.visible_bounds());
    Some(match saved["state"].as_str() {
        Some("fullscreen") => WindowBounds::Fullscreen(b),
        Some("maximized") => WindowBounds::Maximized(b),
        _ => WindowBounds::Windowed(b),
    })
}
pub fn save(window: &Window, cx: &App) {
    save_for(&primary_window_id(), window, cx);
}
pub fn save_for(id: &WindowId, window: &Window, cx: &App) {
    save_inner(id, window, cx, false);
}
fn save_inner(id: &WindowId, window: &Window, cx: &App, native: bool) {
    // The temporary windowed startup host must not overwrite the saved mode,
    // restore bounds or native archive before deferred restoration completes.
    if hidden_restore_blocks_save(window) {
        return;
    }
    let state = if window.is_fullscreen() {
        "fullscreen"
    } else if window.is_maximized() {
        "maximized"
    } else {
        "windowed"
    };
    let bounds = window.bounds();
    let native_bytes = native
        .then(|| window.native_window_state())
        .flatten()
        .filter(|b| b.len() <= MAX_NATIVE_BYTES);
    let result = update(
        |root| {
            let old = placement(root, id).cloned();
            let restore = match window.window_bounds() {
                WindowBounds::Fullscreen(b) => b,
                _ if state == "maximized" => old
                    .as_ref()
                    .and_then(|s| decode(&s["restoreBounds"]))
                    .unwrap_or(bounds),
                _ => bounds,
            };
            let mut value = json!({"version":1,"state":state,"bounds":encode(bounds),"restoreBounds":encode(restore),
            "displayUuid":window.display(cx).and_then(|d|d.uuid().ok()).map(|u|u.to_string()),"displays":displays(cx)});
            if let Some(bytes) = native_bytes {
                value["nativeState"] = json!(bytes);
                value["nativeStateBounds"] = value["bounds"].clone();
                value["nativeStateWindowMode"] = value["state"].clone();
            } else if let Some(old) = old.as_ref() {
                if old["bounds"] == value["bounds"]
                    && old["state"] == value["state"]
                    && old["displays"] == value["displays"]
                {
                    for key in ["nativeState", "nativeStateBounds", "nativeStateWindowMode"] {
                        if let Some(v) = old.get(key) {
                            value[key] = v.clone();
                        }
                    }
                }
            }
            // Root placement replacement must remove obsolete native fields while
            // retaining ownership and secondary placements.
            if id == &WindowId::main() {
                if let Some(fields) = root.as_object_mut() {
                    for key in ["nativeState", "nativeStateBounds", "nativeStateWindowMode"] {
                        fields.remove(key);
                    }
                }
            }
            set_placement(root, id, value);
        },
        native,
    );
    if let Err(error) = result {
        eprintln!("Could not save native window placement: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }
    #[test]
    fn offscreen_and_oversized_frames_are_clamped_for_monitor_migration() {
        let visible = b(-1920., 30., 1920., 1050.);
        let frame = clamp(b(9000., -2000., 2500., 1600.), visible);
        assert_eq!(frame, visible);
        let smaller = clamp(b(2000., 2000., 800., 600.), visible);
        assert!(contains(visible, smaller));
        assert_eq!(smaller.origin.x, px(-800.));
        assert_eq!(smaller.origin.y, px(480.));
    }
    #[test]
    fn main_placement_stays_legacy_compatible_and_secondary_is_independent() {
        let id = WindowId::try_from("workspace-window-1".to_owned()).unwrap();
        let mut root = json!({"version":1,"bounds":[0,0,800,600],"workspaceWindows":{"test":true}});
        set_placement(&mut root, &id, json!({"bounds":[20,30,900,700]}));
        set_placement(
            &mut root,
            &WindowId::main(),
            json!({"bounds":[10,10,600,400]}),
        );
        assert_eq!(root["bounds"], json!([10, 10, 600, 400]));
        assert_eq!(
            placement(&root, &id).unwrap()["bounds"],
            json!([20, 30, 900, 700])
        );
        assert_eq!(root["workspaceWindows"]["test"], true);
        assert!(valid_root(&root));
        root["windows"]["../../evil"] = json!({});
        assert!(!valid_root(&root));
    }
}

/// Resize the outer window on its current screen, using AppKit coordinates
/// throughout so secondary screens and the menu bar/Dock need no conversion.
#[cfg(target_os = "macos")]
pub fn center_at_percent(
    window: &mut Window,
    width_percent: u8,
    height_percent: u8,
    cx: &mut App,
) -> Result<(), String> {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if window.is_fullscreen() || window.is_simple_fullscreen() {
        return Err("Exit full screen before centering the window.".into());
    }
    let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("This command requires a macOS window.".into());
    };
    // Read geometry on the main thread, but defer mutation until the command
    // releases its Desktop/App/Window borrows (as GPUI's macOS resize does).
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let native: *mut AnyObject = msg_send![view, window];
        if native.is_null() {
            return Err("The native window is unavailable.".into());
        }
        let screen: *mut AnyObject = msg_send![native, screen];
        if screen.is_null() {
            return Err("The current screen is unavailable.".into());
        }
        let usable: NSRect = msg_send![screen, visibleFrame];
        let minimum: NSSize = msg_send![native, minSize];
        let width = (usable.size.width * f64::from(width_percent) / 100.)
            .max(minimum.width)
            .min(usable.size.width);
        let height = (usable.size.height * f64::from(height_percent) / 100.)
            .max(minimum.height)
            .min(usable.size.height);
        let frame = NSRect::new(
            NSPoint::new(
                usable.origin.x + (usable.size.width - width) / 2.,
                usable.origin.y + (usable.size.height - height) / 2.,
            ),
            NSSize::new(width, height),
        );
        let native = Retained::retain(native).ok_or("The native window is unavailable.")?;
        cx.foreground_executor()
            .spawn(async move {
                // Retaining prevents use-after-free if the user closes the window
                // before this turn. Do not resize a closed or hidden native window.
                let visible: bool = msg_send![&*native, isVisible];
                if visible {
                    let _: () = msg_send![&*native, setFrame: frame, display: true, animate: false];
                }
                // AppKit now delivers its normal GPUI resize/move callbacks with no
                // active command borrow. Those update the viewport and pane layout.
            })
            .detach();
    }
    Ok(())
}
