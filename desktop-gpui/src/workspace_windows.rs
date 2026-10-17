//! Native hosts for retained workspace views. The Desktop controller remains
//! the single backend subscriber and preference writer.
use crate::workspace_window_state::{WindowId as WorkspaceWindowId, WorkspaceWindowState};
use crate::*;
use std::ops::{Deref, DerefMut};

pub(super) struct DesktopWindowView {
    pub(super) active_workspace_id: Option<String>,
    pub(super) removed_pane_ids: Vec<String>,
    pub(super) projection_revision: Option<String>,
    pub(super) pending_detach_workspace: Option<String>,
    pub(super) pending_pane_focus: Option<(String, String)>,
    pending_workspace_transfer: Option<(String, WorkspaceWindowId, Option<String>)>,
    pub(super) dock: Entity<docking::DockWorkspace>,
    pub(super) palette: Option<Entity<command_palette::CommandPalette>>,
    pub(super) palette_subscription: Option<Subscription>,
    pub(super) palette_terminal_sessions: Vec<Value>,
    pub(super) palette_terminals_loading: bool,
    pub(super) _layout_subscription: Subscription,
    pub(super) _activation_subscription: Option<Subscription>,
    pub(super) sidebar_resize: Entity<ResizableState>,
    pub(super) _sidebar_resize_subscription: Subscription,
    pub(super) sidebar_width_touched: bool,
    pub(super) sidebar_collapsed: bool,
    pub(super) sidebar_hidden: bool,
    pub(super) sidebar_expanded_width: f32,
    pub(super) sidebar_motion: layout_motion::WidthMotion,
    pub(super) sidebar_motion_frame_pending: bool,
    pub(super) sidebar_motion_compact: bool,
    pub(super) sidebar_focus: FocusHandle,
    pub(super) sidebar_toggle_focus: FocusHandle,
    pub(super) preferences_open: bool,
    pub(super) preferences_focus: FocusHandle,
    pub(super) preferences_previous_focus: Option<FocusHandle>,
    pub(super) sidebar_scale: u16,
    pub(super) sidebar_view: SidebarView,
    pub(super) sidebar_tabs_motion: RefCell<crate::sidebar_tabs_motion::SidebarTabsMotion>,
    pub(super) workspace_switch: Option<String>,
    pub(super) pending_sidebar_task: Option<String>,
    pub(super) pending_notification_read: Option<String>,
    pub(super) sidebar_task_navigation_scheduled: bool,
    pub(super) pending_workspace_removal: Option<String>,
    pub(super) workspace_editor: Option<workspace_tabs::WorkspaceEditor>,
    pub(super) workspace_number_hints: bool,
    pub(super) route_touched: bool,
    pub(super) preferences_restore: bool,
    pub(super) preferences_restore_suppressed: bool,
    pub(super) preferences_restoring: bool,
    pub(super) preferences_restore_generation: u64,
    pub(super) sidebar_resize_suppressed: bool,
    pub(super) sidebar_resize_generation: u64,
    pub(super) dock_current: Option<String>,
    pub(super) wallpaper_backdrop: Entity<wallpaper_backdrop::WallpaperBackdrop>,
    pub(super) lock_previous_focus: Option<FocusHandle>,
    pub(super) selected: Option<String>,
    pub(super) sidebar: Sidebar,
    pub(super) filter: Option<String>,
    pub(super) error: Option<String>,
    pub(super) busy: bool,
    pub(super) empty_terminal_pending: bool,
    pub(super) pending_empty_session: Option<(String, String)>,
    pub(super) pending_draft_chats: BTreeMap<String, String>,
    pub(super) native_settings: Option<settings::NativeSettings>,
    pub(super) draft_creation: Option<Entity<draft_creation::DraftCreationPane>>,
    pub(super) draft_creation_open: bool,
    pub(super) applied_style: Option<String>,
    pub(super) applied_window_surface: Option<String>,
    pub(super) workspace: Option<Entity<WorkspacePane>>,
    pub(super) archives: Option<Entity<archives::ArchivedChatsPane>>,
    pub(super) archives_subscription: Option<Subscription>,
    pub(super) archives_revision: Option<String>,
    pub(super) mail: Option<Entity<MailPane>>,
    pub(super) mail_context_task: Option<String>,
    pub(super) collapsed_agents: HashSet<String>,
    pub(super) channels: Option<Entity<ChannelPane>>,
    pub(super) channels_revision: Option<String>,
    pub(super) channel_subscription: Option<Subscription>,
    pub(super) channel_cursors: HashMap<String, Option<String>>,
    pub(super) terminal: Option<Entity<terminal::TerminalPane>>,
    pub(super) terminal_context: Option<(Option<String>, Option<String>)>,
    pub(super) terminal_panes: HashMap<String, Entity<terminal::TerminalPane>>,
    pub(super) neovim_sessions: HashMap<String, Entity<neovim::NeovimSession>>,
    pub(super) neovim_panes: HashMap<String, Entity<neovim::NeovimPane>>,
    pub(super) neovim_layouts: HashMap<String, Value>,
    pub(super) neovim_origins: HashMap<String, String>,
    pub(super) neovim_focus_windows: HashMap<String, i64>,
    pub(super) neovim_pending: bool,
    pub(super) usage: Option<Entity<usage::UsagePane>>,
    pub(super) usage_revision: Option<String>,
    pub(super) app_resources: Option<Entity<usage::AppResourcePane>>,
    pub(super) sidebar_usage: Option<Entity<sidebar_usage::SidebarUsageWidget>>,
    pub(super) workspace_usage_rings: Option<Entity<sidebar_usage::WorkspaceUsageRings>>,
    pub(super) notification_bell: Option<Entity<notifications::NotificationBell>>,
    pub(super) notification_subscription: Option<Subscription>,
    pub(super) admin_activity: Option<Entity<admin_activity::AdminActivity>>,
    pub(super) admin_activity_subscription: Option<Subscription>,
    pub(super) sidebar_usage_snapshot: Option<Rc<Value>>,
    pub(super) sidebar_usage_revision: Option<String>,
    pub(super) schedules: Option<Entity<schedules::SchedulePane>>,
    pub(super) schedules_revision: Option<String>,
    pub(super) agent_editor: Option<Entity<agents::AgentHostPane>>,
    pub(super) agent_editor_revision: Option<String>,
    pub(super) subagents: Option<Entity<collaboration::SubagentPane>>,
    pub(super) subagents_context: Option<(Option<String>, Option<String>)>,
    pub(super) git: Option<Entity<git::GitPane>>,
    pub(super) git_panes: HashMap<String, Entity<git::GitPane>>,
    pub(super) git_panes_revision: Option<String>,
    pub(super) git_context: Option<(Option<String>, Option<String>)>,
}
impl Deref for Desktop {
    type Target = DesktopWindowView;
    fn deref(&self) -> &Self::Target {
        &self.window
    }
}
impl DerefMut for Desktop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.window
    }
}

// The controller outlives its original native window. Hosts retain only weak
// references, so the application owns this one strong reference without a cycle.
pub(super) struct DesktopControllerOwner {
    pub(super) _desktop: Entity<Desktop>,
}
impl Global for DesktopControllerOwner {}

pub(super) struct WorkspaceHost {
    pub(super) handle: AnyWindowHandle,
    pub(super) root: Option<Entity<WorkspaceWindowRoot>>,
}
pub(super) struct WorkspaceWindowRoot {
    desktop: WeakEntity<Desktop>,
    ready: Rc<std::cell::Cell<bool>>,
    _desktop_observer: Subscription,
    _host_observers: Vec<Subscription>,
}

/// A workspace-scoped async continuation follows the workspace when its native
/// host changes. The service operation itself remains independent of windows.
pub(super) struct WorkspaceWeakDesktop {
    desktop: WeakEntity<Desktop>,
    workspace_id: Option<String>,
    fallback_window: WorkspaceWindowId,
}
impl WorkspaceWeakDesktop {
    pub(super) fn update_in<R>(
        &self,
        cx: &mut AsyncWindowContext,
        update: impl FnOnce(&mut Desktop, &mut Window, &mut Context<Desktop>) -> R,
    ) -> anyhow::Result<R> {
        let handle = self
            .desktop
            .read_with(cx, |desktop, _| {
                let id = self
                    .workspace_id
                    .as_deref()
                    .and_then(|id| {
                        desktop
                            .workspace_window_assignments
                            .window_for_workspace(id)
                    })
                    .unwrap_or(&self.fallback_window);
                desktop.window_hosts.get(id).map(|host| host.handle)
            })?
            .ok_or_else(|| anyhow::anyhow!("Workspace window is unavailable"))?;
        handle.update(cx, |_, window, cx| {
            self.desktop.update(cx, |desktop, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    desktop.with_retained_workspace(
                        self.workspace_id.as_deref(),
                        window,
                        cx,
                        update,
                    )
                })
            })?
        })?
    }
}
impl Render for WorkspaceWindowRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Opening a native window draws it synchronously while its caller may
        // still lease Desktop. Ownership and the retained view are committed
        // before the deferred readiness notification mounts its content.
        if !self.ready.get() {
            return div().size_full().into_any_element();
        }
        self.desktop
            .update(cx, |desktop, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    desktop.render(window, cx).into_any_element()
                })
            })
            .unwrap_or_else(|_| {
                div()
                    .size_full()
                    .child("Workspace unavailable")
                    .into_any_element()
            })
    }
}

