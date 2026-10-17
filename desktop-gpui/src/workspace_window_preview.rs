//! Profiling-only native Desktop preview. All drafts and window ownership are
//! retained in memory; the optional presentation profile is restricted to /tmp.
use super::*;
use monitter_lib::experimental_desktop::{NativeWorkspace, NativeWorkspaceSet};
use std::{path::PathBuf, sync::OnceLock};

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("MONITTER_GPUI_WORKSPACE_WINDOW_PREVIEW").is_ok_and(|value| value == "1")
    })
}

fn isolated_profile() -> Result<PathBuf, String> {
    let profile = std::env::var_os("MONITTER_GPUI_PROFILE_DIR").map(PathBuf::from)
        .ok_or("Workspace preview requires MONITTER_GPUI_PROFILE_DIR=/tmp/monitter-detached-preview-<id>/profile.")?;
    let directory = profile
        .parent()
        .ok_or("Preview profile has no parent directory.")?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let temporary = std::fs::canonicalize("/tmp").map_err(|error| error.to_string())?;
    if !profile.is_absolute()
        || profile.file_name().and_then(|name| name.to_str()) != Some("profile")
        || !name.starts_with("monitter-detached-preview-")
        || directory
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .as_ref()
            != Some(&temporary)
    {
        return Err(
            "Workspace preview profile must be /tmp/monitter-detached-preview-<id>/profile.".into(),
        );
    }
    for path in [directory, profile.as_path()] {
        if std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
        {
            return Err(
                "Workspace preview profile must use ordinary temporary directories.".into(),
            );
        }
    }
    std::fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for path in [directory, profile.as_path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(profile)
}

impl Desktop {
    fn seed_workspace_window_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences = Some(NativePreferences::default());
        self.preferences_busy = false;
        self.preferences_blocked = false;
        self.preferences_dirty = true;
        self.startup_pending = false;
        self.startup_first_frame = false;
        self.startup_live_frame_pending = false;
        self.workspace_window_restore_pending = false;
        self.browser_requests_pending = false;
        self.snapshot = Rc::new(json!({
            "settings":{"theme":"dark","windowSurface":"opaque","interfaceFontSize":14},
            "agents":[{"id":"offline-preview-agent","name":"Offline preview","provider":"codex","sandbox":"read-only"}],
            "tasks":[],"projects":[],"hosts":[],"messages":[],"channels":[],"approvalRequests":[]
        }));
        self.revision = Some("offline-workspace-preview:1".into());
        let service = DesktopService::disconnected_for_ui_fixture();
        // Production draft creation supplies the actual input entities,
        // subscriptions, lifecycle hooks and focus route. The facade cannot
        // connect; the controller loses even this facade after seeding.
        self.service = Some(service);
        let mut workspaces = Vec::new();
        let mut first = None;
        for (id, label) in [
            ("offline-preview-a", "Preview A"),
            ("offline-preview-b", "Preview B"),
        ] {
            self.window.active_workspace_id = Some(id.into());
            self.window.preferences_restore = false;
            self.window.preferences_restoring = false;
            self.window.preferences_restore_suppressed = false;
            self.open_chat_draft(None, window, cx);
            let pane = self
                .window
                .draft_creation
                .clone()
                .expect("offline preview draft exists");
            let mut form = pane
                .read(cx)
                .export_state(cx)
                .expect("bounded preview draft");
            form.title = format!("{label} — disconnected workspace preview");
            form.message = format!(
                "Unsent draft in {label}.\n\nOffline preview: no backend or agent connection. Drafts stay in memory.\n\nType here, then drag the workspace tab outside this window or use its Move to New Window menu. Return from the tab menu or close the secondary window."
            );
            pane.update(cx, |pane, cx| pane.restore_state(form, window, cx))
                .expect("restore offline preview draft");
            self.window.draft_creation_open = false;
            workspaces.push(NativeWorkspace {
                id: id.into(),
                name: label.into(),
                icon: "folder".into(),
                color: "#38BFD2".into(),
                url: None,
                layout: Some(
                    self.window
                        .dock
                        .read(cx)
                        .serialize(cx)
                        .expect("preview dock serializes"),
                ),
            });
            if first.is_none() {
                let next = Self::create_workspace_view(
                    self.chat_panes.clone(),
                    self.draft_panes.clone(),
                    Sidebar::Chats,
                    window,
                    cx,
                );
                let active = std::mem::replace(&mut self.window, next);
                active
                    .dock
                    .update(cx, |dock, cx| dock.set_shown(false, window, cx));
                first = Some(active);
            }
        }
        let second =
            std::mem::replace(&mut self.window, first.expect("preview A remains retained"));
        second
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        self.workspace_view_cache
            .insert("offline-preview-b".into(), second);
        // A third real dock hosts the retained private WKWebView. Construct the
        // manager directly from its static fixture so no production datastore
        // or browser restoration path can be opened during this preview.
        let browser_view = Self::create_workspace_view(
            self.chat_panes.clone(),
            self.draft_panes.clone(),
            Sidebar::Browser,
            window,
            cx,
        );
        let primary = std::mem::replace(&mut self.window, browser_view);
        self.window.active_workspace_id = Some("offline-preview-browser".into());
        self.window.preferences_restore = false;
        self.window.preferences_restoring = false;
        self.window.preferences_restore_suppressed = false;
        let manager = cx.new(|cx| {
            browser::BrowserPane::workspace_window_fixture(window, cx)
                .expect("private static workspace browser")
        });
        self.native_browser_metadata = Some(manager.read(cx).metadata_snapshot(cx));
        self.browser_restore_revision = None;
        self.browser = Some(manager);
        self.open_browser_tab(1, window, cx)
            .expect("open retained static browser dock");
        workspaces.push(NativeWorkspace {
            id: "offline-preview-browser".into(),
            name: "Browser".into(),
            icon: "globe".into(),
            color: "#74D8EA".into(),
            url: None,
            layout: Some(self.window.dock.read(cx).serialize(cx).expect("browser preview dock serializes")),
        });
        let browser_view = std::mem::replace(&mut self.window, primary);
        browser_view.dock.update(cx, |dock, cx| dock.set_shown(false, window, cx));
        self.workspace_view_cache.insert("offline-preview-browser".into(), browser_view);
        self.named_workspaces = Some(NativeWorkspaceSet {
            active_id: "offline-preview-a".into(),
            items: workspaces,
        });
        self.workspace_window_assignments = workspace_window_state::WorkspaceWindowState::new(&[
            "offline-preview-a".into(),
            "offline-preview-b".into(),
            "offline-preview-browser".into(),
        ])
        .expect("bounded preview ownership");
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(true, window, cx));
        self.service = None;
    }
}

