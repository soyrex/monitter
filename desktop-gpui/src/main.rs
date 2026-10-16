//! Experimental native desktop. The backend remains the sole state authority.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{
        Disableable,
        WindowExt as _,
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        resizable::{ResizablePanelEvent, ResizableState, h_resizable, resizable_panel},
    },
    prelude::*,
    *,
};
use monitter_gpui_desktop::native_state::{ApplyResult, NativeState};
use monitter_gpui_desktop::native_publication::{self, PublicationChanges, PublicationIndex, PublicationQueue, PreparedPublication};
use std::sync::Arc;
use monitter_lib::experimental_desktop::{
    ChannelMessagesPage, DesktopService, NativeAttachmentDraft, NativeBrowserMetadata,
    NativeChatPanePreferences, NativeDetailTab, NativePreferences, TaskMessagesPage,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
};
mod agents;
mod archives;
mod backend_launcher;
mod branding;
mod browser;
mod browser_workspace;
mod channels;
mod chat;
mod composer_scroll;
mod chat_task_actions;
mod fork_dialog;
mod chat_voice;
mod collaboration;
mod agent_graph;
mod plans;
#[cfg(feature = "visual-export")]
mod design_system_preview;
mod command_palette;
mod palette_catalog;
mod companion_sidebar;
mod composer_footer;
mod composer_git;
mod docking;
mod draft_creation;
mod empty_pane;
mod git;
mod git_workspace;
mod native_clock;
mod window_state;
mod drawer_preferences;
mod wallpaper_accent;
mod wallpaper_backdrop;
#[cfg(target_os = "macos")]
mod wallpaper_macos;
mod wallpaper_platform;
mod wallpaper_theme;
mod session_lock;
mod inert_workspace;
mod session_lock_store;
mod native_icons;
mod notifications;
mod admin_activity;
mod notification_model;
mod notification_center;
mod workspace_notification_scope;
#[cfg(test)]
mod workspace_notification_tests;
mod native_menu;
mod preferences_modal;
mod blurred_backdrop;
mod dialog_backdrop;
mod native_motion;
use monitter_gpui_desktop::streaming_display;
mod streaming_message;
mod activity_sparkles;
mod activity_patterns;
mod activity_effects;
mod naming_title;
mod native_navigation;
mod shortcut_desktop;
mod native_voice_capture;
mod slash_palette;
mod task_project_picker;
mod terminal;
mod neovim;
mod neovim_files;
mod neovim_workspace;
mod terminal_workspace;
mod native_session_restore;
mod terminal_theme;
mod timestamps;
mod workspace_visibility;
mod workspace_tabs;
mod workspace_favicon;
mod report_theme;
mod usage;
mod vim_command;
use channels::ChannelPane;
mod history;
use browser::BrowserPane;
mod design;
mod interactions;
mod mail;
mod pane_appearance;
mod native_pane_highlight;
mod native_popup_occlusion;
#[cfg(all(test, feature = "visual-export"))]
mod workspace_restore_regression_tests;
mod pane_highlight_geometry;
mod surface_blend;
mod window_surface;
mod pane_approvals;
mod approval_prompt;
mod pane_details;
mod pointer_guard;
#[cfg(test)]
extern crate gpui_kit as gpui;
#[cfg(test)]
mod cached_modal_replay_tests;
#[cfg(test)]
mod resize_release_tests;
#[cfg(test)]
mod sidebar_resize_fixture_tests;
#[cfg(test)]
mod tab_drag_tests;
#[cfg(test)]
mod composer_enter_tests;
#[cfg(test)]
mod shortcut_input_tests;
mod response_metadata;
mod schedules;
mod settings;
mod shell;
mod sidebar_usage;
mod sidebar_tabs_motion;
mod layout_motion;
mod chrome_motion;
mod gui_process_metrics;
mod startup_workspace;
use mail::MailPane;
mod transcript;
mod transcript_edge_fade;
#[cfg(feature = "visual-export")]
mod visual_export;
#[cfg(feature = "performance-profiling")]
mod performance_metrics;
#[cfg(feature = "performance-profiling")]
mod performance_scenario;
#[cfg(feature = "performance-profiling")]
mod surface_preview;
mod workspace;
use transcript::TranscriptState;
use workspace::WorkspacePane;
gpui_kit::actions!(
    monitter_desktop,
    [SendMessage, Quit]
);

fn retire_snapshot(snapshot: Rc<Value>, cx: &App) {
    if let Some(snapshot) = native_publication::take_unshared_snapshot(snapshot) {
        cx.background_executor().spawn(async move { drop(snapshot) }).detach();
    }
}
fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

const SIDEBAR_ICON_THRESHOLD_PX: f32 = 120.;
fn expanded_sidebar_width(width: f32) -> f32 {
    expanded_sidebar_width_for_scale(width, 100)
}
fn expanded_sidebar_width_for_scale(width: f32, scale: u16) -> f32 {
    let minimum = SIDEBAR_ICON_THRESHOLD_PX / f32::from(design::ui_px(1., scale));
    if width.is_finite() {
        width.clamp(minimum, design::SIDEBAR_MAX_WIDTH)
    } else {
        252.
    }
}
fn saved_sidebar_width_target(
    saved: Option<f32>,
    current: f32,
    touched: bool,
    collapsed: bool,
    is_resizing: bool,
    scale: u16,
) -> Option<f32> {
    if touched || collapsed || is_resizing {
        return None;
    }
    saved
        .map(|width| expanded_sidebar_width_for_scale(width, scale))
        .filter(|desired| (current - desired).abs() >= 0.5)
}
fn sidebar_split_element(
    state: &Entity<ResizableState>, width_design: f32, scale: u16,
    sidebar: AnyElement, content: AnyElement,
) -> AnyElement {
    sidebar_split_with_visibility(state, width_design, scale, sidebar, content, false, false)
}

fn sidebar_split_with_visibility(
    state: &Entity<ResizableState>, width_design: f32, scale: u16,
    sidebar: AnyElement, content: AnyElement, hidden: bool, moving: bool,
) -> AnyElement {
    h_resizable("native-sidebar-split")
        .resizable(!hidden)
        .when(design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern, |split| {
            split.with_handle_appearance(gpui_kit::component::resizable::rounded_resize_handle_appearance())
        })
        .with_state(state)
        .child(resizable_panel()
            .size(design::ui_px(width_design, scale))
            .size_range((if moving || hidden { gpui_kit::px(0.) } else { gpui_kit::px(design::COMPACT_SIDEBAR_WIDTH_POINTS) })..design::ui_px(design::SIDEBAR_MAX_WIDTH, scale))
            .flex_none().child(div().size_full().overflow_hidden().child(sidebar)))
        .child(resizable_panel().child(div().size_full().child(content)))
        .into_any_element()
}

fn sidebar_resize_target(
    width_design: f32,
    collapsed: bool,
    is_resizing: bool,
    interface_scale: u16,
) -> Option<f32> {
    if is_resizing {
        return None;
    }
    let desired = if collapsed {
        design::compact_sidebar_width_design(interface_scale)
    } else {
        expanded_sidebar_width_for_scale(width_design, interface_scale)
    };
    ((width_design - desired).abs() > 0.5).then_some(desired)
}

fn sidebar_should_collapse(width_design: f32, _collapsed: bool, interface_scale: u16) -> bool {
    width_design.is_finite() && f32::from(design::ui_px(width_design,interface_scale)) < SIDEBAR_ICON_THRESHOLD_PX
}

fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq)]
enum Sidebar {
    Chats,
    Archived,
    Agents,
    Projects,
    Browser,
    Settings,
    Mail,
    Channels,
    Terminal,
    Usage,
    Subagents,
    Schedules,
    Git,
}
/// A retained focus ancestor for auxiliary views whose own buttons and inputs
/// have separate focus handles. Dock shortcuts can then resolve their pane.
struct PaneFocusFrame {
    content: AnyView,
    focus: FocusHandle,
}
impl Render for PaneFocusFrame {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    // Preserve focused inputs; otherwise clicking auxiliary content
                    // establishes the owning pane for native dock shortcuts.
                    if !view.focus.contains_focused(window, cx) {
                        window.focus(&view.focus, cx);
                    }
                }),
            )
            .child(self.content.clone())
    }
}