pub(super) trait DesktopContextExt {
    fn workspace_listener<E: ?Sized + 'static>(
        &self,
        listener: impl Fn(&mut Desktop, &E, &mut Window, &mut Context<Desktop>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static;
    fn subscribe_workspace<Emitter, Event>(
        &mut self,
        emitter: &Entity<Emitter>,
        window: &Window,
        on_event: impl FnMut(&mut Desktop, &Entity<Emitter>, &Event, &mut Window, &mut Context<Desktop>)
        + 'static,
    ) -> Subscription
    where
        Emitter: EventEmitter<Event>,
        Event: 'static;
}
impl DesktopContextExt for Context<'_, Desktop> {
    fn workspace_listener<E: ?Sized + 'static>(
        &self,
        listener: impl Fn(&mut Desktop, &E, &mut Window, &mut Context<Desktop>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        self.listener(move |desktop, event, window, cx| {
            desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                listener(desktop, event, window, cx)
            });
        })
    }
    fn subscribe_workspace<Emitter, Event>(
        &mut self,
        emitter: &Entity<Emitter>,
        window: &Window,
        mut on_event: impl FnMut(
            &mut Desktop,
            &Entity<Emitter>,
            &Event,
            &mut Window,
            &mut Context<Desktop>,
        ) + 'static,
    ) -> Subscription
    where
        Emitter: EventEmitter<Event>,
        Event: 'static,
    {
        self.subscribe_in_emitter_window(
            emitter,
            window,
            move |desktop, emitter, event, window, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    let owner = desktop.workspace_for_emitter(emitter.entity_id(), cx);
                    let _ = desktop.with_retained_workspace(
                        owner.as_deref(),
                        window,
                        cx,
                        |desktop, window, cx| on_event(desktop, emitter, event, window, cx),
                    );
                });
            },
        )
    }
}
impl Desktop {
    fn workspace_for_emitter(&self, entity: EntityId, cx: &App) -> Option<String> {
        std::iter::once(&self.window)
            .chain(self.parked_window_views.values())
            .chain(self.workspace_view_cache.values())
            .find(|view| {
                view.dock.entity_id() == entity
                    || view.dock.read(cx).retained_entity_ids(cx).contains(&entity)
                    || view.sidebar_resize.entity_id() == entity
                    || view
                        .workspace
                        .as_ref()
                        .is_some_and(|pane| pane.entity_id() == entity)
                    || view
                        .channels
                        .as_ref()
                        .is_some_and(|pane| pane.entity_id() == entity)
                    || view
                        .terminal
                        .as_ref()
                        .is_some_and(|pane| pane.entity_id() == entity)
            })
            .and_then(|view| view.active_workspace_id.clone())
    }
    pub(super) fn capture_window_layouts(&mut self, cx: &App) -> Result<(), String> {
        let mut layouts = Vec::new();
        let mut capture = |view: &DesktopWindowView| -> Result<(), String> {
            if !view.preferences_restore && !view.preferences_restoring {
                if let Some(id) = &view.active_workspace_id {
                    let mut layout = view.dock.read(cx).serialize(cx)?;
                    crate::workspace_visibility::remember_focus(
                        &mut layout,
                        view.dock_current.as_deref(),
                    );
                    layouts.push((id.clone(), layout));
                }
            }
            Ok(())
        };
        capture(&self.window)?;
        for view in self
            .parked_window_views
            .values()
            .chain(self.workspace_view_cache.values())
        {
            capture(view)?;
        }
        if let Some(set) = &mut self.named_workspaces {
            for (id, mut layout) in layouts {
                if let Some(item) = set.items.iter_mut().find(|item| item.id == id) {
                    crate::workspace_visibility::preserve_focus_metadata(
                        &mut layout,
                        item.layout.as_ref(),
                    );
                    if item.layout.as_ref() != Some(&layout) {
                        item.layout = Some(layout);
                        self.workspaces_dirty = true;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn reconcile_workspace_window_assignments(&mut self) -> Result<(), String> {
        let Some(set) = &self.named_workspaces else {
            return Ok(());
        };
        self.workspace_window_assignments.reconcile(
            &set.items
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>(),
        )
    }
    pub(super) fn persist_workspace_window_assignments(
        &mut self,
        flush: bool,
    ) -> Result<(), String> {
        self.reconcile_workspace_window_assignments()?;
        window_state::save_workspace_windows(&self.workspace_window_assignments, flush)
    }
    fn active_layout_ids(view: &DesktopWindowView, cx: &App) -> Result<HashSet<String>, String> {
        docking::DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx)?)
            .map(|ids| ids.into_iter().collect())
    }
    fn preflight_workspace_view(
        &self,
        view: &DesktopWindowView,
        target: &WorkspaceWindowId,
        cx: &App,
    ) -> Result<(), String> {
        let ids = Self::active_layout_ids(view, cx)?;
        for (id, other) in &self.parked_window_views {
            if id == target {
                continue;
            }
            let other_ids = Self::active_layout_ids(other, cx)?;
            if ids.intersection(&other_ids).any(|id| {
                id.starts_with("chat:") || id.starts_with("browser:") || id.starts_with("draft:")
            }) {
                return Err("This workspace shares a live pane with another window. Close that duplicate pane before moving the workspace.".into());
            }
        }
        Ok(())
    }
    fn rehost_workspace_view(
        view: &mut DesktopWindowView,
        browser: Option<&Entity<BrowserPane>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String> {
        let ids = view.dock.read(cx).retained_browser_ids();
        if let Some(browser) = browser {
            browser.read(cx).preflight_tab_transfer(&ids, window, cx)?;
        } else if !ids.is_empty() {
            return Err("The browser manager is unavailable; the workspace was not moved.".into());
        }
        if let Some(browser) = browser {
            browser.update(cx, |browser, cx| {
                browser.transfer_tabs_to_window(&ids, window, cx)
            })?;
        }
        view.dock
            .update(cx, |dock, cx| dock.rehost_to_window(window, cx))?;
        for (_, content) in view.dock.read(cx).retained_content_views(cx) {
            if let Ok(pane) = content.clone().downcast::<chat::ChatPane>() {
                pane.update(cx, |pane, cx| pane.rehost_to_window(window, cx))?;
            } else if let Ok(pane) = content
                .clone()
                .downcast::<draft_creation::DraftCreationPane>()
            {
                pane.update(cx, |pane, cx| pane.rehost_to_window(window, cx))?;
            } else if let Ok(pane) = content.clone().downcast::<neovim::NeovimPane>() {
                pane.update(cx, |pane, cx| pane.rehost_to_window(window, cx))?;
            } else if let Ok(pane) = content.downcast::<terminal::TerminalPane>() {
                pane.update(cx, |pane, cx| pane.rehost_to_window(window, cx))?;
            }
        }
        cx.rehost_entity_window(view.sidebar_resize.entity_id(), window)
            .map_err(|error| error.to_string())?;
        cx.rehost_entity_window(view.wallpaper_backdrop.entity_id(), window)
            .map_err(|error| error.to_string())?;
        macro_rules! pin_optional {($($field:ident),* $(,)?)=>{$(if let Some(entity)=&view.$field {
            cx.rehost_entity_window(entity.entity_id(),window).map_err(|error|error.to_string())?;
        })*}}
        pin_optional!(
            palette,
            draft_creation,
            workspace,
            archives,
            mail,
            channels,
            terminal,
            usage,
            app_resources,
            sidebar_usage,
            workspace_usage_rings,
            notification_bell,
            admin_activity,
            schedules,
            agent_editor,
            subagents,
            git
        );
        for entity in view.neovim_sessions.values() {
            cx.rehost_entity_window(entity.entity_id(), window)
                .map_err(|error| error.to_string())?;
        }
        if let Some(settings) = &mut view.native_settings {
            settings.rehost_to_window(window, cx)?;
        }
        view._activation_subscription.take();
        view.applied_style = None;
        view.applied_window_surface = None;
        view.sidebar_motion.cancel();
        view.sidebar_motion_frame_pending = false;
        view.dock.update(cx, |dock, cx| {
            dock.set_shown(!session_lock::locked(cx), window, cx)
        });
        Ok(())
    }
    fn create_workspace_host(
        &mut self,
        id: &WorkspaceWindowId,
        placement_id: &WorkspaceWindowId,
        cx: &mut Context<Self>,
    ) -> Result<AnyWindowHandle, String> {
        if let Some(host) = self.window_hosts.get(id) {
            return Ok(host.handle);
        }
        let weak = cx.weak_entity();
        let host_id = id.clone();
        let placement_id = placement_id.clone();
        let ready = Rc::new(std::cell::Cell::new(false));
        let initial_ready = ready.clone();
        let (handle, root) = gpui_kit::open_window(
            WindowOptions {
                display_id: window_state::display_id_for(&placement_id, cx),
                window_bounds: Some(window_state::initial_for(&placement_id, cx).unwrap_or_else(
                    || {
                        WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(gpui_kit::px(1000.), gpui_kit::px(760.)),
                            cx,
                        ))
                    },
                )),
                focus: false,
                app_owns_titlebar_drag: true,
                titlebar: Some(TitlebarOptions {
                    title: Some("Monitter".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(gpui_kit::px(12.), gpui_kit::px(22.))),
                }),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                window_state::restore_native_for(&placement_id, window, cx);
                let close_weak = weak.clone();
                let close_id = host_id.clone();
                let close_ready = initial_ready.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    if !close_ready.get() {
                        return false;
                    }
                    close_weak
                        .update(cx, |desktop, cx| {
                            desktop.request_workspace_window_close(&close_id, window, cx)
                        })
                        .unwrap_or(true)
                });
                cx.new(|cx| {
                    let desktop = weak
                        .upgrade()
                        .expect("Desktop remains retained while opening a workspace host");
                    let observer = cx.observe(&desktop, |_, _, cx| cx.notify());
                    let bounds_id = host_id.clone();
                    let bounds_weak = weak.clone();
                    let bounds_ready = initial_ready.clone();
                    let bounds = cx.observe_window_bounds(window, move |_, window, cx| {
                        if !bounds_ready.get() {
                            return;
                        }
                        window_state::save_for(&bounds_id, window, cx);
                        let _ = bounds_weak.update(cx, |desktop, cx| {
                            desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                                desktop.refresh_wallpaper(window, cx)
                            })
                        });
                    });
                    let activation_weak = weak.clone();
                    let activation_ready = initial_ready.clone();
                    let activation = cx.observe_window_activation(window, move |_, window, cx| {
                        if !activation_ready.get() {
                            return;
                        }
                        let _ = activation_weak.update(cx, |desktop, cx| {
                            desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                                if window.is_window_active() {
                                    desktop.refresh_wallpaper(window, cx);
                                } else {
                                    desktop.set_workspace_number_hints(false, cx);
                                    if !cx.has_platform_owned_drag() {
                                        cx.stop_active_drag(window);
                                    }
                                }
                            })
                        });
                    });
                    WorkspaceWindowRoot {
                        desktop: weak,
                        ready: initial_ready,
                        _desktop_observer: observer,
                        _host_observers: vec![bounds, activation],
                    }
                })
            },
        )
        .map_err(|error| format!("Cannot open workspace window: {error}"))?;
        // Kit installs its component window state after the content constructor
        // returns. Dialog and popup configuration must use that initialized
        // state, just as it does in the primary window.
        let native: AnyWindowHandle = handle.into();
        native
            .update(cx, |_, window, cx| dialog_backdrop::install(window, cx))
            .map_err(|error| format!("Cannot configure workspace window: {error}"))?;
        let pending_root = root.downgrade();
        self.window_hosts.insert(
            id.clone(),
            WorkspaceHost {
                handle: native,
                root: Some(root),
            },
        );
        cx.defer(move |cx| {
            let _ = pending_root.update(cx, |_, cx| {
                ready.set(true);
                cx.notify();
            });
        });
        Ok(native)
    }
    pub(super) fn request_detach_workspace(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_ready()
            || self.window.preferences_open
            || self.workspace_window_transfer
            || session_lock::locked(cx)
        {
            return;
        }
        if self.window.active_workspace_id.as_deref() != Some(&id) {
            self.window.pending_detach_workspace = Some(id.clone());
            self.switch_named_workspace(id, window, cx);
            return;
        }
        if let Err(error) = self.detach_active_workspace(window, cx) {
            self.window.error = Some(error);
            cx.notify();
        }
    }
    fn detach_active_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = self
            .window
            .active_workspace_id
            .clone()
            .ok_or("Choose a workspace to move.")?;
        self.reconcile_workspace_window_assignments()?;
        let mut assignments = self.workspace_window_assignments.clone();
        let destination = assignments.detach_workspace(&id)?;
        self.capture_native_drafts(cx);
        self.capture_window_layouts(cx)?;
        self.window
            .dock
            .update(cx, |dock, cx| dock.retain_displayed_panels(cx))?;
        self.preflight_workspace_view(&self.window, &destination, cx)?;
        let handle = self.create_workspace_host(&destination, &destination, cx)?;
        match self.transfer_current_workspace(&destination, assignments, window, cx) {
            Ok(()) => {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                Ok(())
            }
            Err(error) => {
                self.window_hosts.remove(&destination);
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                Err(error)
            }
        }
    }
    fn transfer_current_workspace(
        &mut self,
        target: &WorkspaceWindowId,
        assignments: WorkspaceWindowState,
        source_window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if target == &self.current_workspace_window {
            return Ok(());
        }
        let workspace_id = self
            .window
            .active_workspace_id
            .clone()
            .ok_or("No workspace is selected.")?;
        let handle = self
            .window_hosts
            .get(target)
            .ok_or("The destination window is unavailable.")?
            .handle;
        if self
            .parked_window_views
            .get(target)
            .is_some_and(|view| view.preferences_open)
        {
            return Err(
                "Close Preferences in the destination window before moving a workspace.".into(),
            );
        }
        self.validate_workspace_assignments(&assignments, cx)?;
        self.capture_native_drafts(cx);
        self.capture_window_layouts(cx)?;
        self.window
            .dock
            .update(cx, |dock, cx| dock.retain_displayed_panels(cx))?;
        if let Some(previous) = self.parked_window_views.get(target) {
            previous
                .dock
                .update(cx, |dock, cx| dock.retain_displayed_panels(cx))?;
        }
        let browser = self.browser.clone();
        let browser_ids = self.window.dock.read(cx).retained_browser_ids();
        handle
            .update(cx, |_, window, cx| {
                if let Some(browser) = &browser {
                    browser
                        .read(cx)
                        .preflight_tab_transfer(&browser_ids, window, cx)
                } else if browser_ids.is_empty() {
                    Ok(())
                } else {
                    Err("The browser manager is unavailable.".into())
                }
            })
            .map_err(|error| error.to_string())??;
        let next = assignments
            .window(&self.current_workspace_window)
            .and_then(|window| window.active_workspace_id())
            .map(str::to_owned);
        if let Some(replacement) = next
            .as_ref()
            .and_then(|id| self.workspace_view_cache.get(id))
        {
            if let Some(browser) = &browser {
                browser.read(cx).preflight_tab_transfer(
                    &replacement.dock.read(cx).retained_browser_ids(),
                    source_window,
                    cx,
                )?;
            }
        }
        let mut replacement = next
            .as_ref()
            .and_then(|id| self.workspace_view_cache.remove(id));
        let needs_restore = replacement.is_none() && next.is_some();
        let mut replacement = replacement.take().unwrap_or_else(|| {
            Self::create_workspace_view(
                self.chat_panes.clone(),
                self.draft_panes.clone(),
                Sidebar::Chats,
                source_window,
                cx,
            )
        });
        replacement.active_workspace_id = next.clone();
        self.workspace_window_transfer = true;
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, source_window, cx));
        let mut incoming = std::mem::replace(&mut self.window, replacement);
        let moved = handle
            .update(cx, |_, window, cx| {
                Self::rehost_workspace_view(&mut incoming, browser.as_ref(), window, cx)
            })
            .map_err(|error| error.to_string())
            .and_then(|result| result);
        if let Err(error) = moved {
            let rollback =
                Self::rehost_workspace_view(&mut incoming, browser.as_ref(), source_window, cx);
            let replacement = std::mem::replace(&mut self.window, incoming);
            if !needs_restore {
                if let Some(id) = next {
                    self.workspace_view_cache.insert(id, replacement);
                }
            }
            self.workspace_window_transfer = false;
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => format!(
                    "{error}. Returning the workspace to its source also failed: {rollback}"
                ),
            });
        }
        // No fallible operation below this point may remove the destination.
        // Both live views remain retained after the ownership commit.
        if let Some(mut previous) = self.parked_window_views.remove(target) {
            let _ = handle.update(cx, |_, window, cx| {
                previous
                    .dock
                    .update(cx, |dock, cx| dock.set_shown(false, window, cx))
            });
            if let Some(old_id) = previous.active_workspace_id.clone() {
                self.workspace_view_cache.insert(old_id, previous);
            }
        }
        self.parked_window_views.insert(target.clone(), incoming);
        self.workspace_window_assignments = assignments;
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = next.clone().unwrap_or_default();
        }
        self.workspace_window_transfer = false;
        if let Some(next) = next {
            if needs_restore {
                self.window.workspace_switch = Some(next);
                self.window.preferences_restore = true;
                self.restore_preferences(source_window, cx);
            } else if let Err(error) =
                Self::rehost_workspace_view(&mut self.window, browser.as_ref(), source_window, cx)
            {
                self.window.error = Some(error);
            }
        }
        self.workspaces_dirty = true;
        if let Err(error) = self.persist_workspace_window_assignments(false) {
            self.preferences_error = Some(error);
        }
        self.queue_preferences_save(cx);
        cx.notify();
        eprintln!("[workspace-window] operation=transfer workspace={workspace_id} result=success");
        if self.window.active_workspace_id.is_none() {
            let source = self.current_workspace_window.clone();
            let target = target.clone();
            let handle = source_window.window_handle();
            let weak = cx.weak_entity();
            cx.defer(move |cx| {
                // A close action can already have removed this host. Recheck
                // after dispatch; no failed or cancelled drag reaches here.
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |desktop, cx| {
                        if desktop.window_hosts.contains_key(&source) {
                            desktop.retire_empty_workspace_host(&source, &target, window, cx);
                        }
                    });
                });
            });
        }
        Ok(())
    }
    fn retire_empty_workspace_host(
        &mut self,
        source: &WorkspaceWindowId,
        target: &WorkspaceWindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let empty = if &self.current_workspace_window == source {
            self.window.active_workspace_id.is_none()
        } else {
            self.parked_window_views.get(source).is_some_and(|view| view.active_workspace_id.is_none())
        };
        if !empty || self.closing {
            return;
        }
        let Some(handle) = self.window_hosts.get(target).map(|host| host.handle) else {
            return;
        };
        let mut assignments = self.workspace_window_assignments.clone();
        if assignments.retire_empty_window(source, target).is_err() {
            return;
        }
        if source == self.workspace_window_assignments.primary_window_id() {
            let lock = self.session_lock.clone();
            let controller = cx.entity_id();
            let result = handle.update(cx, |_, destination, cx| {
                lock.update(cx, |lock, cx| lock.rehost_to_window(destination, cx))?;
                cx.rehost_entity_window(controller, destination).map_err(|error| error.to_string())
            }).map_err(|error| error.to_string()).and_then(|result| result);
            if let Err(error) = result {
                self.preferences_error = Some(error);
                cx.notify();
                return;
            }
        }
        if &self.current_workspace_window == source {
            let Some(view) = self.parked_window_views.remove(target) else { return; };
            self.window = view;
            self.current_workspace_window = target.clone();
        }
        self.workspace_window_assignments = assignments;
        self.parked_window_views.remove(source);
        self.window_hosts.remove(source);
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = self.window.active_workspace_id.clone().unwrap_or_default();
        }
        vim_command::forget_window(window.window_handle().window_id(), cx);
        if let Err(error) = self.persist_workspace_window_assignments(false) {
            self.preferences_error = Some(error);
        }
        window.remove_window();
        cx.notify();
    }
    fn validate_workspace_assignments(
        &self,
        assignments: &WorkspaceWindowState,
        cx: &App,
    ) -> Result<(), String> {
        let Some(set) = &self.named_workspaces else {
            return Err("Workspace preferences are not loaded.".into());
        };
        let mut live = HashMap::<String, WorkspaceWindowId>::new();
        for item in &set.items {
            let Some(host) = assignments.window_for_workspace(&item.id) else {
                continue;
            };
            let layout = if self.window.active_workspace_id.as_deref() == Some(&item.id) {
                Some(self.window.dock.read(cx).serialize(cx)?)
            } else if let Some(view) = self
                .parked_window_views
                .values()
                .chain(self.workspace_view_cache.values())
                .find(|view| view.active_workspace_id.as_deref() == Some(&item.id))
            {
                Some(view.dock.read(cx).serialize(cx)?)
            } else {
                item.layout.clone()
            };
            for id in layout
                .as_ref()
                .map(docking::DockWorkspace::saved_ids)
                .transpose()?
                .unwrap_or_default()
            {
                if !(id.starts_with("chat:")
                    || id.starts_with("browser:")
                    || id.starts_with("draft:"))
                {
                    continue;
                }
                if live.get(&id).is_some_and(|other| other != host) {
                    return Err("A chat or browser pane is included in workspaces on different windows. Remove the duplicate pane from one workspace before moving it.".into());
                }
                live.insert(id, host.clone());
            }
        }
        Ok(())
    }
    pub(super) fn focus_workspace_owner(&self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(host) = self.workspace_window_assignments.window_for_workspace(id) else {
            return false;
        };
        if host == &self.current_workspace_window {
            return false;
        }
        if let Some(handle) = self.window_hosts.get(host).map(|host| host.handle) {
            let weak = cx.weak_entity();
            let id = id.to_owned();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |desktop, cx| {
                        desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                            desktop.switch_named_workspace(id, window, cx);
                            window.activate_window();
                        })
                    });
                });
            });
        }
        true
    }
    pub(super) fn activate_retained_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if !self.workspace_view_cache.contains_key(id) {
            return Ok(false);
        }
        self.capture_native_drafts(cx);
        self.capture_window_layouts(cx)?;
        self.validate_workspace_assignments(&self.workspace_window_assignments, cx)?;
        self.window
            .dock
            .update(cx, |dock, cx| dock.retain_displayed_panels(cx))?;
        let mut next = self
            .workspace_view_cache
            .remove(id)
            .expect("Retained workspace was checked");
        let browser = self.browser.clone();
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        if let Err(error) = Self::rehost_workspace_view(&mut next, browser.as_ref(), window, cx) {
            self.workspace_view_cache.insert(id.to_owned(), next);
            self.window.dock.update(cx, |dock, cx| {
                dock.set_shown(!session_lock::locked(cx), window, cx)
            });
            return Err(error);
        }
        next.pending_workspace_transfer = self.window.pending_workspace_transfer.take();
        next.pending_detach_workspace = self.window.pending_detach_workspace.take();
        next.pending_sidebar_task = self.window.pending_sidebar_task.take();
        next.pending_notification_read = self.window.pending_notification_read.take();
        next.sidebar_task_navigation_scheduled =
            std::mem::take(&mut self.window.sidebar_task_navigation_scheduled);
        let removal = self.window.pending_workspace_removal.take();
        let old = std::mem::replace(&mut self.window, next);
        if let Some(id) = old.active_workspace_id.clone() {
            self.workspace_view_cache.insert(id, old);
        }
        if let Some(set) = &mut self.named_workspaces {
            if let Some(removal) = &removal {
                monitter_gpui_desktop::workspace_order::remove(set, removal);
            }
            set.active_id = id.to_owned();
        }
        if let Some(removal) = removal {
            self.workspace_view_cache.remove(&removal);
        }
        self.reconcile_workspace_window_assignments()?;
        self.workspace_window_assignments
            .set_active_workspace(&self.current_workspace_window, id)?;
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(true, window, cx));
        if let Err(error) = self.persist_workspace_window_assignments(false) {
            self.preferences_error = Some(error);
        }
        self.queue_preferences_save(cx);
        cx.notify();
        Ok(true)
    }
    pub(super) fn request_reattach_workspace(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_ready()
            || self.window.preferences_open
            || self.workspace_window_transfer
            || session_lock::locked(cx)
        {
            return;
        }
        if self.window.active_workspace_id.as_deref() != Some(&id) {
            self.window.pending_detach_workspace = Some(format!("return:{id}"));
            self.switch_named_workspace(id, window, cx);
            return;
        }
        let mut assignments = self.workspace_window_assignments.clone();
        let primary = self.workspace_window_assignments.primary_window_id().clone();
        let result = assignments
            .reattach_workspace(&id)
            .and_then(|_| self.transfer_current_workspace(&primary, assignments, window, cx));
        match result {
            Ok(()) => self.focus_primary_workspace_window(cx),
            Err(error) => self.window.error = Some(error),
        }
        cx.notify();
    }
    fn install_workspace_vim_bar(window: &mut Window, cx: &mut Context<Self>) {
        if vim_command::bar_for(window, cx).is_some() {
            return;
        }
        let bar = vim_command::VimCommandBar::new(window, cx);
        cx.subscribe_workspace(&bar, window, |desktop, _, event, window, cx| {
            if let vim_command::VimCommandBarEvent::Run(command) = event {
                desktop.run_vim_command(command.clone(), window, cx);
            }
        })
        .detach();
    }
    fn request_workspace_window_close(
        &mut self,
        id: &WorkspaceWindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing {
            window_state::save_native_for(id, window, cx);
            return true;
        }
        if id == self.workspace_window_assignments.primary_window_id() {
            self.with_workspace_window(window, cx, |desktop, window, cx| desktop.request_close(window, cx));
            return false;
        }
        if session_lock::locked(cx) {
            self.focus_primary_workspace_window(cx);
            return false;
        }
        let id = id.clone();
        self.with_workspace_window(window, cx, |desktop, window, cx| {
            if desktop.workspace_window_transfer || !desktop.workspace_ready() {
                return;
            }
            let primary = desktop.workspace_window_assignments.primary_window_id().clone();
            let Some(main) = desktop.window_hosts.get(&primary).map(|host| host.handle) else {
                return;
            };
            let mut assignments = desktop.workspace_window_assignments.clone();
            let owned = match assignments.close_window(&id) {
                Ok(owned) => owned,
                Err(error) => {
                    desktop.window.error = Some(error);
                    return;
                }
            };
            if let Some(active) = desktop.window.active_workspace_id.clone() {
                let _ = assignments.set_active_workspace(&primary, &active);
            }
            let browser = desktop.browser.clone();
            // Every inactive workspace must leave the closing native parent too.
            let mut moved = Vec::new();
            for workspace in &owned {
                let Some(mut view) = desktop.workspace_view_cache.remove(workspace) else {
                    continue;
                };
                let result = main
                    .update(cx, |_, target, cx| {
                        Self::rehost_workspace_view(&mut view, browser.as_ref(), target, cx)
                    })
                    .map_err(|error| error.to_string())
                    .and_then(|result| result);
                if let Err(error) = result {
                    for workspace in &moved {
                        if let Some(view) = desktop.workspace_view_cache.get_mut(workspace) {
                            let _ = Self::rehost_workspace_view(view, browser.as_ref(), window, cx);
                        }
                    }
                    let _ = Self::rehost_workspace_view(&mut view, browser.as_ref(), window, cx);
                    desktop.workspace_view_cache.insert(workspace.clone(), view);
                    desktop.window.error = Some(error);
                    cx.notify();
                    return;
                }
                // Inactive pages remain hidden until their workspace is selected.
                let _ = main.update(cx, |_, target, cx| {
                    view.dock
                        .update(cx, |dock, cx| dock.set_shown(false, target, cx))
                });
                desktop.workspace_view_cache.insert(workspace.clone(), view);
                moved.push(workspace.clone());
            }
            let result = if desktop.window.active_workspace_id.is_some() {
                desktop.transfer_current_workspace(&primary, assignments.clone(), window, cx)
            } else {
                desktop.workspace_window_assignments = assignments;
                Ok(())
            };
            match result {
                Ok(()) => {
                    window_state::save_native_for(&id, window, cx);
                    desktop.window_hosts.remove(&id);
                    desktop.parked_window_views.remove(&id);
                    vim_command::forget_window(window.window_handle().window_id(), cx);
                    if let Err(error) = desktop.persist_workspace_window_assignments(false) {
                        desktop.preferences_error = Some(error);
                    }
                    window.remove_window();
                    desktop.focus_primary_workspace_window(cx);
                }
                Err(error) => {
                    for workspace in moved {
                        if let Some(view) = desktop.workspace_view_cache.get_mut(&workspace) {
                            let _ = Self::rehost_workspace_view(view, browser.as_ref(), window, cx);
                        }
                    }
                    desktop.window.error = Some(error);
                    cx.notify();
                }
            }
        });
        false
    }
    pub(super) fn process_workspace_window_intent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_ready() {
            return;
        }
        if self.window.pending_pane_focus.is_some() {
            self.drain_workspace_pane_focus(window, cx);
            return;
        }
        if let Some((id, target, before)) = self.window.pending_workspace_transfer.take() {
            self.request_workspace_transfer(id, target, before, window, cx);
            return;
        }
        let Some(intent) = self.window.pending_detach_workspace.take() else {
            return;
        };
        if let Some(id) = intent.strip_prefix("return:") {
            self.request_reattach_workspace(id.to_owned(), window, cx);
        } else {
            self.request_detach_workspace(intent, window, cx);
        }
    }
    pub(super) fn queue_workspace_tab_drop(
        &mut self,
        id: String,
        before: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let target = self.current_workspace_window.clone();
        let source = self
            .workspace_window_assignments
            .window_for_workspace(&id)
            .cloned();
        if source.as_ref() == Some(&target) {
            if let Some(before) = before {
                self.reorder_named_workspace(&id, &before, cx);
            }
            return;
        }
        let Some(source) = source else {
            return;
        };
        let Some(handle) = self.window_hosts.get(&source).map(|host| host.handle) else {
            return;
        };
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |desktop, cx| {
                    desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                        desktop.request_workspace_transfer(id, target, before, window, cx)
                    })
                });
            });
        });
    }
    fn request_workspace_transfer(
        &mut self,
        id: String,
        target: WorkspaceWindowId,
        before: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_ready()
            || self.window.preferences_open
            || self.workspace_window_transfer
            || session_lock::locked(cx)
        {
            return;
        }
        if self.window.active_workspace_id.as_deref() != Some(&id) {
            self.window.pending_workspace_transfer = Some((id.clone(), target, before));
            self.switch_named_workspace(id, window, cx);
            return;
        }
        let mut assignments = self.workspace_window_assignments.clone();
        if let Err(error) = assignments.ensure_window(&target) {
            self.window.error = Some(error);
            return;
        }
        let index = before
            .as_deref()
            .and_then(|id| {
                assignments
                    .window(&target)?
                    .workspace_ids()
                    .iter()
                    .position(|item| item == id)
            })
            .unwrap_or(usize::MAX);
        let result = assignments
            .transfer_workspace(&id, &target, index)
            .and_then(|_| self.transfer_current_workspace(&target, assignments, window, cx));
        if let Err(error) = result {
            self.window.error = Some(error);
        } else if let Some(handle) = self.window_hosts.get(&target).map(|host| host.handle) {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        cx.notify();
    }
    pub(super) fn workspace_tab_drag_completed(
        &mut self,
        id: &str,
        completion: &DragCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(feature = "performance-profiling")]
        if crate::workspace_window_preview::enabled() {
            eprintln!("[workspace-preview] drag-complete workspace={id} result={completion:?} source={:?}", window.global_bounds());
        }
        if !matches!(completion.outcome, DragCompletionOutcome::Released)
            || session_lock::locked(cx)
        {
            return;
        }
        let Some(position) = completion.screen_position else {
            return;
        };
        if window
            .global_bounds()
            .is_none_or(|bounds| bounds.contains(&position))
        {
            return;
        }
        let source = window.window_handle();
        let mut inside = false;
        for handle in self
            .window_hosts
            .values()
            .map(|host| host.handle)
            .filter(|handle| *handle != source)
            .collect::<Vec<_>>()
        {
            inside |= handle
                .update(cx, |_, window, _| {
                    window
                        .global_bounds()
                        .is_none_or(|bounds| bounds.contains(&position))
                })
                .unwrap_or(true);
        }
        if !inside {
            self.request_detach_workspace(id.to_owned(), window, cx);
        }
    }
    pub(super) fn pane_is_owned_by_another_window(&self, id: &str, cx: &App) -> bool {
        (id.starts_with("chat:") || id.starts_with("browser:") || id.starts_with("draft:"))
            && self
                .pane_workspace_owner(id, cx)
                .is_some_and(|(_, host)| host != self.current_workspace_window)
    }
    pub(super) fn sync_workspace_projection(&mut self, cx: &mut Context<Self>) {
        if self.window.projection_revision == self.revision {
            return;
        }
        self.window.projection_revision = self.revision.clone();
        let snapshot = self.snapshot.clone();
        self.window.dock.update(cx, |dock, cx| {
            dock.set_close_buttons_visible(
                snapshot["settings"]["showTabCloseButtons"]
                    .as_bool()
                    .unwrap_or(true),
                cx,
            );
            dock.set_tab_appearance(&snapshot["settings"], self.window.sidebar_scale, cx);
            for task in rows(&snapshot, "tasks") {
                dock.set_title(
                    &format!("chat:{}", field(&task, "id")),
                    field(&task, "title").into(),
                    cx,
                );
            }
        });
        for pane in self.window.terminal_panes.values() {
            pane.update(cx, |pane, cx| pane.set_context(&snapshot, None, cx));
        }
        if let Some(workspace) = &self.window.workspace {
            workspace.update(cx, |workspace, cx| workspace.sync(snapshot.clone(), cx));
        }
        if let Some(mail) = &self.window.mail {
            mail.update(cx, |mail, cx| {
                mail.set_context(&snapshot, self.window.selected.clone(), cx)
            });
        }
        if let Some(channels) = &self.window.channels {
            channels::reconcile_cursors(&mut self.window.channel_cursors, &snapshot, true, &[]);
            channels.update(cx, |channels, cx| {
                channels.sync_scoped(
                    snapshot,
                    self.revision.as_deref().unwrap_or_default(),
                    true,
                    &[],
                    &self.window.channel_cursors,
                    &HashSet::new(),
                    cx,
                )
            });
            self.window.channels_revision = self.revision.clone();
        }
    }
    pub(super) fn primary_workspace_window(&self) -> Option<AnyWindowHandle> {
        self.window_hosts.get(self.workspace_window_assignments.primary_window_id()).map(|host| host.handle)
    }
    pub(super) fn focus_primary_workspace_window(&self, cx: &mut App) {
        if let Some(host) = self.window_hosts.get(self.workspace_window_assignments.primary_window_id()) {
            let _ = host
                .handle
                .update(cx, |_, window, _| window.activate_window());
        }
    }
    pub(super) fn spawn_workspace<F, R>(&self, window: &Window, cx: &Context<Self>, f: F) -> Task<R>
    where
        R: 'static,
        F: AsyncFnOnce(WorkspaceWeakDesktop, &mut AsyncWindowContext) -> R + 'static,
    {
        let view = WorkspaceWeakDesktop {
            desktop: cx.weak_entity(),
            workspace_id: self.window.active_workspace_id.clone(),
            fallback_window: self.current_workspace_window.clone(),
        };
        window.spawn(cx, async move |cx| f(view, cx).await)
    }
    /// Navigation waits belong to the native host. A workspace switch replaces
    /// its current view, whereas service completions follow a retained workspace.
    pub(super) fn spawn_workspace_host<F, R>(
        &self,
        window: &Window,
        cx: &Context<Self>,
        f: F,
    ) -> Task<R>
    where
        R: 'static,
        F: AsyncFnOnce(WorkspaceWeakDesktop, &mut AsyncWindowContext) -> R + 'static,
    {
        let view = WorkspaceWeakDesktop {
            desktop: cx.weak_entity(),
            workspace_id: None,
            fallback_window: self.current_workspace_window.clone(),
        };
        window.spawn(cx, async move |cx| f(view, cx).await)
    }
    pub(super) fn with_retained_workspace<R>(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> anyhow::Result<R> {
        let Some(id) = id else {
            return Ok(f(self, window, cx));
        };
        if self.window.active_workspace_id.as_deref() == Some(id) {
            return Ok(f(self, window, cx));
        }
        let next = self
            .workspace_view_cache
            .remove(id)
            .ok_or_else(|| anyhow::anyhow!("Workspace is no longer retained"))?;
        let focus = window.focused(cx);
        let previous = std::mem::replace(&mut self.window, next);
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = id.to_owned();
        }
        let result = f(self, window, cx);
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        let next = std::mem::replace(&mut self.window, previous);
        self.workspace_view_cache.insert(id.to_owned(), next);
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = self.window.active_workspace_id.clone().unwrap_or_default();
        }
        self.window.dock.update(cx, |dock, cx| {
            dock.set_shown(!session_lock::locked(cx), window, cx)
        });
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        Ok(result)
    }
    pub(super) fn prepare_uncached_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.capture_window_layouts(cx)?;
        self.window
            .dock
            .update(cx, |dock, cx| dock.retain_displayed_panels(cx))?;
        let mut next = Self::create_workspace_view(
            self.chat_panes.clone(),
            self.draft_panes.clone(),
            Sidebar::Chats,
            window,
            cx,
        );
        next.active_workspace_id = Some(id.to_owned());
        next.pending_workspace_transfer = self.window.pending_workspace_transfer.take();
        next.pending_detach_workspace = self.window.pending_detach_workspace.take();
        next.pending_workspace_removal = self.window.pending_workspace_removal.take();
        next.pending_sidebar_task = self.window.pending_sidebar_task.take();
        next.pending_notification_read = self.window.pending_notification_read.take();
        next.sidebar_task_navigation_scheduled =
            std::mem::take(&mut self.window.sidebar_task_navigation_scheduled);
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        let old = std::mem::replace(&mut self.window, next);
        if let Some(id) = old.active_workspace_id.clone() {
            self.workspace_view_cache.insert(id, old);
        }
        Ok(())
    }
    pub(super) fn remove_last_host_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.capture_native_drafts(cx);
        let next = Self::create_workspace_view(
            self.chat_panes.clone(),
            self.draft_panes.clone(),
            Sidebar::Chats,
            window,
            cx,
        );
        self.window
            .dock
            .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        let old = std::mem::replace(&mut self.window, next);
        // Removing a workspace does not terminate its backend sessions.
        drop(old);
        self.workspace_view_cache.remove(id);
        if let Some(set) = &mut self.named_workspaces {
            monitter_gpui_desktop::workspace_order::remove(set, id);
            set.active_id = String::new();
        }
        self.persist_workspace_window_assignments(false)?;
        self.workspaces_dirty = true;
        self.queue_preferences_save(cx);
        Ok(())
    }
    pub(super) fn pane_workspace_owner(
        &self,
        pane_id: &str,
        cx: &App,
    ) -> Option<(String, WorkspaceWindowId)> {
        let views = std::iter::once(&self.window)
            .chain(self.parked_window_views.values())
            .chain(self.workspace_view_cache.values());
        for view in views {
            let Some(workspace) = &view.active_workspace_id else {
                continue;
            };
            if Self::active_layout_ids(view, cx)
                .ok()
                .is_some_and(|ids| ids.contains(pane_id))
            {
                if let Some(host) = self
                    .workspace_window_assignments
                    .window_for_workspace(workspace)
                {
                    return Some((workspace.clone(), host.clone()));
                }
            }
        }
        None
    }
    pub(super) fn with_workspace_for_pane<R>(
        &mut self,
        pane_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> Option<R> {
        let (workspace, host) = self.pane_workspace_owner(pane_id, cx)?;
        let handle = self.window_hosts.get(&host)?.handle;
        cx.update_window_context(handle, |window, cx| {
            self.with_workspace_window(window, cx, |desktop, window, cx| {
                desktop.with_retained_workspace(Some(&workspace), window, cx, f)
            })
        })
        .ok()?
        .ok()
    }
    pub(super) fn with_workspace_for_pane_in<R>(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> Option<R> {
        let (workspace, host) = self.pane_workspace_owner(pane_id, cx)?;
        let handle = self.window_hosts.get(&host)?.handle;
        if handle == window.window_handle() {
            self.with_workspace_window(window, cx, |desktop, window, cx| {
                desktop.with_retained_workspace(Some(&workspace), window, cx, f)
            })
            .ok()
        } else {
            self.with_workspace_for_pane(pane_id, cx, f)
        }
    }
    pub(super) fn focus_pane_owner(&mut self, pane_id: &str, cx: &mut Context<Self>) -> bool {
        let Some((workspace, host)) = self.pane_workspace_owner(pane_id, cx) else {
            return false;
        };
        if host == self.current_workspace_window {
            return false;
        }
        if let Some(handle) = self.window_hosts.get(&host).map(|host| host.handle) {
            let _ = cx.update_window_context(handle, |window, cx| {
                self.with_workspace_window(window, cx, |desktop, window, cx| {
                    desktop.window.pending_pane_focus = Some((workspace, pane_id.to_owned()));
                    desktop.drain_workspace_pane_focus(window, cx);
                    window.activate_window();
                    cx.notify();
                })
            });
        }
        true
    }
    fn drain_workspace_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready() || self.window.preferences_open || session_lock::locked(cx) {
            return;
        }
        let Some((workspace, pane_id)) = self.window.pending_pane_focus.take() else {
            return;
        };
        if self
            .workspace_window_assignments
            .window_for_workspace(&workspace)
            != Some(&self.current_workspace_window)
        {
            return;
        }
        if self.window.active_workspace_id.as_deref() != Some(&workspace) {
            self.switch_named_workspace(workspace.clone(), window, cx);
            if self.window.active_workspace_id.as_deref() != Some(&workspace)
                || !self.workspace_ready()
            {
                self.window.pending_pane_focus = Some((workspace, pane_id));
                return;
            }
        }
        if self
            .window
            .dock
            .update(cx, |dock, cx| dock.reveal_and_focus(&pane_id, window, cx))
        {
            self.window.dock_current = Some(pane_id.clone());
            if let Some(id) = pane_id.strip_prefix("chat:") {
                self.window.selected = Some(id.to_owned());
                self.window.sidebar = Sidebar::Chats;
            } else if pane_id.starts_with("browser:") {
                self.window.sidebar = Sidebar::Browser;
            }
        }
        cx.notify();
    }
    pub(super) fn with_workspace_window<R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> R {
        let id = self
            .window_hosts
            .iter()
            .find(|(_, host)| host.handle == window.window_handle())
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| self.current_workspace_window.clone());
        if id == self.current_workspace_window {
            return f(self, window, cx);
        }
        let Some(destination) = self.parked_window_views.remove(&id) else {
            return f(self, window, cx);
        };
        let previous_id = std::mem::replace(&mut self.current_workspace_window, id.clone());
        let previous = std::mem::replace(&mut self.window, destination);
        self.parked_window_views
            .insert(previous_id.clone(), previous);
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = self.window.active_workspace_id.clone().unwrap_or_default();
        }
        let result = f(self, window, cx);
        let previous = self
            .parked_window_views
            .remove(&previous_id)
            .expect("The calling workspace host remains retained during dispatch");
        let destination = std::mem::replace(&mut self.window, previous);
        self.current_workspace_window = previous_id;
        if self.window_hosts.contains_key(&id) {
            self.parked_window_views.insert(id, destination);
        }
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = self.window.active_workspace_id.clone().unwrap_or_default();
        }
        result
    }
    pub(super) fn active_workspace_window(&self, cx: &App) -> Option<AnyWindowHandle> {
        cx.active_window().and_then(|active| {
            self.window_hosts.values().find(|host| host.handle == active).map(|host| host.handle)
        }).or_else(|| self.primary_workspace_window())
    }
    pub(super) fn ensure_workspace_window_registry(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Self::install_workspace_vim_bar(window, cx);
        if self.window_hosts.is_empty() {
            self.window_hosts.insert(
                self.current_workspace_window.clone(),
                WorkspaceHost {
                    handle: window.window_handle(),
                    root: None,
                },
            );
            let entity = cx.entity_id();
            let _ = cx.rehost_entity_window(entity, window);
        }
        if self.workspace_window_restore_pending && self.window.active_workspace_id.is_none() {
            self.window.active_workspace_id = self
                .named_workspaces
                .as_ref()
                .map(|set| set.active_id.clone())
                .filter(|id| !id.is_empty());
        }
    }
    pub(super) fn initialize_workspace_window_ownership(&mut self) {
        if !self.workspace_window_restore_pending {
            return;
        }
        let Some(set) = &self.named_workspaces else {
            return;
        };
        let ids = set
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        #[cfg(not(test))]
        let saved = window_state::load_workspace_windows();
        #[cfg(test)]
        let saved: Option<WorkspaceWindowState> = None;
        self.workspace_window_assignments = saved.unwrap_or_default();
        if let Err(error) = self.workspace_window_assignments.reconcile(&ids) {
            self.preferences_error = Some(error);
            self.workspace_window_assignments = WorkspaceWindowState::new(&ids).unwrap_or_default();
        }
        let active = self
            .workspace_window_assignments
            .window(self.workspace_window_assignments.primary_window_id())
            .and_then(|host| host.active_workspace_id())
            .map(str::to_owned);
        // The first native host restores the saved primary, even if the
        // original main window was retired after its last tab moved.
        let primary = self.workspace_window_assignments.primary_window_id().clone();
        if self.current_workspace_window != primary {
            if let Some(host) = self.window_hosts.remove(&self.current_workspace_window) {
                self.window_hosts.insert(primary.clone(), host);
            }
            self.current_workspace_window = primary;
        }
        self.window.active_workspace_id = active.clone();
        self.window.workspace_switch = active.clone();
        self.window.preferences_restore = active.is_some();
        if let Some(set) = &mut self.named_workspaces {
            set.active_id = active.unwrap_or_default();
        }
    }
    pub(super) fn restore_workspace_window_hosts(
        &mut self,
        source: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_window_restore_pending
            || self.preferences.is_none()
            || self.named_workspaces.is_none()
            || self.publication_index.is_none()
        {
            return;
        }
        self.workspace_window_restore_pending = false;
        if let Err(error) =
            self.validate_workspace_assignments(&self.workspace_window_assignments, cx)
        {
            self.preferences_error = Some(error);
            let ids = self
                .named_workspaces
                .as_ref()
                .unwrap()
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            self.workspace_window_assignments = WorkspaceWindowState::new(&ids).unwrap_or_default();
            if let Some(host) = self.window_hosts.remove(&self.current_workspace_window) {
                self.window_hosts.insert(WorkspaceWindowId::main(), host);
            }
            self.current_workspace_window = WorkspaceWindowId::main();
            let active = self
                .workspace_window_assignments
                .window(self.workspace_window_assignments.primary_window_id())
                .and_then(|host| host.active_workspace_id())
                .map(str::to_owned);
            self.window.active_workspace_id = active.clone();
            self.window.workspace_switch = active;
            self.window.preferences_restore = true;
            return;
        }
        let hosts = self
            .workspace_window_assignments
            .windows()
            .iter()
            .filter(|host| host.window_id() != self.workspace_window_assignments.primary_window_id())
            .cloned()
            .collect::<Vec<_>>();
        for host in hosts {
            let id = host.window_id().clone();
            let handle = match self.create_workspace_host(&id, &id, cx) {
                Ok(handle) => handle,
                Err(error) => {
                    self.preferences_error = Some(error);
                    let _ = self.workspace_window_assignments.close_window(&id);
                    continue;
                }
            };
            let mut view = Self::create_workspace_view(
                self.chat_panes.clone(),
                self.draft_panes.clone(),
                Sidebar::Chats,
                source,
                cx,
            );
            view.active_workspace_id = host.active_workspace_id().map(str::to_owned);
            view.workspace_switch = view.active_workspace_id.clone();
            view.preferences_restore = view.active_workspace_id.is_some();
            let browser = self.browser.clone();
            let _ = handle.update(cx, |_, window, cx| {
                Self::rehost_workspace_view(&mut view, browser.as_ref(), window, cx)
            });
            self.parked_window_views.insert(id, view);
        }
        cx.notify();
    }
    pub(super) fn create_workspace_view(
        chat_panes: Rc<RefCell<HashMap<String, Entity<chat::ChatPane>>>>,
        draft_panes: Rc<RefCell<HashMap<String, Entity<draft_creation::DraftCreationPane>>>>,
        route: Sidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DesktopWindowView {
        let retained_drafts = draft_panes.clone();
        let retained = chat_panes.clone();
        let factory: docking::PaneFactory = Rc::new(move |id, _, cx| {
            if id.starts_with("draft:") {
                let pane = retained_drafts.borrow().get(id)?.clone();
                return Some(docking::PaneSpec {
                    id: id.into(),
                    title: "New Chat".into(),
                    content: pane.clone().into(),
                    focus: pane.focus_handle(cx),
                    lifecycle: None,
                    closable: true,
                });
            }
            let task_id = id.strip_prefix("chat:")?;
            let pane = retained.borrow().get(task_id)?.clone();
            Some(docking::PaneSpec {
                id: id.into(),
                title: "Chat".into(),
                content: pane.clone().into(),
                focus: pane.focus_handle(cx),
                lifecycle: None,
                closable: true,
            })
        });
        let dock =
            cx.new(|cx| docking::DockWorkspace::new("monitter-workspace", factory, window, cx));
        let layout_subscription =
            cx.subscribe_workspace(&dock, window, |view, source_dock, event, window, cx| {
                if let docking::WorkspaceEvent::VisibilityChanged { id, visible } = event {
                    if let Some(task_id) = id.strip_prefix("chat:") {
                        let pane = view.chat_panes.borrow().get(task_id).cloned();
                        if let Some(pane) = pane {
                            pane.update(cx, |pane, cx| {
                                pane.set_dock_visible(*visible, cx);
                                if *visible {
                                    pane.ensure_latest_history(cx);
                                }
                            });
                        }
                        if !visible {
                            view.compact_inactive_task(task_id, cx);
                        }
                    }
                }
                if let docking::WorkspaceEvent::Removed { id } = event {
                    view.window.git_panes.remove(id);
                    view.release_removed_neovim_pane(id, cx);
                    if browser_workspace::parse_browser_pane_id(id).is_some() {
                        let removed = HashSet::from([id.clone()]);
                        if let Some(set) = &mut view.named_workspaces {
                            for workspace in &mut set.items {
                                if let Some(layout) = &mut workspace.layout {
                                    match workspace_visibility::prune_native_pane_ids(
                                        layout, &removed,
                                    ) {
                                        Ok(cleaned) if cleaned != *layout => {
                                            *layout = cleaned;
                                            view.workspaces_dirty = true;
                                        }
                                        Err(error) => view.preferences_error = Some(error),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        if view.workspaces_dirty {
                            view.queue_preferences_save(cx);
                        }
                    }
                    if id.starts_with("draft:") {
                        view.draft_panes.borrow_mut().remove(id);
                        view.draft_subscriptions.remove(id);
                        if view.window.dock_current.as_deref() == Some(id.as_str()) {
                            view.window.draft_creation = None;
                            view.window.dock_current = None;
                        }
                    }
                    if let Some(task_id) = id.strip_prefix("chat:") {
                        // A cached workspace can still mount this same retained
                        // chat. Release the global entity/history only after the
                        // final actual layout reference disappears.
                        let retained = std::iter::once(&view.window)
                            .chain(view.parked_window_views.values())
                            .chain(view.workspace_view_cache.values())
                            .any(|retained| {
                                Self::active_layout_ids(retained, cx)
                                    .ok()
                                    .is_some_and(|ids| ids.contains(id))
                            });
                        if !retained {
                            view.chat_panes.borrow_mut().remove(task_id);
                            view.chat_subscriptions.remove(task_id);
                            view.queue_native_publication(
                                PublicationChanges {
                                    trim_tasks: HashSet::from([task_id.to_owned()]),
                                    ..Default::default()
                                },
                                cx,
                            );
                        }
                        for retained in std::iter::once(&mut view.window)
                            .chain(view.parked_window_views.values_mut())
                            .chain(view.workspace_view_cache.values_mut())
                        {
                            if retained.dock.entity_id() == source_dock.entity_id()
                                && retained.selected.as_deref() == Some(task_id)
                                && retained.dock_current.as_deref() == Some(id.as_str())
                            {
                                retained.selected = None;
                                retained.dock_current = None;
                            }
                        }
                    }
                }
                if let docking::WorkspaceEvent::EmptyClose { pane_id } = event {
                    if view
                        .window
                        .dock
                        .update(cx, |dock, cx| dock.close_empty(pane_id, window, cx))
                    {
                        view.queue_preferences_save(cx);
                    }
                    return;
                }
                if let docking::WorkspaceEvent::EmptyAction { pane_id, action } = event {
                    if let Err(error) = view.empty_pane_action(pane_id, *action, window, cx) {
                        view.window.error = Some(error);
                        cx.notify();
                    }
                    return;
                }
                if let docking::WorkspaceEvent::EmptyWorkspaceAction(action) = event {
                    if view.closing
                        || view.window.busy
                        || view.window.empty_terminal_pending
                        || view.service.is_none()
                        || view.preferences.is_none()
                        || view.window.preferences_restore
                        || view.window.preferences_restoring
                    {
                        return;
                    }
                    let result = view
                        .window
                        .dock
                        .update(cx, |dock, cx| dock.add_focused_empty_tab(window, cx))
                        .and_then(|id| view.empty_pane_action(&id, *action, window, cx));
                    if let Err(error) = result {
                        view.window.error = Some(error);
                        cx.notify();
                    }
                    return;
                }
                if let docking::WorkspaceEvent::EmptySession { pane_id, task_id } = event {
                    let valid = rows(&view.snapshot, "tasks")
                        .iter()
                        .any(|task| field(task, "id") == *task_id && task["archived"] != true);
                    if valid
                        && !view.closing
                        && !view.window.busy
                        && view.service.is_some()
                        && view.window.dock.read(cx).panel_key(pane_id).is_some()
                    {
                        view.window.pending_empty_session =
                            Some((pane_id.clone(), task_id.clone()));
                        view.window.selected = Some(task_id.clone());
                        view.window.sidebar = Sidebar::Chats;
                        view.window.draft_creation_open = false;
                        view.window.dock_current = None;
                        cx.notify();
                    }
                    return;
                }
                if matches!(event, docking::WorkspaceEvent::LayoutChanged)
                    && !view.closing
                    && view.preferences.is_some()
                    && !view.window.preferences_restore
                    && !view.window.preferences_restore_suppressed
                {
                    view.workspaces_dirty = true;
                    view.queue_preferences_save(cx);
                }
                if let docking::WorkspaceEvent::AutoSizePanesChanged(_) = event {
                    cx.notify();
                    if !view.closing
                        && view.preferences.is_some()
                        && !view.window.preferences_restore_suppressed
                    {
                        view.workspaces_dirty = true;
                        view.queue_preferences_save(cx);
                    }
                }
            });
        Self::install_workspace_vim_bar(window, cx);
        let wallpaper_backdrop = cx.new(|_| wallpaper_backdrop::WallpaperBackdrop::default());
        cx.observe(&wallpaper_backdrop, |_, _, cx| cx.notify())
            .detach();
        let activation_subscription = cx.observe_window_activation(window, |view, window, cx| {
            view.with_workspace_window(window, cx, |view, window, cx| {
                if window.is_window_active() {
                    view.refresh_wallpaper(window, cx);
                }
                if !window.is_window_active() {
                    view.set_workspace_number_hints(false, cx);
                    if !cx.has_platform_owned_drag() {
                        cx.stop_active_drag(window);
                    }
                }
            })
        });
        let sidebar_resize = cx.new(|_| ResizableState::default());
        let sidebar_resize_subscription =
            cx.subscribe_workspace(&sidebar_resize, window, |view, state, event, _, cx| {
                if matches!(event, ResizablePanelEvent::Resized) {
                    if let Some(width) = state.read(cx).sizes().first() {
                        let width = *width / design::ui_px(1., view.window.sidebar_scale);
                        if width.is_finite()
                            && !view.window.sidebar_resize_suppressed
                            && !view.sidebar_is_hidden()
                            && !view.window.preferences_restore_suppressed
                        {
                            view.window.sidebar_width_touched = true;
                            let collapse = sidebar_should_collapse(
                                width,
                                view.window.sidebar_collapsed,
                                view.window.sidebar_scale,
                            );
                            if collapse != view.window.sidebar_collapsed {
                                if collapse {
                                    view.window.sidebar_expanded_width =
                                        expanded_sidebar_width_for_scale(
                                            view.window.sidebar_expanded_width,
                                            view.window.sidebar_scale,
                                        );
                                }
                                view.window.sidebar_collapsed = collapse;
                            }
                            // Drag takeover can clear a Hidden target without
                            // changing the collapse flag. Commit the resolved
                            // visible mode on release even when that flag agrees.
                            preferences_modal::save_sidebar_collapse(collapse);
                            if !collapse {
                                view.window.sidebar_expanded_width =
                                    expanded_sidebar_width_for_scale(
                                        width,
                                        view.window.sidebar_scale,
                                    );
                            }
                            view.queue_preferences_save(cx);
                        }
                    }
                }
            });

        DesktopWindowView {
            active_workspace_id: None,
            removed_pane_ids: Vec::new(),
            projection_revision: None,
            pending_detach_workspace: None,
            pending_workspace_transfer: None,
            pending_pane_focus: None,
            dock: dock,
            palette: None,
            palette_subscription: None,
            palette_terminal_sessions: vec![],
            palette_terminals_loading: false,
            _layout_subscription: layout_subscription,
            _activation_subscription: Some(activation_subscription),
            sidebar_resize: sidebar_resize,
            _sidebar_resize_subscription: sidebar_resize_subscription,
            sidebar_width_touched: false,
            sidebar_collapsed: preferences_modal::load_sidebar_collapse(),
            sidebar_hidden: preferences_modal::load_sidebar_hidden(),
            sidebar_expanded_width: 252.,
            sidebar_motion: Default::default(),
            sidebar_motion_frame_pending: false,
            sidebar_motion_compact: false,
            sidebar_focus: cx.focus_handle(),
            sidebar_toggle_focus: cx.focus_handle(),
            preferences_open: false,
            preferences_focus: cx.focus_handle(),
            preferences_previous_focus: None,
            sidebar_scale: design::INTERFACE_SCALE_DEFAULT,
            sidebar_view: SidebarView::Activity,
            sidebar_tabs_motion: Default::default(),
            workspace_switch: None,
            pending_sidebar_task: None,
            pending_notification_read: None,
            sidebar_task_navigation_scheduled: false,
            pending_workspace_removal: None,
            workspace_editor: None,
            workspace_number_hints: false,
            route_touched: false,
            preferences_restore: false,
            preferences_restore_suppressed: false,
            preferences_restoring: false,
            preferences_restore_generation: 0,
            sidebar_resize_suppressed: false,
            sidebar_resize_generation: 0,
            dock_current: None,
            wallpaper_backdrop: wallpaper_backdrop,
            lock_previous_focus: None,
            selected: None,
            sidebar: route,
            filter: None,
            error: None,
            busy: false,
            empty_terminal_pending: false,
            pending_empty_session: None,
            pending_draft_chats: BTreeMap::new(),
            native_settings: None,
            draft_creation: None,
            draft_creation_open: false,
            applied_style: None,
            applied_window_surface: None,
            workspace: None,
            archives: None,
            archives_subscription: None,
            archives_revision: None,
            mail: None,
            mail_context_task: None,
            collapsed_agents: HashSet::new(),
            channels: None,
            channels_revision: None,
            channel_subscription: None,
            channel_cursors: HashMap::new(),
            terminal: None,
            terminal_context: None,
            terminal_panes: HashMap::new(),
            neovim_sessions: HashMap::new(),
            neovim_panes: HashMap::new(),
            neovim_layouts: HashMap::new(),
            neovim_origins: HashMap::new(),
            neovim_focus_windows: HashMap::new(),
            neovim_pending: false,
            usage: None,
            usage_revision: None,
            app_resources: None,
            sidebar_usage: None,
            workspace_usage_rings: None,
            notification_bell: None,
            notification_subscription: None,
            admin_activity: None,
            admin_activity_subscription: None,
            sidebar_usage_snapshot: None,
            sidebar_usage_revision: None,
            schedules: None,
            schedules_revision: None,
            agent_editor: None,
            agent_editor_revision: None,
            subagents: None,
            subagents_context: None,
            git: None,
            git_panes: HashMap::new(),
            git_panes_revision: None,
            git_context: None,
        }
    }
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "workspace_window_tests.rs"]
mod tests;

impl WorkspaceWeakDesktop {
    /// App-scoped work follows its captured workspace even while that view is cached.
    pub(super) fn update<C: AppContext, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut Desktop, &mut Window, &mut Context<Desktop>) -> R,
    ) -> anyhow::Result<R> {
        self.desktop.update(cx, |desktop, cx| {
            let id = self
                .workspace_id
                .as_deref()
                .and_then(|id| {
                    desktop
                        .workspace_window_assignments
                        .window_for_workspace(id)
                })
                .unwrap_or(&self.fallback_window);
            let handle = desktop
                .window_hosts
                .get(id)
                .map(|host| host.handle)
                .ok_or_else(|| anyhow::anyhow!("Workspace window is unavailable"))?;
            cx.update_window_context(handle, |window, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    desktop.with_retained_workspace(
                        self.workspace_id.as_deref(),
                        window,
                        cx,
                        update,
                    )
                })
            })?
        })?
    }
}

