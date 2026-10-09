//! Native macOS menu-bar presence for the separate backend server process.
//!
//! The menu lives on AppKit's main thread. Menu actions cross to the server
//! owner through a standard channel; they never start or stop a server inline.

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSObject, NSString};
    use std::sync::{
        Mutex, OnceLock,
        mpsc::{self, Receiver, Sender},
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ServerMenuEvent {
        OpenMonitter,
        QuitServer,
    }

    static ACTION_SENDER: OnceLock<Mutex<Option<(u64, Sender<ServerMenuEvent>)>>> = OnceLock::new();
    static NEXT_MENU_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn safe_label(label: &str) -> String {
        let mut bounded = label
            .chars()
            .filter(|character| !character.is_control())
            .take(120)
            .collect::<String>();
        if label
            .chars()
            .filter(|character| !character.is_control())
            .count()
            > 120
        {
            bounded.push('…');
        }
        bounded
    }

    fn emit(event: ServerMenuEvent) {
        if let Some(sender) = ACTION_SENDER
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|sender| sender.as_ref().map(|(_, sender)| sender.clone()))
        {
            let _ = sender.send(event);
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = ()]
        struct ServerMenuActionTarget;

        impl ServerMenuActionTarget {
            #[unsafe(method_id(init))]
            fn init(this: Allocated<Self>) -> Retained<Self> {
                let this = this.set_ivars(());
                unsafe { msg_send![super(this), init] }
            }

            #[unsafe(method(openMonitter:))]
            fn open_monitter(&self, _sender: &AnyObject) {
                emit(ServerMenuEvent::OpenMonitter);
            }

            #[unsafe(method(quitServer:))]
            fn quit_server(&self, _sender: &AnyObject) {
                emit(ServerMenuEvent::QuitServer);
            }
        }
    );

    impl ServerMenuActionTarget {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // NSObject's initializer is the designated initializer for this
            // stateless action target subclass.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    /// Owns the AppKit status item. Construct and drop this value on the main
    /// thread, and retain it for as long as the backend server should be shown.
    pub struct ServerMenu {
        status_bar: Retained<NSStatusBar>,
        status_item: Retained<NSStatusItem>,
        menu: Retained<NSMenu>,
        status_line: Option<Retained<NSMenuItem>>,
        _target: Retained<ServerMenuActionTarget>,
        menu_id: u64,
    }

    impl ServerMenu {
        /// Install a native status item and return the receiver used by the
        /// server's existing event loop. No window or webview is created.
        pub fn install(
            mtm: MainThreadMarker,
            status_label: Option<&str>,
        ) -> (Self, Receiver<ServerMenuEvent>) {
            let (sender, receiver) = mpsc::channel();
            let menu_id = NEXT_MENU_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let action_sender = ACTION_SENDER.get_or_init(|| Mutex::new(None));
            if let Ok(mut current) = action_sender.lock() {
                *current = Some((menu_id, sender));
            }

            let status_bar = NSStatusBar::systemStatusBar();
            let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
            let menu_title = NSString::from_str("Monitter Backend");
            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &menu_title);
            let target = ServerMenuActionTarget::new(mtm);
            let target_object: &AnyObject = target.as_super().as_super();

            let open_title = NSString::from_str("Open Monitter");
            let empty_key = NSString::from_str("");
            // SAFETY: selectors name methods implemented by the retained target
            // object below. AppKit invokes them on its main thread.
            let open_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &open_title,
                    Some(sel!(openMonitter:)),
                    &empty_key,
                )
            };
            // SAFETY: the target is retained in ServerMenu for at least as long
            // as the menu item can be invoked; the selector is implemented.
            unsafe {
                open_item.setTarget(Some(target_object));
                open_item.setAction(Some(sel!(openMonitter:)));
            }
            menu.addItem(&open_item);

            let status_line = status_label.map(|label| {
                let title = NSString::from_str(&safe_label(label));
                // SAFETY: a missing action makes this an informational row.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &title,
                        None,
                        &empty_key,
                    )
                };
                item.setEnabled(false);
                menu.addItem(&NSMenuItem::separatorItem(mtm));
                menu.addItem(&item);
                item
            });

            menu.addItem(&NSMenuItem::separatorItem(mtm));
            let quit_title = NSString::from_str("Quit Server");
            // SAFETY: selector is implemented by the retained action target.
            let quit_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &quit_title,
                    Some(sel!(quitServer:)),
                    &empty_key,
                )
            };
            // SAFETY: target and selector remain valid for the menu lifetime.
            unsafe {
                quit_item.setTarget(Some(target_object));
                quit_item.setAction(Some(sel!(quitServer:)));
            }
            menu.addItem(&quit_item);

            status_item.setMenu(Some(&menu));
            if let Some(button) = status_item.button(mtm) {
                button.setTitle(&NSString::from_str("Monitter"));
                button.setToolTip(Some(&NSString::from_str("Monitter backend server")));
            }
            status_item.setVisible(true);

            (
                Self {
                    status_bar,
                    status_item,
                    menu,
                    status_line,
                    _target: target,
                    menu_id,
                },
                receiver,
            )
        }

        /// Update the informational menu row with state supplied by the server
        /// owner. `None` removes it; this module never infers run activity.
        pub fn set_status_label(&mut self, mtm: MainThreadMarker, label: Option<&str>) {
            match (self.status_line.take(), label) {
                (Some(item), Some(label)) => {
                    item.setTitle(&NSString::from_str(&safe_label(label)));
                    self.status_line = Some(item);
                }
                (Some(item), None) => {
                    self.menu.removeItem(&item);
                    // The informational row is always preceded by exactly
                    // one separator at index 1 (Open Monitter is index 0).
                    self.menu.removeItemAtIndex(1);
                }
                (None, Some(label)) => {
                    let title = NSString::from_str(&safe_label(label));
                    // SAFETY: an informational row has no action target.
                    let item = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &title,
                            None,
                            &NSString::from_str(""),
                        )
                    };
                    item.setEnabled(false);
                    self.menu
                        .insertItem_atIndex(&NSMenuItem::separatorItem(mtm), 1);
                    self.menu.insertItem_atIndex(&item, 2);
                    self.status_line = Some(item);
                }
                (None, None) => {}
            }
        }
    }

    impl Drop for ServerMenu {
        fn drop(&mut self) {
            self.status_bar.removeStatusItem(&self.status_item);
            if let Some(state) = ACTION_SENDER.get()
                && let Ok(mut current) = state.lock()
                && current
                    .as_ref()
                    .is_some_and(|(menu_id, _)| *menu_id == self.menu_id)
            {
                *current = None;
            }
        }
    }

    pub use ServerMenu as Menu;
    pub use ServerMenuEvent as Event;
}

#[cfg(target_os = "macos")]
pub use macos::{Event as ServerMenuEvent, Menu as ServerMenu};

#[cfg(not(target_os = "macos"))]
compile_error!("The native backend server menu is only available on macOS.");