pub(super) fn open_if_requested(cx: &mut App) -> bool {
    if !enabled() {
        return false;
    }
    let profile = match isolated_profile() {
        Ok(profile) => profile,
        Err(error) => {
            eprintln!("[workspace-preview] {error}");
            cx.quit();
            return true;
        }
    };
    crate::surface_blend::initialize_for_ui_fixture();
    crate::native_motion::initialize_for_ui_fixture(cx);
    cx.set_global(streaming_display::Selection {
        options: Default::default(),
        error: None,
    });
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    native_menu::install(cx);
    if let Err(error) = monitter_gpui_desktop::fonts::register(cx) {
        eprintln!("{error}");
    }
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    let (handle, desktop) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(gpui_kit::px(1200.), gpui_kit::px(820.)),
                cx,
            ))),
            app_owns_titlebar_drag: true,
            titlebar: Some(TitlebarOptions {
                title: Some("Monitter — disconnected workspace preview".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(gpui_kit::px(12.), gpui_kit::px(22.))),
            }),
            ..Default::default()
        },
        cx,
        |window, cx| {
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                true
            });
            cx.new(|cx| Desktop::new(window, cx))
        },
    )
    .expect("open disconnected native workspace preview");
    // open_window installs its component Root after the constructor returns.
    // Draft creation and browser input require that per-window state, and an
    // erased update avoids holding the root lease while those inputs use it.
    let native: AnyWindowHandle = handle.into();
    native.update(cx, |_, window, cx| {
        desktop.update(cx, |desktop, cx| desktop.seed_workspace_window_preview(window, cx));
    }).expect("seed disconnected workspace preview after component root");
    let _ = handle.update(cx, |_, window, cx| dialog_backdrop::install(window, cx));
    native_navigation::install(
        handle,
        native_navigation::NativeNavigation::new(desktop.downgrade()),
        cx,
    );
    shortcut_desktop::install(
        handle,
        shortcut_desktop::DesktopShortcuts::new(desktop.downgrade()),
        cx,
    );
    eprintln!(
        "[workspace-preview] ready profile={} backend=none preferences=in-memory window-state=in-memory",
        profile.display()
    );
    cx.activate(true);
    true
}