impl Desktop {
    pub(super) fn workspace_weak_desktop(&self, cx: &Context<Self>) -> WorkspaceWeakDesktop {
        WorkspaceWeakDesktop {
            desktop: cx.weak_entity(),
            workspace_id: self.window.active_workspace_id.clone(),
            fallback_window: self.current_workspace_window.clone(),
        }
    }
    pub(super) fn retained_workspace_restore_state(&self) -> (bool, bool) {
        let views = std::iter::once(&self.window)
            .chain(self.parked_window_views.values())
            .chain(self.workspace_view_cache.values());
        views.fold((false, false), |(pending, suppressed), view| {
            (
                pending || view.preferences_restore || view.preferences_restoring,
                suppressed || view.preferences_restore_suppressed,
            )
        })
    }
    pub(super) fn workspace_window_handles(&self) -> Vec<AnyWindowHandle> {
        let mut hosts = self.window_hosts.iter().collect::<Vec<_>>();
        hosts.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
        hosts.into_iter().map(|(_, host)| host.handle).collect()
    }
    /// Save keyed placements before flushing the common ownership/placement writer.
    pub(super) fn capture_all_workspace_window_placements(
        &mut self,
        current: Option<&Window>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let hosts = self
            .window_hosts
            .iter()
            .map(|(id, host)| (id.clone(), host.handle))
            .collect::<Vec<_>>();
        if hosts.is_empty() {
            if let Some(window) = current {
                window_state::save_native_for(&WorkspaceWindowId::main(), window, cx);
            }
        }
        for (id, handle) in hosts {
            if let Some(window) = current.filter(|window| window.window_handle() == handle) {
                window_state::save_native_for(&id, window, cx);
            } else {
                cx.update_window_context(handle, |window, cx| {
                    window_state::save_native_for(&id, window, cx)
                })
                .map_err(|error| error.to_string())?;
            }
        }
        self.persist_workspace_window_assignments(true)
    }
    fn close_workspace_view_for_quit(
        view: &mut DesktopWindowView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(mail) = &view.mail {
            mail.update(cx, |mail, cx| mail.deactivate(window, cx));
        }
        view.dock.update(cx, |dock, cx| dock.close_all(window, cx));
    }
    fn close_workspace_host_views_for_quit(
        &mut self,
        id: &WorkspaceWindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if id == &self.current_workspace_window {
            Self::close_workspace_view_for_quit(&mut self.window, window, cx);
        } else if let Some(view) = self.parked_window_views.get_mut(id) {
            Self::close_workspace_view_for_quit(view, window, cx);
        }
        let cached = self
            .workspace_view_cache
            .keys()
            .filter(|workspace| {
                self.workspace_window_assignments
                    .window_for_workspace(workspace)
                    .unwrap_or(self.workspace_window_assignments.primary_window_id())
                    == id
            })
            .cloned()
            .collect::<Vec<_>>();
        for workspace in cached {
            if let Some(view) = self.workspace_view_cache.get_mut(&workspace) {
                Self::close_workspace_view_for_quit(view, window, cx);
            }
        }
    }
    /// This only releases native surfaces during actual global quit. It never
    /// runs when an individual detached window returns workspaces to main.
    pub(super) fn close_all_workspace_native_views(
        &mut self,
        mut current: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let hosts = self
            .window_hosts
            .iter()
            .map(|(id, host)| (id.clone(), host.handle))
            .collect::<Vec<_>>();
        if hosts.is_empty() {
            if let Some(window) = current.as_deref_mut() {
                let id = self.current_workspace_window.clone();
                self.close_workspace_host_views_for_quit(&id, window, cx);
            }
        }
        for (id, handle) in hosts {
            if let Some(window) = current
                .as_deref_mut()
                .filter(|window| window.window_handle() == handle)
            {
                self.close_workspace_host_views_for_quit(&id, window, cx);
            } else if let Err(error) = cx.update_window_context(handle, |window, cx| {
                self.close_workspace_host_views_for_quit(&id, window, cx);
            }) {
                eprintln!("[workspace-window] operation=quit-close result=error error={error}");
            }
        }
    }
}