struct Desktop {
    dock: Entity<docking::DockWorkspace>,
    palette: Option<Entity<command_palette::CommandPalette>>,
    palette_subscription: Option<Subscription>,
    palette_terminal_sessions: Vec<Value>,
    palette_terminals_loading: bool,
    _layout_subscription: Subscription,
    _activation_subscription: Subscription,
    sidebar_resize: Entity<ResizableState>,
    _sidebar_resize_subscription: Subscription,
    sidebar_width_touched: bool,
    sidebar_collapsed: bool,
    sidebar_hidden: bool,
    sidebar_expanded_width: f32,
    sidebar_motion: layout_motion::WidthMotion,
    sidebar_motion_frame_pending: bool,
    sidebar_motion_compact: bool,
    sidebar_focus: FocusHandle,
    sidebar_toggle_focus: FocusHandle,
    preferences_open: bool,
    preferences_focus: FocusHandle,
    preferences_previous_focus: Option<FocusHandle>,
    sidebar_scale: u16,
    sidebar_view: SidebarView,
    sidebar_tabs_motion: RefCell<crate::sidebar_tabs_motion::SidebarTabsMotion>,
    preferences: Option<NativePreferences>,
    named_workspaces: Option<monitter_lib::experimental_desktop::NativeWorkspaceSet>,
    workspaces_dirty: bool,
    workspace_store_revision: i64,
    pending_auto_size_panes: Option<bool>,
    workspace_switch: Option<String>,
    pending_sidebar_task: Option<String>,
    pending_notification_read: Option<String>,
    sidebar_task_navigation_scheduled: bool,
    pending_workspace_removal: Option<String>,
    workspace_editor: Option<workspace_tabs::WorkspaceEditor>,
    workspace_number_hints: bool,
    native_drafts: BTreeMap<String, String>,
    dirty_draft_ids: HashSet<String>,
    native_attachment_drafts: BTreeMap<String, NativeAttachmentDraft>,
    dirty_attachment_draft_ids: HashSet<String>,
    native_browser_metadata: Option<NativeBrowserMetadata>,
    native_browser_dirty: bool,
    browser_restore_revision: Option<u64>,
    browser_subscription: Option<Subscription>,
    native_chat_panes: BTreeMap<String, NativeChatPanePreferences>,
    dirty_chat_pane_ids: HashSet<String>,
    route_touched: bool,
    preferences_restore: bool,
    preferences_restore_suppressed: bool,
    preferences_restoring: bool,
    preferences_generation: u64,
    preferences_restore_generation: u64,
    sidebar_resize_suppressed: bool,
    sidebar_resize_generation: u64,
    preferences_blocked: bool,
    preferences_dirty: bool,
    preferences_busy: bool,
    preferences_error: Option<String>,
    preferences_save_timer: Option<Task<()>>,
    close_pending: bool,
    chat_panes: Rc<RefCell<HashMap<String, Entity<chat::ChatPane>>>>,
    chat_subscriptions: HashMap<String, Subscription>,
    dock_current: Option<String>,
    service: Option<DesktopService>,
    snapshot: Rc<Value>,
    revision: Option<String>,
    native_state: NativeState,
    publication: PublicationQueue,
    publication_index: Option<Arc<PublicationIndex>>,
    publication_error: Option<String>,
    notification_projection: Option<Arc<notification_model::NotificationProjection>>,
    empty_sessions: monitter_gpui_desktop::session_picker::PickerGroups,
    collaboration_records: HashMap<String, Arc<Vec<Value>>>,
    collaboration_transcripts: HashMap<String, native_publication::PreparedChildTranscripts>,
    full_reset_requested: bool,
    startup_cache: Option<monitter_gpui_desktop::startup_cache::Presentation>,
    startup_preview: Option<Entity<docking::DockWorkspace>>,
    startup_pending: bool,
    startup_first_frame: bool,
    startup_live_frame_pending: bool,
    startup_cache_timer: Option<Task<()>>,
    wallpaper_accent: Entity<wallpaper_accent::WallpaperAccent>,
    wallpaper_backdrop: Entity<wallpaper_backdrop::WallpaperBackdrop>,
    session_lock: Entity<session_lock::SessionLock>,
    lock_previous_focus: Option<FocusHandle>,
    lock_close_saving: bool,
    lock_close_confirmed: bool,
    hidden_task_ids: HashSet<String>,
    removed_dock_ids: Vec<String>,
    selected: Option<String>,
    sidebar: Sidebar,
    filter: Option<String>,
    error: Option<String>,
    busy: bool,
    empty_terminal_pending: bool,
    pending_empty_session: Option<(String, String)>,
    pending_draft_chats: BTreeMap<String, String>,
    browser_requests_pending: bool,
    browser_requests_busy: bool,
    browser_requests_seen: HashSet<String>,
    refreshing: bool,
    refresh_requested: bool,
    _changes: Option<Task<()>>,
    browser: Option<Entity<BrowserPane>>,
    closing: bool,
    native_settings: Option<settings::NativeSettings>,
    draft_panes: Rc<RefCell<HashMap<String, Entity<draft_creation::DraftCreationPane>>>>,
    draft_subscriptions: HashMap<String, Subscription>,
    draft_creation: Option<Entity<draft_creation::DraftCreationPane>>,
    draft_creation_open: bool,
    applied_style: Option<String>,
    report_theme_publisher: report_theme::Publisher,
    applied_window_surface: Option<String>,
    appearance_reference_pending: bool,
    workspace: Option<Entity<WorkspacePane>>,
    archives: Option<Entity<archives::ArchivedChatsPane>>,
    archives_subscription: Option<Subscription>,
    archives_revision: Option<String>,
    mail: Option<Entity<MailPane>>,
    mail_context_task: Option<String>,
    collapsed_agents: HashSet<String>,
    native_theme_preset: String,
    channels: Option<Entity<ChannelPane>>,
    channels_revision: Option<String>,
    channel_subscription: Option<Subscription>,
    channel_cursors: HashMap<String, Option<String>>,
    terminal: Option<Entity<terminal::TerminalPane>>,
    terminal_context: Option<(Option<String>, Option<String>)>,
    terminal_panes: HashMap<String, Entity<terminal::TerminalPane>>,
    neovim_sessions: HashMap<String, Entity<neovim::NeovimSession>>,
    neovim_panes: HashMap<String, Entity<neovim::NeovimPane>>,
    neovim_layouts: HashMap<String, Value>,
    neovim_origins: HashMap<String, String>,
    neovim_focus_windows: HashMap<String, i64>,
    neovim_pending: bool,
    usage: Option<Entity<usage::UsagePane>>,
    usage_revision: Option<String>,
    app_resources: Option<Entity<usage::AppResourcePane>>,
    sidebar_usage: Option<Entity<sidebar_usage::SidebarUsageWidget>>,
    workspace_usage_rings: Option<Entity<sidebar_usage::WorkspaceUsageRings>>,
    notification_bell: Option<Entity<notifications::NotificationBell>>,
    notification_model: notification_model::NotificationModel,
    notification_subscription: Option<Subscription>,
    workspace_notifications: HashMap<String, notification_center::WorkspaceNotificationFeed>,
    admin_activity: Option<Entity<admin_activity::AdminActivity>>,
    admin_activity_subscription: Option<Subscription>,
    sidebar_usage_snapshot: Option<Rc<Value>>,
    sidebar_usage_revision: Option<String>,
    schedules: Option<Entity<schedules::SchedulePane>>,
    schedules_revision: Option<String>,
    agent_editor: Option<Entity<agents::AgentHostPane>>,
    agent_editor_revision: Option<String>,
    subagents: Option<Entity<collaboration::SubagentPane>>,
    subagents_context: Option<(Option<String>, Option<String>)>,
    git: Option<Entity<git::GitPane>>,
    git_panes: HashMap<String, Entity<git::GitPane>>,
    git_panes_revision: Option<String>,
    git_context: Option<(Option<String>, Option<String>)>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Agents,
    Projects,
    Activity,
}
impl Desktop {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A small bounded local read is the only data dependency of the first
        // workspace frame. It never seeds NativeState or a journal cursor.
        #[cfg(not(test))]
        let cached = monitter_gpui_desktop::startup_cache::load();
        #[cfg(test)]
        let cached: Option<monitter_gpui_desktop::startup_cache::Presentation> = None;
        cx.observe_global::<streaming_display::Selection>(|_, cx| cx.notify()).detach();
        let session_lock = cx.new(|cx| session_lock::SessionLock::new(window,cx));
        cx.observe(&session_lock, |_,_,cx|cx.notify()).detach();
        cx.subscribe_in(&session_lock,window,|view,_,event:&session_lock::LockEvent,window,cx| {
            match event {
                session_lock::LockEvent::Locked => {
                    view.lock_close_confirmed=false;
                    view.lock_previous_focus=if view.preferences_open {view.preferences_previous_focus.clone()}else{window.focused(cx)};
                    native_pane_highlight::clear(window);
                    if view.preferences_open {view.close_preferences(window,cx);}
                    cx.stop_active_drag(window);
                    window.close_dialog(cx);
                    view.dock.update(cx,|dock,cx|dock.set_shown(false,window,cx));
                    if let Some(browser)=&view.browser {browser.update(cx,|browser,cx|browser.hide(cx));}
                }
                session_lock::LockEvent::Unlocked => {
                    view.dock.update(cx,|dock,cx|dock.set_shown(true,window,cx));
                    if let Some(focus)=view.lock_previous_focus.take() {window.focus(&focus,cx);}
                }
            }
            cx.notify();
        }).detach();
        let chat_panes = Rc::new(RefCell::new(
            HashMap::<String, Entity<chat::ChatPane>>::new(),
        ));
        let draft_panes = Rc::new(RefCell::new(HashMap::<String, Entity<draft_creation::DraftCreationPane>>::new()));
        let retained_drafts = draft_panes.clone();
        let retained = chat_panes.clone();
        let factory: docking::PaneFactory = Rc::new(move |id, _, cx| {
            if id.starts_with("draft:") {
                let pane = retained_drafts.borrow().get(id)?.clone();
                return Some(docking::PaneSpec { id: id.into(), title: "New Chat".into(), content: pane.clone().into(), focus: pane.focus_handle(cx), lifecycle: None, closable: true });
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
        let layout_subscription = cx.subscribe_in(&dock, window, |view, _, event, window, cx| {
            if let docking::WorkspaceEvent::VisibilityChanged { id, visible } = event {
                if let Some(task_id) = id.strip_prefix("chat:") {
                    let pane = view.chat_panes.borrow().get(task_id).cloned();
                    if let Some(pane) = pane {
                        pane.update(cx, |pane, cx| {
                            pane.set_dock_visible(*visible, cx);
                            if *visible { pane.ensure_latest_history(cx); }
                        });
                    }
                    if !visible { view.compact_inactive_task(task_id, cx); }
                }
            }
            if let docking::WorkspaceEvent::Removed { id } = event {
                view.git_panes.remove(id);
                view.release_removed_neovim_pane(id, cx);
                if browser_workspace::parse_browser_pane_id(id).is_some() {
                    let removed = HashSet::from([id.clone()]);
                    if let Some(set) = &mut view.named_workspaces {
                        for workspace in &mut set.items {
                            if let Some(layout) = &mut workspace.layout {
                                match workspace_visibility::prune_native_pane_ids(layout, &removed) {
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
                    if view.workspaces_dirty { view.queue_preferences_save(cx); }
                }
                if id.starts_with("draft:") {
                    view.draft_panes.borrow_mut().remove(id);
                    view.draft_subscriptions.remove(id);
                    if view.dock_current.as_deref() == Some(id.as_str()) { view.draft_creation = None; view.dock_current = None; }
                }
                if let Some(task_id) = id.strip_prefix("chat:") {
                    // Persisted draft/sidebar state lives separately. Release the
                    // closed pane's transcript, row entities, and animation state.
                    view.chat_panes.borrow_mut().remove(task_id);
                    view.chat_subscriptions.remove(task_id);
                    if view.selected.as_deref() == Some(task_id) && view.dock_current.as_deref() == Some(id.as_str()) {
                        view.selected = None;
                        view.dock_current = None;
                    }
                    view.queue_native_publication(PublicationChanges {
                        trim_tasks: HashSet::from([task_id.to_owned()]),
                        ..Default::default()
                    }, cx);
                }
            }
            if let docking::WorkspaceEvent::EmptyClose { pane_id } = event {
                if view.dock.update(cx, |dock, cx| dock.close_empty(pane_id, window, cx)) {
                    view.queue_preferences_save(cx);
                }
                return;
            }
            if let docking::WorkspaceEvent::EmptyAction { pane_id, action } = event {
                if let Err(error) = view.empty_pane_action(pane_id, *action, window, cx) {
                    view.error = Some(error);
                    cx.notify();
                }
                return;
            }
            if let docking::WorkspaceEvent::EmptyWorkspaceAction(action) = event {
                if view.closing || view.busy || view.empty_terminal_pending
                    || view.service.is_none() || view.preferences.is_none()
                    || view.preferences_restore || view.preferences_restoring {
                    return;
                }
                let result = view.dock.update(cx, |dock, cx| dock.add_focused_empty_tab(window, cx))
                    .and_then(|id| view.empty_pane_action(&id, *action, window, cx));
                if let Err(error) = result {
                    view.error = Some(error);
                    cx.notify();
                }
                return;
            }
            if let docking::WorkspaceEvent::EmptySession { pane_id, task_id } = event {
                let valid = rows(&view.snapshot, "tasks").iter().any(|task| field(task, "id") == *task_id && task["archived"] != true);
                if valid && !view.closing && !view.busy && view.service.is_some()
                    && view.dock.read(cx).panel_key(pane_id).is_some() {
                    view.pending_empty_session = Some((pane_id.clone(), task_id.clone()));
                    view.selected = Some(task_id.clone());
                    view.sidebar = Sidebar::Chats;
                    view.draft_creation_open = false;
                    view.dock_current = None;
                    cx.notify();
                }
                return;
            }
            if matches!(event, docking::WorkspaceEvent::LayoutChanged)
                && !view.closing
                && view.preferences.is_some()
                && !view.preferences_restore
                && !view.preferences_restore_suppressed
            {
                view.workspaces_dirty = true;
                view.queue_preferences_save(cx);
            }
            if let docking::WorkspaceEvent::AutoSizePanesChanged(_) = event {
                cx.notify();
                if !view.closing && view.preferences.is_some() && !view.preferences_restore_suppressed {
                    view.workspaces_dirty = true;
                    view.queue_preferences_save(cx);
                }
            }
        });
        let vim_command_bar = vim_command::VimCommandBar::new(window, cx);
        cx.subscribe_in(&vim_command_bar, window, |desktop, _, event, window, cx| {
            if let vim_command::VimCommandBarEvent::Run(command) = event {
                desktop.run_vim_command(command.clone(), window, cx);
            }
        }).detach();
        let wallpaper_backdrop = cx.new(|_| wallpaper_backdrop::WallpaperBackdrop::default());
        cx.observe(&wallpaper_backdrop, |_, _, cx| cx.notify()).detach();
        let wallpaper_accent = cx.new(|_| wallpaper_accent::WallpaperAccent::new());
        cx.observe(&wallpaper_accent, |view, _, cx| { view.applied_style = None; view.invalidate_surface_blends(cx); }).detach();
        if wallpaper_theme::mode() != wallpaper_theme::Mode::Manual {
            wallpaper_accent.update(cx, |sampler, cx| sampler.refresh(window, cx));
        }
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(30)).await;
                if view.update_in(cx, |view, window, cx| view.refresh_wallpaper(window, cx)).is_err() { break; }
            }
        }).detach();
        cx.observe_window_bounds(window, |view, window, cx| { window_state::save(window, cx); view.refresh_wallpaper(window, cx); }).detach();
        let activation_subscription = cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() { view.refresh_wallpaper(window, cx); }
            if !window.is_window_active() {
                view.set_workspace_number_hints(false, cx);
                cx.stop_active_drag(window);
            }
        });
        let sidebar_resize = cx.new(|_| ResizableState::default());
        let sidebar_resize_subscription =
            cx.subscribe(&sidebar_resize, |view, state, event, cx| {
                if matches!(event, ResizablePanelEvent::Resized) {
                    if let Some(width) = state.read(cx).sizes().first() {
                        let width = *width / design::ui_px(1., view.sidebar_scale);
                        if width.is_finite()
                            && !view.sidebar_resize_suppressed
                            && !view.sidebar_is_hidden()
                            && !view.preferences_restore_suppressed
                        {
                            view.sidebar_width_touched = true;
                            let collapse = sidebar_should_collapse(
                                width,
                                view.sidebar_collapsed,
                                view.sidebar_scale,
                            );
                            if collapse != view.sidebar_collapsed {
                                if collapse { view.sidebar_expanded_width = expanded_sidebar_width_for_scale(view.sidebar_expanded_width,view.sidebar_scale); }
                                view.sidebar_collapsed = collapse;
                            }
                            // Drag takeover can clear a Hidden target without
                            // changing the collapse flag. Commit the resolved
                            // visible mode on release even when that flag agrees.
                            preferences_modal::save_sidebar_collapse(collapse);
                            if !collapse { view.sidebar_expanded_width = expanded_sidebar_width_for_scale(width,view.sidebar_scale); }
                            view.queue_preferences_save(cx);
                        }
                    }
                }
            });
        // Headless UI fixtures must not connect to the user's live backend.
        #[cfg(not(test))]
        {
        let open = cx
            .background_executor()
            .spawn(async {
                backend_launcher::connect().map(|service| {
                    let subscription = service.subscribe_changes();
                    (service, subscription)
                })
            });
        cx.spawn(async move |view, cx| {
            let result = open.await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok((service, subscription)) => {
                        match subscription {
                            Ok(receiver) => {
                                view._changes = Some(cx.spawn(async move |view, cx| {
                                    while receiver.recv().await.is_ok() {
                                        if view.update(cx, |view, cx| { view.browser_requests_pending = true; view.refresh(cx); cx.notify(); }).is_err() {
                                            break;
                                        }
                                    }
                                }));
                            }
                            Err(error) => view.error = Some(error),
                        }
                        if let Some(activity) = &view.admin_activity {
                            activity.update(cx, |activity, cx| activity.connect(service.clone(), cx));
                        }
                        view.service = Some(service);
                        view.load_preferences(cx);
                        view.refresh(cx);
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        }
        let cached_theme = if cached.as_ref().is_some_and(|cache| cache.preferences.data.appearance.warm_paper) { "reference" } else { "monitter" };
        let cached_route = cached.as_ref().and_then(|cache| cache.preferences.data.appearance.last_route.as_ref())
            .and_then(|route| Self::palette_routes().into_iter().find(|(id, _, _)| id == route).map(|(_, _, route)| route))
            .unwrap_or(Sidebar::Chats);
        Self {
            dock,
            sidebar_resize,
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
            palette: None,
            palette_subscription: None,
            palette_terminal_sessions: vec![],
            palette_terminals_loading: false,
            _layout_subscription: layout_subscription,
            _activation_subscription: activation_subscription,
            preferences: cached.as_ref().map(|cache| cache.preferences.clone()),
            named_workspaces: cached.as_ref().and_then(|cache| cache.workspaces.clone()),
            workspace_number_hints: false,
            workspaces_dirty: false,
            workspace_store_revision: 0,
            pending_auto_size_panes: None,
            workspace_switch: None,
            pending_sidebar_task: None,
            pending_notification_read: None,
            sidebar_task_navigation_scheduled: false,
            pending_workspace_removal: None,
            workspace_editor: None,
            native_drafts: BTreeMap::new(),
            dirty_draft_ids: HashSet::new(),
            native_attachment_drafts: BTreeMap::new(),
            dirty_attachment_draft_ids: HashSet::new(),
            native_browser_metadata: None,
            native_browser_dirty: false,
            browser_restore_revision: None,
            browser_subscription: None,
            native_chat_panes: BTreeMap::new(),
            dirty_chat_pane_ids: HashSet::new(),
            route_touched: false,
            preferences_restore: false,
            preferences_restore_suppressed: false,
            preferences_restoring: false,
            preferences_generation: 0,
            preferences_restore_generation: 0,
            sidebar_resize_suppressed: false,
            sidebar_resize_generation: 0,
            preferences_blocked: cached.is_some(),
            preferences_dirty: false,
            preferences_busy: false,
            preferences_error: None,
            preferences_save_timer: None,
            close_pending: false,
            chat_panes,
            chat_subscriptions: HashMap::new(),
            dock_current: None,
            service: None,
            snapshot: Rc::new(cached.as_ref().map(|cache| cache.snapshot.clone()).unwrap_or_else(|| json!({}))),
            revision: None,
            native_state: NativeState::default(),
            publication: PublicationQueue::default(),
            publication_index: None,
            publication_error: None,
            notification_projection: None,
            empty_sessions: Default::default(),
            collaboration_records: HashMap::new(),
            collaboration_transcripts: HashMap::new(),
            full_reset_requested: false,
            startup_cache: cached,
            startup_preview: None,
            startup_pending: cfg!(not(test)),
            startup_first_frame: cfg!(not(test)),
            startup_live_frame_pending: false,
            startup_cache_timer: None,
            wallpaper_accent,
            wallpaper_backdrop,
            session_lock,
            lock_previous_focus: None,
            lock_close_saving: false,
            lock_close_confirmed: false,
            hidden_task_ids: HashSet::new(),
            removed_dock_ids: Vec::new(),
            selected: None,
            sidebar: cached_route,
            filter: None,
            error: None,
            busy: false,
            empty_terminal_pending: false,
            pending_empty_session: None,
            pending_draft_chats: BTreeMap::new(),
            browser_requests_pending: true,
            browser_requests_busy: false,
            browser_requests_seen: HashSet::new(),
            refreshing: false,
            refresh_requested: false,
            _changes: None,
            browser: None,
            closing: false,
            native_settings: None,
            draft_panes,
            draft_subscriptions: HashMap::new(),
            draft_creation: None,
            draft_creation_open: false,
            applied_style: None,
            report_theme_publisher: report_theme::Publisher::default(),
            applied_window_surface: None,
            appearance_reference_pending: std::env::var_os("MONITTER_GPUI_APPEARANCE_REFERENCE")
                .is_some(),
            workspace: None,
            archives: None,
            archives_subscription: None,
            archives_revision: None,
            mail: None,
            mail_context_task: None,
            collapsed_agents: HashSet::new(),
            native_theme_preset: cached_theme.into(),
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
            notification_model: Default::default(),
            notification_subscription: None,
            workspace_notifications: HashMap::new(),
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
    fn schedule_startup_cache(&mut self, cx: &mut Context<Self>) {
        if self.startup_pending || self.startup_cache_timer.is_some() || self.closing { return; }
        self.startup_cache_timer = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
            let cache = view.update(cx, |view, cx| {
                view.startup_cache_timer = None;
                if view.preferences_busy || view.preferences_restore || view.preferences_restoring || view.closing { return None; }
                let mut preferences = view.preferences.clone()?;
                preferences.data.chat_panes = view.native_chat_panes.clone();
                preferences.data.appearance.sidebar_width = Some(view.current_sidebar_width(cx));
                let mut layout = view.dock.read(cx).serialize(cx).ok()?;
                workspace_visibility::remember_focus(&mut layout, view.dock_current.as_deref());
                let ids = docking::DockWorkspace::saved_ids(&layout).ok()?;
                Some(monitter_gpui_desktop::startup_cache::Presentation::capture(&view.snapshot, &preferences, view.named_workspaces.clone(), layout, &ids))
            }).ok().flatten();
            if let Some(cache) = cache {
                // Clearing startup_cache_timer above drops this task's own
                // handle. The file write must survive that cancellation.
                cx.background_executor().spawn(async move {
                    if let Err(error) = monitter_gpui_desktop::startup_cache::save(&cache) {
                        eprintln!("[native-startup] cache_write={error}");
                    }
                }).detach();
            }
        }));
    }
    fn finish_startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_sidebar_presentation(window, cx);
        if !self.startup_pending { return; }
        self.startup_pending = false;
        self.startup_cache = None;
        self.startup_preview = None;
        self.startup_live_frame_pending = true;
        self.schedule_startup_cache(cx);
    }
    fn deactivate_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            mail.update(cx, |mail, cx| mail.deactivate(window, cx));
        }
    }
    fn switch_surface(&mut self, surface: Sidebar, window: &mut Window, cx: &mut Context<Self>) {
        if session_lock::locked(cx) {return;}
        if surface == Sidebar::Settings { self.show_preferences(window, cx); return; }
        if surface == Sidebar::Subagents {
            let focused = self.dock.read(cx).focused_pane_id(window, cx);
            let pane = focused
                .as_deref()
                .and_then(|id| id.strip_prefix("chat:"))
                .and_then(|id| self.chat_panes.borrow().get(id).cloned());
            if let Some(pane) = pane {
                pane.update(cx, |pane, cx| pane.toggle_companion(cx));
            } else {
                self.error = Some("Focus a chat pane to toggle its secondary sidebar.".into());
                cx.notify();
            }
            return;
        }
        self.route_touched = true;
        self.draft_creation_open = false;
        if surface == Sidebar::Archived && self.sidebar != Sidebar::Archived {
            self.ensure_archives(window, cx);
            if let Some(pane) = &self.archives {
                pane.update(cx, |pane, cx| pane.activate(window, cx));
            }
        }
        if self.sidebar == Sidebar::Mail && surface != Sidebar::Mail {
            self.deactivate_mail(window, cx);
        }
        if surface == Sidebar::Browser {
            self.ensure_browser(window, cx);
            self.sidebar = surface;
            if self.preferences.is_some()
                && !self.preferences_restore
                && !self.preferences_restoring
            {
                if let Err(error) = self.open_selected_browser(window, cx) {
                    self.error = Some(error);
                }
            }
            cx.notify();
            return;
        }
        self.dock_current = None;
        self.dock.update(cx, |dock, cx| {
            dock.set_shown(surface != Sidebar::Settings, window, cx)
        });
        self.sidebar = surface;
        self.filter = None;
        cx.notify();
    }
    // A retained browser owns native views; preferences contain only bounded
    // tab metadata. Re-creation restores unloaded tabs and never loads a page.
    fn ensure_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.browser.is_none() {
            let browser = cx.new(|cx| {
                let mut browser = BrowserPane::new(window, cx);
                browser.set_docked_mode(true, cx);
                browser
            });
            self.browser_restore_revision = Some(browser.read(cx).metadata_revision());
            self.browser_subscription =
                Some(
                    cx.subscribe_in(&browser, window, |view, _, event, window, cx| match event {
                        browser::BrowserPaneEvent::MetadataChanged(metadata) => {
                            for tab in &metadata.tabs {
                                let id = browser_workspace::browser_pane_id(tab.id);
                                let title = if tab.title.is_empty() {
                                    "Browser"
                                } else {
                                    &tab.title
                                };
                                view.dock.update(cx, |dock, cx| {
                                    dock.set_title(&id, title.to_owned().into(), cx)
                                });
                            }
                            if view.native_browser_metadata.as_ref() != Some(metadata) {
                                view.native_browser_metadata = Some(metadata.clone());
                                view.native_browser_dirty = true;
                                view.queue_preferences_save(cx);
                            }
                        }
                        browser::BrowserPaneEvent::OpenInNewPane { source_id, url } => {
                            if view.closing || !view.browser.as_ref().is_some_and(|manager| manager.read(cx).tab_entity(*source_id).is_some()) {
                                return;
                            }
                            let origin = browser_workspace::browser_pane_id(*source_id);
                            if let Err(error) = view.open_browser_target(url, None, Some(&origin), window, cx) {
                                view.error = Some(error);
                                cx.notify();
                            }
                        }
                        browser::BrowserPaneEvent::WorkspaceAction { source_id, action } => {
                            if view.closing {
                                return;
                            }
                            if let Err(error) =
                                view.browser_workspace_action(*source_id, *action, window, cx)
                            {
                                view.error = Some(error);
                                cx.notify();
                            }
                        }
                    }),
                );
            self.browser = Some(browser);
        }
        self.restore_browser_metadata(window, cx);
    }
    fn restore_browser_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preferences.is_none() {
            return;
        }
        let Some(expected_revision) = self.browser_restore_revision.take() else {
            return;
        };
        // This projection already preserves dirty local metadata during reload.
        // A newly created pane must also hydrate it before its save settles;
        // the entity revision guard rejects overwriting a pane the user edited.
        let (Some(browser), Some(metadata)) = (&self.browser, &self.native_browser_metadata) else {
            return;
        };
        let restored = browser.update(cx, |browser, cx| {
            browser.restore_metadata(metadata.clone(), expected_revision, window, cx)
        });
        if let Err(error) = restored {
            self.preferences_error = Some(error);
            cx.notify();
        }
    }
    fn browser_spec(&self, id: u64, cx: &mut Context<Self>) -> Result<docking::PaneSpec, String> {
        let manager = self.browser.clone().ok_or("Browser is unavailable.")?;
        let tab = manager
            .read(cx)
            .tab_entity(id)
            .ok_or("Browser tab is unavailable.")?;
        let key = browser_workspace::browser_pane_id(id);
        let title = manager
            .read(cx)
            .metadata_snapshot(cx)
            .tabs
            .into_iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.title)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Browser".into());
        let weak = cx.weak_entity();
        let native = tab.clone();
        let active_key = key.clone();
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
            match event {
                docking::PaneLifecycle::Visible(true) => native.update(cx, |tab, cx| tab.show(cx)),
                docking::PaneLifecycle::Visible(false) => native.update(cx, |tab, cx| tab.hide(cx)),
                docking::PaneLifecycle::Active(true) => {
                    let weak = weak.clone();
                    let key = active_key.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |view, cx| {
                            if view.closing {
                                return;
                            }
                            if let Some(manager) = &view.browser {
                                if manager.read(cx).tab_entity(id).is_none() {
                                    return;
                                }
                                manager.update(cx, |manager, cx| manager.select_docked(id, cx));
                            } else {
                                return;
                            }
                            view.sidebar = Sidebar::Browser;
                            view.dock_current = Some(key);
                            cx.notify();
                        });
                    });
                }
                docking::PaneLifecycle::Removed => {
                    // Hide happened above. Finish release outside the dock's
                    // mutation, while the window and prior frame are still alive.
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |view, cx| {
                            if view.closing {
                                return;
                            }
                            if let Some(manager) = &view.browser {
                                manager.update(cx, |manager, cx| manager.remove_docked(id, cx));
                                if manager.read(cx).selected_tab_id().is_none()
                                    && view.sidebar == Sidebar::Browser
                                {
                                    view.sidebar = Sidebar::Chats;
                                    view.dock_current = None;
                                }
                            }
                            cx.notify();
                        });
                    });
                }
                _ => {}
            }
        });
        Ok(docking::PaneSpec {
            id: key,
            title: title.into(),
            content: tab.clone().into(),
            focus: tab.focus_handle(cx),
            lifecycle: Some(lifecycle),
            closable: true,
        })
    }
    fn open_browser_tab(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // Hydrate saved browser entities before any dock spec captures them.
        // Otherwise a late startup restore can replace a still-hosted blank tab.
        if self.preferences.is_none() || self.preferences_restore || self.preferences_restoring {
            return Err("Browser workspace preferences are still loading.".into());
        }
        let key = browser_workspace::browser_pane_id(id);
        if !self
            .dock
            .update(cx, |dock, cx| dock.focus(&key, window, cx))
        {
            let layout = self.dock.read(cx).serialize(cx)?;
            if docking::DockWorkspace::saved_ids(&layout)?.len() >= 64 {
                return Err("Native workspace supports at most 64 panes.".into());
            }
            let group = self.dock.read(cx).focused_group_node(window, cx);
            let spec = self.browser_spec(id, cx)?;
            self.dock.update(cx, |dock, cx| {
                dock.add(
                    spec,
                    gpui_kit::component::dock::DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
            })?;
            self.dock.update(cx, |dock, cx| {
                if let Some(group) = group {
                    dock.move_pane(
                        &key,
                        gpui_kit::component::dock::InsertTarget::Tabs {
                            node: group,
                            ix: None,
                            activate: true,
                        },
                        window,
                        cx,
                    );
                }
                dock.focus(&key, window, cx);
            });
        }
        self.sidebar = Sidebar::Browser;
        self.dock_current = Some(key);
        self.dock
            .update(cx, |dock, cx| dock.set_shown(true, window, cx));
        Ok(())
    }
    fn open_selected_browser(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let manager = self.browser.clone().ok_or("Browser is unavailable.")?;
        let id = if let Some(id) = manager.read(cx).selected_tab_id() {
            id
        } else {
            self.create_browser_tab(window, cx)?
        };
        self.open_browser_tab(id, window, cx)
    }
    fn create_browser_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        if self.preferences.is_none() || self.preferences_restore || self.preferences_restoring {
            return Err("Browser workspace preferences are still loading.".into());
        }
        let layout = self.dock.read(cx).serialize(cx)?;
        if docking::DockWorkspace::saved_ids(&layout)?.len() >= 64 {
            return Err("Native workspace supports at most 64 panes.".into());
        }
        self.browser
            .as_ref()
            .ok_or("Browser is unavailable.")?
            .update(cx, |manager, cx| manager.add_docked(window, cx))
    }
    fn process_browser_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.startup_pending { return; }
        if !self.browser_requests_pending || self.browser_requests_busy || self.closing
            || self.preferences.is_none() || self.preferences_restore || self.preferences_restoring
            || self.session_lock.read(cx).is_locked() {
            return;
        }
        let Some(service) = self.service.clone() else { return; };
        self.browser_requests_pending = false;
        self.browser_requests_busy = true;
        let reader = service.clone();
        let worker = cx.background_executor().spawn(async move { reader.read_browser_open_requests() });
        cx.spawn_in(window, async move |view, cx| {
            let requests = worker.await;
            let mut acknowledged = Vec::new();
            let _ = view.update_in(cx, |view, window, cx| {
                view.browser_requests_busy = false;
                if view.closing || view.session_lock.read(cx).is_locked()
                    || view.preferences.is_none() || view.preferences_restore || view.preferences_restoring {
                    view.browser_requests_pending = true;
                    return;
                }
                match requests {
                    Ok(requests) => {
                        view.browser_requests_seen.retain(|id| requests.iter().any(|request| &request.id == id));
                        for request in requests {
                            if !view.browser_requests_seen.contains(&request.id) {
                                if let Err(error) = view.open_browser_target(&request.url, request.title.as_deref(), Some(&format!("chat:{}", request.task_id)), window, cx) {
                                    view.error = Some(format!("Could not open agent browser link: {error}"));
                                }
                                // Retain receipt locally until the broker stops returning it.
                                // Lost acknowledgements must not create duplicate visible tabs.
                                view.browser_requests_seen.insert(request.id.clone());
                            }
                            acknowledged.push(request.id);
                        }
                    }
                    Err(error) => {
                        // Older installed backends do not implement this private method.
                        if !error.contains("Unknown") && !error.contains("not supported") && !error.contains("not available") {
                            view.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
            if !acknowledged.is_empty() {
                let ack = cx.background_executor().spawn(async move {
                    for id in acknowledged { service.ack_browser_open_request(&id)?; }
                    Ok::<(), String>(())
                });
                if let Err(error) = ack.await {
                    let _ = view.update_in(cx, |view, _, cx| {
                        view.error = Some(error);
                        cx.notify();
                    });
                }
            }
        }).detach();
    }

    fn open_chat_browser_link(&mut self, task_id: &str, url: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        if url.starts_with('/') || url::Url::parse(url).is_ok_and(|url| url.scheme() == "file") {
            let host_id = self.snapshot["tasks"].as_array().and_then(|tasks| tasks.iter().find(|task| task["id"] == task_id)).and_then(|task| task["hostId"].as_str());
            let local = host_id.is_some_and(|id| self.snapshot["hosts"].as_array().is_some_and(|hosts| hosts.iter().any(|host| host["id"] == id && host["kind"] == "local")));
            if !local { return Err("This file link belongs to a remote task. Ask the agent for an HTTP(S) preview URL.".into()); }
        }
        self.open_browser_target(url, None, Some(&format!("chat:{task_id}")), window, cx)
    }

    fn open_browser_link(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        self.open_browser_target(target, None, None, window, cx)
    }

    fn open_browser_target(&mut self, target: &str, title: Option<&str>, origin: Option<&str>, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let url = monitter_lib::browser_links::normalize_target(target)?;
        // Capture the source group before creating/focusing a native browser.
        let group = origin.and_then(|id| self.dock.read(cx).group_for_pane(id, cx))
            .or_else(|| self.dock.read(cx).focused_group_node(window, cx));
        self.ensure_browser(window, cx);
        let id = self.create_browser_tab(window, cx)?;
        let manager = self.browser.clone().ok_or("Browser is unavailable.")?;
        let key = browser_workspace::browser_pane_id(id);
        let result = self.browser_spec(id, cx).and_then(|spec| {
            self.dock.update(cx, |dock, cx| dock.add_browser_beside(spec, group, window, cx))
        }).and_then(|()| {
            manager.read(cx).tab_entity(id).ok_or("Browser tab is unavailable.")?
                .update(cx, |tab, cx| tab.open_url(&url, title, cx))
        });
        if result.is_err() {
            self.dock.update(cx, |dock, cx| dock.remove(&key, window, cx));
            manager.update(cx, |manager, cx| manager.remove_docked(id, cx));
        } else {
            self.sidebar = Sidebar::Browser;
            self.dock_current = Some(key);
            self.dock.update(cx, |dock, cx| dock.set_shown(true, window, cx));
        }
        result
    }

    fn browser_workspace_action(
        &mut self,
        source_id: u64,
        action: browser::BrowserWorkspaceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let manager = self.browser.clone().ok_or("Browser is unavailable.")?;
        if manager.read(cx).tab_entity(source_id).is_none() {
            return Ok(());
        }
        match action {
            browser::BrowserWorkspaceAction::NewTab => {
                if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
                    let key = browser_workspace::browser_pane_id(source_id);
                    self.dock.update(cx, |dock, cx| dock.focus(&key, window, cx));
                    return self.open_focused_empty_tab(window, cx);
                }
                let id = self.create_browser_tab(window, cx)?;
                if let Err(error) = self.open_browser_tab(id, window, cx) {
                    manager.update(cx, |manager, cx| manager.remove_docked(id, cx));
                    return Err(error);
                }
            }
            browser::BrowserWorkspaceAction::CloseTab => {
                let key = browser_workspace::browser_pane_id(source_id);
                self.dock.update(cx, |dock, cx| {
                    dock.remove(&key, window, cx);
                });
            }
            browser::BrowserWorkspaceAction::ToggleZoom => {
                let key = browser_workspace::browser_pane_id(source_id);
                let Some(group) = self.dock.read(cx).group_for_pane(&key, cx) else {
                    return Ok(());
                };
                let area = self.dock.read(cx).area();
                if area.read(cx).zoomed_group() == Some(group) {
                    area.update(cx, |area, cx| area.set_zoomed_out(window, cx));
                } else {
                    area.update(cx, |area, cx| area.set_zoomed_in(group, window, cx));
                }
            }
            browser::BrowserWorkspaceAction::NextTab
            | browser::BrowserWorkspaceAction::PreviousTab => {
                let forward = matches!(action, browser::BrowserWorkspaceAction::NextTab);
                self.dock
                    .update(cx, |dock, cx| dock.cycle_focused_tab(forward, window, cx))?;
            }
        }
        Ok(())
    }
    fn empty_pane_action(
        &mut self,
        pane_id: &str,
        action: empty_pane::EmptyPaneAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.closing || !pane_id.starts_with("empty:") {
            return Ok(());
        }
        let Some(expected_panel) = self.dock.read(cx).panel_key(pane_id) else {
            return Ok(());
        };
        let service = self
            .service
            .clone()
            .ok_or("The native backend is not ready.")?;
        if self.preferences.is_none() || self.preferences_restore || self.preferences_restoring {
            return Err("Native workspace preferences are still loading.".into());
        }
        if action != empty_pane::EmptyPaneAction::Browser
            && (self.busy || self.empty_terminal_pending)
        {
            return Ok(());
        }
        self.dock.update(cx, |dock, cx| {
            dock.focus(pane_id, window, cx);
        });
        match action {
            empty_pane::EmptyPaneAction::Neovim => {
                self.open_neovim_pane(Some(pane_id.to_owned()), window, cx)?;
            }
            empty_pane::EmptyPaneAction::Chat => {
                if rows(&self.snapshot, "agents").is_empty() {
                    self.switch_surface(Sidebar::Agents, window, cx);
                    return Ok(());
                }
                self.open_chat_draft(None, window, cx);
                if self.draft_creation_open {
                    self.dock.update(cx, |dock, cx| {
                        dock.remove(pane_id, window, cx);
                    });
                }
            }
            empty_pane::EmptyPaneAction::Browser => {
                self.ensure_browser(window, cx);
                let id = self.create_browser_tab(window, cx)?;
                if let Err(error) = self.open_browser_tab(id, window, cx) {
                    if let Some(browser) = &self.browser {
                        browser.update(cx, |browser, cx| browser.remove_docked(id, cx));
                    }
                    return Err(error);
                }
                self.dock.update(cx, |dock, cx| {
                    dock.remove(pane_id, window, cx);
                });
            }
            empty_pane::EmptyPaneAction::Terminal => {
                let host = rows(&self.snapshot, "hosts")
                    .into_iter()
                    .find(|host| field(host, "kind") == "local")
                    .ok_or("No local host is configured.")?;
                let host_id = field(&host, "id");
                let pane_id = pane_id.to_owned();
                let (sender, receiver) = futures::channel::oneshot::channel();
                std::thread::Builder::new()
                    .name("monitter-empty-terminal".into())
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || {
                        let result = service.invoke(
                            "open_terminal",
                            json!({
                                "target": {"hostId": host_id}, "cols": 80, "rows": 24,
                            }),
                        );
                        let _ = sender.send(result);
                    })
                    .map_err(|error| error.to_string())?;
                self.empty_terminal_pending = true;
                cx.spawn_in(window, async move |view, cx| {
                    let result = receiver.await.map_err(|error| error.to_string())
                        .and_then(|result| result);
                    let _ = view.update_in(cx, |view, window, cx| {
                        view.empty_terminal_pending = false;
                        if view.closing { return; }
                        // An intervening close/recreate/restore must not consume
                        // another empty pane that happens to reuse the same ID.
                        let pane_still_live = {
                            let dock = view.dock.read(cx);
                            dock.panel_key(&pane_id) == Some(expected_panel)
                                && dock.area().read(cx).panel(expected_panel).is_some()
                        };
                        if !pane_still_live {
                            match result {
                                Err(error) => view.error = Some(error),
                                Ok(session) => {
                                    let id = field(&session, "id");
                                    if id.is_empty() {
                                        view.error = Some("Terminal open response omitted its ID after its pane closed.".into());
                                    } else {
                                        // Cancel only this new, never-adopted local
                                        // session after its originating pane closed.
                                        view.close_unclaimed_terminal(id, cx);
                                    }
                                }
                            }
                            cx.notify();
                            return;
                        }
                        match result {
                            Err(error) => view.error = Some(error),
                            Ok(session) => {
                                let adopted = view.open_terminal_pane(session, Some(&pane_id), window, cx);
                                if let Err(error) = adopted { view.error = Some(error); }

                            }
                        }
                        cx.notify();
                    });
                }).detach();
            }
        }
        cx.notify();
        Ok(())
    }
    fn close_ended_terminal_pane(&mut self, session_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Normal removal releases the retained view without killing an
        // already-exited server-owned PTY or any manager session.
        terminal::remove_exited_terminal_pane(&self.dock,session_id,window,cx);
        if let Ok(key)=terminal_workspace::terminal_pane_id(session_id) {
            let removed=HashSet::from([key.clone()]);
            if let Some(workspaces)=&mut self.named_workspaces {
                for workspace in &mut workspaces.items {
                    if let Some(layout)=&mut workspace.layout {
                        match workspace_visibility::prune_native_pane_ids(layout,&removed) {
                            Ok(pruned)=>*layout=pruned,
                            Err(error)=>self.error=Some(error),
                        }
                    }
                }
            }
            if !self.dock.read(cx).contains(&key) {
                self.terminal_panes.remove(session_id);
                if self.dock_current.as_ref()==Some(&key) {self.dock_current=None;self.sidebar=Sidebar::Chats;}
            }
        }
        self.palette_terminal_sessions.retain(|session|field(session,"id")!=session_id);
        self.queue_preferences_save(cx);
        cx.notify();
    }
    fn new_terminal_manager(&mut self, service: DesktopService, window: &mut Window, cx: &mut Context<Self>) -> Entity<terminal::TerminalPane> {
        let pane=cx.new(|cx|terminal::TerminalPane::new(service,&self.snapshot,window,cx));
        cx.subscribe_in(&pane,window,|view,source,event:&terminal::TerminalEvent,window,cx| {
            let terminal::TerminalEvent::SessionClosed {session_id}=event else {return;};
            if view.closing || !view.terminal.as_ref().is_some_and(|manager|manager.entity_id()==source.entity_id()) {return;}
            view.close_ended_terminal_pane(session_id,window,cx);
        }).detach();
        pane
    }
    fn terminal_spec(
        &mut self,
        session: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<docking::PaneSpec, String> {
        let session_id = field(&session, "id");
        let key = terminal_workspace::terminal_pane_id(&session_id)?;
        let validated = terminal::PinnedSession::try_from(session)?;
        let pane = if let Some(pane) = self.terminal_panes.get(&session_id) {
            pane.clone()
        } else {
            let service = self
                .service
                .clone()
                .ok_or("Native service has not loaded.")?;
            let pane = cx.new(|cx| {
                terminal::TerminalPane::new_pinned(service, &self.snapshot, validated, window, cx)
            });
            let expected_id=session_id.clone();
            cx.subscribe_in(&pane,window,move |view,source,event:&terminal::TerminalEvent,window,cx| {
                let terminal::TerminalEvent::Exited {session_id} = event else {return;};
                if view.closing || session_id != &expected_id || !view.terminal_panes.get(session_id).is_some_and(|current|current.entity_id()==source.entity_id()) {return;}
                view.close_ended_terminal_pane(session_id,window,cx);
            }).detach();
            self.terminal_panes.insert(session_id.clone(), pane.clone());
            pane
        };
        let native = pane.downgrade();
        let weak = cx.weak_entity();
        let active_key = key.clone();
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
            if let docking::PaneLifecycle::Visible(visible) = event {
                let _ = native.update(cx, |pane, cx| pane.set_visible(visible, cx));
            }
            if matches!(event, docking::PaneLifecycle::Removed) {
                let _ = native.update(cx, |pane, cx| pane.set_visible(false, cx));
            }
            if matches!(
                event,
                docking::PaneLifecycle::Active(true) | docking::PaneLifecycle::Removed
            ) {
                let weak = weak.clone();
                let session_id = session_id.clone();
                let key = active_key.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if view.closing {
                            return;
                        }
                        if matches!(event, docking::PaneLifecycle::Removed) {
                            // Removing a view never terminates its server-owned PTY.
                            // Keep a replacement with the same identity if one exists.
                            if !view.dock.read(cx).contains(&key) {
                                view.terminal_panes.remove(&session_id);
                                if view.dock_current.as_ref() == Some(&key) {
                                    view.dock_current = None;
                                    view.sidebar = Sidebar::Chats;
                                }
                            }
                        } else if view.dock.read(cx).contains(&key) {
                            view.sidebar = Sidebar::Terminal;
                            view.dock_current = Some(key);
                        }
                        cx.notify();
                    });
                });
            }
        });
        Ok(docking::PaneSpec {
            id: key,
            title: "Terminal".into(),
            content: pane.clone().into(),
            focus: pane.focus_handle(cx),
            lifecycle: Some(lifecycle),
            closable: true,
        })
    }
    fn open_terminal_pane(
        &mut self,
        session: Value,
        replace_empty: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(origin) = replace_empty {
            self.dock.update(cx, |dock, cx| {
                dock.focus(origin, window, cx);
            });
        }
        let group = self.dock.read(cx).focused_group_node(window, cx);
        let spec = self.terminal_spec(session, window, cx)?;
        let key = spec.id.clone();
        self.dock.update(cx, |dock, cx| -> Result<(), String> {
            if !dock.contains(&key) {
                dock.add(spec, gpui_kit::component::dock::DockPlacement::Center, None, window, cx)?;
                if let Some(group) = group {
                    if !dock.move_pane(&key, gpui_kit::component::dock::InsertTarget::Tabs {
                        node: group, ix: None, activate: true,
                    }, window, cx) {
                        return Err("Terminal opened, but could not join its originating tab group. Its session remains available in Terminal.".into());
                    }
                }
            }
            if !dock.focus(&key, window, cx) {
                return Err("Terminal opened, but its pane is unavailable. Its session remains available in Terminal.".into());
            }
            dock.set_shown(true, window, cx);
            if let Some(origin) = replace_empty { dock.remove(origin, window, cx); }
            Ok(())
        })?;
        self.sidebar = Sidebar::Terminal;
        self.dock_current = Some(key);
        cx.notify();
        Ok(())
    }
    fn close_unclaimed_terminal(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else {
            self.error = Some(
                "A terminal opened after its pane closed; reconnect and close it from Terminal."
                    .into(),
            );
            return;
        };
        let (sender, receiver) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("monitter-unclaimed-terminal".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let _ = sender.send(service.invoke("close_terminal", json!({"id": id})));
            });
        if let Err(error) = spawned {
            self.error = Some(format!(
                "Terminal cleanup could not start: {error}. Close it from Terminal."
            ));
            return;
        }
        cx.spawn(async move |view, cx| {
            let result = receiver
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            if let Err(error) = result {
                let _ = view.update(cx, |view, cx| {
                    view.error = Some(format!(
                        "Unclaimed terminal cleanup failed: {error}. Close it from Terminal."
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn prepare_close(&mut self, cx: &mut Context<Self>) -> Option<DesktopService> {
        self.closing = true;
        if let Some(browser) = &self.browser {
            browser.update(cx, |browser, cx| browser.close(cx));
        }
        cx.notify();
        self.service.clone()
    }
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_lock.read(cx).is_locked() && !self.lock_close_confirmed {
            if self.lock_close_saving { return; }
            if self.session_lock.read(cx).is_busy() {
                // Retain Quit while the initial lock write finishes. Its
                // notification redraws the desktop and resumes pending close.
                self.close_pending = true;
                return;
            }
            self.close_pending = false;
            self.lock_close_saving=true;
            let worker=cx.background_executor().spawn(async {session_lock_store::set_locked(true)});
            cx.spawn_in(window,async move|view,cx| {let result=worker.await;let _=view.update_in(cx,|view,window,cx| {
                view.lock_close_saving=false;
                match result {Ok(())=>{view.lock_close_confirmed=true;view.request_close(window,cx);},Err(error)=>view.session_lock.update(cx,|lock,cx|lock.show_error(error,cx))}
            });}).detach();
            return;
        }
        if self.closing {
            return;
        }
        // The startup presentation is inert and has no unsaved input. Quit
        // must not wait for backend connection or try to save cached state.
        if !self.startup_pending {
            self.capture_native_drafts(cx);
            self.preferences_save_timer = None;
            if self.preferences_busy {
                self.close_pending = true;
                cx.notify();
                return;
            }
            if self.preferences_dirty && self.preferences.is_some() {
                if self.preferences_blocked {
                    self.close_pending = false;
                    cx.notify();
                    return;
                }
                self.save_preferences(cx, self.preferences_generation);
                if self.preferences_busy {
                    self.close_pending = true;
                    cx.notify();
                    return;
                }
            }
        }
        self.close_pending = false;
        self.deactivate_mail(window, cx);
        self.dock.update(cx, |dock, cx| dock.close_all(window, cx));
        self.prepare_close(cx);
        // Callback is deferred until the current entity update has ended.
        // Explicitly draw/clear the empty closing surface so no native WebView
        // frame Rc survives into GPUI's window destruction.
        window.on_next_frame(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            // Release the current Window borrow before platform quit observers
            // access it for their final synchronous cleanup.
            cx.defer(|cx| cx.quit());
        });
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        // Backpressure is one read/apply followed by one publication. Continuous
        // backend events coalesce into one next read rather than starving draws.
        if self.refreshing || self.publication.busy {
            self.refresh_requested = true;
            return;
        }
        let Some(service) = self.service.clone() else { return; };
        self.refreshing = true;
        let force_reset = std::mem::take(&mut self.full_reset_requested);
        let mut state = self.native_state.clone();
        let generation = self.publication.generation;
        let revision = if force_reset { None } else { state.revision().map(str::to_owned) };
        let message_task_ids: Vec<String> = self.chat_panes.borrow().iter()
            .filter(|(_, pane)| pane.read(cx).wants_history_publication())
            .map(|(id, _)| id.clone()).collect();
        let task = cx.background_executor().spawn(async move {
            let mut result = service.read_native_ui_changes(revision.as_deref(), &message_task_ids);
            for wait_ms in [150, 450] {
                if !result.as_ref().err().is_some_and(|error| chat::connection_recovery::transient(error)) { break; }
                std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                result = service.read_native_ui_changes(revision.as_deref(), &message_task_ids);
            }
            result.map(|response| {
                let applied = state.apply(response);
                (state, applied)
            })
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.refreshing = false;
                if generation != view.publication.generation {
                    // A history merge/eviction changed the cache while the read
                    // was running. Rebase the next delta on that current cache.
                    view.full_reset_requested |= force_reset;
                    view.refresh_requested = true;
                } else {
                    if result.is_ok() && view.error.as_deref().is_some_and(|error|
                        chat::connection_recovery::transient(error)
                            || error == "Could not reconnect to Monitter. Your agents may still be running; try again shortly.") {
                        view.error = None;
                        cx.notify();
                    }
                    match result {
                        Ok((state, applied)) => match applied {
                            ApplyResult::Unchanged => {
                                if view.publication.resume_failed() { view.start_native_publication(cx); }
                                if force_reset {
                                    view.error = Some("Backend did not provide the requested native state reset.".into());
                                    cx.notify();
                                }
                            }
                            ApplyResult::Gap => {
                                if force_reset {
                                    view.error = Some("Backend did not provide the requested native state reset.".into());
                                    cx.notify();
                                } else {
                                    view.request_full_reset(cx);
                                }
                            }
                            ApplyResult::FullReset(changes) => {
                                if let Some(pane) = &view.subagents { pane.update(cx, |pane, cx| pane.suppress_completion_motion(cx)); }
                                view.native_state = state;
                                view.queue_native_publication(PublicationChanges::from_changes(true, changes), cx);
                            }
                            ApplyResult::Delta(changes) => {
                                if force_reset {
                                    view.error = Some("Backend did not provide the requested native state reset.".into());
                                    cx.notify();
                                } else {
                                    view.native_state = state;
                                    view.queue_native_publication(PublicationChanges::from_changes(false, changes), cx);
                                }
                            }
                        },
                        Err(error) => {
                            if !chat::connection_recovery::transient(&error) { view.error = Some(error); }
                            cx.notify();
                        }
                    }
                }
                if std::mem::take(&mut view.refresh_requested) { view.refresh(cx); }
            });
        }).detach();
    }
    fn request_full_reset(&mut self, cx: &mut Context<Self>) {
        self.full_reset_requested = true;
        self.refresh(cx);
    }
    fn queue_native_publication(&mut self, changes: PublicationChanges, cx: &mut Context<Self>) {
        self.publication.enqueue(changes);
        self.start_native_publication(cx);
    }
    fn start_native_publication(&mut self, cx: &mut Context<Self>) {
        if self.native_state.snapshot().is_none() { return; }
        let Some((generation, mut changes)) = self.publication.start() else { return; };
        let mut state = self.native_state.clone();
        changes.trim_tasks.extend(self.chat_panes.borrow().iter()
            .filter(|(_, pane)| pane.read(cx).can_trim_hidden_history())
            .map(|(id, _)| id.clone()));
        let revision = self.native_state.revision().map(str::to_owned);
        let visible: HashSet<_> = self.chat_panes.borrow().iter()
            .filter(|(_, pane)| pane.read(cx).wants_history_publication()).map(|(id, _)| id.clone()).collect();
        let mut panes: HashSet<_> = self.chat_panes.borrow().keys().cloned().collect();
        if let Some(selected) = &self.selected { panes.insert(selected.clone()); }
        let previous_records = self.collaboration_records.clone();
        let previous_transcripts = self.collaboration_transcripts.clone();
        let previous = self.publication_index.clone();
        let notification_projection = self.notification_projection.clone();
        let mut worker_changes = changes.clone();
        let worker = cx.background_executor().spawn(async move {
            let mut evicted = Vec::new();
            for id in &worker_changes.trim_tasks {
                if state.trim_task_messages(id, 256) {
                    worker_changes.message_tasks.insert(id.clone());
                    evicted.push(id.clone());
                }
            }
            let panes_for_records = panes.clone();
            native_publication::prepare_visible(state.snapshot().unwrap().clone(), panes, visible, &worker_changes, previous.clone())
                .map(|mut prepared| {
                    for id in &panes_for_records {
                        let unchanged = prepared.transcript_context_unchanged_tasks.contains(id);
                        let items = if (!unchanged && prepared.sync_tasks.contains(id)) || !previous_records.contains_key(id) {
                            Arc::new(collaboration::records(&prepared.snapshot, id))
                        } else { previous_records[id].clone() };
                        let transcripts = if (!unchanged && prepared.sync_tasks.contains(id)) || !previous_transcripts.contains_key(id) {
                            collaboration::prepare_child_transcripts_indexed(&prepared.snapshot, &prepared.index, &items, previous_transcripts.get(id))
                        } else { previous_transcripts[id].clone() };
                        prepared.collaboration_records.insert(id.clone(), items);
                        prepared.collaboration_transcripts.insert(id.clone(), transcripts);
                    }
                    let notifications = notification_model::prepare_notifications(&prepared.snapshot,
                        &prepared.index, &prepared.message_updates, &worker_changes, notification_projection.as_deref());
                    let sessions = previous.as_ref().is_none_or(|previous| !prepared.index.picker_inputs_unchanged(previous)).then(|| monitter_gpui_desktop::session_picker::build_entries(
                        prepared.snapshot["tasks"].as_array().map(Vec::as_slice).unwrap_or_default(),
                        prepared.snapshot["agents"].as_array().map(Vec::as_slice).unwrap_or_default(),
                        prepared.snapshot["projects"].as_array().map(Vec::as_slice).unwrap_or_default(), ""));
                    (state, prepared, evicted, notifications, sessions)
                })
        });
        cx.spawn(async move |view, cx| {
            let prepared = worker.await;
            let _ = view.update(cx, |view, cx| {
                if view.publication.finish(generation, changes.clone()) {
                    match prepared {
                        Ok((state, prepared, evicted, notifications, sessions)) => {
                            if view.publication_error.is_some() && view.error.as_ref() == view.publication_error.as_ref() { view.error = None; cx.notify(); }
                            view.publication_error = None;
                            view.native_state = state;
                            let mut changes = changes;
                            for id in evicted {
                                // A page can be accepted while its pane is hidden;
                                // eviction makes its older boundary unknown again.
                                changes.task_page_boundaries.remove(&id);
                                if let Some(pane) = view.chat_panes.borrow().get(&id) {
                                    pane.update(cx, |pane, _| pane.mark_hidden_page_eviction());
                                }
                            }
                            view.commit_native_publication(prepared, revision, changes, notifications, sessions, cx);
                        },
                        Err(error) => {
                            view.publication.fail(changes);
                            view.publication_error = Some(error.clone());
                            view.error = Some(error);
                            cx.notify();
                            // Retain pending refresh intent, but do not turn a
                            // serialization failure into a self-retrying loop.
                            return;
                        }
                    }
                }
                view.start_native_publication(cx);
                if !view.publication.busy && std::mem::take(&mut view.refresh_requested) {
                    view.refresh(cx);
                }
            });
        }).detach();
    }
    fn commit_native_publication(
        &mut self,
        mut prepared: PreparedPublication,
        revision: Option<String>,
        changes: PublicationChanges,
        notifications: notification_model::PreparedNotifications,
        sessions: Option<monitter_gpui_desktop::session_picker::PickerGroups>,
        cx: &mut Context<Self>,
    ) {
        let reset = changes.reset;
        let reset_channels: Vec<_> = changes.reset_channels.iter().cloned().collect();
        let removed_tasks: Vec<_> = changes.removed_tasks.iter().cloned().collect();
        let newly_hidden: Vec<_> = prepared.hidden_tasks.difference(&self.hidden_task_ids).cloned().collect();
        self.hidden_task_ids = prepared.hidden_tasks;
        self.publication_index = Some(prepared.index);
        self.collaboration_records = prepared.collaboration_records;
        self.collaboration_transcripts = prepared.collaboration_transcripts;
        let retired_snapshot = std::mem::replace(&mut self.snapshot, Rc::new(prepared.snapshot));
        let shell_changed = reset || retired_snapshot != self.snapshot;
        if let Some(index) = self.publication_index.as_ref() {
            shell::sync_task_markers(index, reset, cx);
        }
        self.revision = revision;
        self.notification_model.sync_prepared(&self.snapshot, &notifications);
        self.notification_projection = Some(notifications.projection);
        if let Some(sessions) = sessions { self.empty_sessions = sessions; }
        self.sync_notification_bell(cx);
        for pane in self.terminal_panes.values() {
            pane.update(cx, |pane, cx| pane.set_context(&self.snapshot, None, cx));
        }
        for draft in self.draft_panes.borrow().values() {
            draft.update(cx, |draft, cx| {
                draft.set_snapshot(self.snapshot.clone(), cx)
            });
        }
        let close_buttons = self.snapshot["settings"]["showTabCloseButtons"]
            .as_bool()
            .unwrap_or(true);
        self.dock.update(cx, |dock, cx| {
            dock.set_close_buttons_visible(close_buttons, cx);
            dock.set_tab_appearance(&self.snapshot["settings"], self.sidebar_scale, cx);
        });
        // Explicit preview-only import. Preserve all non-appearance configuration
        // and let the existing native command validate and persist the settings.
        if std::mem::take(&mut self.appearance_reference_pending) {
            let reference = std::env::var_os("MONITTER_GPUI_APPEARANCE_REFERENCE")
                .ok_or_else(|| "Appearance reference path is unavailable.".to_string())
                .and_then(|path| std::fs::read(path).map_err(|error| error.to_string()))
                .and_then(|bytes| {
                    if bytes.len() > 64 * 1024 {
                        return Err("Appearance reference exceeds 64 KiB.".to_string());
                    }
                    serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
                });
            match reference {
                Ok(reference) => {
                    let mut settings = self.snapshot["settings"].clone();
                    for key in [
                        "theme",
                        "accent",
                        "interfaceScale",
                        "interfaceFont",
                        "interfaceFontSize",
                        "interfaceFontWeight",
                        "chatFont",
                        "chatFontSize",
                        "chatFontWeight",
                        "chatLineHeight",
                        "terminalFont",
                        "terminalFontSize",
                        "terminalFontWeight",
                        "tintUserMessages",
                        "showActivePaneBorder",
                        "showTabCloseButtons",
                        "autoHideTabs",
                        "tabStyle",
                        "interfaceDensity",
                    ] {
                        if let Some(value) = reference["appearance"].get(key) {
                            settings[key] = value.clone();
                        }
                    }
                    self.command("save_settings", json!({"settings": settings}), cx);
                }
                Err(error) => self.error = Some(format!("Appearance reference: {error}")),
            }
        }

        channels::reconcile_cursors(
            &mut self.channel_cursors,
            &self.snapshot,
            reset,
            &reset_channels,
        );
        // A page accepted after the coalesced reset carries a newer valid
        // boundary. Earlier pages and later backend resets remain invalidated.
        for id in changes.channel_page_boundaries.keys() {
            if let Some(boundary) = changes.channel_boundary(id, self.revision.as_deref(), &self.snapshot) {
                self.channel_cursors.insert(id.clone(), boundary.next_before_id.clone());
            }
        }
        if let Some(channels) = &self.channels {
            let mut changed_channels = changes.changed_channels.clone();
            changed_channels.extend(changes.channel_page_boundaries.keys().cloned());
            channels.update(cx, |channels, cx| {
                channels.sync_scoped(
                    self.snapshot.clone(),
                    self.revision.as_deref().unwrap_or_default(),
                    reset,
                    &reset_channels,
                    &self.channel_cursors,
                    &changed_channels,
                    cx,
                )
            });
            self.channels_revision = self.revision.clone();
        }
        if let Some(palette) = &self.palette {
            if palette.read(cx).is_open() && (["tasks", "agents", "projects", "settings"].iter()
                .any(|key| retired_snapshot[*key] != self.snapshot[*key])
                || retired_snapshot["channels"].as_array().into_iter().flatten().map(|channel| (&channel["id"], &channel["name"], &channel["description"]))
                    .ne(self.snapshot["channels"].as_array().into_iter().flatten().map(|channel| (&channel["id"], &channel["name"], &channel["description"])))) {
                let entries = self.palette_entries(palette.read(cx).mode(), cx);
                let result = palette.update(cx, |palette, cx| palette.set_entries(entries, cx));
                if let Err(error) = result {
                    self.error = Some(error);
                }
            }
        }
        let mut removed_tasks = removed_tasks;
        for id in newly_hidden {
            if !removed_tasks.contains(&id) {
                removed_tasks.push(id);
            }
        }
        for id in removed_tasks {
            // A coalesced reset may remove then recreate an ID. The current
            // projected task list decides whether its pane is still removed.
            if self.snapshot["tasks"].as_array().is_some_and(|tasks| tasks.iter().any(|task| task["id"].as_str() == Some(id.as_str()))) { continue; }
            if self.native_chat_panes.remove(&id).is_some() {
                self.dirty_chat_pane_ids.insert(id.clone());
                self.queue_preferences_save(cx);
            }
            if self.native_drafts.remove(&id).is_some() {
                self.dirty_draft_ids.insert(id.clone());
                self.queue_preferences_save(cx);
            }
            if self.native_attachment_drafts.remove(&id).is_some() {
                self.dirty_attachment_draft_ids.insert(id.clone());
                self.queue_preferences_save(cx);
            }
            self.chat_panes.borrow_mut().remove(&id);
            self.chat_subscriptions.remove(&id);
            self.removed_dock_ids.push(format!("chat:{id}"));
            if self.selected.as_ref() == Some(&id) {
                self.selected = None;
                self.dock_current = None;
            }
        }
        {
            let panes = self.chat_panes.borrow();
            for (id, pane) in panes.iter() {
                if !prepared.sync_tasks.contains(id) {
                    pane.update(cx, |pane, cx| pane.advance_revision(self.snapshot.clone(), self.revision.clone(), cx));
                    continue;
                }
                let history = prepared.histories.remove(id);
                let boundary = changes.task_boundary_index(id, self.revision.as_deref(), &self.snapshot, self.publication_index.as_ref().unwrap());
                pane.update(cx, |pane, cx| {
                    let patches = prepared.body_only_tasks.contains(id).then(|| prepared.message_updates.remove(id).unwrap_or_default());
                    pane.publish_indexed(self.snapshot.clone(), self.revision.clone(), self.collaboration_records[id].clone(), self.collaboration_transcripts[id].clone(), prepared.transcript_context_unchanged_tasks.contains(id), history, reset,
                        boundary.map(|boundary| boundary.next_before_id.clone()), self.publication_index.clone(), patches, prepared.lazy_history_tasks.contains(id), cx);
                    pane.ensure_latest_history(cx);
                });
            }
        }
        self.dock.update(cx, |dock, cx| {
            let previous: HashMap<_, _> = retired_snapshot["tasks"].as_array().into_iter().flatten()
                .filter_map(|task| Some((task["id"].as_str()?, task["title"].as_str()?))).collect();
            for task in self.snapshot["tasks"].as_array().into_iter().flatten() {
                let id = task["id"].as_str().unwrap_or_default();
                let title = task["title"].as_str().unwrap_or_default();
                if previous.get(id).copied() != Some(title) {
                    dock.set_title(&format!("chat:{id}"), title.to_owned().into(), cx);
                }
            }
        });
        if let Some(workspace) = &self.workspace {
            workspace.update(cx, |workspace, cx| {
                workspace.sync(self.snapshot.clone(), cx)
            });
        }
        if let Some(mail) = &self.mail {
            let selected = self.selected.clone();
            mail.update(cx, |mail, cx| {
                mail.set_context(&self.snapshot, selected, cx)
            });
        }
        retire_snapshot(retired_snapshot, cx);
        if shell_changed { self.schedule_startup_cache(cx); cx.notify(); }
    }
    fn compact_inactive_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        const INACTIVE_MESSAGE_LIMIT: usize = 256;
        let pane = self.chat_panes.borrow().get(task_id).cloned();
        let Some(pane) = pane else { return; };
        if !pane.read(cx).can_trim_hidden_history() { return; }
        pane.update(cx, |pane, _| pane.trim_hidden_history(INACTIVE_MESSAGE_LIMIT));
        self.queue_native_publication(PublicationChanges {
            trim_tasks: HashSet::from([task_id.to_owned()]),
            ..Default::default()
        }, cx);
    }
    fn accept_history_page(
        &mut self,
        task_id: &str,
        page: Result<TaskMessagesPage, String>,
        cx: &mut Context<Self>,
    ) {
        self.accept_history_page_attempt(task_id.to_owned(), Arc::new(page), 0, cx);
    }
    fn accept_history_page_attempt(
        &mut self, task_id: String, page: Arc<Result<TaskMessagesPage, String>>,
        attempt: u8, cx: &mut Context<Self>,
    ) {
        let mut state = self.native_state.clone();
        let generation = self.publication.generation;
        let worker_id = task_id.clone();
        let worker_page = page.clone();
        let worker = cx.background_executor().spawn(async move {
            worker_page.as_ref().clone().and_then(|page| {
                let value = serde_json::to_value(&page).map_err(|error| error.to_string())?;
                let changes = state.merge_task_page(&worker_id, page)?;
                Ok((state, value, changes))
            })
        });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                if result.is_ok() && native_publication::rebase_history_once(
                    generation, view.publication.generation,
                    page.as_ref().as_ref().ok().map(|page| page.revision.as_str()),
                    view.native_state.revision(), attempt,
                ) {
                    view.accept_history_page_attempt(task_id, page, attempt + 1, cx);
                    return;
                }
                let result = result.and_then(|(state, value, changes)| {
                    if generation != view.publication.generation {
                        return Err("Task history page is stale; retry at the current revision.".into());
                    }
                    view.native_state = state;
                    let mut changes = PublicationChanges::from_changes(false, changes);
                    let accepted_page = page.as_ref().as_ref().unwrap();
                    changes.task_page_boundaries.insert(task_id.clone(), native_publication::PageBoundary::new(accepted_page.revision.clone(), accepted_page.next_before_id.clone()));
                    view.queue_native_publication(changes, cx);
                    Ok(value)
                });
                let pane = view.chat_panes.borrow().get(&task_id).cloned();
                if let Some(pane) = pane {
                    pane.update(cx, |pane, cx| {
                        // An accepted local page must follow the deferred base,
                        // never be overwritten later by an older hidden update.
                        pane.apply_deferred_publication(cx);
                        pane.accept_history_page(result, cx);
                    });
                    view.compact_inactive_task(&task_id, cx);
                }
            });
        }).detach();
    }
    fn accept_channel_page(
        &mut self,
        channel: &str,
        before: Option<&str>,
        page: Result<ChannelMessagesPage, String>,
        cx: &mut Context<Self>,
    ) {
        self.accept_channel_page_attempt(channel.to_owned(), before.map(str::to_owned), Arc::new(page), 0, cx);
    }
    fn accept_channel_page_attempt(
        &mut self, channel: String, before: Option<String>,
        page: Arc<Result<ChannelMessagesPage, String>>, attempt: u8, cx: &mut Context<Self>,
    ) {
        let mut state = self.native_state.clone();
        let generation = self.publication.generation;
        let worker_channel = channel.clone();
        let worker_page = page.clone();
        let worker_before = before.clone();
        let worker = cx.background_executor().spawn(async move {
            worker_page.as_ref().clone().and_then(|page| {
                if page.next_before_id.is_some() && page.next_before_id == worker_before {
                    return Err("Channel history returned a non-advancing cursor; refresh before retrying.".into());
                }
                let next = page.next_before_id.clone();
                let value = serde_json::to_value(&page).map_err(|error| error.to_string())?;
                let changes = state.merge_channel_page(&worker_channel, page)?;
                Ok((state, value, changes, next))
            })
        });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                if result.is_ok() && native_publication::rebase_history_once(
                    generation, view.publication.generation,
                    page.as_ref().as_ref().ok().map(|page| page.revision.as_str()),
                    view.native_state.revision(), attempt,
                ) {
                    view.accept_channel_page_attempt(channel, before, page, attempt + 1, cx);
                    return;
                }
                let result = result.and_then(|(state, value, changes, next)| {
                    if generation != view.publication.generation {
                        return Err("Channel history page is stale; retry at the current revision.".into());
                    }
                    view.native_state = state;
                    view.channel_cursors.insert(channel.clone(), next.clone());
                    let mut changes = PublicationChanges::from_changes(false, changes);
                    let accepted_page = page.as_ref().as_ref().unwrap();
                    changes.channel_page_boundaries.insert(channel.clone(), native_publication::PageBoundary::new(accepted_page.revision.clone(), next));
                    view.queue_native_publication(changes, cx);
                    Ok(value)
                });
                if let Some(channels) = &view.channels {
                    channels.update(cx, |channels, cx| channels.accept_history_page(&channel, result, cx));
                }
            });
        }).detach();
    }
    fn command(&mut self, command: &'static str, args: Value, cx: &mut Context<Self>) {
        if self.startup_pending || self.busy {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { service.invoke(command, args) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(result) => {
                        // Clear a command failure after the command has
                        // completed successfully. Workspace preference
                        // failures have their own state and remain visible.
                        view.error = None;
                        if command == "create_task" {
                            view.selected = Some(field(&result, "id"));
                            view.sidebar = Sidebar::Chats;
                            view.dock_current = None;
                        }
                        view.refresh(cx);
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn palette_routes() -> Vec<(&'static str, &'static str, Sidebar)> {
        vec![
            ("chats", "Chats", Sidebar::Chats),
            ("archives", "Archives", Sidebar::Archived),
            ("agents", "Agents", Sidebar::Agents),
            ("projects", "Projects", Sidebar::Projects),
            ("browser", "Browser", Sidebar::Browser),
            ("settings", "Settings", Sidebar::Settings),
            ("mail", "Mail", Sidebar::Mail),
            ("channels", "Channels", Sidebar::Channels),
            ("terminal", "Terminal", Sidebar::Terminal),
            ("usage", "Usage", Sidebar::Usage),
            ("schedules", "Schedules", Sidebar::Schedules),
            ("collaboration", "Subagents", Sidebar::Subagents),
            ("git", "Repository", Sidebar::Git),
        ]
    }
    fn palette_entries(&self, _mode: command_palette::PaletteMode, cx: &App) -> Vec<command_palette::PaletteEntry> {
        palette_catalog::entries(self, cx)
    }
    fn open_palette(
        &mut self,
        mode: command_palette::PaletteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        if self.palette.is_none() {
            let pane = cx.new(|cx| command_palette::CommandPalette::new(window, cx));
            self.palette_subscription =
                Some(
                    cx.subscribe_in(&pane, window, |view, _, event, window, cx| {
                        view.dock.update(cx, |dock, cx| {
                            dock.set_shown(view.sidebar != Sidebar::Settings, window, cx)
                        });
                        if let command_palette::PaletteEvent::Selected(entry) = event {
                            view.execute_palette(entry, window, cx);
                        }
                        cx.notify();
                    }),
                );
            self.palette = Some(pane);
        }
        if !self.palette_terminals_loading {
            if let Some(service)=self.service.clone() {
                self.palette_terminals_loading=true;
                let lookup=cx.background_executor().spawn(async move {service.invoke("list_terminals",json!({}))});
                cx.spawn(async move |view,cx| {
                    let result=lookup.await;
                    let _=view.update(cx,|view,cx| {
                        view.palette_terminals_loading=false;
                        match result {
                            Ok(value)=>view.palette_terminal_sessions=value.as_array().cloned().unwrap_or_default(),
                            Err(error)=>{view.error=Some(format!("Terminal palette lookup: {error}"));}
                        }
                        if let Some(palette)=view.palette.clone() {
                            let entries=view.palette_entries(palette.read(cx).mode(),cx);
                            if let Err(error)=palette.update(cx,|palette,cx|palette.set_entries(entries,cx)){view.error=Some(error);}
                        }
                        cx.notify();
                    });
                }).detach();
            }
        }
        let entries = self.palette_entries(mode, cx);
        let search_pane_ids = entries.iter().filter(|entry| entry.open_priority < 2).map(|entry| entry.id.clone()).collect();
        let result = self
            .palette
            .as_ref()
            .unwrap()
            .update(cx, |palette, cx| palette.open(mode, entries, window, cx));
        if let Err(error) = result {
            self.error = Some(error);
        } else {
            self.refresh_palette_browser_text(search_pane_ids, cx);
            self.dock
                .update(cx, |dock, cx| dock.set_shown(false, window, cx));
        }
        cx.notify();
    }
    fn refresh_palette_browser_text(&self, pane_ids: Vec<String>, cx: &mut Context<Self>) {
        use futures::StreamExt as _;
        let (Some(browser), Some(palette)) = (&self.browser, &self.palette) else { return; };
        let generation = palette.read(cx).generation();
        let revision = browser.read(cx).metadata_revision();
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        for id in browser.read(cx).ordered_tab_ids() {
            let Some(tab) = browser.read(cx).tab_entity(id) else { continue; };
            let sender = sender.clone();
            tab.read(cx).request_search_text(move |text| { let _ = sender.unbounded_send((id, text)); }, cx);
        }
        drop(sender);
        cx.spawn(async move |view, cx| {
            while let Some((id, text)) = receiver.next().await {
                if text.is_empty() { continue; }
                let _ = view.update(cx, |view, cx| {
                    if view.browser.as_ref().is_none_or(|b| b.read(cx).metadata_revision() != revision) { return; }
                    if let Some(palette) = &view.palette {
                        let suffix = format!(":browser:{id}");
                        let ids: Vec<_> = pane_ids.iter().filter(|id| id.ends_with(&suffix)).cloned().collect();
                        palette.update(cx, |palette, cx| {
                            for id in ids { palette.set_search_text(&id, generation, text.clone(), cx); }
                        });
                    }
                });
            }
        }).detach();
    }
    fn execute_palette(
        &mut self,
        entry: &command_palette::PaletteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self
            .palette
            .as_ref()
            .map(|pane| pane.read(cx).mode())
            .unwrap_or(command_palette::PaletteMode::Switch);
        let current = self
            .palette_entries(mode, cx)
            .into_iter()
            .find(|current| current.id == entry.id && !current.disabled);
        let Some(entry) = current.as_ref() else {
            self.error = Some(
                "The selected workspace item changed or is unavailable. Open the palette again."
                    .into(),
            );
            return;
        };
        if self.execute_catalog_entry(entry, window, cx) { return; }
        if let Some(route) = entry.id.strip_prefix("route:") {
            if let Some((_, _, surface)) = Self::palette_routes()
                .into_iter()
                .find(|(id, _, _)| *id == route)
            {
                self.switch_surface(surface, window, cx);
            }
            return;
        }
        if let Some(task) = entry.id.strip_prefix("task:") {
            self.selected = Some(task.into());
            self.switch_surface(Sidebar::Chats, window, cx);
            if entry.kind == command_palette::EntryKind::ArchivedTask {
                self.command(
                    "set_task_archived",
                    json!({"taskId":task,"archived":false}),
                    cx,
                );
            }
            return;
        }
        if let Some(theme) = entry.id.strip_prefix("theme:") {
            let mut settings = self.snapshot["settings"].clone();
            settings["theme"] = json!(theme);
            self.command("save_settings", json!({"settings":settings}), cx);
        } else if let Some(preset) = entry.id.strip_prefix("native-palette:") {
            self.native_theme_preset = preset.into();
            self.queue_preferences_save(cx);
        } else if let Some(task) = self.selected.clone() {
            match entry.id.as_str() {
                "current:stop" => self.command("cancel_task", json!({"taskId":task}), cx),
                "current:archive" => self.command(
                    "set_task_archived",
                    json!({"taskId":task,"archived":true}),
                    cx,
                ),
                _ => {}
            }
        }
    }
    fn ensure_archives(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.archives.is_some() {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        let pane =
            cx.new(|cx| archives::ArchivedChatsPane::new(service, &self.snapshot, window, cx));
        self.archives_subscription = Some(cx.subscribe(&pane, |view, _, event, cx| match event {
            archives::ArchiveEvent::Refresh => view.refresh(cx),
        }));
        self.archives = Some(pane);
    }
    fn capture_native_drafts(&mut self, cx: &mut Context<Self>) {
        let drafts: Vec<_> = self
            .chat_panes
            .borrow()
            .iter()
            .filter_map(|(id, pane)| {
                let pane = pane.read(cx);
                let text = pane.draft(cx);
                // An untouched empty pane awaiting startup restore cannot erase disk text.
                if text.is_empty()
                    && !pane.draft_was_edited()
                    && self.native_drafts.contains_key(id)
                {
                    None
                } else {
                    Some((id.clone(), text))
                }
            })
            .collect();
        for (id, text) in drafts {
            self.update_native_draft(&id, &text, cx);
        }
        let attachments: Vec<_> = self
            .chat_panes
            .borrow()
            .iter()
            .filter_map(|(id, pane)| {
                let pane = pane.read(cx);
                let state = pane.attachment_draft(cx);
                if state.attachments.is_empty()
                    && !pane.attachment_draft_was_edited(cx)
                    && self.native_attachment_drafts.get(id).is_some_and(|saved| {
                        saved.host_id == state.host_id && saved.cwd == state.cwd
                    })
                {
                    None
                } else {
                    Some((id.clone(), state))
                }
            })
            .collect();
        for (id, state) in attachments {
            self.update_native_attachment_draft(&id, &state, cx);
        }
    }
    fn update_native_draft(&mut self, id: &str, text: &str, cx: &mut Context<Self>) {
        let old = self
            .native_drafts
            .get(id)
            .map(String::as_str)
            .unwrap_or_default();
        if old == text {
            return;
        }
        if text.is_empty() {
            self.native_drafts.remove(id);
        } else {
            self.native_drafts.insert(id.into(), text.into());
        }
        self.dirty_draft_ids.insert(id.into());
        if self.preferences_error.as_deref().is_some_and(|error| {
            error.starts_with("Native chat drafts exceed")
                || error.starts_with("Native preferences exceed")
        }) {
            self.preferences_blocked = false;
        }
        // The originating ChatPane already invalidates its composer controls.
        // Persisting a keystroke must not invalidate the entire desktop and
        // rebuild the sidebar and every other chat pane as well.
        self.schedule_preferences_save(cx);
    }
    fn update_native_attachment_draft(
        &mut self,
        id: &str,
        state: &NativeAttachmentDraft,
        cx: &mut Context<Self>,
    ) {
        let next = (!state.attachments.is_empty()).then_some(state);
        if self.native_attachment_drafts.get(id) == next {
            return;
        }
        if let Some(state) = next {
            self.native_attachment_drafts
                .insert(id.to_owned(), state.clone());
        } else {
            self.native_attachment_drafts.remove(id);
        }
        self.dirty_attachment_draft_ids.insert(id.to_owned());
        if self.preferences_error.as_deref().is_some_and(|error| {
            error.starts_with("Native chat attachment drafts exceed")
                || error.starts_with("Native preferences exceed")
        }) {
            self.preferences_blocked = false;
        }
        self.queue_preferences_save(cx);
    }
    fn load_preferences(&mut self, cx: &mut Context<Self>) {
        if self.preferences_busy {
            self.preferences_error = Some(
                "Workspace preferences are loading or saving. Wait for that operation to finish, then reload.".into(),
            );
            cx.notify();
            return;
        }
        if self.workspaces_dirty && !self.preferences_restoring && !self.preferences_restore {
            if let Ok(layout) = self.dock.read(cx).serialize(cx) { self.capture_workspace_layout(layout); }
        }
        let Some(service) = self.service.clone() else {
            self.preferences_error = Some("Workspace preferences service is unavailable.".into());
            cx.notify();
            return;
        };
        self.preferences_generation = self.preferences_generation.wrapping_add(1);
        self.preferences_save_timer = None;
        self.preferences_restore_generation = self.preferences_restore_generation.wrapping_add(1);
        self.preferences_restoring = false;
        self.preferences_restore_suppressed = false;
        let generation = self.preferences_generation;
        eprintln!("[native-preferences] operation=load generation={generation} result=started");
        self.preferences_busy = true;
        let worker = cx
            .background_executor()
            .spawn(async move {
                let mut preferences = service.load_native_preferences()?;
                let (revision, workspaces, theme, auto_size_panes) = monitter_gpui_desktop::workspace_store::load()?;
                if let Some(theme) = theme { preferences.data.appearance.tile_theme = theme; }
                Ok::<_, String>((preferences, revision, workspaces, auto_size_panes))
            });
        cx.spawn(async move |view, cx| {
            let result = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.preferences_busy = false;
                match result {
                    Ok((mut preferences, workspace_revision, stored_workspaces, auto_size_panes)) => {
                        if view.workspaces_dirty {
                            if let Some(current) = &view.preferences { preferences.data.appearance.tile_theme = current.data.appearance.tile_theme; }
                        }
                        eprintln!("[native-preferences] operation=load generation={generation} result=success");
                        view.native_theme_preset = if preferences.data.appearance.warm_paper {
                            "reference"
                        } else {
                            "monitter"
                        }
                        .into();
                        let loaded = merge_native_reload(
                            &preferences.data.drafts,
                            &view.native_drafts,
                            &view.dirty_draft_ids,
                        );
                        view.native_drafts = loaded;
                        view.native_attachment_drafts = merge_native_reload(&preferences.data.attachment_drafts, &view.native_attachment_drafts, &view.dirty_attachment_draft_ids);
                        if !view.native_browser_dirty {
                            view.native_browser_metadata = preferences.data.browser_metadata.clone();
                        }
                        view.native_chat_panes = merge_native_reload(&preferences.data.chat_panes, &view.native_chat_panes, &view.dirty_chat_pane_ids);
                        if !view.route_touched {
                            if let Some(route) = &preferences.data.appearance.last_route {
                                if let Some((_, _, surface)) = Self::palette_routes()
                                    .into_iter()
                                    .find(|(id, _, _)| *id == route)
                                {
                                    view.sidebar = surface;
                                    view.dock_current = None;
                                }
                            }
                        }
                        if view.workspaces_dirty {
                            if let Ok(layout) = view.dock.read(cx).serialize(cx) { view.capture_workspace_layout(layout); }
                        }
                        let missing_local_workspaces = stored_workspaces.is_none();
                        view.workspace_store_revision = workspace_revision;
                        if !view.workspaces_dirty {
                            view.pending_auto_size_panes = Some(auto_size_panes);
                            let mut source = preferences.data.clone();
                            source.workspaces = stored_workspaces.or(source.workspaces);
                            view.named_workspaces = Some(workspace_tabs::loaded_workspaces(&source));
                            view.selected = None;
                            view.dock_current = None;
                        }
                        view.workspaces_dirty |= missing_local_workspaces;
                        view.preferences = Some(preferences);
                        view.preferences_restore = true;
                        view.preferences_blocked = false;
                        view.preferences_dirty = !view.dirty_draft_ids.is_empty();
                        view.preferences_dirty |= view.workspaces_dirty || view.native_browser_dirty || view.sidebar_width_touched || !view.dirty_chat_pane_ids.is_empty() || !view.dirty_attachment_draft_ids.is_empty();
                        view.preferences_error = None;
                    }
                    Err(error) => {
                        eprintln!("[native-preferences] operation=load generation={generation} result=error");
                        view.preferences_error = Some(error);
                        view.preferences_blocked = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn queue_preferences_save(&mut self, cx: &mut Context<Self>) {
        self.schedule_startup_cache(cx);
        self.schedule_preferences_save(cx);
        cx.notify();
    }
    fn schedule_preferences_save(&mut self, cx: &mut Context<Self>) {
        self.preferences_dirty = true;
        self.preferences_generation = self.preferences_generation.wrapping_add(1);
        let generation = self.preferences_generation;
        self.preferences_save_timer = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(350))
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.preferences_generation == generation {
                    view.save_preferences(cx, generation);
                }
            });
        }));
    }
    fn save_preferences(&mut self, cx: &mut Context<Self>, generation: u64) {
        self.capture_native_drafts(cx);
        if self.startup_pending || self.preferences_busy
            || self.preferences_blocked
            || self.preferences_restore
            || self.preferences_restore_suppressed
            || self.preferences_restoring
            || self.closing
        {
            return;
        }
        let (Some(service), Some(preferences)) = (self.service.clone(), self.preferences.clone())
        else {
            return;
        };
        let layout = match self.dock.read(cx).serialize(cx) {
            Ok(layout) => layout,
            Err(error) => {
                self.preferences_error = Some(error);
                self.preferences_blocked = true;
                return;
            }
        };
        let layout_structure = Self::native_layout_structure(&layout);
        let mut data = preferences.data;
        self.capture_workspace_layout(layout.clone());
        let saved_workspaces = self.named_workspaces.clone();
        let workspace_revision = self.workspace_store_revision;
        let save_workspaces = self.workspaces_dirty;
        // Client-local layouts must not require a running-backend schema upgrade.
        data.workspaces = None;
        let saved_tile_theme = data.appearance.tile_theme;
        let saved_auto_size_panes = self.dock.read(cx).auto_size_panes();
        data.appearance.tile_theme = monitter_lib::experimental_desktop::NativeTileTheme::Monitter;
        data.layout = Some(layout);
        data.appearance.sidebar_width = Some(self.current_sidebar_width(cx));
        data.appearance.warm_paper = self.native_theme_preset == "reference";
        data.drafts = self.native_drafts.clone();
        data.attachment_drafts = self.native_attachment_drafts.clone();
        data.browser_metadata = self.native_browser_metadata.clone();
        data.chat_panes = self.native_chat_panes.clone();
        data.appearance.last_route = Self::palette_routes()
            .into_iter()
            .find(|(_, _, surface)| *surface == self.sidebar)
            .map(|(id, _, _)| id.to_owned());
        eprintln!("[native-preferences] operation=save generation={generation} result=started");
        self.preferences_busy = true;
        self.preferences_dirty = false;
        let acknowledged_workspaces = saved_workspaces.clone();
        let cache_preferences = NativePreferences { revision: preferences.revision.clone(), data: data.clone() };
        let mut cache_preferences = cache_preferences;
        cache_preferences.data.appearance.tile_theme = saved_tile_theme;
        let cache_layout = cache_preferences.data.layout.clone().unwrap_or_else(workspace_tabs::empty_layout);
        let cache_ids = docking::DockWorkspace::saved_ids(&cache_layout).unwrap_or_default();
        let presentation = monitter_gpui_desktop::startup_cache::Presentation::capture(&self.snapshot, &cache_preferences, saved_workspaces.clone(), cache_layout, &cache_ids);
        let worker = cx.background_executor().spawn(async move {
            let revision = if save_workspaces {
                match saved_workspaces.as_ref().ok_or_else(|| "Workspace preferences are not loaded.".to_owned()).and_then(|set| monitter_gpui_desktop::workspace_store::save(workspace_revision, set, saved_tile_theme, saved_auto_size_panes)) {
                    Ok(revision) => revision,
                    Err(error) => return (None, Err(error)),
                }
            } else { workspace_revision };
            let result = service.save_native_preferences(&preferences.revision, data);
            if result.is_ok() {
                if let Err(error) = monitter_gpui_desktop::startup_cache::save(&presentation) {
                    eprintln!("[native-startup] cache_write={error}");
                }
            }
            (Some(revision), result)
        });
        cx.spawn(async move |view, cx| {
            let (workspace_revision, result) = worker.await;
            let _ = view.update(cx, |view, cx| {
                view.preferences_busy = false;
                if let Some(revision) = workspace_revision {
                    view.workspace_store_revision = revision;
                    view.workspaces_dirty = view.named_workspaces != acknowledged_workspaces
                        || view.preferences.as_ref().is_some_and(|current| current.data.appearance.tile_theme != saved_tile_theme)
                        || view.dock.read(cx).auto_size_panes() != saved_auto_size_panes;
                }
                match result {
                    Ok(mut preferences) => {
                        preferences.data.appearance.tile_theme = view.preferences.as_ref().map(|current| current.data.appearance.tile_theme).unwrap_or(saved_tile_theme);
                        eprintln!("[native-preferences] operation=save generation={generation} result=success");
                        view.dirty_draft_ids.retain(|id| {
                            view.native_drafts.get(id) != preferences.data.drafts.get(id)
                        });
                        view.dirty_chat_pane_ids.retain(|id| view.native_chat_panes.get(id) != preferences.data.chat_panes.get(id));
                        view.dirty_attachment_draft_ids.retain(|id| view.native_attachment_drafts.get(id) != preferences.data.attachment_drafts.get(id));
                        view.native_browser_dirty = view.native_browser_dirty
                            && view.native_browser_metadata != preferences.data.browser_metadata;
                        view.preferences_dirty |= view.native_browser_dirty || !view.dirty_attachment_draft_ids.is_empty();
                        view.preferences_dirty |= view.workspaces_dirty;
                        view.preferences = Some(preferences);
                        view.preferences_error = None;
                        if view.preferences_dirty && !view.close_pending {
                            view.queue_preferences_save(cx);
                        }
                    }
                    Err(error) => {
                        eprintln!("[native-preferences] operation=save generation={generation} result=error: {error}");
                        if error.to_ascii_lowercase().contains("pane identity") {
                            // Log only structural layout data for this
                            // validation failure. Pane titles and preference
                            // payloads are deliberately excluded.
                            eprintln!("[native-preferences] operation=save generation={generation} result=layout-validation-error panes={layout_structure:?}");
                        }
                        view.preferences_error = Some(error);
                        view.preferences_blocked = true;
                        view.preferences_dirty = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn native_layout_structure(layout: &Value) -> Vec<String> {
        fn visit(pane: &Value, path: &str, entries: &mut Vec<String>) {
            let panel_name = pane["panel_name"].as_str().or_else(|| pane["panelName"].as_str()).unwrap_or("unknown");
            let kind = match panel_name {
                "MonitterNativePane" => "native-pane",
                "StackPanel" => "split",
                "TabPanel" => "tabs",
                _ => "other",
            };
            if panel_name == "MonitterNativePane" {
                let id = pane["info"]["panel"]["id"]
                    .as_str()
                    .or_else(|| pane["info"]["id"].as_str())
                    .unwrap_or("<missing>");
                entries.push(format!("{path}: kind={kind} id={id:?}"));
            } else if matches!(panel_name, "StackPanel" | "TabPanel") {
                entries.push(format!("{path}: kind={kind}"));
            }
            if let Some(children) = pane["children"].as_array() {
                for (index, child) in children.iter().enumerate() {
                    visit(child, &format!("{path}/{index}"), entries);
                }
            }
        }

        let mut entries = Vec::new();
        for (region, key) in [
            ("center", "center"),
            ("left", "left_dock"),
            ("right", "right_dock"),
            ("bottom", "bottom_dock"),
        ] {
            let pane = if key == "center" {
                Some(&layout[key])
            } else {
                layout[key]["panel"].as_object().map(|_| &layout[key]["panel"])
            };
            if let Some(pane) = pane {
                visit(pane, region, &mut entries);
            }
        }
        entries
    }
    fn current_sidebar_width(&self, cx: &App) -> f32 {
        if self.sidebar_motion.active() || self.sidebar_is_hidden() { return expanded_sidebar_width_for_scale(self.sidebar_expanded_width,self.sidebar_scale); }
        if self.sidebar_collapsed { return expanded_sidebar_width_for_scale(self.sidebar_expanded_width,self.sidebar_scale); }
        self.sidebar_resize
            .read(cx)
            .sizes()
            .first()
            .map(|width| *width / design::ui_px(1., self.sidebar_scale))
            .filter(|width| width.is_finite() && !sidebar_should_collapse(*width,false,self.sidebar_scale))
            .or_else(|| {
                self.preferences
                    .as_ref()
                    .and_then(|preferences| preferences.data.appearance.sidebar_width)
            })
            .map(|width|expanded_sidebar_width_for_scale(width,self.sidebar_scale))
            .unwrap_or(252.0)
    }
    fn workspace_route_background(&self, palette: design::Palette, cx: &App) -> gpui::Background {
        let has_dock = self.dock.read(cx).has_panes()
            && self.sidebar != Sidebar::Settings && !self.close_pending;
        if design::workspace_route_needs_fill(design::tile_theme(), has_dock) {
            crate::surface_blend::background(crate::surface_blend::Area::Workspace, palette.paper)
        } else {
            // Retained terminal/auxiliary panes share chat's one-layer backdrop.
            rgba(0x00000000).into()
        }
    }
    fn sidebar_is_hidden(&self) -> bool {
        self.sidebar_hidden
    }
    fn displayed_sidebar_width(&self, cx: &App) -> f32 {
        self.sidebar_resize.read(cx).sizes().first()
            .map(|width| *width / design::ui_px(1., self.sidebar_scale))
            .filter(|width| width.is_finite())
            .unwrap_or_else(|| if self.sidebar_is_hidden() { 0. } else if self.sidebar_collapsed { design::compact_sidebar_width_design(self.sidebar_scale) } else { self.sidebar_expanded_width })
    }
    fn start_sidebar_motion(&mut self, from: f32, target: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_width_touched = true;
        self.sidebar_resize_generation = self.sidebar_resize_generation.wrapping_add(1);
        self.sidebar_motion.start(from, target, std::time::Instant::now(), native_motion::motion_allowed());
        self.sidebar_resize_suppressed = true;
        if self.sidebar_motion.active() {
            self.schedule_sidebar_motion_frame(window, cx);
        } else {
            self.set_sidebar_displayed_width(target, window, cx);
            self.sidebar_resize_suppressed = false;
            self.queue_preferences_save(cx);
        }
    }
    fn set_sidebar_displayed_width(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) {
        if (self.displayed_sidebar_width(cx) - width).abs() > 0.01 {
            self.sidebar_resize.update(cx, |state, cx| state.resize_panel_for_layout(0, design::ui_px(width, self.sidebar_scale), window, cx));
        }
    }
    fn finish_sidebar_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.sidebar_motion.target() {
            self.sidebar_motion.cancel();
            self.set_sidebar_displayed_width(target, window, cx);
            self.sidebar_resize_suppressed = false;
            self.queue_preferences_save(cx);
        }
    }
    fn settle_sidebar_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_motion.cancel();
        self.sidebar_resize_generation = self.sidebar_resize_generation.wrapping_add(1);
        self.sidebar_resize_suppressed = false;
        self.sidebar_motion_compact = self.sidebar_collapsed;
        self.sidebar_tabs_motion.borrow_mut().settle(match self.sidebar_view {
            SidebarView::Agents => 0, SidebarView::Projects => 1, SidebarView::Activity => 2,
        });
        let width = if self.sidebar_is_hidden() { 0. } else if self.sidebar_collapsed {
            design::compact_sidebar_width_design(self.sidebar_scale)
        } else { expanded_sidebar_width_for_scale(self.sidebar_expanded_width, self.sidebar_scale) };
        self.set_sidebar_displayed_width(width, window, cx);
    }
    fn schedule_sidebar_motion_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_motion_frame_pending || !self.sidebar_motion.active() { return; }
        self.sidebar_motion_frame_pending = true;
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.sidebar_motion_frame_pending = false;
                if !view.sidebar_motion.active() { return; }
                if view.sidebar_resize.read(cx).is_resizing() {
                    view.sidebar_motion.cancel();
                    view.sidebar_resize_suppressed = false;
                    view.sidebar_hidden = false;
                    return;
                }
                if !native_motion::motion_allowed() || view.closing || view.close_pending || crate::session_lock::locked(cx) {
                    view.finish_sidebar_motion(window, cx);
                    cx.notify();
                    return;
                }
                if let Some(width) = view.sidebar_motion.sample(std::time::Instant::now()) {
                    view.set_sidebar_displayed_width(width, window, cx);
                }
                if view.sidebar_motion.active() {
                    view.schedule_sidebar_motion_frame(window, cx);
                } else {
                    view.sidebar_resize_suppressed = false;
                    view.queue_preferences_save(cx);
                }
                cx.notify();
            });
        });
    }
    fn sidebar_split(&mut self, sidebar: AnyElement, content: AnyElement, palette: design::Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = if self.sidebar_motion.active() { self.displayed_sidebar_width(cx) } else if self.sidebar_is_hidden() { 0. } else if self.sidebar_collapsed { design::compact_sidebar_width_design(design::interface_scale()) } else if self.sidebar_resize_suppressed { expanded_sidebar_width_for_scale(self.sidebar_expanded_width,self.sidebar_scale) } else if self.sidebar_width_touched {
            self.current_sidebar_width(cx)
        } else { self.preferences.as_ref().and_then(|preferences| preferences.data.appearance.sidebar_width).map(|width|expanded_sidebar_width_for_scale(width,self.sidebar_scale)).unwrap_or(252.0) };
        if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
            let spacing = design::tern_spacing(&self.snapshot["settings"]);
            // Keep the newer flush sidebar edge while it slides out; restore
            // the hidden workspace gutter continuously near zero width.
            let edge = (width / design::compact_sidebar_width_design(self.sidebar_scale)).clamp(0., 1.);
            let tabs = self.render_workspace_tabs(palette, window, cx);
            let column = div().size_full().flex().flex_col().min_w_0()
                .child(tabs)
                .child(div().flex_1().min_h_0().w_full().flex()
                    .border(design::ui_scaled_px(spacing.outer_padding()))
                    // The dock frame and its tiles already provide two half
                    // gutters between the workspace tabs and the pane edge.
                    .border_t_0()
                    .border_l(design::ui_scaled_px(spacing.outer_padding() * (1. - edge)))
                    .border_color(rgba(0x00000000))
                    .child(div().flex_1().min_w_0().h_full()
                        // Cancel the dock frame and tile's left half-gutters
                        // only at the sidebar divider. Other pane gaps stay intact.
                        .ml(-design::ui_scaled_px(spacing.gutter * edge))
                        .child(content)));
            return sidebar_split_with_visibility(&self.sidebar_resize, width, self.sidebar_scale, sidebar, column.into_any_element(), self.sidebar_is_hidden() && !self.sidebar_motion.active(), self.sidebar_motion.active());
        }
        sidebar_split_with_visibility(&self.sidebar_resize, width, self.sidebar_scale, sidebar, content, self.sidebar_is_hidden() && !self.sidebar_motion.active(), self.sidebar_motion.active())
    }
    fn apply_saved_sidebar_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar_width_touched && !self.sidebar_motion.active() {
            if let Some(width) = self.preferences.as_ref().and_then(|preferences| preferences.data.appearance.sidebar_width) {
                self.sidebar_expanded_width = expanded_sidebar_width_for_scale(width,self.sidebar_scale);
            }
        }
        if self.sidebar_motion.active() || self.sidebar_is_hidden() { return; }
        let has_panels = self.sidebar_resize.read(cx).sizes().len() == 2;
        if !has_panels {
            return;
        }
        // resize_panel commits the programmatic change and clears the active
        // gesture. Never replay the saved width over a pointer-owned resize.
        let Some(width) = saved_sidebar_width_target(
            self.preferences.as_ref().and_then(|preferences| preferences.data.appearance.sidebar_width).map(|width|expanded_sidebar_width_for_scale(width,self.sidebar_scale)),
            self.current_sidebar_width(cx), self.sidebar_width_touched,
            self.sidebar_collapsed, self.sidebar_resize.read(cx).is_resizing(), self.sidebar_scale,
        ) else { return; };
        self.sidebar_resize_suppressed = true;
        self.sidebar_resize_generation = self.sidebar_resize_generation.wrapping_add(1);
        let generation = self.sidebar_resize_generation;
        self.sidebar_resize.update(cx, |state, cx| {
            state.resize_panel_for_layout(0, design::ui_px(width,self.sidebar_scale), window, cx);
        });
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.sidebar_resize_generation == generation {
                    view.sidebar_resize_suppressed = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn restore_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preferences_restore
            || self.preferences_restoring
            || self.native_state.snapshot().is_none()
            // The raw backend state arrives before its background UI
            // projection. Pane preparation uses the published snapshot;
            // consuming restore intent early falsely reports saved chats
            // missing and permanently blocks workspace navigation.
            || self.publication_index.is_none()
            || self.revision.as_deref() != self.native_state.revision()
        {
            return;
        }
        self.preferences_restore = false;
        self.preferences_restoring = true;
        self.preferences_restore_suppressed = true;
        self.preferences_restore_generation = self.preferences_restore_generation.wrapping_add(1);
        let restore_generation = self.preferences_restore_generation;
        eprintln!(
            "[native-preferences] operation=restore generation={restore_generation} result=started"
        );
        // This path runs only after an authoritative snapshot exists. Prune
        // deleted task identities, retaining archived tasks still in that snapshot.
        let existing_tasks: HashSet<String> = rows(&self.snapshot, "tasks")
            .iter()
            .map(|task| field(task, "id"))
            .collect();
        let obsolete: Vec<String> = self
            .native_chat_panes
            .keys()
            .filter(|id| !existing_tasks.contains(*id))
            .cloned()
            .collect();
        for id in obsolete {
            self.native_chat_panes.remove(&id);
            self.dirty_chat_pane_ids.insert(id);
            self.preferences_dirty = true;
        }
        let obsolete_attachments: Vec<String> = self
            .native_attachment_drafts
            .keys()
            .filter(|id| !existing_tasks.contains(*id))
            .cloned()
            .collect();
        for id in obsolete_attachments {
            self.native_attachment_drafts.remove(&id);
            self.dirty_attachment_draft_ids.insert(id);
            self.preferences_dirty = true;
        }
        self.restore_browser_metadata(window, cx);
        for (id, pane) in self.chat_panes.borrow().iter() {
            if let Some(state) = self.native_attachment_drafts.get(id) {
                pane.update(cx, |pane, cx| pane.restore_attachment_draft(id, state, cx));
            }
            if let Some(state) = self.native_chat_panes.get(id) {
                pane.update(cx, |pane, cx| {
                    pane.restore_secondary_sidebar(state, window, cx)
                });
            }
            if let Some(text) = self.native_drafts.get(id) {
                pane.update(cx, |pane, cx| {
                    pane.restore_draft(text, window, cx);
                });
            }
        }
        let layout = self.named_workspaces.as_ref().and_then(|set| {
            let id = self.workspace_switch.as_ref().unwrap_or(&set.active_id);
            set.items.iter().find(|item| &item.id == id).map(|item| item.layout.clone().unwrap_or_else(workspace_tabs::empty_layout))
        }).or_else(|| self.preferences.as_ref().and_then(|preferences| preferences.data.layout.clone()));
        let Some(mut layout) = layout else {
            self.preferences_restore_suppressed = false;
            self.preferences_restoring = false;
            self.finish_startup(window, cx);
            if self.preferences_dirty {
                self.queue_preferences_save(cx);
            }
            return;
        };
        let obsolete: HashSet<String> = docking::DockWorkspace::saved_ids(&layout).unwrap_or_default().into_iter()
            .filter(|id| id.starts_with("neovim-files:") || (id.starts_with("draft:") && !self.draft_panes.borrow().contains_key(id))).collect();
        match workspace_visibility::prune_native_pane_ids(&layout, &obsolete) {
            Ok(cleaned) => { if cleaned != layout { layout = cleaned; self.preferences_dirty = true; } },
            Err(error) => { self.fail_workspace_restore(error, cx); return; }
        }
        match workspace_visibility::prune_layout(&layout, &self.hidden_task_ids) {
            Ok(pruned) => {
                if pruned != layout {
                    layout = pruned;
                    self.preferences_dirty = true;
                }
            }
            Err(error) => {
                self.fail_workspace_restore(
                    format!("Saved workspace visibility cleanup failed: {error}"),
                    cx,
                );
                return;
            }
        }
        let browser_ids = docking::DockWorkspace::saved_ids(&layout)
            .unwrap_or_default()
            .into_iter()
            .any(|id| id == "route:browser" || id.starts_with("browser:"));
        if browser_ids {
            self.ensure_browser(window, cx);
            if let Some(manager) = &self.browser {
                match browser_workspace::migrate_browser_layout(
                    &layout,
                    &manager.read(cx).ordered_tab_ids(),
                    manager.read(cx).selected_tab_id(),
                ) {
                    Ok(migrated) => {
                        if migrated != layout {
                            self.preferences_dirty = true;
                        }
                        layout = migrated;
                    }
                    Err(error) => {
                        self.fail_workspace_restore(error, cx);
                        return;
                    }
                }
            }
        }
        let saved_ids = docking::DockWorkspace::saved_ids(&layout).unwrap_or_default();
        let needs_terminal_lookup = saved_ids.iter().any(|id| id.starts_with("terminal:"));
        let needs_neovim_lookup = saved_ids.iter().any(|id| id.starts_with("neovim:"));
        if !needs_terminal_lookup && !needs_neovim_lookup {
            self.finish_workspace_restore(
                layout,
                Vec::new(),
                Vec::new(),
                restore_generation,
                window,
                cx,
            );
            return;
        }
        let Some(service) = self.service.clone() else {
            self.fail_workspace_restore("Native service has not loaded.".into(), cx);
            return;
        };
        let (sender, receiver) = futures::channel::oneshot::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("monitter-session-restore".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = (|| {
                    let terminals = if needs_terminal_lookup {
                        service
                            .invoke("list_terminals", json!({}))?
                            .as_array()
                            .cloned()
                            .ok_or("Terminal list response is invalid.".to_owned())?
                    } else {
                        Vec::new()
                    };
                    let neovim = if needs_neovim_lookup {
                        let descriptors = service
                            .invoke("list_neovim", json!({}))?
                            .as_array()
                            .cloned()
                            .ok_or("Neovim list response is invalid.".to_owned())?;
                        if descriptors.iter().any(|session| {
                            session["id"].as_str().is_none()
                                || session["exited"].as_bool().is_none()
                        }) {
                            return Err("Neovim list response is invalid.".to_owned());
                        }
                        descriptors
                    } else {
                        Vec::new()
                    };
                    Ok((terminals, neovim))
                })();
                let _ = sender.send(result);
            })
        {
            self.fail_workspace_restore(error.to_string(), cx);
            return;
        }
        cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await.map_err(|error| error.to_string()).and_then(|result| result);
            let _ = view.update_in(cx, |view, window, cx| {
                if view.closing || view.preferences_restore_generation != restore_generation {
                    return;
                }
                match result {
                    Ok((sessions, neovim_sessions)) => view.finish_workspace_restore(
                        layout,
                        sessions,
                        neovim_sessions,
                        restore_generation,
                        window,
                        cx,
                    ),
                    Err(error) => view.fail_workspace_restore(
                        format!("Workspace session restore failed: {error}"),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }
    fn fail_workspace_restore(&mut self, error: String, cx: &mut Context<Self>) {
        self.pending_sidebar_task = None;
        self.pending_notification_read = None;
        let switching_workspace = self.workspace_switch.take().is_some();
        self.pending_workspace_removal = None;
        self.preferences_restore_suppressed = false;
        self.preferences_restoring = false;
        self.preferences_error = Some(error);
        // A rejected destination leaves the outgoing workspace intact. Keep
        // other workspace tabs usable; initial-load failures still protect
        // the saved preferences from being overwritten.
        self.preferences_blocked = !switching_workspace;
        cx.notify();
    }
    fn finish_workspace_restore(
        &mut self,
        layout: Value,
        sessions: Vec<Value>,
        neovim_sessions: Vec<Value>,
        restore_generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Backend sessions are process-local; prune stale pane references from
        // authoritative listings before constructing retained panes.
        let original_layout = layout.clone();
        let layout = match native_session_restore::prune_missing_session_panes(
            &layout,
            &sessions,
            &neovim_sessions,
        ) {
            Ok(layout) => layout,
            Err(error) => { self.fail_workspace_restore(error, cx); return; }
        };
        if layout != original_layout {
            self.preferences_dirty = true;
            self.workspaces_dirty = true;
        }
        let restored_layout = layout.clone();
        let result = self
            .prepare_saved_panes(&layout, &sessions, window, cx)
            .and_then(|()| {
                self.dock
                    .update(cx, |dock, cx| dock.restore_retaining(layout, window, cx))
            });
        match result {
            Ok(()) => {
                if let Some(id) = self.workspace_switch.take() {
                    if let Some(set) = &mut self.named_workspaces { set.active_id = id; }
                    self.workspaces_dirty = true;
                    self.preferences_dirty = true;
                }
                if let Some(id) = self.pending_workspace_removal.take() {
                    if let Some(set) = &mut self.named_workspaces {
                        monitter_gpui_desktop::workspace_order::remove(set, &id);
                    }
                    self.workspaces_dirty = true;
                    self.preferences_dirty = true;
                }
                self.preferences_error = None;
                self.preferences_blocked = false;
                self.restore_named_workspace_focus(&restored_layout, window, cx);
                self.dock_current = self.dock.read(cx).focused_pane_id(window, cx);
                // The new layout is usable now. Only persistence needs the
                // settling delay; keeping the UI in its loading state makes
                // pane controls dim and brighten on every workspace switch.
                self.preferences_restoring = false;
                self.finish_startup(window, cx);
                let neovim_session_ids = docking::DockWorkspace::saved_ids(&restored_layout)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|pane_id| {
                        neovim_workspace::parse_id(&pane_id)
                            .map(|(session_id, _)| session_id.to_owned())
                    })
                    .collect::<HashSet<_>>();
                for session_id in neovim_session_ids {
                    if let Err(error) = self.reconcile_neovim(&session_id, window, cx) {
                        self.error = Some(error);
                    }
                }
                let generation = restore_generation;
                cx.spawn(async move |view, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    let _ = view.update(cx, |view, cx| {
                        if view.preferences_restore_generation == generation {
                            view.preferences_restore_suppressed = false;
                            view.preferences_restoring = false;
                            view.preferences_dirty |= view.native_browser_dirty || !view.dirty_draft_ids.is_empty() || !view.dirty_chat_pane_ids.is_empty() || !view.dirty_attachment_draft_ids.is_empty();
                            if view.preferences_dirty {
                                view.queue_preferences_save(cx);
                            }
                            eprintln!("[native-preferences] operation=restore generation={generation} result=settled");
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            Err(error) => {
                self.pending_sidebar_task = None;
                self.pending_notification_read = None;
                let switching_workspace = self.workspace_switch.take().is_some();
                self.pending_workspace_removal = None;
                self.preferences_restore_suppressed = false;
                self.preferences_restoring = false;
                eprintln!(
                    "[native-preferences] operation=restore generation={restore_generation} result=error: {error}"
                );
                self.preferences_error = Some(error);
                self.preferences_blocked = !switching_workspace;
            }
        }
    }
    fn prepare_saved_panes(
        &mut self,
        layout: &Value,
        sessions: &[Value],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let ids = docking::DockWorkspace::saved_ids(layout)?;
        let task_ids: HashSet<String> = rows(&self.snapshot, "tasks")
            .iter()
            .map(|task| field(task, "id"))
            .collect();
        // Validate every saved identity before constructing entities or changing layout.
        for id in &ids {
            if let Some(task) = id.strip_prefix("chat:") {
                if !task_ids.contains(task) {
                    return Err(format!(
                        "Saved chat {task} no longer exists. Current layout retained."
                    ));
                }
            } else if let Some(task) = git_workspace::task_id(id) {
                if !task_ids.contains(task) {
                    return Err(format!("Saved Git task {task} no longer exists. Current layout retained."));
                }
            } else if id.starts_with("terminal:") {
                match terminal_workspace::lookup_terminal_session(id, sessions) {
                    terminal_workspace::TerminalSessionLookup::Running(_)
                    | terminal_workspace::TerminalSessionLookup::Exited(_) => {}
                    _ => {
                        return Err(format!(
                            "Saved terminal pane {id} is unavailable. Current layout retained."
                        ));
                    }
                }
            } else if neovim_workspace::parse_id(id).is_some() || neovim_workspace::parse_files_id(id).is_some() {
                // The retained session reconnects through the private backend.
                // Missing/exited sessions were pruned from the authoritative listing.
            } else if id.starts_with("draft:") && self.draft_panes.borrow().contains_key(id) {
                // Retained unsent form, including inactive named workspaces.
            } else if id.starts_with("empty:") {
                // DockWorkspace restores these local, unsent overview panes.
            } else if let Some(browser_id) = browser_workspace::parse_browser_pane_id(id) {
                if self
                    .browser
                    .as_ref()
                    .and_then(|manager| manager.read(cx).tab_entity(browser_id))
                    .is_none()
                {
                    return Err(format!(
                        "Saved browser tab {browser_id} is unavailable. Current layout retained."
                    ));
                }
            } else if ![
                "route:archives",
                "route:repository",
                "route:subagents",
                "route:agents",
                "route:schedules",
                "route:usage",
                "route:app-resource-usage",
                "route:terminal",
                "route:channels",
                "route:mail",
                "route:projects",
            ]
            .contains(&id.as_str())
            {
                return Err(format!(
                    "Saved pane {id} is unavailable. Current layout retained."
                ));
            }
        }
        let service = self
            .service
            .clone()
            .ok_or("Native service has not loaded.")?;
        for id in ids {
            if id.starts_with("empty:") || id.starts_with("draft:") {
                continue;
            }
            if self.dock.read(cx).contains(&id) {
                continue;
            }
            let spec = if git_workspace::task_id(&id).is_some() {
                self.task_git_spec(&id, window, cx)?
            } else if let Some((session,win)) = neovim_workspace::parse_id(&id) {
                self.neovim_spec(session,win,window,cx)?

            } else if id.starts_with("terminal:") {
                let session = match terminal_workspace::lookup_terminal_session(&id, sessions) {
                    terminal_workspace::TerminalSessionLookup::Running(session)
                    | terminal_workspace::TerminalSessionLookup::Exited(session) => session.clone(),
                    _ => return Err("Saved terminal session is unavailable.".into()),
                };
                self.terminal_spec(session, window, cx)?
            } else if let Some(browser_id) = browser_workspace::parse_browser_pane_id(&id) {
                self.browser_spec(browser_id, cx)?
            } else if let Some(task) = id.strip_prefix("chat:") {
                let pane = self.ensure_chat_pane(task, service.clone(), window, cx);
                let title = rows(&self.snapshot, "tasks")
                    .iter()
                    .find(|row| field(row, "id") == task)
                    .map(|row| field(row, "title"))
                    .unwrap_or_else(|| "Chat".into());
                let weak = cx.weak_entity();
                let task = task.to_owned();
                let voice_pane = pane.downgrade();
                let hook: docking::LifecycleHook = Rc::new(move |event, _, cx| {
                    if matches!(
                        event,
                        docking::PaneLifecycle::Visible(false) | docking::PaneLifecycle::Removed
                    ) {
                        let _ = voice_pane.update(cx, |pane, cx| pane.deactivate_voice(cx));
                    }
                    if matches!(event, docking::PaneLifecycle::Active(true)) {
                        let _ = weak.update(cx, |view, cx| {
                            view.selected = Some(task.clone());
                            view.sidebar = Sidebar::Chats;
                            view.dock_current = Some(format!("chat:{task}"));
                            cx.notify();
                        });
                    }
                });
                docking::PaneSpec {
                    id: id.clone(),
                    title: title.into(),
                    content: pane.clone().into(),
                    focus: pane.focus_handle(cx),
                    lifecycle: Some(hook),
                    closable: true,
                }
            } else {
                let (surface, title, content): (_, _, AnyView) = match id.as_str() {
                    "route:archives" => {
                        self.ensure_archives(window, cx);
                        (
                            Sidebar::Archived,
                            "Archives",
                            self.archives.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:repository" => {
                        if self.git.is_none() {
                            self.git = Some(cx.new(|cx| {
                                git::GitPane::new(service.clone(), &self.snapshot, window, cx)
                            }));
                        }
                        (
                            Sidebar::Git,
                            "Repository",
                            self.git.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:subagents" => {
                        if self.subagents.is_none() {
                            self.subagents = Some(cx.new(|cx| {
                                collaboration::SubagentPane::new(
                                    service.clone(),
                                    self.snapshot.clone(),
                                    window,
                                    cx,
                                )
                            }));
                        }
                        (
                            Sidebar::Subagents,
                            "Subagents",
                            self.subagents.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:agents" => {
                        if self.agent_editor.is_none() {
                            self.agent_editor = Some(cx.new(|cx| {
                                agents::AgentHostPane::new(
                                    service.clone(),
                                    (*self.snapshot).clone(),
                                    window,
                                    cx,
                                )
                            }));
                        }
                        (
                            Sidebar::Agents,
                            "Agents",
                            self.agent_editor.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:schedules" => {
                        if self.schedules.is_none() {
                            self.schedules = Some(cx.new(|cx| {
                                schedules::SchedulePane::new(
                                    service.clone(),
                                    &self.snapshot,
                                    window,
                                    cx,
                                )
                            }));
                        }
                        (
                            Sidebar::Schedules,
                            "Schedules",
                            self.schedules.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:app-resource-usage" => {
                        let spec = self.resource_spec(window, cx)?;
                        self.dock.update(cx, |dock, cx| dock.register(spec, cx))?;
                        continue;
                    }
                    "route:usage" => {
                        if self.usage.is_none() {
                            self.usage = Some(cx.new(|cx| {
                                usage::UsagePane::new(service.clone(), &self.snapshot, window, cx)
                            }));
                        }
                        (
                            Sidebar::Usage,
                            "Usage",
                            self.usage.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:terminal" => {
                        if self.terminal.is_none() {
                            self.terminal = Some(self.new_terminal_manager(service.clone(),window,cx));
                        }
                        (
                            Sidebar::Terminal,
                            "Terminal",
                            self.terminal.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:mail" => {
                        if self.mail.is_none() {
                            self.mail =
                                Some(cx.new(|cx| MailPane::new(service.clone(), window, cx)));
                        }
                        (
                            Sidebar::Mail,
                            "Mail",
                            self.mail.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:projects" => {
                        if self.workspace.is_none() {
                            self.workspace = Some(cx.new(|cx| {
                                WorkspacePane::new(
                                    service.clone(),
                                    self.snapshot.clone(),
                                    window,
                                    cx,
                                )
                            }));
                        }
                        (
                            Sidebar::Projects,
                            "Projects",
                            self.workspace.as_ref().unwrap().clone().into(),
                        )
                    }
                    "route:channels" => {
                        if self.channels.is_none() {
                            let channels =
                                cx.new(|cx| ChannelPane::new(service.clone(), window, cx));
                            self.channel_subscription =
                                Some(cx.subscribe(&channels, |view, _, event, cx| match event {
                                    channels::ChannelEvent::Refresh => view.refresh(cx),
                                    channels::ChannelEvent::HistoryPage {
                                        channel,
                                        before,
                                        page,
                                    } => view.accept_channel_page(
                                        channel,
                                        before.as_deref(),
                                        page.clone(),
                                        cx,
                                    ),
                                }));
                            channels.update(cx, |channels, cx| {
                                channels.sync(
                                    self.snapshot.clone(),
                                    self.revision.as_deref().unwrap_or_default(),
                                    false,
                                    &[],
                                    &self.channel_cursors,
                                    cx,
                                )
                            });
                            self.channels = Some(channels);
                        }
                        (
                            Sidebar::Channels,
                            "Channels",
                            self.channels.as_ref().unwrap().clone().into(),
                        )
                    }
                    _ => unreachable!("identities checked before construction"),
                };
                self.auxiliary_spec(surface, title, content, cx)
            };
            self.dock.update(cx, |dock, cx| dock.register(spec, cx))?;
        }
        Ok(())
    }
    fn ensure_sidebar_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_usage.is_some() { return; }
        let Some(service) = self.service.clone() else { return; };
        let snapshot = self.snapshot.clone();
        let revision = self.revision.clone();
        let widget = cx.new(|cx| sidebar_usage::SidebarUsageWidget::new(
            service, snapshot.clone(), revision.as_deref(), window, cx,
        ));
        cx.subscribe_in(&widget, window, |desktop, _, _: &sidebar_usage::ResourceUsageEvent, window, cx| {
            match desktop.resource_spec(window, cx) {
                Ok(spec) => {
                    desktop.dock.update(cx, |dock, cx| {
                        if !dock.focus("route:app-resource-usage", window, cx) {
                            if let Err(error) = dock.add(spec, gpui_kit::component::dock::DockPlacement::Center, None, window, cx) {
                                eprintln!("Could not open resource tab: {error}");
                                return;
                            }
                            dock.focus("route:app-resource-usage", window, cx);
                        }
                        dock.set_shown(true, window, cx);
                    });
                    desktop.sidebar = Sidebar::Chats;
                    desktop.dock_current = Some("route:app-resource-usage".into());
                    cx.notify();
                }
                Err(error) => { desktop.error = Some(error); cx.notify(); }
            }
        }).detach();
        self.workspace_usage_rings = Some(cx.new(|cx| sidebar_usage::WorkspaceUsageRings::new(widget.clone(), cx)));
        self.sidebar_usage = Some(widget);
        self.sidebar_usage_snapshot = Some(snapshot);
        self.sidebar_usage_revision = revision;
    }

    fn resource_spec(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<docking::PaneSpec, String> {
        self.ensure_sidebar_usage(window, cx);
        let widget = self.sidebar_usage.clone().ok_or("Native service has not loaded.")?;
        if self.app_resources.is_none() {
            self.app_resources = Some(cx.new(|cx| usage::AppResourcePane::new(widget, cx)));
        }
        let focus = cx.focus_handle();
        let content = self.app_resources.as_ref().unwrap().clone().into();
        let frame = cx.new(|_| PaneFocusFrame { content, focus: focus.clone() });
        let owner = cx.weak_entity();
        Ok(docking::PaneSpec {
            id: "route:app-resource-usage".into(),
            title: "App resource usage".into(),
            content: frame.into(), focus,
            lifecycle: Some(Rc::new(move |event, _, cx| {
                if matches!(event, docking::PaneLifecycle::Active(true)) {
                    let _ = owner.update(cx, |desktop, cx| {
                        desktop.sidebar = Sidebar::Chats;
                        desktop.dock_current = Some("route:app-resource-usage".into());
                        cx.notify();
                    });
                }
            })),
            closable: true,
        })
    }

    fn task_git_spec(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<docking::PaneSpec, String> {
        let task_id = git_workspace::task_id(id).ok_or("Invalid task Git pane identity.")?.to_owned();
        let pane = if let Some(pane) = self.git_panes.get(id) { pane.clone() } else {
            let service = self.service.clone().ok_or("Native service has not loaded.")?;
            let pane = cx.new(|cx| git::GitPane::new(service, &self.snapshot, window, cx));
            pane.update(cx, |pane, cx| pane.set_context(&self.snapshot, &task_id, cx));
            self.git_panes.insert(id.to_owned(), pane.clone());
            pane
        };
        let weak = cx.weak_entity();
        let key = id.to_owned();
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
            if matches!(event, docking::PaneLifecycle::Active(true)) {
                let _ = weak.update(cx, |desktop, cx| {
                    desktop.sidebar = Sidebar::Chats;
                    desktop.selected = Some(task_id.clone());
                    desktop.dock_current = Some(key.clone());
                    cx.notify();
                });
            }
        });
        let focus = cx.focus_handle();
        let content = cx.new(|_| PaneFocusFrame { content: pane.into(), focus: focus.clone() }).into();
        Ok(docking::PaneSpec { id: id.to_owned(), title: "Git Changes".into(), content, focus, lifecycle: Some(lifecycle), closable: true })
    }

    fn open_task_git_below(&mut self, task_id: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let origin = format!("chat:{task_id}");
        let id = git_workspace::next_id(task_id, |id| self.dock.read(cx).contains(id));
        let spec = self.task_git_spec(&id, window, cx)?;
        let result = self.dock.update(cx, |dock, cx| dock.add_below(spec, &origin, window, cx));
        if let Err(error) = result { self.git_panes.remove(&id); return Err(error); }
        self.selected = Some(task_id.to_owned());
        self.sidebar = Sidebar::Chats;
        self.dock_current = Some(id);
        cx.notify();
        Ok(())
    }

    fn auxiliary_spec(
        &self,
        surface: Sidebar,
        title: &str,
        content: AnyView,
        cx: &mut Context<Self>,
    ) -> docking::PaneSpec {
        let stable_id = format!("route:{}", title.to_ascii_lowercase());
        let weak = cx.weak_entity();
        let key = stable_id.clone();
        let browser = if surface == Sidebar::Browser {
            self.browser.clone()
        } else {
            None
        };
        let mail = if surface == Sidebar::Mail {
            self.mail.clone()
        } else {
            None
        };
        let subagents = if surface == Sidebar::Subagents { self.subagents.clone() } else { None };
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, window, cx| {
            if let Some(pane) = &subagents {
                match event {
                    docking::PaneLifecycle::Visible(visible) => pane.update(cx, |pane, cx| pane.set_visible(visible, cx)),
                    docking::PaneLifecycle::Removed => pane.update(cx, |pane, cx| pane.set_visible(false, cx)),
                    _ => {}
                }
            }
            if let Some(browser) = &browser {
                browser.update(cx, |browser, cx| match event {
                    docking::PaneLifecycle::Visible(true) => browser.show(cx),
                    docking::PaneLifecycle::Visible(false) => browser.hide(cx),
                    docking::PaneLifecycle::Removed => browser.close(cx),
                    _ => {}
                });
            }
            if let Some(mail) = &mail {
                if matches!(
                    event,
                    docking::PaneLifecycle::Visible(false) | docking::PaneLifecycle::Removed
                ) {
                    mail.update(cx, |mail, cx| mail.deactivate(window, cx));
                }
            }
            if matches!(event, docking::PaneLifecycle::Active(true)) {
                let _ = weak.update(cx, |desktop, cx| {
                    if surface == Sidebar::Archived && desktop.sidebar != surface {
                        if let Some(pane) = &desktop.archives {
                            pane.update(cx, |pane, cx| pane.activate(window, cx));
                        }
                    }
                    desktop.sidebar = surface;
                    desktop.dock_current = Some(key.clone());
                    cx.notify();
                });
            }
            if surface == Sidebar::Browser && matches!(event, docking::PaneLifecycle::Removed) {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |desktop, _| {
                        desktop.browser = None;
                        desktop.browser_subscription = None;
                        desktop.browser_restore_revision = None;
                    });
                });
            }
        });
        let focus = cx.focus_handle();
        let content = cx
            .new(|_| PaneFocusFrame {
                content,
                focus: focus.clone(),
            })
            .into();
        docking::PaneSpec {
            id: stable_id,
            title: title.to_owned().into(),
            content,
            focus,
            lifecycle: Some(lifecycle),
            closable: true,
        }
    }
    fn auxiliary_dock(
        &mut self,
        surface: Sidebar,
        title: &str,
        content: AnyView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<docking::DockWorkspace> {
        let stable_id = format!("route:{}", title.to_ascii_lowercase());
        if self.dock_current.as_ref() != Some(&stable_id) {
            let spec = self.auxiliary_spec(surface, title, content, cx);
            self.dock.update(cx, |dock, cx| {
                if !dock.focus(&stable_id, window, cx) {
                    let _ = dock.add(
                        spec,
                        gpui_kit::component::dock::DockPlacement::Center,
                        None,
                        window,
                        cx,
                    );
                    dock.focus(&stable_id, window, cx);
                }
                dock.set_shown(true, window, cx);
            });
            self.dock_current = Some(stable_id);
        }
        self.dock.clone()
    }
    fn ensure_chat_pane(
        &mut self,
        task_id: &str,
        service: DesktopService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<chat::ChatPane> {
        let pane = if let Some(pane) = self.chat_panes.borrow().get(task_id).cloned() {
            Some(pane)
        } else {
            None
        };
        let pane = pane.unwrap_or_else(|| {
            let pane = cx.new(|cx| {
                chat::ChatPane::new(
                    task_id.to_owned(),
                    service,
                    self.snapshot.clone(),
                    self.revision.clone(),
                    window,
                    cx,
                )
            });
            pane.update(cx, |pane, cx| pane.set_dock_visible(false, cx));
            if let Some(state) = self.native_chat_panes.get(task_id) {
                pane.update(cx, |pane, cx| {
                    pane.restore_secondary_sidebar(state, window, cx)
                });
            }
            if let Some(text) = self.native_drafts.get(task_id) {
                pane.update(cx, |pane, cx| {
                    pane.restore_draft(text, window, cx);
                });
            }
            if let Some(state) = self.native_attachment_drafts.get(task_id) {
                pane.update(cx, |pane, cx| {
                    pane.restore_attachment_draft(task_id, state, cx)
                });
            }
            let known = self.publication_index.as_ref().map(|index| index.task_history(task_id))
                .unwrap_or_else(|| rows(&self.snapshot, "messages").into_iter()
                    .filter(|message| field(message, "taskId") == task_id).collect());
            pane.update(cx, |pane, cx| pane.reconcile_history(known, true, cx));
            let subscription =
                cx.subscribe_in(&pane, window, |view, _, event, window, cx| match event {
                    chat::ChatEvent::SecondarySidebarChanged { task_id, state } => {
                        if view.native_chat_panes.get(task_id) != Some(state) {
                            view.native_chat_panes
                                .insert(task_id.clone(), state.clone());
                            view.dirty_chat_pane_ids.insert(task_id.clone());
                            view.queue_preferences_save(cx);
                        }
                    }
                    chat::ChatEvent::OpenBrowser { url, task_id } => { if let Err(error) = view.open_chat_browser_link(task_id, url, window, cx) { view.error = Some(error); cx.notify(); } },
                    chat::ChatEvent::Refresh => view.refresh(cx),
                    chat::ChatEvent::Archived(task_id) => {
                        view.removed_dock_ids.push(format!("chat:{task_id}"));
                        if view.selected.as_ref() == Some(task_id) {
                            view.selected = None;
                            view.dock_current = None;
                        }
                        view.refresh(cx);
                        cx.notify();
                    }
                    chat::ChatEvent::SnapshotUpdated(_snapshot) => {
                        // Command responses are domain snapshots without a journal
                        // cursor. Fetch the authoritative delta rather than merging
                        // an unversioned response over a newer native cache.
                        view.refresh(cx);
                    }
                    chat::ChatEvent::DraftChanged { task_id, text } => {
                        view.update_native_draft(task_id, text, cx)
                    }
                    chat::ChatEvent::AttachmentDraftChanged { task_id, state } => {
                        view.update_native_attachment_draft(task_id, state, cx)
                    }
                    chat::ChatEvent::HistoryPage { task_id, page } => {
                        view.accept_history_page(task_id, page.clone(), cx)
                    }
                    chat::ChatEvent::Focused(id) => {
                        view.selected = Some(id.clone());
                        view.sidebar = Sidebar::Chats;
                        cx.notify();
                    }
                    chat::ChatEvent::Git(id) => {
                        if let Err(error) = view.open_task_git_below(id, window, cx) {
                            view.error = Some(error);
                            cx.notify();
                        }
                    }
                    chat::ChatEvent::Terminal { task_id, command } => {
                        if let Some(service) = view.service.clone() {
                            if view.terminal.is_none() {
                                view.terminal = Some(view.new_terminal_manager(service,window,cx));
                            }
                            if let Some(terminal) = &view.terminal {
                                terminal.update(cx, |terminal, cx| {
                                    terminal.set_context(&view.snapshot, Some(task_id.clone()), cx);
                                    terminal.open_command(task_id, command.as_deref(), cx);
                                });
                            }
                            view.selected = Some(task_id.clone());
                            view.sidebar = Sidebar::Terminal;
                            view.dock_current = None;
                            cx.notify();
                        } else {
                            view.error = Some("The experimental backend is not connected.".into());
                            cx.notify();
                        }
                    }
                    chat::ChatEvent::OpenTask(id) => {
                        if !view.hidden_task_ids.contains(id)
                            && rows(&view.snapshot, "tasks")
                                .iter()
                                .any(|task| field(task, "id") == *id)
                        {
                            view.selected = Some(id.clone());
                            view.sidebar = Sidebar::Chats;
                            view.draft_creation_open = false;
                            view.dock_current = None;
                            cx.notify();
                        }
                    }
                    chat::ChatEvent::DelegateTask(id) => {
                        view.open_chat_draft(None, window, cx);
                        if let Some(draft) = view.draft_creation.clone() {
                            let result =
                                draft.update(cx, |draft, cx| draft.set_parent_task(id, window, cx));
                            if let Err(error) = result {
                                draft.update(cx, |draft, cx| draft.set_error(Some(error), cx));
                            }
                        }
                    }
                    chat::ChatEvent::Settings(id) => {
                        view.selected = Some(id.clone());
                        view.sidebar = Sidebar::Settings;
                        view.dock_current = None;
                        cx.notify();
                    }
                });
            self.chat_subscriptions
                .insert(task_id.to_owned(), subscription);
            self.chat_panes
                .borrow_mut()
                .insert(task_id.to_owned(), pane.clone());
            // Compact snapshots omit message bodies. Even an idle parent
            // needs indexed child transcripts when its pane is first opened.
            self.queue_native_publication(PublicationChanges {
                message_tasks: HashSet::from([task_id.to_owned()]),
                ..Default::default()
            }, cx);
            pane
        });
        pane
    }
    fn render_chat_workspace(
        &mut self,
        sidebar: AnyElement,
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let live_ids = if self.pending_draft_chats.is_empty() { Vec::new() } else { self.dock.read(cx).serialize(cx).ok().and_then(|layout| docking::DockWorkspace::saved_ids(&layout).ok()).unwrap_or_default() };
        if let Some((task_id, _)) = self.pending_draft_chats.iter().find(|(task_id, origin)| live_ids.contains(origin) && rows(&self.snapshot, "tasks").iter().any(|task| field(task, "id") == **task_id)) {
            self.selected = Some(task_id.clone());
            self.dock_current = None;
        }
        let hidden_selection = self
            .selected
            .as_ref()
            .filter(|task_id| self.hidden_task_ids.contains(*task_id))
            .cloned();
        if let Some(hidden_selection) = hidden_selection {
            self.selected = None;
            let hidden_chat_id = format!("chat:{hidden_selection}");
            if self.dock_current.as_deref() == Some(hidden_chat_id.as_str()) {
                self.dock_current = None;
            }
        }
        if let (Some(task_id), Some(service)) = (self.selected.clone(), self.service.clone()) {
            let pane = self.ensure_chat_pane(&task_id, service, window, cx);
            let stable_id = format!("chat:{task_id}");
            if self.dock_current.as_ref() != Some(&stable_id)
                && !self.dock_current.as_deref().is_some_and(|id| git_workspace::task_id(id).is_some() || neovim_workspace::parse_id(id).is_some() || neovim_workspace::parse_files_id(id).is_some()) {
                let weak = cx.weak_entity();
                let selected_id = task_id.clone();
                let voice_pane = pane.downgrade();
                let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
                    if let docking::PaneLifecycle::Visible(visible) = event {
                        activity_effects::set_pane_visible(&selected_id, visible, cx);
                    }
                    if matches!(
                        event,
                        docking::PaneLifecycle::Visible(false) | docking::PaneLifecycle::Removed
                    ) {
                        let _ = voice_pane.update(cx, |pane, cx| pane.deactivate_voice(cx));
                    }
                    if matches!(event, docking::PaneLifecycle::Active(true)) {
                        let _ = weak.update(cx, |desktop, cx| {
                            desktop.sidebar = Sidebar::Chats;
                            desktop.selected = Some(selected_id.clone());
                            desktop.dock_current = Some(format!("chat:{selected_id}"));
                            cx.notify();
                        });
                    }
                });
                let title = rows(&self.snapshot, "tasks")
                    .iter()
                    .find(|task| field(task, "id") == task_id)
                    .map(|task| field(task, "title"))
                    .unwrap_or_else(|| "Chat".into());
                let focus = pane.focus_handle(cx);
                self.dock.update(cx, |dock, cx| {
                    if !dock.focus(&stable_id, window, cx) {
                        let _ = dock.add(
                            docking::PaneSpec {
                                id: stable_id.clone(),
                                title: title.into(),
                                content: pane.into(),
                                focus,
                                lifecycle: Some(lifecycle),
                                closable: true,
                            },
                            gpui_kit::component::dock::DockPlacement::Center,
                            None,
                            window,
                            cx,
                        );
                        dock.focus(&stable_id, window, cx);
                    }
                    dock.set_shown(true, window, cx);
                });
                if let Some(origin) = self.pending_draft_chats.remove(&task_id) {
                    self.pending_empty_session = Some((origin, task_id.clone()));
                }
                if self.pending_empty_session.as_ref().is_some_and(|(_, requested)| requested == &task_id) {
                    if let Some((origin, _)) = self.pending_empty_session.take() {
                        if self.dock.update(cx, |dock, cx| dock.replace_pane(&origin, &stable_id, window, cx)) {
                            self.draft_panes.borrow_mut().remove(&origin);
                            self.draft_subscriptions.remove(&origin);
                        } else { self.error = Some("Could not place this chat in the selected pane. The chat remains open.".into()); }
                    }
                }
                self.dock_current = Some(stable_id);
            }
        }
        for pane in self.chat_panes.borrow().values() {
            pane.update(cx, |pane, cx| pane.set_composed_palette(palette, cx));
        }
        let mut content = div().flex_1().h_full().min_w_0();
        if !self.dock.read(cx).has_panes() {
            content = content.bg(self.workspace_route_background(palette, cx)).child(self.dock.clone());
        } else {
            content = content.child(self.dock.clone());
        }
        let workspace = self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx);
        // Each non-overlapping pane and the sidebar own their single surface
        // fill. A second workspace fill here would compound the glass alpha.
        div()
            .size_full()
            .flex()
            .text_color(rgb(palette.ink))
            .font_weight(FontWeight(
                self.snapshot["settings"]["interfaceFontWeight"]
                    .as_u64()
                    .unwrap_or(400)
                    .clamp(300, 700) as f32,
            ))
            .child(workspace)
    }
}

impl Desktop {
    fn refresh_wallpaper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = matches!(self.snapshot["settings"]["windowSurface"].as_str(), Some("glass" | "translucent"));
        self.wallpaper_backdrop.update(cx, |backdrop, cx| backdrop.refresh(window, enabled, cx));
        if wallpaper_theme::mode() != wallpaper_theme::Mode::Manual {
            self.wallpaper_accent.update(cx, |sampler, cx| sampler.refresh(window, cx));
        }
    }
    fn invalidate_surface_blends(&mut self, cx: &mut Context<Self>) {
        gpui_kit::component::resizable::set_separator_blend_mode(crate::surface_blend::separator_mode());
        for pane in self.chat_panes.borrow().values() { pane.update(cx, |_, cx| cx.notify()); }
        for pane in self.terminal_panes.values() { pane.update(cx, |_, cx| cx.notify()); }
        if let Some(pane) = &self.terminal { pane.update(cx, |_, cx| cx.notify()); }
        if let Some(pane) = &self.browser { pane.update(cx, |_, cx| cx.notify()); }
        self.dock.update(cx, |dock, cx| {
            dock.refresh_activity(cx);
            cx.notify();
        });
        cx.notify();
    }

    fn render_surface(&mut self, palette: design::Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        for id in std::mem::take(&mut self.removed_dock_ids) {
            self.dock.update(cx, |dock, cx| {
                dock.remove(&id, window, cx);
            });
        }

        if self.close_pending && !self.preferences_busy {
            self.request_close(window, cx);
        }
        if !self.closing {
            self.restore_preferences(window, cx);
            self.apply_saved_sidebar_width(window, cx);
            if self.preferences.as_ref().is_some_and(|preferences| {
                preferences.data.appearance.warm_paper != (self.native_theme_preset == "reference")
                    || preferences.data.appearance.last_route.as_deref()
                        != Self::palette_routes()
                            .into_iter()
                            .find(|(_, _, surface)| *surface == self.sidebar)
                            .map(|(id, _, _)| id)
                    || preferences
                        .data
                        .appearance
                        .sidebar_width
                        .is_some_and(|width| (self.current_sidebar_width(cx) - width).abs() >= 0.5)
            }) {
                if !self.preferences_dirty {
                    self.queue_preferences_save(cx);
                }
            }
        }
        let font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let ready = self.service.is_some()
            && self.preferences.is_some()
            && !self.preferences_restore
            && !self.preferences_restoring
            && !self.closing;
        self.dock.update(cx, |dock, cx| {
            dock.set_empty_presentation(
                palette,
                font_ratio,
                self.busy || self.empty_terminal_pending || !ready,
                ready,
                cx,
            )
        });
        self.dock.update(cx, |dock, cx| dock.set_empty_sessions(&self.empty_sessions, cx));
        if self.closing {
            return div()
                .size_full()
                .bg(self.workspace_route_background(palette, cx))
                .child("Closing native surfaces…");
        }
        if let Some(browser) = &self.browser {
            let font_ratio = self.snapshot["settings"]["interfaceFontSize"]
                .as_f64()
                .unwrap_or(14.)
                .clamp(8., 32.) as f32
                / 14.;
            browser.update(cx, |browser, cx| {
                browser.set_appearance(palette, font_ratio, cx);
            });
        }
        if self.mail.is_some() && self.mail_context_task != self.selected {
            let selected = self.selected.clone();
            self.mail.as_ref().unwrap().update(cx, |mail, cx| {
                mail.set_context(&self.snapshot, selected.clone(), cx)
            });
            self.mail_context_task = selected;
        }
        // Retained split panes keep receiving state even when another route is active.
        if self.git_panes_revision != self.revision {
            for (id, pane) in &self.git_panes {
                if let Some(task) = git_workspace::task_id(id) {
                    pane.update(cx, |pane, cx| pane.set_context(&self.snapshot, task, cx));
                }
            }
            self.git_panes_revision = self.revision.clone();
        }
        let context = (self.revision.clone(), self.selected.clone());
        if let Some(pane) = &self.git {
            if self.git_context.as_ref() != Some(&context) {
                pane.update(cx, |pane, cx| {
                    pane.set_context(
                        &self.snapshot,
                        self.selected.as_deref().unwrap_or_default(),
                        cx,
                    )
                });
                self.git_context = Some(context.clone());
            }
        }
        if let Some(pane) = &self.subagents {
            if self.subagents_context.as_ref() != Some(&context) {
                let task = self.selected.as_deref().unwrap_or_default();
                pane.update(cx, |pane, cx| {
                    if let Some(items) = self.collaboration_records.get(task) {
                        pane.set_prepared_context(self.snapshot.clone(), task, items.clone(), self.collaboration_transcripts[task].clone(), cx);
                    } else { pane.set_context(self.snapshot.clone(), task, cx); }
                });
                self.subagents_context = Some(context.clone());
            }
        }
        if let Some(pane) = &self.terminal {
            if self.terminal_context.as_ref() != Some(&context) {
                pane.update(cx, |pane, cx| {
                    pane.set_context(&self.snapshot, self.selected.clone(), cx)
                });
                self.terminal_context = Some(context);
            }
        }
        if let Some(pane) = &self.archives {
            if self.archives_revision != self.revision {
                pane.update(cx, |pane, cx| pane.set_snapshot(&self.snapshot, cx));
                self.archives_revision = self.revision.clone();
            }
        }
        if let Some(pane) = &self.agent_editor {
            if self.agent_editor_revision != self.revision {
                pane.update(cx, |pane, cx| pane.set_snapshot(&self.snapshot, window, cx));
                self.agent_editor_revision = self.revision.clone();
            }
        }
        if let Some(pane) = &self.schedules {
            if self.schedules_revision != self.revision {
                pane.update(cx, |pane, cx| pane.set_snapshot(&self.snapshot, cx));
                self.schedules_revision = self.revision.clone();
            }
        }
        if let Some(pane) = &self.usage {
            if self.usage_revision != self.revision {
                pane.update(cx, |pane, cx| pane.set_snapshot(&self.snapshot, cx));
                self.usage_revision = self.revision.clone();
            }
        }
        self.ensure_sidebar_usage(window, cx);
        let sidebar_snapshot_changed = self
            .sidebar_usage_snapshot
            .as_ref()
            .is_none_or(|snapshot| !Rc::ptr_eq(snapshot, &self.snapshot))
            || self.sidebar_usage_revision != self.revision;
        if sidebar_snapshot_changed {
            if let Some(widget) = &self.sidebar_usage {
                widget.update(cx, |widget, cx| {
                    widget.set_snapshot_shared(self.snapshot.clone(), self.revision.as_deref(), cx)
                });
                let retired_snapshot = self.sidebar_usage_snapshot.replace(self.snapshot.clone());
                self.sidebar_usage_revision = self.revision.clone();
                if let Some(snapshot) = retired_snapshot {
                    retire_snapshot(snapshot, cx);
                }
            }
        }
        let agents: Vec<_> = rows(&self.snapshot, "agents")
            .into_iter()
            .filter(|a| a["internal"] != true)
            .collect();
        let tasks: Vec<_> = rows(&self.snapshot, "tasks")
            .into_iter()
            .filter(|t| t["archived"] != true)
            .collect();
        shell::set_task_markers_visible(
            (!self.sidebar_is_hidden() || self.sidebar_motion.active())
                && !self.sidebar_collapsed && !(self.sidebar_is_hidden() && self.sidebar_motion_compact)
                && !self.preferences_open && !session_lock::locked(cx) && !self.closing, self.sidebar_view, cx);
        let sidebar = if self.sidebar_is_hidden() && !self.sidebar_motion.active() { div().into_any_element() } else if self.sidebar_collapsed || (self.sidebar_is_hidden() && self.sidebar_motion_compact) {
            self.render_compact_sidebar(&agents, &tasks, palette, window, cx)
        } else { self.render_sidebar(&agents, &tasks, palette, window, cx) };
        let sidebar = div()
            .id("native-preferences-sidebar")
            .track_focus(&self.sidebar_focus)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .h_full()
            .child(sidebar);
        let sidebar = sidebar.into_any_element();
        if self.startup_pending {
            if self.startup_preview.is_none() {
                if let Some(cache) = &self.startup_cache {
                    match startup_workspace::restore(cache, palette, window, cx) {
                        Ok(preview) => self.startup_preview = Some(preview),
                        Err(error) => {
                            eprintln!("[native-startup] cache_ignored={error}");
                            self.startup_cache = None;
                        }
                    }
                }
            }
            let content = div().flex_1().h_full().min_w_0().relative()
                .when_some(self.startup_preview.clone(), |content, preview| content.child(preview))
                .when_some(self.error.as_deref().and_then(chat::connection_recovery::visible_error).map(str::to_owned), |content, error| content.child(div().absolute().bottom_4().left_4().p_3().text_sm().child(error)));
            let workspace = self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx);
            return div().size_full().bg(self.workspace_route_background(palette, cx))
                .child(inert_workspace::InertWorkspace::new(workspace));
        }
        if self.sidebar == Sidebar::Archived {
            self.ensure_archives(window, cx);
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(pane) = self.archives.clone() {
                if self.archives_revision != self.revision {
                    pane.update(cx, |pane, cx| pane.set_snapshot(&self.snapshot, cx));
                    self.archives_revision = self.revision.clone();
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Archived,
                    "Archives",
                    pane.into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening private archived chats…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Git {
            if self.git.is_none() {
                if let Some(service) = self.service.clone() {
                    self.git =
                        Some(cx.new(|cx| git::GitPane::new(service, &self.snapshot, window, cx)));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(pane) = &self.git {
                let context = (self.revision.clone(), self.selected.clone());
                if self.git_context.as_ref() != Some(&context) {
                    pane.update(cx, |pane, cx| {
                        pane.set_context(
                            &self.snapshot,
                            self.selected.as_deref().unwrap_or_default(),
                            cx,
                        )
                    });
                    self.git_context = Some(context);
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Git,
                    "Repository",
                    pane.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening repository view…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Subagents {
            if self.subagents.is_none() {
                if let Some(service) = self.service.clone() {
                    self.subagents = Some(cx.new(|cx| {
                        collaboration::SubagentPane::new(service, self.snapshot.clone(), window, cx)
                    }));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(pane) = &self.subagents {
                let context = (self.revision.clone(), self.selected.clone());
                if self.subagents_context.as_ref() != Some(&context) {
                    pane.update(cx, |pane, cx| {
                        let task = self.selected.as_deref().unwrap_or_default();
                        if let Some(items) = self.collaboration_records.get(task) {
                            pane.set_prepared_context(self.snapshot.clone(), task, items.clone(), self.collaboration_transcripts[task].clone(), cx);
                        } else { pane.set_context(self.snapshot.clone(), task, cx); }
                    });
                    self.subagents_context = Some(context);
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Subagents,
                    "Subagents",
                    pane.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening collaboration service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Agents {
            if self.agent_editor.is_none() {
                if let Some(service) = self.service.clone() {
                    self.agent_editor = Some(cx.new(|cx| {
                        agents::AgentHostPane::new(
                            service,
                            self.snapshot.as_ref().clone(),
                            window,
                            cx,
                        )
                    }));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(editor) = &self.agent_editor {
                if self.agent_editor_revision != self.revision {
                    editor.update(cx, |editor, cx| {
                        editor.set_snapshot(&self.snapshot, window, cx)
                    });
                    self.agent_editor_revision = self.revision.clone();
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Agents,
                    "Agents",
                    editor.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening agent settings…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Schedules {
            if self.schedules.is_none() {
                if let Some(service) = self.service.clone() {
                    self.schedules = Some(cx.new(|cx| {
                        schedules::SchedulePane::new(service, &self.snapshot, window, cx)
                    }));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(usage) = &self.schedules {
                if self.schedules_revision != self.revision {
                    usage.update(cx, |usage, cx| usage.set_snapshot(&self.snapshot, cx));
                    self.schedules_revision = self.revision.clone();
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Schedules,
                    "Schedules",
                    usage.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening schedules…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Usage {
            if self.usage.is_none() {
                if let Some(service) = self.service.clone() {
                    self.usage = Some(
                        cx.new(|cx| usage::UsagePane::new(service, &self.snapshot, window, cx)),
                    );
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(usage) = &self.usage {
                if self.usage_revision != self.revision {
                    usage.update(cx, |usage, cx| usage.set_snapshot(&self.snapshot, cx));
                    self.usage_revision = self.revision.clone();
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Usage,
                    "Usage",
                    usage.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening usage service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Terminal {
            if self
                .dock_current
                .as_deref()
                .is_some_and(|key| key.starts_with("terminal:"))
            {
                let content = div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.dock.clone())
                    .into_any_element();
                return div()
                    .size_full()
                    .flex()
                    .bg(self.workspace_route_background(palette, cx))
                    .text_color(rgb(palette.ink))
                    .child(self.sidebar_split(sidebar, content, palette, window, cx));
            }
            if self.terminal.is_none() {
                if let Some(service) = self.service.clone() {
                    self.terminal = Some(self.new_terminal_manager(service,window,cx));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(terminal) = &self.terminal {
                let context = (self.revision.clone(), self.selected.clone());
                if self.terminal_context.as_ref() != Some(&context) {
                    terminal.update(cx, |terminal, cx| {
                        terminal.set_context(&self.snapshot, self.selected.clone(), cx)
                    });
                    self.terminal_context = Some(context);
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Terminal,
                    "Terminal",
                    terminal.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening terminal service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Channels {
            if self.channels.is_none() {
                if let Some(service) = self.service.clone() {
                    let channels = cx.new(|cx| ChannelPane::new(service, window, cx));
                    self.channel_subscription =
                        Some(cx.subscribe(&channels, |view, _, event, cx| match event {
                            channels::ChannelEvent::Refresh => view.refresh(cx),
                            channels::ChannelEvent::HistoryPage {
                                channel,
                                before,
                                page,
                            } => view.accept_channel_page(
                                channel,
                                before.as_deref(),
                                page.clone(),
                                cx,
                            ),
                        }));
                    self.channels = Some(channels);
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(channels) = &self.channels {
                if self.channels_revision != self.revision {
                    channels.update(cx, |channels, cx| {
                        channels.sync(
                            self.snapshot.clone(),
                            self.revision.as_deref().unwrap_or_default(),
                            false,
                            &[],
                            &self.channel_cursors,
                            cx,
                        )
                    });
                    self.channels_revision = self.revision.clone();
                }
                content = content.child(self.auxiliary_dock(
                    Sidebar::Channels,
                    "Channels",
                    channels.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening channel service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Mail {
            if self.mail.is_none() {
                if let Some(service) = self.service.clone() {
                    let mail = cx.new(|cx| MailPane::new(service, window, cx));
                    mail.update(cx, |mail, cx| {
                        mail.set_context(&self.snapshot, self.selected.clone(), cx)
                    });
                    self.mail_context_task = self.selected.clone();
                    self.mail = Some(mail);
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(mail) = &self.mail {
                content = content.child(self.auxiliary_dock(
                    Sidebar::Mail,
                    "Mail",
                    mail.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening native mail service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Projects {
            if self.workspace.is_none() {
                if let Some(service) = self.service.clone() {
                    self.workspace = Some(cx.new(|cx| {
                        WorkspacePane::new(service, self.snapshot.clone(), window, cx)
                    }));
                }
            }
            let mut content = div().flex_1().h_full().min_w_0();
            if let Some(workspace) = &self.workspace {
                content = content.child(self.auxiliary_dock(
                    Sidebar::Projects,
                    "Projects",
                    workspace.clone().into(),
                    window,
                    cx,
                ));
            } else {
                content = content.child("Opening project service…");
            }
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        if self.sidebar == Sidebar::Settings {
            self.dock
                .update(cx, |dock, cx| dock.set_shown(false, window, cx));
            let settings_content = div()
                .flex_1()
                .h_full()
                .min_w_0()
                .child(self.render_settings(window, cx))
                .into_any_element();
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, settings_content, palette, window, cx));
        }
        if self.sidebar == Sidebar::Browser {
            if self.browser.is_none() {
                self.ensure_browser(window, cx);
            }
            if self.preferences.is_some()
                && !self.preferences_restore
                && !self.preferences_restoring
                && !self
                    .dock_current
                    .as_deref()
                    .is_some_and(|id| id.starts_with("browser:"))
            {
                if let Err(error) = self.open_selected_browser(window, cx) {
                    self.error = Some(error);
                }
            }
            let content = div().flex_1().h_full().min_w_0().child(self.dock.clone());
            return div()
                .size_full()
                .flex()
                .bg(self.workspace_route_background(palette, cx))
                .text_color(rgb(palette.ink))
                .font_weight(FontWeight(
                    self.snapshot["settings"]["interfaceFontWeight"]
                        .as_u64()
                        .unwrap_or(400)
                        .clamp(300, 700) as f32,
                ))
                .child(self.sidebar_split(sidebar, content.into_any_element(), palette, window, cx));
        }
        self.render_chat_workspace(sidebar, palette, window, cx)
    }
}
impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_browser_requests(window, cx);
        if let Some(enabled) = self.pending_auto_size_panes.take() {
            self.dock.update(cx, |dock, cx| dock.set_auto_size_panes(enabled, window, cx));
        }
        // Begin before any child renders: NativePanel records highlight
        // candidates during render, before deferred paint callbacks run.
        native_pane_highlight::begin_frame(window);
        if self.session_lock.read(cx).is_locked() {
            native_pane_highlight::clear(window);
            native_popup_occlusion::clear(window);
            let palette=self.apply_native_theme(window,cx);
            self.session_lock.update(cx,|lock,_|lock.set_palette(palette));
            self.dock.update(cx,|dock,cx|dock.set_shown(false,window,cx));
            let workspace = self.render_surface(palette, window, cx);
            return div().size_full().relative()
                .child(inert_workspace::InertWorkspace::new(workspace.into_any_element()))
                .child(div().absolute().inset_0().child(self.session_lock.clone()));
        }
        let palette = self.apply_native_theme(window, cx);
        for session in self.neovim_sessions.values() {
            session.update(cx, |session, cx| session.set_theme(palette, &self.snapshot["settings"], cx));
        }
        // Synchronize only when the surface setting changes. No wallpaper I/O per frame.
        let wallpaper_enabled = matches!(self.snapshot["settings"]["windowSurface"].as_str(), Some("glass" | "translucent"));
        if self.wallpaper_backdrop.read(cx).enabled() != wallpaper_enabled {
            self.wallpaper_backdrop.update(cx, |backdrop, cx| backdrop.refresh(window, wallpaper_enabled, cx));
        }
        window.set_rem_size(design::ui_scaled_px(16.));
        let new_scale = design::interface_scale();
        if self.sidebar_motion.active() && self.sidebar_resize.read(cx).is_resizing() {
            self.sidebar_motion.cancel();
            self.sidebar_resize_suppressed = false;
            self.sidebar_hidden = false;
        }
        let resize_target = {
            let state = self.sidebar_resize.read(cx);
            if state.sizes().len() == 2 && !self.sidebar_resize_suppressed && !self.sidebar_is_hidden() {
                let actual = state.sizes()[0] / design::ui_px(1., self.sidebar_scale);
                sidebar_resize_target(
                    actual,
                    self.sidebar_collapsed,
                    state.is_resizing(),
                    self.sidebar_scale,
                )
            } else {
                None
            }
        };
        if let Some(desired) = resize_target {
            self.sidebar_resize_suppressed = true;
            self.sidebar_resize.update(cx, |state, cx| {
                state.resize_panel_for_layout(0, design::ui_px(desired, self.sidebar_scale), window, cx)
            });
            self.sidebar_resize_suppressed = false;
        }
        if self.sidebar_scale != new_scale {
            self.finish_sidebar_motion(window, cx);
            self.dock.update(cx, |dock, cx| {
                dock.set_tab_appearance(&self.snapshot["settings"], new_scale, cx)
            });
            // Persist the splitter in design units, while measuring/hit testing
            // in the same scaled GPUI pixels as the rest of the native tree.
            if !self.sidebar_resize.read(cx).is_resizing() {
                let width = if self.sidebar_is_hidden() {0.} else if self.sidebar_collapsed { design::compact_sidebar_width_design(new_scale) } else { expanded_sidebar_width_for_scale(self.current_sidebar_width(cx),new_scale) };
                self.sidebar_scale = new_scale;
                if self.sidebar_resize.read(cx).sizes().len() == 2 {
                    self.sidebar_resize_suppressed = true;
                    self.sidebar_resize.update(cx, |state, cx| {
                        state.resize_panel_for_layout(0, design::ui_px(width, new_scale), window, cx);
                    });
                    self.sidebar_resize_suppressed = false;
                }
            }
        }
        let open = !self.closing
            && !self.close_pending
            && self
                .palette
                .as_ref()
                .is_some_and(|pane| pane.read(cx).is_open());
        if self.preferences_open || open {
            native_pane_highlight::clear(window);
        }
        // Hidden native children must not enter the frame arena behind a modal.
        // Their entities and workspace layout remain retained for dismissal.
        let mut root = div()
            .size_full()
            .relative()
            .on_modifiers_changed(cx.listener(|view, event: &ModifiersChangedEvent, window, cx| {
                view.set_workspace_number_hints(event.modifiers.platform && window.is_window_active(), cx);
                window.refresh();
            }))
            .key_context(if self.snapshot["settings"]["shortcutMode"] == "vim" { "MonitterDesktop shortcutMode=vim" } else { "MonitterDesktop shortcutMode=standard" })
            .child(pointer_guard::PointerReleaseGuard)
            .capture_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" && cx.stop_active_drag(window) {
                    cx.stop_propagation();
                }
            });
        if wallpaper_backdrop::needed(window.is_fullscreen(), self.snapshot["settings"]["windowSurface"].as_str().unwrap_or("opaque")) {
            if let Some(image) = self.wallpaper_backdrop.read(cx).image() {
                // This image is painted above AppKit's backdrop. Restore the
                // configured window tint above it across the entire window,
                // including workspace chrome and the spaces between panes.
                root = root.child(img(image).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))
                    .child(div().absolute().inset_0().bg(design::surface_color(palette.paper)));
            } else {
                // Missing/unsupported wallpaper remains legible while loading or on failure.
                root = root.bg(rgb(palette.paper));
            }
        }
        if open {
            root = root.bg(design::surface_color(palette.paper));
            let palette = self.palette.as_ref().unwrap().clone();
            root = root.child(
                div()
                    .id("native-palette-overlay")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_start()
                    .justify_center()
                    .pt(px(80.))
                    .bg(rgba(0x00000080))
                    .child(palette),
            );
        } else {
            // Keep the workspace painted beneath ordered native backdrop filters.
            let surface = self.render_surface(palette, window, cx);
            if design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern {
                // The sidebar and the workspace column share full window height.
                root = root.child(surface);
            } else {
                root = root.child(div().size_full().flex().flex_col()
                    .child(self.render_workspace_tabs(palette, window, cx))
                    .child(div().flex_1().min_h_0().w_full().child(surface)));
            }
        }
        if self.startup_pending {
            root = div().size_full().relative().child(inert_workspace::InertWorkspace::new(root.into_any_element()));
        }
        if !self.preferences_open {
            if let Some(error) = self.preferences_error.clone() {
                root = root.child(div().id("workspace-layout-error").absolute().top(px(70.)).right(px(16.))
                    .w(px(420.)).max_w_full().p_3().rounded_lg().occlude()
                    .bg(crate::design::surface_color(palette.paper)).border_1().border_color(rgb(palette.line))
                    .child(div().text_color(rgb(0xd35e5e)).child(format!("Workspace layout: {error}")))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(Button::new("reload-native-layout").compact().label("Reload saved workspace")
                            .disabled(self.preferences_busy)
                            .on_click(cx.listener(|view, _, _, cx| view.load_preferences(cx))))
                        .child(Button::new("save-current-native-layout").compact().label("Retry save")
                            .disabled(self.preferences_busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.preferences_restore = false;
                                view.preferences_blocked = false;
                                view.queue_preferences_save(cx);
                            })))
                        .child(Button::new("dismiss-native-layout-error").compact().label("Dismiss")
                            .on_click(cx.listener(|view, _, _, cx| { view.preferences_error = None; cx.notify(); })))))
            }
        }
        if self.preferences_open {
            // Dock corner masks and focus contours are deferred at priorities 2/3.
            // Paint the modal afterward so tile chrome cannot cross its card.
            root = root.child(gpui_kit::deferred(self.render_preferences_modal(window, cx)).with_priority(4));
        }
        if !self.closing && !self.preferences_open {
            if let Some(command_bar) = vim_command::bar(cx) {
                if let Some(element) = command_bar.update(cx, |bar, cx| {
                    bar.set_palette(palette, cx);
                    bar.render_overlay(palette, cx)
                }) {
                    root = root.child(element);
                }
            }
        }
        if std::mem::take(&mut self.startup_first_frame) {
            let stage = if self.startup_preview.is_some() { "cached-workspace-frame" } else { "workspace-frame" };
            root = root.child(canvas(|_, _, _| (), move |_, _, _, _| startup_workspace::mark(stage)).absolute().inset_0());
        }
        if std::mem::take(&mut self.startup_live_frame_pending) {
            root = root.child(canvas(|_, _, _| (), |_, _, _, _| startup_workspace::mark("live-workspace-frame")).absolute().inset_0());
        }
        root
            // Pane highlights are deferred (priorities 1 and 3). Finalize
            // native overlay visibility after those candidates have painted.
            .child(gpui_kit::deferred(canvas(
                |_, _, _| (),
                |_, _, window, _| native_pane_highlight::finish_frame(window),
            ).absolute().inset_0()).with_priority(5))
            .child(gpui_kit::deferred(canvas(
                |_, _, _| (),
                |_, _, window, cx| native_popup_occlusion::finish_frame(window, cx),
            ).absolute().inset_0()).with_priority(usize::MAX - 1))
    }
}

#[cfg(test)]
mod sidebar_resize_tests {
    use super::{expanded_sidebar_width, sidebar_resize_target, sidebar_should_collapse};

    #[test]
    fn expanded_width_caps_saved_and_restored_values_without_clamping_an_active_gesture() {
        assert_eq!(expanded_sidebar_width(557.9), 360.);
        assert_eq!(expanded_sidebar_width(252.), 252.);
        assert_eq!(expanded_sidebar_width(f32::NAN), 252.);
        assert_eq!(sidebar_resize_target(557.9, false, false, 135), Some(360.));
        assert_eq!(sidebar_resize_target(120., false, true, 135), None);
    }

    #[test]
    fn sidebar_icon_threshold_is_120_logical_pixels_at_every_interface_scale() {
        for scale in [80,100,125,150,200] {
            let ratio=f32::from(super::design::ui_px(1.,scale));
            for previous in [false,true] {
                assert!(sidebar_should_collapse(119./ratio,previous,scale));
                assert!(!sidebar_should_collapse(120./ratio,previous,scale));
            }
            assert_eq!(sidebar_resize_target(119./ratio,false,true,scale),None);
            assert_eq!(sidebar_resize_target(120./ratio,false,false,scale),None);
            let compact=super::design::compact_sidebar_width_design(scale);
            assert!(sidebar_should_collapse(compact,true,scale));
            assert!((f32::from(super::design::ui_px(compact,scale))-80.).abs()<0.001);
            let expanded=super::expanded_sidebar_width_for_scale(40.,scale);
            assert!((f32::from(super::design::ui_px(expanded,scale))-120.).abs()<0.001);
        }
    }

}

// -----------------------------------------------------------------
// Keyboard-navigation glue (Cmd/Ctrl+T, Cmd/Ctrl+W, Cmd/Ctrl+1..9).
//
// The actual command surface lives in `native_navigation`. These three
// methods are the only point where desktop policy meets dock geometry:
// they route the action to the focused dock tab group and translate
// failure into a visible error rather than swallowing it.
// -----------------------------------------------------------------

impl Desktop {
    /// Open a native empty tab in the center dock. It remains a real blank
    /// surface and does not create, replace, or stop an agent task.
    fn open_focused_empty_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.closing {
            return Err("Desktop is closing.".into());
        }
        if design::tile_theme() != monitter_lib::experimental_desktop::NativeTileTheme::Tern && self
            .dock
            .read(cx)
            .focused_pane_id(window, cx)
            .is_some_and(|id| browser_workspace::parse_browser_pane_id(&id).is_some())
        {
            let id = self.create_browser_tab(window, cx)?;
            return self.open_browser_tab(id, window, cx);
        }
        self.dock.update(cx, |dock, cx| {
            dock.add_focused_empty_tab(window, cx).map(|_| ())
        })
    }

    /// Split beside the focused pane with a blank surface, without starting work.
    fn open_focused_empty_split(
        &mut self,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.closing { return Err("Desktop is closing.".into()); }
        self.dock.update(cx, |dock, cx| {
            dock.add_focused_empty_split(vertical, window, cx).map(|_| ())
        })
    }

    /// Close the focused tab through the dock's normal Removed lifecycle.
    fn close_focused_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.closing {
            return Err("Desktop is closing.".into());
        }
        if self.neovim_close_focused(window, cx) { return Ok(()); }
        self.dock
            .update(cx, |dock, cx| dock.close_focused_tab(window, cx))
    }

    /// Select the n-th tab (1-based) in the focused pane.
    fn select_focused_tab_at(
        &mut self,
        window: &mut Window,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.closing {
            return Err("Desktop is closing.".into());
        }
        self.dock
            .update(cx, |dock, cx| dock.select_focused_tab_at(index, window, cx))
    }
}

fn merge_native_reload<T: Clone>(
    saved: &BTreeMap<String, T>,
    local: &BTreeMap<String, T>,
    dirty: &HashSet<String>,
) -> BTreeMap<String, T> {
    let mut merged = saved.clone();
    for id in dirty {
        match local.get(id) {
            Some(text) => {
                merged.insert(id.clone(), text.clone());
            }
            None => {
                merged.remove(id);
            }
        }
    }
    merged
}
#[cfg(test)]
mod native_draft_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn sidebar_reload_keeps_local_edits_and_deletion_without_losing_other_panes() {
        let original = NativeChatPanePreferences::default();
        let edited = NativeChatPanePreferences {
            visible: false,
            tab: NativeDetailTab::Timeline,
            width: 416.0,
        };
        let saved = BTreeMap::from([
            ("edited".to_owned(), original.clone()),
            ("removed".to_owned(), original.clone()),
            ("other".to_owned(), original.clone()),
        ]);
        let local = BTreeMap::from([("edited".to_owned(), edited.clone())]);
        let dirty = HashSet::from(["edited".to_owned(), "removed".to_owned()]);
        let merged = merge_native_reload(&saved, &local, &dirty);
        assert_eq!(merged.get("edited"), Some(&edited));
        assert!(!merged.contains_key("removed"));
        assert_eq!(merged.get("other"), Some(&original));
    }
    #[::core::prelude::v1::test]
    fn attachment_reload_preserves_local_add_and_clear_without_losing_other_tasks() {
        let original = NativeAttachmentDraft {
            host_id: "host".into(),
            cwd: "/original".into(),
            attachments: vec![monitter_lib::experimental_desktop::NativeAttachmentRef {
                id: "registered-id".into(),
                name: "draft.txt".into(),
                mime_type: "text/plain".into(),
                size: 42,
            }],
        };
        let edited = NativeAttachmentDraft {
            cwd: "/changed".into(),
            ..original.clone()
        };
        let disk = BTreeMap::from([
            ("edited".into(), original.clone()),
            ("cleared".into(), original.clone()),
            ("other".into(), original.clone()),
        ]);
        let local = BTreeMap::from([("edited".into(), edited.clone())]);
        let merged = merge_native_reload(
            &disk,
            &local,
            &HashSet::from(["edited".into(), "cleared".into()]),
        );
        assert_eq!(merged.get("edited"), Some(&edited));
        assert!(!merged.contains_key("cleared"));
        assert_eq!(merged.get("other"), Some(&original));
    }
    #[::core::prelude::v1::test]
    fn reload_keeps_pending_typing_and_explicit_clear_while_loading_other_saved_drafts() {
        let saved = BTreeMap::from([
            ("edited".into(), "disk version".into()),
            ("cleared".into(), "must not return".into()),
            ("other".into(), "untouched disk draft".into()),
        ]);
        let local = BTreeMap::from([("edited".into(), "new unsent text".into())]);
        let dirty = HashSet::from(["edited".into(), "cleared".into()]);
        let merged = merge_native_reload(&saved, &local, &dirty);
        assert_eq!(
            merged.get("edited").map(String::as_str),
            Some("new unsent text")
        );
        assert!(!merged.contains_key("cleared"));
        assert_eq!(merged.get("other"), saved.get("other"));
    }
    #[::core::prelude::v1::test]
    fn reload_does_not_truncate_pending_multibyte_text_or_discard_over_limit_drafts() {
        let text = "é".repeat(17000);
        let local = BTreeMap::from([("large".into(), text.clone())]);
        let merged =
            merge_native_reload(&BTreeMap::new(), &local, &HashSet::from(["large".into()]));
        assert_eq!(merged["large"], text);
        assert!(
            merged["large"].len() > 32 * 1024,
            "Backend must reject visibly, UI must retain all unsent bytes"
        );
    }
}

impl Desktop {
    fn open_chat_draft(
        &mut self,
        agent_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.service.clone() else {
            self.error = Some("The native backend is not ready.".into());
            cx.notify();
            return;
        };
        let origin = self.dock.read(cx).focused_pane_id(window, cx).filter(|id| id.starts_with("empty:"));
        let destination = self.dock.read(cx).focused_group_node(window, cx);
        let pane = cx.new(|cx| draft_creation::DraftCreationPane::new(service, self.snapshot.as_ref().clone(), window, cx));
        let id = format!("draft:{}", pane.entity_id());
        if let Some(agent) = agent_id { pane.update(cx, |draft, cx| draft.select_agent(agent, window, cx)); }
        let source = id.clone();
        let subscription = cx.subscribe_in(&pane, window, move |view, pane, event: &draft_creation::DraftCreationEvent, window, cx| {
            match event {
                draft_creation::DraftCreationEvent::Changed => {},
                draft_creation::DraftCreationEvent::Send(submission) => view.submit_chat_draft(source.clone(), pane.clone(), submission.clone(), cx),
                draft_creation::DraftCreationEvent::Close => { view.dock.update(cx, |dock, cx| dock.remove(&source, window, cx)); },
            }
        });
        self.draft_panes.borrow_mut().insert(id.clone(), pane.clone());
        self.draft_subscriptions.insert(id.clone(), subscription);
        self.draft_creation = Some(pane.clone());
        self.deactivate_mail(window, cx);
        self.sidebar = Sidebar::Chats;
        self.selected = None;
        let focus = pane.focus_handle(cx);
        let weak = cx.entity().downgrade();
        let active_pane = pane.clone();
        let active_id = id.clone();
        let lifecycle: docking::LifecycleHook = Rc::new(move |event, _, cx| {
            if matches!(event, docking::PaneLifecycle::Active(true)) {
                let _ = weak.update(cx, |view, cx| {
                    view.selected = None;
                    view.sidebar = Sidebar::Chats;
                    view.dock_current = Some(active_id.clone());
                    view.draft_creation = Some(active_pane.clone());
                    cx.notify();
                });
            }
        });
        let result = self.dock.update(cx, |dock, cx| {
            dock.add(docking::PaneSpec { id: id.clone(), title: "New Chat".into(), content: pane.into(), focus, lifecycle: Some(lifecycle), closable: true }, gpui_kit::component::dock::DockPlacement::Center, None, window, cx)?;
            if let Some(origin) = &origin { dock.replace_pane(origin, &id, window, cx); }
            else if let Some(node) = destination { dock.move_pane(&id, gpui_kit::component::dock::InsertTarget::Tabs { node, ix: None, activate: true }, window, cx); }
            dock.set_shown(true, window, cx);
            dock.focus(&id, window, cx);
            Ok::<_, String>(())
        });
        if let Err(error) = result { self.error = Some(error); }
        self.dock_current = Some(id);
        self.draft_creation_open = false;
        cx.notify();
    }
    fn submit_chat_draft(
        &mut self,
        draft_id: String,
        pane: Entity<draft_creation::DraftCreationPane>,
        submission: draft_creation::DraftSubmission,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.service.clone() else {
            return;
        };
        pane.update(cx, |draft, cx| draft.set_busy(true, cx));
        let (sender, receiver) = futures::channel::oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("monitter-draft-send".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut task_id = submission.task_id;
                let result = (|| {
                    if task_id.is_none() {
                        let task = service
                            .invoke("create_task", json!({"input": submission.task_input}))?;
                        let id = field(&task, "id");
                        if id.is_empty() {
                            return Err(
                                "Created task response omitted its ID; refresh before retrying."
                                    .into(),
                            );
                        }
                        task_id = Some(id);
                    }
                    service.invoke(
                        "send_message_fast",
                        json!({"taskId":task_id,"text":submission.text,"attachmentIds":[]}),
                    )?;
                    Ok(())
                })();
                let _ = sender.send((task_id, result));
            });
        if let Err(error) = worker {
            pane.update(cx, |draft, cx| {
                draft.set_busy(false, cx);
                draft.set_error(Some(error.to_string()), cx);
            });
            return;
        }
        cx.spawn(async move |view, cx| {
            let outcome = receiver.await;
            let _ = view.update(cx, |view, cx| {
                match outcome {
                    Ok((task_id, result)) => {
                        pane.update(cx, |draft, cx| {
                            draft.set_busy(false, cx);
                            draft.set_created_task_id(task_id.clone(), cx);
                            draft.set_error(result.as_ref().err().cloned(), cx);
                        });
                        if result.is_ok() {
                            if let Some(id) = &task_id { if view.draft_panes.borrow().contains_key(&draft_id) { view.pending_draft_chats.insert(id.clone(), draft_id.clone()); } }
                            view.sidebar = Sidebar::Chats;
                            view.draft_creation_open = false;

                            view.refresh(cx);
                        }
                    }
                    Err(error) => pane.update(cx, |draft, cx| {
                        draft.set_busy(false, cx);
                        draft.set_error(Some(format!("Draft worker ended: {error}")), cx);
                    }),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn main() {
    startup_workspace::start();
    gpui_kit::application()
        .with_assets(native_icons::DesktopIcons)
        .run(|cx| {
            gpui_kit::init(cx);
            #[cfg(feature = "visual-export")]
            if git::open_preview_if_requested(cx) { return; }
            #[cfg(feature = "visual-export")]
            if design_system_preview::open_if_requested(cx) { return; }
            #[cfg(feature = "visual-export")]
            if notifications::open_preview_if_requested(cx) { return; }
            #[cfg(feature = "visual-export")]
            if browser::open_html_smoke_if_requested(cx) { return; }
            #[cfg(feature = "visual-export")]
            if admin_activity::open_preview_if_requested(cx) { return; }
            #[cfg(feature = "performance-profiling")]
            if performance_scenario::open_if_requested(cx) { return; }
            gpui_kit::component::resizable::set_separator_blend_mode(crate::surface_blend::separator_mode());
            native_motion::initialize(cx);
            streaming_display::initialize(cx);
            native_menu::install(cx);
            #[cfg(feature = "visual-export")]
            visual_export::install(cx);
            if let Err(error) = monitter_gpui_desktop::fonts::register(cx) {
                eprintln!("{error}");
            }
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-enter", gpui_kit::component::input::Enter { secondary: true, shift: false }, Some("MonitterChat && Input")),
            ]);
            let (window_handle, desktop) = gpui_kit::open_window(
                WindowOptions {
                    display_id: window_state::display_id(cx),
                    window_bounds: Some(window_state::initial(cx).unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(gpui_kit::px(1200.), gpui_kit::px(820.)),
                        cx,
                    )))),
                    app_owns_titlebar_drag: true,
                    titlebar: Some(TitlebarOptions {
                        // Native window identity stays available to macOS and
                        // accessibility; the transparent titlebar hides its text.
                        title: Some("Monitter GPUI".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(gpui_kit::px(12.), gpui_kit::px(22.))),
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window_state::restore_native(window, cx);
                    let entity = cx.new(|cx| Desktop::new(window, cx));
                    let weak = entity.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        window_state::save_native(window, cx);
                        let _ = weak.update(cx, |desktop, cx| desktop.request_close(window, cx));
                        false
                    });
                    entity
                },
            )
            .expect("open GPUI desktop window");
            let _ = window_handle.update(cx, |_, window, cx| dialog_backdrop::install(window, cx));
            #[cfg(feature = "visual-export")]
            visual_export::schedule(window_handle, cx);
            #[cfg(feature = "performance-profiling")]
            performance_metrics::schedule(window_handle, cx);
            // Cmd/Ctrl+T/W/1..9: bind the three actions and key chords. The
            // module owns the action types and keymap; this is the only
            // wiring site.
            native_navigation::install(
                window_handle,
                native_navigation::NativeNavigation::new(desktop.downgrade()),
                cx,
            );
            shortcut_desktop::install(window_handle, shortcut_desktop::DesktopShortcuts::new(desktop.downgrade()), cx);
            let weak = desktop.downgrade();
            cx.on_action(move |_: &native_menu::LockSession,cx| {
                let weak=weak.clone(); cx.defer(move|cx| {let _=window_handle.update(cx,|_,window,cx| {let _=weak.update(cx,|desktop,cx| {
                    if session_lock::locked(cx) {return;}
                    if desktop.session_lock.read(cx).configured() {desktop.session_lock.update(cx,|lock,cx|lock.lock(window,cx));}
                    else {
                        desktop.show_preferences(window,cx);
                        if desktop.native_settings.is_none() {
                            desktop.native_settings = Some(settings::NativeSettings::new(&desktop.snapshot["settings"], window, cx));
                        }
                        if let Some(settings)=&mut desktop.native_settings {settings.select_security();}
                        cx.notify();
                    }
                });});});
            });
            let weak = desktop.downgrade();
            cx.on_action(move |_: &native_menu::OpenSettings, cx| {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |desktop, cx| {
                            desktop.switch_surface(Sidebar::Settings, window, cx);
                        });
                    });
                });
            });
            let weak = desktop.downgrade();
            cx.on_action(move |_: &Quit, cx| {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |desktop, cx| desktop.request_close(window, cx));
                    });
                });
            });
            let weak = desktop.downgrade();
            cx.on_app_quit(move |cx| {
                let locked=session_lock::locked(cx);
                let service = weak
                    .update(cx, |desktop, cx| desktop.prepare_close(cx))
                    .ok()
                    .flatten();
                let _ = window_handle.update(cx, |_, window, cx| window_state::save_native(window, cx));
                // Platform-initiated app quit bypasses our deferred action handler.
                // GPUI destroys windows immediately after observers return, so flush
                // the closing frame here, outside the Desktop entity borrow.
                let _ = window_handle.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |desktop, cx| {
                        desktop.deactivate_mail(window, cx);
                        desktop
                            .dock
                            .update(cx, |dock, cx| dock.close_all(window, cx));
                    });
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
                let executor = cx.background_executor().clone();
                async move {
                    if locked {let _=executor.spawn(async {session_lock_store::set_locked(true)}).await;}
                    if let Some(service) = service {
                        executor
                            .spawn(async move {
                                service.shutdown();
                            })
                            .await;
                    }
                }
            })
            .detach();
            cx.activate(true);
        });
}

#[cfg(all(test, feature = "visual-export"))]
mod message_action_hover_tests;

#[cfg(test)]
mod control_hover_tests;
