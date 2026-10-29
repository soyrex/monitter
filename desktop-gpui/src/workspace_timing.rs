//! Opt-in workspace latency observations. No names, pane IDs, or content are
//! logged. Paint means GPUI's paint observer ran, not physical presentation.
use gpui::{App, EntityId, Global, Window, WindowId};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
        OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILE_QUEUE_CAPACITY: usize = 256;
static FILE_SENDER: OnceLock<Option<SyncSender<String>>> = OnceLock::new();
static DROPPED_FILE_RECORDS: AtomicU64 = AtomicU64::new(0);

fn file_sender() -> Option<&'static SyncSender<String>> {
    FILE_SENDER.get_or_init(|| {
        let path = PathBuf::from(std::env::var_os("MONITTER_WORKSPACE_TIMING_LOG")?);
        if !path.is_absolute() {
            eprintln!("[workspace-timing-diagnostic] error=log-path-must-be-absolute");
            return None;
        }
        let (sender, receiver) = mpsc::sync_channel::<String>(FILE_QUEUE_CAPACITY);
        let writer = std::thread::Builder::new()
            .name("workspace-timing-log".into())
            .spawn(move || {
                let write = || -> std::io::Result<()> {
                    let mut options = std::fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)] {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut file = options.open(&path)?;
                    let session_ms = SystemTime::now().duration_since(UNIX_EPOCH)
                        .unwrap_or_default().as_millis();
                    writeln!(file, "[workspace-timing-diagnostic] pid={} session_unix_ms={} queue_capacity={}",
                        std::process::id(), session_ms, FILE_QUEUE_CAPACITY)?;
                    file.flush()?;
                    loop {
                        let disconnected = match receiver.recv_timeout(Duration::from_millis(100)) {
                            Ok(line) => { writeln!(file, "{line}")?; false },
                            Err(RecvTimeoutError::Timeout) => false,
                            Err(RecvTimeoutError::Disconnected) => true,
                        };
                        let dropped = DROPPED_FILE_RECORDS.swap(0, Ordering::Relaxed);
                        if dropped != 0 {
                            writeln!(file, "[workspace-timing-diagnostic] dropped_records={dropped}")?;
                        }
                        file.flush()?;
                        if disconnected { break; }
                    }
                    Ok(())
                };
                if let Err(error) = write() {
                    eprintln!("[workspace-timing-diagnostic] error=log-write-failed detail={error}");
                }
            });
        match writer {
            Ok(_) => Some(sender),
            Err(error) => {
                eprintln!("[workspace-timing-diagnostic] error=log-thread-failed detail={error}");
                None
            }
        }
    }).as_ref()
}

