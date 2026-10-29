//! Read-only startup work that can run while the native window is constructed.
//! The connected service is never installed in a pane; Desktop binds its
//! pending facade only after the GPUI owner exists.
use crate::{backend_launcher, chat::connection_recovery, startup_workspace};
use async_channel::{Receiver, Sender};
use monitter_gpui_desktop::{native_state::{ApplyResult, NativeState}, workspace_store};
use monitter_lib::experimental_desktop::{DesktopService, NativePreferences, NativeWorkspaceSet};

pub(super) type LoadedPreferences = (NativePreferences, i64, Option<NativeWorkspaceSet>, bool);
pub(super) enum Event {
    Connected(Result<(DesktopService, Result<Receiver<()>, String>), String>),
    Preferences(Result<LoadedPreferences, String>),
    NativeState(Result<(NativeState, ApplyResult), String>),
}

fn send(sender: &Sender<Event>, event: Event) -> bool {
    sender.send_blocking(event).is_ok()
}

/// The only initial connection attempt. Three bounded receipts preserve the
/// current independent preferences/state progress when either read is slow.
pub(super) fn begin() -> Receiver<Event> {
    let (sender, receiver) = async_channel::bounded(3);
    let worker_sender = sender.clone();
    let started = std::thread::Builder::new()
        .name("monitter-early-bootstrap".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let connected = match backend_launcher::connect() {
                Ok(connected) => connected,
                Err(error) => {
                    send(&worker_sender, Event::Connected(Err(error)));
                    return;
                }
            };
            startup_workspace::mark("backend-worker-ready");
            let subscription = connected.subscribe_changes();
            if !send(&worker_sender, Event::Connected(Ok((connected.clone(), subscription)))) {
                return;
            }

            let preferences_sender = worker_sender.clone();
            let preferences_service = connected.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("monitter-early-preferences".into())
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let result = (|| {
                        let mut preferences = preferences_service.load_native_preferences()?;
                        let (revision, workspaces, theme, auto_size_panes) = workspace_store::load()?;
                        if let Some(theme) = theme { preferences.data.appearance.tile_theme = theme; }
                        Ok::<_, String>((preferences, revision, workspaces, auto_size_panes))
                    })();
                    startup_workspace::mark("preferences-worker-ready");
                    send(&preferences_sender, Event::Preferences(result));
                })
            {
                send(&worker_sender, Event::Preferences(Err(error.to_string())));
            }

            let native_sender = worker_sender.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("monitter-early-native-state".into())
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let mut result = connected.read_native_ui_changes(None, &[]);
                    for wait_ms in [150, 450] {
                        if !result.as_ref().err().is_some_and(|error| connection_recovery::transient(error)) { break; }
                        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                        result = connected.read_native_ui_changes(None, &[]);
                    }
                    let result = result.map(|response| {
                        let mut state = NativeState::default();
                        let applied = state.apply(response);
                        (state, applied)
                    });
                    startup_workspace::mark("native-state-worker-ready");
                    send(&native_sender, Event::NativeState(result));
                })
            {
                send(&worker_sender, Event::NativeState(Err(error.to_string())));
            }
        });
    if let Err(error) = started {
        send(&sender, Event::Connected(Err(error.to_string())));
    }
    receiver
}