struct Pending {
    serial: u64,
    started: Instant,
    warmed: bool,
    shell_ready: bool,
    content_ready: bool,
    shell_painted: bool,
    unprepared_painted: bool,
    dock: Option<EntityId>,
    expected: HashSet<String>,
}
#[derive(Default)]
struct Timings { serial: u64, pending: HashMap<WindowId, Pending> }
impl Global for Timings {}

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MONITTER_WORKSPACE_TIMING").is_some_and(|value| value == "1"))
}
fn emit(pending: &Pending, phase: &str) {
    let line = format!("[workspace-timing] request={} warmed={} phase={} elapsed_us={}",
        pending.serial, pending.warmed, phase, pending.started.elapsed().as_micros());
    eprintln!("{line}");
    if let Some(sender) = file_sender() {
        match sender.try_send(line) {
            Ok(()) => {},
            Err(TrySendError::Full(_)) => {
                if DROPPED_FILE_RECORDS.fetch_add(1, Ordering::Relaxed) == 0 {
                    eprintln!("[workspace-timing-diagnostic] error=log-queue-full");
                }
            },
            Err(TrySendError::Disconnected(_)) => {
                eprintln!("[workspace-timing-diagnostic] error=log-writer-disconnected");
            },
        }
    }
}
pub(crate) fn request(window: &Window, started: Instant, warmed: bool, cx: &mut App) {
    if !enabled() { return; }
    if cx.try_global::<Timings>().is_none() { cx.set_global(Timings::default()); }
    let timings = cx.global_mut::<Timings>();
    let host = window.window_handle().window_id();
    if let Some(previous) = timings.pending.remove(&host) { emit(&previous, "superseded"); }
    // Instrumentation remains bounded even if native hosts disappear before a
    // requested workspace reaches paint.
    if timings.pending.len() >= 16 {
        if let Some(oldest) = timings.pending.iter().min_by_key(|(_, pending)| pending.started).map(|(host, _)| *host) {
            if let Some(evicted) = timings.pending.remove(&oldest) {
                emit(&evicted, "evicted-capacity");
            }
        }
    }
    timings.serial = timings.serial.wrapping_add(1);
    let pending = Pending { serial: timings.serial, started, warmed,
        shell_ready: false, content_ready: false, shell_painted: false, unprepared_painted: false,
        dock: None, expected: HashSet::new() };
    emit(&pending, "requested");
    timings.pending.insert(host, pending);
}
/// Emits no workspace, pane, task, or content identifier.
pub(crate) fn readiness(reason: &'static str, rows: usize) {
    if !enabled() { return; }
    let line = format!("[workspace-timing-diagnostic] readiness={} rows={}", reason, rows);
    eprintln!("{line}");
    if let Some(sender) = file_sender() {
        match sender.try_send(line) {
            Ok(()) => {},
            Err(TrySendError::Full(_)) => { DROPPED_FILE_RECORDS.fetch_add(1, Ordering::Relaxed); },
            Err(TrySendError::Disconnected(_)) => {},
        }
    }
}
pub(crate) fn cancel(window: &Window, cx: &mut App) {
    end(window, "superseded", cx);
}
pub(crate) fn failed(window: &Window, cx: &mut App) {
    end(window, "failed", cx);
}
fn end(window: &Window, phase: &str, cx: &mut App) {
    if !enabled() || cx.try_global::<Timings>().is_none() { return; }
    if let Some(pending) = cx.global_mut::<Timings>().pending.remove(&window.window_handle().window_id()) {
        emit(&pending, phase);
    }
}
pub(crate) fn shell_ready(window: &Window, cx: &mut App) {
    if !enabled() || cx.try_global::<Timings>().is_none() { return; }
    if let Some(pending) = cx.global_mut::<Timings>().pending.get_mut(&window.window_handle().window_id()) {
        if !pending.shell_ready { pending.shell_ready = true; emit(pending, "shell-ready"); }
    }
}
pub(crate) fn content_ready(window: &Window, cx: &mut App) {
    if !enabled() || cx.try_global::<Timings>().is_none() { return; }
    if let Some(pending) = cx.global_mut::<Timings>().pending.get_mut(&window.window_handle().window_id()) {
        if !pending.content_ready { pending.content_ready = true; emit(pending, "content-ready"); }
    }
}
pub(crate) fn mount(window: &Window, dock: EntityId, expected: HashSet<String>, cx: &mut App) {
    if !enabled() || cx.try_global::<Timings>().is_none() { return; }
    if let Some(pending) = cx.global_mut::<Timings>().pending.get_mut(&window.window_handle().window_id()) {
        pending.dock = Some(dock);
        pending.expected = expected;
    }
}
pub(crate) fn needs_content_readiness(window: &Window, dock: EntityId, cx: &App) -> bool {
    enabled() && cx.try_global::<Timings>().and_then(|timings|
        timings.pending.get(&window.window_handle().window_id()))
        .is_some_and(|pending| pending.dock == Some(dock))
}
/// Called by the existing dock visibility probe after GPUI observed pane paint.
/// It adds no callbacks, frame demand, or periodic sampling of its own.
pub(crate) fn observed_paint(window: &Window, dock: EntityId, expected: &HashSet<String>, painted: &HashSet<String>, ready_after_render: bool, cx: &mut App) {
    if !enabled() || cx.try_global::<Timings>().is_none() { return; }
    let host = window.window_handle().window_id();
    let timings = cx.global_mut::<Timings>();
    let Some(pending) = timings.pending.get_mut(&host).filter(|pending| pending.dock == Some(dock)) else { return; };
    pending.expected.clone_from(expected);
    if !pending.shell_painted { pending.shell_painted = true; emit(pending, "shell-paint-observed"); }
    if pending.content_ready && !ready_after_render {
        pending.content_ready = false;
        emit(pending, "content-invalidated-before-paint");
    }
    if !pending.content_ready && ready_after_render && pending.expected.is_subset(painted) {
        pending.content_ready = true;
        emit(pending, "content-ready-after-render");
    }
    if pending.content_ready && ready_after_render && pending.expected.is_subset(painted) {
        emit(pending, "content-paint-observed");
        timings.pending.remove(&host);
    } else if !pending.content_ready && !pending.unprepared_painted && pending.expected.is_subset(painted) {
        pending.unprepared_painted = true;
        emit(pending, "unprepared-paint-observed");
    }
}
