//! Named, client-local pane layouts. Agent and PTY ownership stays with the backend.
use crate::workspace_windows::DesktopContextExt as _;
use crate::workspace_window_state::{WindowId as WorkspaceWindowId, WorkspaceWindowState};
use crate::{
    Desktop, Sidebar,
    design::{self, ui_scaled_px as px},
};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable, Icon, Sizable,
        button::*,
        input::{Input, InputEvent, InputState},
        popover::Popover,
        menu::{ContextMenuExt as _, PopupMenuItem},
        color_picker::{ColorPicker, ColorPickerState, ColorPickerEvent},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::{
    NativePreferenceData, NativeWorkspace, NativeWorkspaceSet,
};
use rand_core::{OsRng, RngCore};
use crate::workspace_favicon::workspace_favicon;
use serde_json::{Value, json};

// Kept mounted at zero opacity so releasing Command can finish its fade. Frame
// requests belong to this small retained view, never Desktop/backend publication.
const NUMBER_HINT_FADE: std::time::Duration = std::time::Duration::from_millis(140);
struct WorkspaceNumberHint {
    number: usize,
    from: f32,
    target: f32,
    ink: Rgba,
    background: Rgba,
    clock: crate::chat::feedback::Clock,
}
impl WorkspaceNumberHint {
    fn opacity(&mut self, reduced: bool) -> f32 {
        let progress = self.clock.borrow_mut().progress(reduced);
        self.from + (self.target - self.from) * progress
    }
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let target = if visible { 1. } else { 0. };
        if self.target == target { return; }
        let reduced = cx.reduce_motion() || !self.clock.borrow().painted;
        self.from = self.opacity(reduced);
        self.target = target;
        if reduced { self.clock.borrow_mut().finish(); }
        else { self.clock.borrow_mut().start(NUMBER_HINT_FADE); }
        cx.notify();
    }
}
impl Render for WorkspaceNumberHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let opacity = self.opacity(cx.reduce_motion());
        crate::chat::feedback::painted(
            div().absolute().inset_0().size_full().child(
                div().absolute().top(px(-4.)).right(px(-6.))
                    .min_w(px(13.)).h(px(13.)).px(px(3.))
                    .flex().items_center().justify_center().rounded_full()
                    .border_1().border_color(self.ink.opacity(0.28))
                    .bg(self.background).text_color(self.ink)
                    .text_size(px(9.)).font_weight(FontWeight::SEMIBOLD)
                    .opacity(opacity).child(self.number.to_string()),
            ), self.clock.clone(),
        )
    }
}
fn workspace_number_hint(
    id: &str, number: usize, visible: bool, palette: design::Palette,
    window: &mut Window, cx: &mut App,
) -> Entity<WorkspaceNumberHint> {
    let ink = rgb(palette.ink);
    let background = rgb(palette.panel);
    let holder = window.use_keyed_state(
        SharedString::from(format!("workspace-number-hint-{id}")), cx,
        |_, cx| cx.new(|_| WorkspaceNumberHint {
            number, from: 0., target: 0., ink, background,
            clock: crate::chat::feedback::FeedbackClock::new(),
        }),
    );
    let entity = holder.read(cx).clone();
    entity.update(cx, |hint, cx| {
        hint.set_visible(visible, cx);
        if hint.number != number || hint.ink != ink || hint.background != background {
            hint.number = number; hint.ink = ink; hint.background = background;
            cx.notify();
        }
    });
    entity
}

#[derive(Clone)]
struct WorkspaceTabDrag {
    id: String,
    name: String,
    icon: IconName,
    color: Rgba,
    ink: Rgba,
    background: Rgba,
}
impl Render for WorkspaceTabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = design::workspace_tab_metrics(cx.theme().font_size);
        div().h(metrics.height).px(px(12.)).flex().items_center().gap(px(8.))
            .rounded(px(12.)).border_1().border_color(self.color)
            .bg(self.background).text_color(self.ink)
            .font_family(cx.theme().font_family.clone())
            .text_size(metrics.text_size)
            .child(Icon::new(self.icon.clone()).with_size(metrics.icon_size).text_color(self.color))
            .child(self.name.clone())
    }
}
fn reorderable_tab(
    tab: Stateful<Div>,
    drag: WorkspaceTabDrag,
    on_drop: impl Fn(&WorkspaceTabDrag, &mut Window, &mut App) + 'static,
    on_complete: impl Fn(&WorkspaceTabDrag, &DragCompletion, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let target = drag.id.clone();
    let color = drag.color;
    tab.on_drag(drag, |drag, _, _, cx| {
        #[cfg(feature = "performance-profiling")]
        if crate::workspace_window_preview::enabled() {
            eprintln!("[workspace-preview] drag-start workspace={}", drag.id);
        }
        cx.stop_propagation();
        cx.new(|_| drag.clone())
    })
    .external_drag_payload::<WorkspaceTabDrag>(|_,_,_|Some(ExternalDragPayload::Internal))
    .on_drag_complete::<WorkspaceTabDrag>(on_complete)
    .can_drop(move |value, _, _| value.downcast_ref::<WorkspaceTabDrag>()
        .is_some_and(|source| source.id != target))
    .drag_over::<WorkspaceTabDrag>(move |style, _, _, _|
        style.border_color(color).bg(color.opacity(0.18)))
    .on_drop(on_drop)
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
pub(super) fn empty_layout() -> Value {
    json!({"version":1,"center":{"panel_name":"StackPanel","children":[{"panel_name":"TabPanel","children":[{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":format!("empty:{}", new_id())}}}],"info":{"tabs":{"active_index":0}}}],"info":{"stack":{"sizes":[600.0],"axis":0}}}})
}
pub fn loaded_workspaces(data: &NativePreferenceData) -> NativeWorkspaceSet {
    if let Some(mut set) = data.workspaces.clone() {
        for item in &mut set.items {
            if item.layout.is_none() {
                item.layout = Some(empty_layout());
            }
        }
        return set;
    }
    {
        let id = new_id();
        NativeWorkspaceSet {
            active_id: id.clone(),
            items: vec![NativeWorkspace {
                id,
                name: "Monitter".into(),
                color: "#38BFD2".into(),
                icon: "folder".into(),
                url: None, layout: data.layout.clone(),
            }],
        }
    }
}
fn icon(name: &str) -> IconName {
    if let Some(key) = name.strip_prefix("lucide:") {
        if let Some(icon) = IconName::ALL.iter().find(|icon| icon_key(**icon) == key) { return *icon; }
    }
    match name {
        "code" => IconName::Code,
        "terminal" => IconName::Terminal,
        "globe" => IconName::Globe,
        "sparkles" => IconName::Star,
        _ => IconName::Folder,
    }
}
fn icon_key(icon: IconName) -> String {
    icon.path().as_ref().strip_prefix("icons/").unwrap_or_default().trim_end_matches(".svg").to_owned()
}
pub struct WorkspaceEditor {
    id: String,
    name: Entity<InputState>,
    color: Entity<InputState>,
    url: Entity<InputState>,
    icon_query: Entity<InputState>,
    icon_picker_open: bool,
    color_picker: Entity<ColorPickerState>,
    icon: String,
    error: Option<String>,
    confirm_remove: bool,
    _subscriptions: Vec<Subscription>,
}
fn relative_workspace_id(set: &NativeWorkspaceSet, direction: i8) -> Option<String> {
    if set.items.len() < 2 || direction == 0 { return None; }
    let current = set.items.iter().position(|item| item.id == set.active_id)?;
    let next = (current as isize + direction.signum() as isize)
        .rem_euclid(set.items.len() as isize) as usize;
    Some(set.items[next].id.clone())
}

fn indexed_workspace_id(set: &NativeWorkspaceSet, index: usize) -> Option<String> {
    if !(1..=9).contains(&index) { return None; }
    set.items.get(index - 1).map(|item| item.id.clone())
}

/// Presentation and numeric shortcuts share the device-local ownership order.
/// Unassigned legacy records are visible only in main until reconciliation runs.
fn window_workspace_set(
    set: &NativeWorkspaceSet,
    assignments: &WorkspaceWindowState,
    host: &WorkspaceWindowId,
    active: Option<&str>,
) -> NativeWorkspaceSet {
    let assignment = assignments.window(host);
    let mut items: Vec<NativeWorkspace> = assignment
        .into_iter()
        .flat_map(|window| window.workspace_ids())
        .filter_map(|id| set.items.iter().find(|item| &item.id == id).cloned())
        .collect();
    if host == assignments.primary_window_id() {
        items.extend(
            set.items
                .iter()
                .filter(|item| assignments.window_for_workspace(&item.id).is_none())
                .cloned(),
        );
    }
    let active_id = [
        active,
        assignment.and_then(|window| window.active_workspace_id()),
        Some(set.active_id.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|id| items.iter().any(|item| item.id == *id))
    .map(str::to_owned)
    .or_else(|| items.first().map(|item| item.id.clone()))
    .unwrap_or_default();
    NativeWorkspaceSet { active_id, items }
}

fn dispatch_workspace_tab_move(
    owner: WeakEntity<Desktop>,
    host: AnyWindowHandle,
    id: String,
    return_to_main: bool,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = host.update(cx, |_, window, cx| {
            let _ = owner.update(cx, |desktop, cx| {
                desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                    if desktop
                        .workspace_window_assignments
                        .window_for_workspace(&id)
                        != Some(&desktop.current_workspace_window)
                    {
                        return;
                    }
                    if return_to_main {
                        desktop.request_reattach_workspace(id, window, cx);
                    } else {
                        desktop.request_detach_workspace(id, window, cx);
                    }
                })
            });
        });
    });
}

impl Desktop {
    fn window_named_workspaces(&self) -> Option<NativeWorkspaceSet> {
        self.named_workspaces.as_ref().map(|set| {
            window_workspace_set(
                set,
                &self.workspace_window_assignments,
                &self.current_workspace_window,
                self.window.active_workspace_id.as_deref(),
            )
        })
    }
    /// Expanded rows, Activity icons and compact group menus share this path.
    /// Keep only the latest click while restore/save owns the layout; do not set
    /// selected then, because rendering it would add it to the outgoing dock.
    pub(super) fn navigate_sidebar_task(&mut self, task_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if crate::session_lock::locked(cx) || !self.workspace_controls_available() { return; }
        if self.window.pending_notification_read.as_ref().is_some_and(|pending| pending != &task_id) { self.window.pending_notification_read = None; }
        self.window.pending_sidebar_task = Some(task_id);
        self.drain_sidebar_task_navigation(window, cx);
        if self.window.pending_sidebar_task.is_none() || self.window.sidebar_task_navigation_scheduled { return; }
        self.window.sidebar_task_navigation_scheduled = true;
        self.spawn_workspace_host(window, cx, async move |view, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(25)).await;
                let keep_waiting = view.update_in(cx, |view, window, cx| {
                    if crate::session_lock::locked(cx) || !view.workspace_controls_available() {
                        view.window.pending_sidebar_task = None;
                        view.window.pending_notification_read = None;
                    } else {
                        view.drain_sidebar_task_navigation(window, cx);
                    }
                    let waiting = view.window.pending_sidebar_task.is_some();
                    if !waiting { view.window.sidebar_task_navigation_scheduled = false; }
                    waiting
                }).unwrap_or(false);
                if !keep_waiting { break; }
            }
        }).detach();
    }

    fn drain_sidebar_task_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.window.pending_sidebar_task.clone() else {
            return;
        };
        if self.hidden_task_ids.contains(&task_id) {
            self.window.pending_sidebar_task = None;
            self.window.pending_notification_read = None;
            return;
        }
        if !self.workspace_ready() {
            return;
        }
        let pane_id = format!("chat:{task_id}");
        let layout = match self.window.dock.read(cx).serialize(cx) {
            Ok(layout) => layout,
            Err(error) => {
                self.window.pending_sidebar_task = None;
                self.window.pending_notification_read = None;
                self.preferences_error = Some(error);
                cx.notify();
                return;
            }
        };
        match crate::workspace_visibility::sidebar_task_destination(
            &pane_id,
            &layout,
            self.named_workspaces.as_ref(),
            self.workspace_ready(),
        ) {
            crate::workspace_visibility::SidebarTaskDestination::Wait => {}
            crate::workspace_visibility::SidebarTaskDestination::Current => {
                self.window.pending_sidebar_task = None;
                self.focus_sidebar_task(&task_id, window, cx);
            }
            crate::workspace_visibility::SidebarTaskDestination::Workspace(id) => {
                self.begin_named_workspace_switch(id.clone(), window, cx);
                if self.window.active_workspace_id.as_deref() == Some(id.as_str()) {
                    // A retained target activates synchronously, so there is no
                    // restore completion to consume this host's focus/read intent.
                    if self.workspace_ready() {
                        self.window.pending_sidebar_task = None;
                        if !self.focus_sidebar_task(&task_id, window, cx) {
                            self.window.pending_notification_read = None;
                        }
                    }
                } else if self.window.workspace_switch.is_none()
                    && !self.window.preferences_restore
                    && !self.window.preferences_restoring
                {
                    // Only a real failure may discard the deferred intent.
                    self.window.pending_sidebar_task = None;
                    self.window.pending_notification_read = None;
                }
            }
            crate::workspace_visibility::SidebarTaskDestination::Open => {
                self.window.pending_sidebar_task = None;
                self.switch_surface(Sidebar::Chats, window, cx);
                self.window.selected = Some(task_id.clone());
                self.acknowledge_notification_task(&task_id, cx);
                cx.notify();
            }
        }
    }

    fn focus_sidebar_task(&mut self, task_id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if crate::session_lock::locked(cx) || self.hidden_task_ids.contains(task_id) { return false; }
        let pane_id = format!("chat:{task_id}");
        if !self.window.dock.update(cx, |dock, cx| dock.reveal_and_focus(&pane_id, window, cx)) { return false; }
        self.switch_surface(Sidebar::Chats, window, cx);
        // switch_surface resets dock_current; restore it before rendering so the
        // retained chat is focused without constructing or adding a pane spec.
        self.window.selected = Some(task_id.to_owned());
        self.window.dock_current = Some(pane_id);
        self.acknowledge_notification_task(task_id, cx);
        cx.notify();
        true
    }

    /// A scoped feed explicitly chooses its workspace, even if a chat appears in more than one.
    pub(super) fn navigate_workspace_notification(&mut self, id: &str, task_id: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if crate::session_lock::locked(cx) || !self.workspace_ready() {
            // A later attempted review must not acknowledge an earlier queued review.
            self.window.pending_notification_read = None;
            return false;
        }
        if self.named_workspaces.as_ref().is_none_or(|set| !set.items.iter().any(|item| item.id == id)) { return false; }
        if let Some(host)=self.workspace_window_assignments.window_for_workspace(id).filter(|host|*host!=&self.current_workspace_window) {
            if let Some(handle)=self.window_hosts.get(host).map(|host|host.handle) {
                let weak=cx.weak_entity();let workspace=id.to_owned();
                cx.defer(move|cx| {let _=handle.update(cx,|_,window,cx|weak.update(cx,|desktop,cx|desktop.with_workspace_window(window,cx,|desktop,window,cx| {
                    desktop.navigate_workspace_notification(&workspace,task_id,window,cx);window.activate_window();
                })));});
            }
            return false;
        }
        self.switch_named_workspace(id.to_owned(), window, cx);
        if self.window.workspace_switch.as_deref() != Some(id)
            && self.window.active_workspace_id.as_deref() != Some(id) { return false; }
        self.window.pending_notification_read = Some(task_id.clone());
        self.navigate_sidebar_task(task_id, window, cx);
        true
    }
    fn acknowledge_notification_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        if self.window.pending_notification_read.as_deref() == Some(task_id) {
            self.window.pending_notification_read = None;
            self.notification_model.mark_task_read(task_id);
            self.sync_notification_bell(cx);
        }
    }

    pub(super) fn set_workspace_number_hints(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.window.workspace_number_hints != visible {
            self.window.workspace_number_hints = visible;
            cx.notify();
        }
    }

    pub(super) fn capture_workspace_layout(&mut self, mut layout: Value) {
        let active = self.window.active_workspace_id.clone();
        if let Some(set) = &mut self.named_workspaces {
            if let Some(item) = set.items.iter_mut().find(|item| Some(&item.id) == active.as_ref()) {
                crate::workspace_visibility::preserve_focus_metadata(&mut layout, item.layout.as_ref());
                item.layout = Some(layout);
            }
        }
    }
    /// Called only once the requested layout has finished restoring, including
    /// asynchronous terminal lookup. Candidates belong to the restored layout.
    pub(super) fn restore_named_workspace_focus(&mut self, layout: &Value, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(task_id) = self.window.pending_sidebar_task.clone() {
            if self.focus_sidebar_task(&task_id, window, cx) {
                self.window.pending_sidebar_task = None;
                return;
            }
        }
        let active = self.window.active_workspace_id.clone();
        let remembered = self.named_workspaces.as_ref().and_then(|set|
            set.items.iter().find(|item| Some(&item.id) == active.as_ref()))
            .and_then(|item| item.layout.as_ref())
            .and_then(crate::workspace_visibility::remembered_focus);
        for id in crate::workspace_visibility::focus_candidates(layout, remembered) {
            if self.window.dock.update(cx, |dock, cx| dock.focus(&id, window, cx)) { break; }
        }
    }
    // Restore/save guards serialize mutations, but must not dim every icon
    // during a normal workspace switch or a background preferences save.
    fn workspace_controls_available(&self) -> bool {
        !self.startup_pending && self.preferences.is_some()
            && !self.preferences_blocked
            && !self.closing
            && !self.close_pending
    }
    pub(super) fn workspace_ready(&self) -> bool {
        self.workspace_controls_available()
            && !self.preferences_busy
            && !self.window.preferences_restore
            && !self.window.preferences_restoring
            && !self.window.preferences_restore_suppressed
    }
    pub(super) fn select_named_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = self.window_named_workspaces() else { return; };
        let Some(id) = indexed_workspace_id(&set, index) else { return; };
        self.switch_named_workspace(id, window, cx);
    }
    pub(super) fn cycle_named_workspace(&mut self, direction: i8, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = self.window_named_workspaces() else { return; };
        let Some(id) = relative_workspace_id(&set, direction) else { return; };
        self.switch_named_workspace(id, window, cx);
    }
    pub(super) fn switch_named_workspace(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // An explicit workspace choice supersedes any queued sidebar click.
        self.window.pending_pane_focus = None;
        self.window.pending_sidebar_task = None;
        self.window.pending_notification_read = None;
        self.begin_named_workspace_switch(id, window, cx);
    }
    fn begin_named_workspace_switch(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .named_workspaces
            .as_ref()
            .is_none_or(|set| !set.items.iter().any(|item| item.id == id))
        {
            return;
        }
        if self.focus_workspace_owner(&id, cx) {
            return;
        }
        if !self.workspace_ready()
            || self.window.active_workspace_id.as_deref() == Some(id.as_str())
        {
            return;
        }
        match self.activate_retained_workspace(&id, window, cx) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.preferences_error = Some(error);
                cx.notify();
                return;
            }
        }
        self.capture_native_drafts(cx);
        match self.window.dock.read(cx).serialize(cx) {
            Ok(mut layout) => {
                let focused = self
                    .window
                    .dock
                    .read(cx)
                    .focused_pane_id(window, cx)
                    .or_else(|| self.window.dock_current.clone());
                crate::workspace_visibility::remember_focus(&mut layout, focused.as_deref());
                self.capture_workspace_layout(layout);
            }
            Err(error) => {
                self.preferences_error = Some(error);
                cx.notify();
                return;
            }
        }
        if let Err(error) = self.prepare_uncached_workspace(&id, window, cx) {
            self.preferences_error = Some(error);
            cx.notify();
            return;
        }
        self.workspaces_dirty = true;
        self.preferences_dirty = true;
        self.window.workspace_switch = Some(id);
        self.window.workspace_editor = None;
        self.window.selected = None;
        self.window.dock_current = None;
        self.window.sidebar = Sidebar::Chats;
        self.window.route_touched = true;
        self.window.preferences_restore = true;
        self.restore_preferences(window, cx);
        cx.notify();
    }
    fn add_named_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready() {
            return;
        }
        let Some(set) = &mut self.named_workspaces else {
            return;
        };
        if set.items.len() >= 24 {
            return;
        }
        let id = new_id();
        let index = set.items.len();
        let colors = ["#38BFD2", "#41C968", "#A78BFA", "#F59E42", "#EF709C"];
        set.items.push(NativeWorkspace {
            id: id.clone(),
            name: format!("Workspace {}", index + 1),
            color: colors[index % colors.len()].into(),
            icon: "folder".into(),
            url: None,
            layout: Some(empty_layout()),
        });
        self.workspaces_dirty = true;
        self.preferences_dirty = true;
        let previous_active = self.window.active_workspace_id.clone();
        let assigned = self.reconcile_workspace_window_assignments().and_then(|_| {
            if self.current_workspace_window
                != *self.workspace_window_assignments.primary_window_id()
            {
                self.workspace_window_assignments.transfer_workspace(
                    &id,
                    &self.current_workspace_window,
                    usize::MAX,
                )?;
                if let Some(previous) = &previous_active {
                    self.workspace_window_assignments
                        .set_active_workspace(&self.current_workspace_window, previous)?;
                }
            }
            self.persist_workspace_window_assignments(false)
        });
        if let Err(error) = assigned {
            self.preferences_error = Some(error);
            cx.notify();
            return;
        }
        self.switch_named_workspace(id.clone(), window, cx);
        // The retained editor is separate from layout restoration.
        self.edit_named_workspace(&id, window, cx);
    }
    fn edit_named_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .named_workspaces
            .as_ref()
            .and_then(|set| set.items.iter().find(|item| item.id == id))
        else {
            return;
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(item.name.clone()));
        let color = cx.new(|cx| InputState::new(window, cx).default_value(item.color.clone()));
        let url = cx.new(|cx| InputState::new(window, cx).default_value(item.url.clone().unwrap_or_default()));
        let icon_query = cx.new(|cx| InputState::new(window, cx).placeholder("Search all icons"));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(design::parse_color(&item.color, 0x38bfd2))));
        let mut subscriptions: Vec<Subscription> = [&name, &color, &url]
            .into_iter()
            .map(|input| {
                cx.subscribe_workspace(input, window, |view, _, event, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.save_workspace_editor(cx);
                    }
                })
            })
            .collect();
        subscriptions.push(cx.observe(&color_picker, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe(&icon_query, |_, _, _: &InputEvent, cx| cx.notify()));
        subscriptions.push(cx.subscribe_workspace(&color, window, |view, input, _: &InputEvent, window, cx| {
            let value = input.read(cx).value().trim().to_owned();
            if value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Some(editor) = &view.window.workspace_editor {
                    editor.color_picker.update(cx, |picker, cx| picker.set_value(rgb(design::parse_color(&value, 0x38bfd2)), window, cx));
                }
            }
        }));
        subscriptions.push(cx.subscribe_workspace(&color_picker, window, |view, picker, event: &ColorPickerEvent, window, cx| {
            let ColorPickerEvent::Change(selected) = event;
            if let Some(editor) = &view.window.workspace_editor {
                if let Some(color) = selected {
                    let rgba: Rgba = (*color).into();
                    let opaque = ((rgba.r * 255.).round() as u32) << 16
                        | ((rgba.g * 255.).round() as u32) << 8
                        | (rgba.b * 255.).round() as u32;
                    editor.color.update(cx, |input, cx| input.set_value(format!("#{opaque:06X}"), window, cx));
                    picker.update(cx, |picker, cx| picker.set_value(rgb(opaque), window, cx));
                } else {
                    // Workspace colours are required and opaque; clearing restores the draft.
                    let value = editor.color.read(cx).value().to_owned();
                    picker.update(cx, |picker, cx| picker.set_value(rgb(design::parse_color(&value, 0x38bfd2)), window, cx));
                }
            }
            cx.notify();
        }));
        self.window.workspace_editor = Some(WorkspaceEditor {
            id: id.into(),
            name,
            color,
            url,
            icon_query,
            icon_picker_open: false,
            color_picker,
            icon: item.icon.clone(),
            error: None,
            confirm_remove: false,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }
    fn save_workspace_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.window.workspace_editor else {
            return;
        };
        if editor.confirm_remove { return; }
        let name = editor.name.read(cx).value().trim().to_owned();
        let color = editor.color.read(cx).value().trim().to_owned();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            editor.error = Some("Name must contain 1–80 characters.".into());
            cx.notify();
            return;
        }
        if color.len() != 7
            || !color.starts_with('#')
            || !color[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            editor.error = Some("Enter a colour in #RRGGBB format.".into());
            cx.notify();
            return;
        }
        let url = editor.url.read(cx).value().trim().to_owned();
        if !url.is_empty() {
            if let Err(error) = monitter_lib::experimental_desktop::validate_native_workspace_url(&url) {
                editor.error = Some(error);
                cx.notify();
                return;
            }
        }
        if let Some(item) = self
            .named_workspaces
            .as_mut()
            .and_then(|set| set.items.iter_mut().find(|item| item.id == editor.id))
        {
            item.name = name;
            item.color = color.to_ascii_uppercase();
            item.icon = editor.icon.clone();
            item.url = (!url.is_empty()).then_some(url);
        }
        self.window.workspace_editor = None;
        self.workspaces_dirty = true;
        self.queue_preferences_save(cx);
    }
    pub(super) fn reorder_named_workspace(&mut self, source: &str, target: &str, cx: &mut Context<Self>) {
        if !self.workspace_ready() || source == target {
            return;
        }
        let Some(visible) = self.window_named_workspaces() else {
            return;
        };
        let Some(index) = visible.items.iter().position(|item| item.id == target) else {
            return;
        };
        if !visible.items.iter().any(|item| item.id == source) {
            return;
        }
        let result = self.reconcile_workspace_window_assignments().and_then(|_| {
            self.workspace_window_assignments.transfer_workspace(
                source,
                &self.current_workspace_window,
                index,
            )?;
            if !visible.active_id.is_empty() {
                self.workspace_window_assignments
                    .set_active_workspace(&self.current_workspace_window, &visible.active_id)?;
            }
            self.persist_workspace_window_assignments(false)
        });
        if let Err(error) = result {
            self.preferences_error = Some(error);
            cx.notify();
            return;
        }
        if let Some(set) = &mut self.named_workspaces {
            monitter_gpui_desktop::workspace_order::reorder(set, source, target);
        }
        self.workspaces_dirty = true;
        self.queue_preferences_save(cx);
        cx.notify();
    }
    fn remove_named_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready() {
            return;
        }
        if self
            .named_workspaces
            .as_ref()
            .is_none_or(|set| set.items.len() <= 1)
        {
            return;
        }
        let Some(visible) = self.window_named_workspaces() else {
            return;
        };
        let Some(index) = visible.items.iter().position(|item| item.id == id) else {
            return;
        };
        if visible.active_id == id {
            if visible.items.len() == 1 {
                if let Err(error) = self.remove_last_host_workspace(id, window, cx) {
                    self.preferences_error = Some(error);
                }
            } else {
                let neighbor = visible.items[if index + 1 < visible.items.len() {
                    index + 1
                } else {
                    index - 1
                }]
                .id
                .clone();
                // Commit only after a local neighbour is successfully activated.
                self.window.pending_workspace_removal = Some(id.into());
                self.switch_named_workspace(neighbor, window, cx);
                if self.window.workspace_switch.is_none() {
                    self.window.pending_workspace_removal = None;
                }
            }
        } else {
            monitter_gpui_desktop::workspace_order::remove(
                self.named_workspaces.as_mut().unwrap(),
                id,
            );
            self.workspace_view_cache.remove(id);
            self.window.workspace_editor = None;
            self.workspaces_dirty = true;
            if let Err(error) = self.persist_workspace_window_assignments(false) {
                self.preferences_error = Some(error);
            }
            self.queue_preferences_save(cx);
        }
        cx.notify();
    }
    pub(super) fn render_workspace_tabs(
        &mut self,
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let usage_width = if self.window.sidebar_collapsed {
            design::compact_sidebar_width_design(design::interface_scale())
        } else { self.current_sidebar_width(cx) };
        if let Some(usage) = &self.window.sidebar_usage {
            usage.update(cx, |usage, cx| usage.set_composed_palette(palette, usage_width, cx));
        }
        let notification_pills = self.workspace_notification_pills(window, cx);
        let ready = self.workspace_ready();
        let controls_available = self.workspace_controls_available();
        let tab_metrics = design::workspace_tab_metrics(cx.theme().font_size);
        let tern =
            design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let mut tabs = div()
            .id("named-workspace-tabs")
            .flex()
            .items_center()
            .gap(px(5.))
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .on_drop(cx.workspace_listener(|desktop,drag:&WorkspaceTabDrag,_,cx| {
                desktop.queue_workspace_tab_drop(drag.id.clone(),None,cx);cx.stop_propagation();
            }));
        if let Some(set) = self.window_named_workspaces() {
            for (index, item) in set.items.iter().enumerate() {
                let active = item.id == set.active_id;
                let id = item.id.clone();
                let edit_id = id.clone();
                let menu_id = id.clone();
                let is_open = self.window.workspace_editor
                    .as_ref()
                    .is_some_and(|editor| editor.id == id);
                let owner = cx.entity();
                let menu_owner = owner.clone();
                let key_id = id.clone();
                let focus = window
                    .use_keyed_state(
                        SharedString::from(format!("workspace-focus-{id}")),
                        cx,
                        |_, cx| cx.focus_handle(),
                    )
                    .read(cx)
                    .clone();
                let click_focus = focus.clone();
                let color = rgb(design::parse_color(&item.color, palette.accent));
                let selection_chrome = crate::chrome_motion::selection_chrome(
                    SharedString::from(format!("workspace-selection-motion-{id}")), active,
                    color, px(4.), window, cx,
                );
                let drop_id = id.clone();
                let drag_owner=cx.weak_entity();
                let drag = WorkspaceTabDrag {
                    id: id.clone(), name: item.name.clone(), icon: icon(&item.icon),
                    color, ink: rgb(palette.ink), background: rgb(palette.panel),
                };
                let icon_debug_id = id.clone();
                let trigger = Button::new(SharedString::from(format!("edit-workspace-{id}")))
                    .ghost()
                    .compact()
                    .w(tab_metrics.editor_size)
                    .h(tab_metrics.editor_size)
                    .rounded(px(4.))
                    .p_0()
                    .child(div().relative().size(tab_metrics.icon_size)
                        .debug_selector(move || format!("workspace-edit-icon-{icon_debug_id}"))
                        .child(workspace_favicon(&id, item.url.as_deref(), icon(&item.icon), color, window, cx))
                        .when(index < 9, |icon| icon.child(workspace_number_hint(
                            &id, index + 1, self.window.workspace_number_hints, palette, window, cx,
                        ))))
                    .accessibility_label(format!("Edit {} workspace", item.name))
                    .tooltip("Edit workspace")
                    // Opening the editor is local; pending preference writes
                    // must not block access. Save/removal keep their readiness guards.
                    .disabled(self.closing || self.close_pending)
                    // Popover opens on mouse-down and stops that event before
                    // the containing tab can start a drag or move the window.
                    .on_click(|_, _, cx| cx.stop_propagation());
                let editor_focus = self.window.workspace_editor
                    .as_ref()
                    .filter(|editor| editor.id == id)
                    .map(|editor| editor.name.read(cx).focus_handle(cx));
                let menu = Popover::new(SharedString::from(format!("workspace-edit-menu-{id}")))
                    .rounded(px(design::radius::MODAL))
                    .trigger(trigger)
                    .header("Workspace", "Name, appearance, and workspace settings.")
                    .anchor(Anchor::TopLeft)
                    .open(is_open)
                    .when_some(editor_focus.as_ref(), |menu, focus| menu.track_focus(focus))
                    // Nested picker surfaces are separate native popups. Let an outside
                    // click dismiss the child first instead of destroying the editor draft.
                    .overlay_closable(!self.window.workspace_editor.as_ref().is_some_and(|editor|
                        editor.id == id && (editor.icon_picker_open || editor.color_picker.read(cx).is_open())))
                    .on_open_change(move |open, window, cx| {
                        owner.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, window, cx| {
                            if *open {
                                if !view.workspace_controls_available()
                                    || view.window.preferences_restore
                                    || view.window.preferences_restoring
                                    || view.window.preferences_restore_suppressed { return; }
                                if !view.window.workspace_editor
                                    .as_ref()
                                    .is_some_and(|editor| editor.id == menu_id)
                                {
                                    view.edit_named_workspace(&menu_id, window, cx);
                                    if let Some(editor) = &view.window.workspace_editor {
                                        editor.name.read(cx).focus_handle(cx).focus(window, cx);
                                    }
                                }
                            } else if view.window.workspace_editor
                                .as_ref()
                                .is_some_and(|editor| editor.id == menu_id)
                            {
                                view.window.workspace_editor = None;
                                cx.notify();
                            }
                        }));
                    })
                    .content(move |_, window, cx| {
                        menu_owner.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, window, cx| view.render_workspace_editor(palette, window, cx)))
                    });
                let window_menu_owner = cx.weak_entity();
                let window_menu_id = id.clone();
                let host = window.window_handle();
                let detached = self.current_workspace_window != *self.workspace_window_assignments.primary_window_id();
                tabs = tabs.child(
                    div()
                        .id(SharedString::from(format!("workspace-pill-{id}")))
                        .debug_selector({let id=id.clone();move ||format!("workspace-tab-{id}")})
                        .relative()
                        .flex_shrink_0()
                        .h(tab_metrics.height)
                        .max_w(px(300.))
                        // Match the editor button's vertical inset inside the
                        // tab, accounting for its two 1px borders.
                        .pl(px(4.))
                        .pr(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(palette.line).opacity(0.))
                        .text_color(rgb(if active { palette.ink } else { palette.muted }))
                        .cursor_pointer()
                        .track_focus(&focus)
                        .tab_index(0)
                        .focus(|style| style.border_color(color))
                        .hover(|style| style.bg(color.opacity(0.10)))
                        .when(ready, |tab| reorderable_tab(tab, drag,
                            cx.workspace_listener(move |view, drag: &WorkspaceTabDrag, _, cx| {
                                view.queue_workspace_tab_drop(drag.id.clone(),Some(drop_id.clone()),cx);
                                cx.stop_propagation();
                            }),move |drag,completion,window,cx| {
                                let _=drag_owner.update(cx,|desktop,cx|desktop.with_workspace_window(window,cx,|desktop,window,cx|desktop.workspace_tab_drag_completed(&drag.id,completion,window,cx)));
                            }))
                        .role(Role::Tab)
                        .aria_selected(active)
                        .aria_label(format!(
                            "{} workspace{}",
                            item.name,
                            if active { ", selected" } else { "" }
                        ))
                        .child(div().absolute().inset_0().child(selection_chrome))
                        .child(menu)
                        .child(div().min_w_0().truncate()
                            .text_size(tab_metrics.text_size)
                            .child(item.name.clone()))
                        .child(div().flex_shrink_0()
                            .size(crate::design::ui_scaled_px(16.))
                            .children(notification_pills.get(&id).cloned()))
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            click_focus.focus(window, cx);
                            cx.stop_propagation();
                        })
                        .on_key_down(cx.workspace_listener(move |view, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "f2" {
                                view.edit_named_workspace(&key_id, window, cx);
                                cx.stop_propagation();
                            } else if event.keystroke.key == "enter"
                                || event.keystroke.key == "space"
                            {
                                view.switch_named_workspace(key_id.clone(), window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.workspace_listener(move |view, event: &ClickEvent, window, cx| {
                            if event.click_count() == 2 {
                                view.edit_named_workspace(&edit_id, window, cx);
                            } else {
                                view.switch_named_workspace(id.clone(), window, cx);
                            }
                        }))
                        .context_menu(move |menu, _, _| {
                            let move_owner = window_menu_owner.clone();
                            let move_id = window_menu_id.clone();
                            let menu = menu.item(PopupMenuItem::new("Move to New Window")
                                .disabled(!ready)
                                .on_click(move |_, _, cx| {
                                    dispatch_workspace_tab_move(move_owner.clone(), host, move_id.clone(), false, cx);
                                }));
                            if detached {
                                let return_owner = window_menu_owner.clone();
                                let return_id = window_menu_id.clone();
                                menu.item(PopupMenuItem::new("Return to Main Window")
                                    .disabled(!ready)
                                    .on_click(move |_, _, cx| {
                                        dispatch_workspace_tab_move(return_owner.clone(), host, return_id.clone(), true, cx);
                                    }))
                            } else { menu }
                        }),
                );
            }
        }
        tabs = tabs.child(
            Button::new("add-workspace")
                .ghost()
                .compact()
                .flex_shrink_0()
                .label("+")
                .accessibility_label("New workspace")
                .tooltip("New workspace")
                .disabled(
                    !controls_available
                        || self
                            .named_workspaces
                            .as_ref()
                            .is_none_or(|set| set.items.len() >= 24),
                )
                .on_click(cx.workspace_listener(|view, _, window, cx| view.add_named_workspace(window, cx))),
        );
        let bar = div()
            .w_full()
            .flex_shrink_0()
            .pl(gpui_kit::px(
                if window.is_fullscreen() || (tern && !self.sidebar_is_hidden()) {
                    10.
                } else {
                    82.
                },
            ))
            .pr(px(10.))
            // Share the sidebar controls' centre line and grow only when the
            // configured font/scale needs more than the native titlebar height.
            .h(tab_metrics.row_height)
            .py_0()
            .flex()
            .items_center()
            .gap(px(5.))
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .when(tern, |bar| {
                let label = if self.sidebar_is_hidden() { "Show sidebar" } else { "Hide sidebar" };
                bar.child(
                    Button::new("cycle-main-sidebar")
                        .track_focus(&self.window.sidebar_toggle_focus)
                        .ghost()
                        .compact()
                        .flex_shrink_0()
                        .toggled(!self.sidebar_is_hidden())
                        .icon(Icon::new(IconName::PanelLeft)
                            .text_color(rgb(if self.sidebar_is_hidden() { palette.ink } else { palette.accent }).opacity(0.5)))
                        .accessibility_label(label)
                        .tooltip(label)
                        .on_click(
                            cx.workspace_listener(|view, _, window, cx| view.toggle_sidebar(window, cx)),
                        ),
                )
                .child(div().flex_shrink_0().w(px(1.)).h(px(18.)).mx(px(5.))
                    .bg(rgb(palette.ink).opacity(0.24)))
            })
            .child(tabs)
            .children(self.window.workspace_usage_rings.clone())
            .map(|bar| {
                let enabled = self.window.dock.read(cx).auto_size_panes();
                let label = if enabled { "Turn Auto Size Panes off" } else { "Turn Auto Size Panes on" };
                bar.child(div().flex_shrink_0().w(px(1.)).h(px(18.)).mx(px(5.)).bg(rgb(palette.ink).opacity(0.24)))
                    .child(Button::new("toggle-auto-size-panes")
                        .ghost().compact().flex_shrink_0().toggled(enabled)
                        .icon(Icon::new(IconName::PanelsTopLeft)
                            .text_color(rgb(if enabled { palette.accent } else { palette.ink }).opacity(0.5)))
                        .accessibility_label(label)
                        .tooltip(if enabled { "Auto Size Panes · On" } else { "Auto Size Panes · Off" })
                        .on_click(cx.workspace_listener(|view, _, window, cx| {
                            let enabled = !view.window.dock.read(cx).auto_size_panes();
                            view.window.dock.update(cx, |dock, cx| dock.set_auto_size_panes(enabled, window, cx));
                        })))
                    .child(div().flex_shrink_0().w(px(1.)).h(px(18.)).mx(px(5.)).bg(rgb(palette.ink).opacity(0.24)))
            })
            .child(self.ensure_admin_activity(window, cx))
            .child(self.ensure_notification_bell(window, cx));

        div()
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_col()
            // Reclaim the row's empty lower half without moving its controls.
            // The dock's two half-insets then supply exactly the density gutter.
            .when(tern, |row| row.mb((tab_metrics.height - tab_metrics.row_height) / 2.))
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .text_color(rgb(palette.ink))
            .child(bar)
    }
    fn render_workspace_icon_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.window.workspace_editor else { return div().into_any_element(); };
        let query = editor.icon_query.read(cx).value().trim().to_ascii_lowercase();
        let mut grid = div().id("workspace-icon-grid").flex().flex_wrap().gap(px(4.))
            .max_h(px(320.)).overflow_y_scroll();
        let mut count = 0;
        let selected_icon = icon(&editor.icon);
        for choice in IconName::ALL.iter().copied() {
            let key = icon_key(choice);
            if !query.is_empty() && !key.contains(&query) { continue; }
            count += 1;
            let saved = format!("lucide:{key}");
            let debug_key = key.clone();
            let selected = selected_icon == choice;
            grid = grid.child(Button::new(SharedString::from(format!("workspace-icon-{key}")))
                .compact().w(px(32.)).h(px(32.))
                .child(div().size(px(16.)).debug_selector(move || format!("workspace-icon-choice-{debug_key}"))
                    .child(Icon::new(choice).with_size(px(16.))))
                .accessibility_label(format!("Workspace icon: {key}")).tooltip(key)
                .when(selected, |button| button.primary())
                .on_click(cx.workspace_listener(move |view, _, _, cx| {
                    if let Some(editor) = &mut view.window.workspace_editor {
                        editor.icon = saved.clone(); editor.icon_picker_open = false;
                    }
                    cx.notify();
                })));
        }
        div().w(px(360.)).max_w_full().flex().flex_col().gap(px(8.))
            .on_key_down(|event, _, cx| { if event.keystroke.key == "enter" { cx.stop_propagation(); } })
            .child(Input::new(&editor.icon_query).aria_label("Search workspace icons"))
            .child(format!("{count} icons"))
            .when(count == 0, |picker| picker.child("No matching icons"))
            .child(grid).into_any_element()
    }
    fn render_workspace_editor(
        &mut self,
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(editor) = &self.window.workspace_editor {
            let can_remove = self.workspace_ready()
                && self.named_workspaces.as_ref().is_some_and(|set| set.items.len() > 1);
            if editor.confirm_remove {
                let id = editor.id.clone();
                return div().id("workspace-remove-confirmation")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .w(px(420.)).max_w_full().flex().flex_col().gap(px(10.))
                    .text_color(rgb(palette.ink)).font_family(cx.theme().font_family.clone())
                    .text_size(cx.theme().font_size)
                    .child("Remove this workspace?")
                    .child("Its chats and running agents will remain available.")
                    .child(div().flex().justify_end().gap(px(5.))
                        .child(Button::new("cancel-workspace-removal").compact().label("Cancel")
                            .on_click(cx.workspace_listener(|view, _, _, cx| {
                                if let Some(editor) = &mut view.window.workspace_editor { editor.confirm_remove = false; }
                                cx.notify();
                            })))
                        .child(Button::new("confirm-workspace-removal").compact().label("Remove workspace")
                            .disabled(!can_remove)
                            .on_click(cx.workspace_listener(move |view, _, window, cx|
                                view.remove_named_workspace(&id, window, cx)))))
                    .into_any_element();
            }
            // Reserve space for the trigger row, popup title/subtitle, padding
            // and window edges. Scroll the fields, never the action footer.
            let viewport=window.viewport_size();
            let editor_width=px(440.).min((viewport.width-px(48.)).max(px(200.)));
            let chrome=px(180.)+(cx.theme().font_size-px(14.)).max(gpui_kit::px(0.))*4.;
            let editor_height=px(600.).min((viewport.height-chrome).max(px(120.)));
            let stacked_footer=editor_width < cx.theme().font_size*24.;
            let owner = cx.entity();
            let picker_owner = owner.clone();
            let icons = Popover::new("workspace-full-icon-picker")
                .open(editor.icon_picker_open)
                .on_open_change(move |open, window, cx| picker_owner.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, _, cx| {
                    if let Some(editor) = &mut view.window.workspace_editor { editor.icon_picker_open = *open; }
                    cx.notify();
                })))
                .trigger(Button::new("workspace-icon-picker-trigger").compact()
                    .child(div().size(px(16.)).debug_selector(|| "workspace-icon-picker-trigger".into())
                        .child(Icon::new(icon(&editor.icon)).with_size(px(16.))))
                    .label("Choose").accessibility_label("Choose workspace icon"))
                .header("Workspace icon", "Search the complete bundled icon collection.")
                .anchor(Anchor::TopLeft)
                .content(move |_, window, cx| owner.update(cx, |view, cx| view.with_workspace_window(window, cx, |view, _, cx| view.render_workspace_icon_picker(cx))));
            let colors = div().debug_selector(|| "workspace-colour-picker-trigger".into())
                .child(ColorPicker::new(&editor.color_picker).accessibility_label("Choose workspace colour"));
            div()
                .id("workspace-editor").debug_selector(|| "workspace-editor-surface".into())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .gap(px(12.))
                .w(editor_width)
                .max_w_full()
                .max_h(editor_height)
                .min_h_0()
                .overflow_hidden()
                .font_family(cx.theme().font_family.clone())
                .text_size(cx.theme().font_size)
                .text_color(rgb(palette.ink))
                .on_key_down(cx.workspace_listener(|view, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        view.window.workspace_editor = None;
                        cx.stop_propagation();
                        cx.notify();
                    } else if event.keystroke.key == "enter" {
                        view.save_workspace_editor(cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div().id("workspace-editor-fields").debug_selector(||"workspace-editor-fields".into())
                        .w_full().min_w_0().min_h_0().overflow_y_scroll()
                        .flex_shrink_1()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            div()
                                .w_full().flex_shrink_0()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child("Workspace name")
                                .child(Input::new(&editor.name).aria_label("Workspace name")),
                        )
                        .child(div().w_full().flex_shrink_0().flex().flex_col().gap(px(5.))
                            .child("Website URL (optional)")
                            .child(Input::new(&editor.url).aria_label("Workspace website URL")))
                        .child(div().w_full().flex_shrink_0().min_w_0().flex().items_end().gap(px(12.))
                            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.))
                                .child("Icon").child(icons))
                            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.))
                                .child("Colour")
                                .child(div().flex().items_center().gap(px(8.))
                                    .child(colors)
                                    .child(div().flex_1().min_w_0()
                                        .child(Input::new(&editor.color).aria_label("Workspace colour")))))),
                )
                .children(
                    editor.error.as_ref().map(|error|
                        div().text_color(rgb(0xd35e5e)).child(error.clone())),
                )
                .child(div().debug_selector(|| "workspace-editor-footer".into())
                    .w_full().flex_shrink_0().pt(px(14.)).mt(px(2.)).border_t_1().border_color(rgb(palette.line))
                    .flex().items_center().justify_between().gap(px(8.))
                    .when(stacked_footer,|footer|footer.flex_col().items_stretch())
                    .child(Button::new("remove-workspace").flex_shrink_0().ghost().compact()
                        .label("Remove workspace…").disabled(!can_remove)
                        .tooltip(if can_remove { "Remove this workspace" } else { "Keep at least one workspace" })
                        .on_click(cx.workspace_listener(|view, _, _, cx| {
                            if let Some(editor) = &mut view.window.workspace_editor { editor.confirm_remove = true; }
                            cx.notify();
                        })))
                    .child(div().flex_shrink_0().debug_selector(||"workspace-editor-actions".into()).flex().items_center().justify_end().gap(px(8.))
                        .when(stacked_footer,|actions|actions.w_full())
                        .child(div().flex_shrink_0().debug_selector(||"workspace-editor-cancel".into())
                            .child(Button::new("cancel-workspace-edit").compact().label("Cancel")
                            .on_click(cx.workspace_listener(|view, _, _, cx| {
                                view.window.workspace_editor = None;
                                cx.notify();
                            }))))
                        .child(div().flex_shrink_0().debug_selector(||"workspace-editor-save".into())
                            .child(Button::new("save-workspace").compact().primary().outline().label("Save")
                            .on_click(cx.workspace_listener(|view, _, _, cx| view.save_workspace_editor(cx)))))))
                .into_any_element()
        } else {
            div().into_any_element()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{empty_layout, loaded_workspaces, new_id, relative_workspace_id, indexed_workspace_id, window_workspace_set, WorkspaceWindowState, WorkspaceWindowId};
    use monitter_lib::experimental_desktop::{NativePreferenceData, NativeWorkspace, NativeWorkspaceSet};
    use serde_json::json;
    use gpui_kit::{AppContext as _, Context, Render, IntoElement, TestAppContext, Window,
        MouseButton, Modifiers, SharedString, ParentElement as _, Styled as _,
        InteractiveElement as _, StatefulInteractiveElement as _, div, point, rgb};
    struct DragFixture {
        set: monitter_lib::experimental_desktop::NativeWorkspaceSet,
        clicks: usize,
    }
    impl Render for DragFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut row = div().w(gpui_kit::px(600.)).h(gpui_kit::px(80.)).flex().gap(gpui_kit::px(5.));
            for item in &self.set.items {
                let id = item.id.clone();
                let drag = super::WorkspaceTabDrag { id: id.clone(), name: item.name.clone(),
                    icon: gpui_kit::assets::IconName::Folder, color: rgb(0x38bfd2),
                    ink: rgb(0xffffff), background: rgb(0x111111) };
                let tab = div().id(SharedString::from(format!("drag-fixture-{id}")))
                    .w(gpui_kit::px(160.)).h(gpui_kit::px(32.)).flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|view, _, _, cx| { view.clicks += 1; cx.notify(); }));
                row = row.child(super::reorderable_tab(tab, drag, cx.listener(
                    move |view, drag: &super::WorkspaceTabDrag, _, cx| {
                        monitter_gpui_desktop::workspace_order::reorder(&mut view.set, &drag.id, &id);
                        cx.stop_propagation(); cx.notify();
                    }),|_,_,_,_|{}));
            }
            row
        }
    }
    #[cfg(feature = "performance-profiling")]
    #[gpui::test]
    fn workspace_number_hint_fades_out_and_reverses_without_unmounting(cx: &mut TestAppContext) {
        cx.update(|cx| crate::native_motion::initialize_for_ui_fixture(cx));
        struct HintHost(gpui_kit::Entity<super::WorkspaceNumberHint>);
        impl Render for HintHost {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().relative().size(gpui_kit::px(100.)).child(self.0.clone())
            }
        }
        let (host, visual) = cx.add_window_view(|_, cx| HintHost(cx.new(|_| super::WorkspaceNumberHint {
            number: 1, from: 0., target: 0., ink: rgb(0xffffff), background: rgb(0x111111),
            clock: crate::chat::feedback::FeedbackClock::new(),
        })));
        let hint = visual.update(|_, cx| host.read(cx).0.clone());
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let identity = hint.entity_id();
        // Settle the pressed state, then release with motion enabled.
        visual.update(|_, cx| {
            cx.set_reduce_motion(true);
            hint.update(cx, |hint, cx| hint.set_visible(true, cx));
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|_, cx| {
            cx.set_reduce_motion(false);
            hint.update(cx, |hint, cx| {
                hint.set_visible(false, cx);
                assert_eq!(hint.target, 0.);
                assert!(hint.opacity(false) > 0.9, "release must retain the visible badge for fade-out");
                let displayed = hint.opacity(false);
                hint.set_visible(true, cx);
                assert!((hint.from - displayed).abs() < 0.05, "reversal starts at displayed opacity");
            });
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(hint.entity_id(), identity);
        visual.update(|_, cx| {
            cx.set_reduce_motion(true);
            hint.update(cx, |hint, cx| {
                hint.set_visible(false, cx);
                assert_eq!(hint.opacity(true), 0.);
            });
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); window.simulate_next_frame(cx); });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        assert_eq!(visual.update(|window, cx| window.simulate_next_frame(cx)), 0);
    }

    #[test]
    fn window_tabs_and_shortcuts_use_only_owned_assignment_order() {
        let template = loaded_workspaces(&NativePreferenceData::default())
            .items
            .into_iter()
            .next()
            .unwrap();
        let set = NativeWorkspaceSet {
            active_id: "a".into(),
            items: ["a", "b", "c", "d"]
                .into_iter()
                .map(|id| {
                    let mut item = template.clone();
                    item.id = id.into();
                    item
                })
                .collect(),
        };
        let mut assignments =
            WorkspaceWindowState::new(&["a".into(), "b".into(), "c".into(), "d".into()]).unwrap();
        let detached = assignments.detach_workspace("b").unwrap();
        assignments.transfer_workspace("d", &detached, 0).unwrap();
        assignments.set_active_workspace(&detached, "b").unwrap();
        let visible = window_workspace_set(&set, &assignments, &detached, Some("b"));
        assert_eq!(
            visible
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["d", "b"]
        );
        assert_eq!(visible.active_id, "b");
        assert_eq!(indexed_workspace_id(&visible, 1).as_deref(), Some("d"));
        assert_eq!(indexed_workspace_id(&visible, 3), None);
        assert_eq!(relative_workspace_id(&visible, 1).as_deref(), Some("d"));
        let main = window_workspace_set(&set, &assignments, &WorkspaceWindowId::main(), Some("a"));
        assert_eq!(
            main.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "c"]
        );
        assert_eq!(indexed_workspace_id(&main, 2).as_deref(), Some("c"));
    }

    #[test]
    fn empty_main_stays_empty_and_legacy_unassigned_records_belong_only_to_main() {
        let set = loaded_workspaces(&NativePreferenceData::default());
        let id = set.items[0].id.clone();
        let mut assignments = WorkspaceWindowState::new(&[id.clone()]).unwrap();
        let detached = assignments.detach_workspace(&id).unwrap();
        let main = window_workspace_set(&set, &assignments, &WorkspaceWindowId::main(), None);
        assert!(main.items.is_empty());
        assert!(main.active_id.is_empty());
        assert_eq!(indexed_workspace_id(&main, 1), None);
        assert_eq!(relative_workspace_id(&main, 1), None);
        let legacy = WorkspaceWindowState::default();
        assert_eq!(
            window_workspace_set(&set, &legacy, &WorkspaceWindowId::main(), None)
                .items
                .len(),
            1
        );
        assert!(
            window_workspace_set(&set, &legacy, &detached, None)
                .items
                .is_empty()
        );
        // A stale local/global active ID never selects another host's workspace.
        let destination = window_workspace_set(&set, &assignments, &detached, Some("deleted"));
        assert_eq!(destination.active_id, id);
    }

    #[test]
    fn workspace_numbers_follow_visible_order_and_ignore_missing_tabs() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        let first = set.items[0].id.clone();
        assert_eq!(indexed_workspace_id(&set, 1), Some(first.clone()));
        assert_eq!(indexed_workspace_id(&set, 2), None);
        for index in 2..=10 {
            let mut item = set.items[0].clone(); item.id = format!("workspace-{index}"); set.items.push(item);
        }
        assert_eq!(indexed_workspace_id(&set, 9).as_deref(), Some("workspace-9"));
        assert_eq!(indexed_workspace_id(&set, 0), None);
        assert_eq!(indexed_workspace_id(&set, 10), None);
        set.items.swap(0, 8);
        assert_eq!(indexed_workspace_id(&set, 1).as_deref(), Some("workspace-9"));
        assert_eq!(indexed_workspace_id(&set, 9), Some(first));
    }

    #[test]
    fn workspace_cycle_follows_visible_order_and_wraps() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        let mut second = set.items[0].clone();
        second.id = "second".into();
        let mut third = second.clone();
        third.id = "third".into();
        set.items.extend([second, third]);
        assert_eq!(relative_workspace_id(&set, 1).as_deref(), Some("second"));
        assert_eq!(relative_workspace_id(&set, -1).as_deref(), Some("third"));
        set.active_id = "third".into();
        assert_eq!(relative_workspace_id(&set, 1), Some(set.items[0].id.clone()));
        assert_eq!(relative_workspace_id(&set, 0), None);
        set.items.truncate(1);
        set.active_id = set.items[0].id.clone();
        assert_eq!(relative_workspace_id(&set, 1), None);
    }

    #[gpui::test]
    fn workspace_drag_reorders_both_directions_without_clicking(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| DragFixture {
            set: monitter_lib::experimental_desktop::NativeWorkspaceSet {
                active_id: "b".into(), items: ["a", "b", "c"].into_iter().map(|id| NativeWorkspace {
                    id: id.into(), name: id.into(), icon: "folder".into(), color: "#38BFD2".into(), url: None, layout: None,
                }).collect(),
            }, clicks: 0,
        });
        cx.run_until_parked(); cx.update(|window, cx| window.draw(cx).clear(cx));
        for (from, to, expected) in [(80., 410., vec!["b", "c", "a"]), (410., 80., vec!["a", "b", "c"])] {
            let start = point(gpui_kit::px(from), gpui_kit::px(16.));
            let end = point(gpui_kit::px(to), gpui_kit::px(16.));
            cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(point(gpui_kit::px(from + 10.), gpui_kit::px(16.)), Some(MouseButton::Left), Modifiers::none());
            cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
            cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let state = view.read(cx);
                assert_eq!(state.set.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), expected);
                assert_eq!(state.set.active_id, "b"); assert_eq!(state.clicks, 0);
                window.draw(cx).clear(cx);
            });
        }
        cx.simulate_click(point(gpui_kit::px(80.), gpui_kit::px(16.)), Modifiers::none());
        cx.update(|_, cx| assert_eq!(view.read(cx).clicks, 1));
    }
    #[test]
    fn legacy_layout_becomes_default_workspace_without_losing_panes() {
        let layout = empty_layout();
        let data = NativePreferenceData {
            layout: Some(layout.clone()),
            ..Default::default()
        };
        let set = loaded_workspaces(&data);
        assert_eq!(set.items.len(), 1);
        assert_eq!(set.items[0].layout, Some(layout));
        assert_eq!(set.active_id, set.items[0].id);
    }
    #[test]
    fn missing_named_layout_does_not_inherit_legacy_panes() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        set.items[0].layout = None;
        let data = NativePreferenceData {
            layout: Some(json!({"legacy": true})),
            workspaces: Some(set),
            ..Default::default()
        };
        let loaded = loaded_workspaces(&data);
        let layout = loaded.items[0].layout.as_ref().unwrap();
        assert!(layout.get("legacy").is_none());
        let ids = crate::docking::DockWorkspace::saved_ids(layout).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].starts_with("empty:"));
    }
    #[test]
    fn workspace_records_keep_distinct_layouts_and_selected_identity() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        set.items.push(NativeWorkspace {
            id: new_id(),
            name: "Second".into(),
            color: "#41C968".into(),
            icon: "code".into(),
            url: None, layout: Some(empty_layout()),
        });
        set.active_id = set.items[1].id.clone();
        set.items[0].layout = Some(empty_layout());
        let data = NativePreferenceData {
            workspaces: Some(set.clone()),
            ..Default::default()
        };
        assert_eq!(loaded_workspaces(&data), set);
        assert_ne!(set.items[0].id, set.items[1].id);
    }
}


#[cfg(all(test, feature = "visual-export"))]
mod workspace_editor_click_tests {
    use ::core::prelude::v1::test;
    use crate::{Desktop, design};
    use gpui_kit::{prelude::*, *};
    use monitter_lib::experimental_desktop::{NativePreferences, NativeWorkspace, NativeWorkspaceSet};
    struct Harness(Entity<Desktop>);
    impl Render for Harness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |view, cx| {
                let palette = design::palette(&view.snapshot["settings"], window.appearance());
                view.render_workspace_tabs(palette, window, cx)
            })
        }
    }
    #[gpui_kit::test]
    fn design_system_workspace_editor_preserves_identity_and_picker_drafts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut desktop = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = Desktop::new(window, cx);
                view.preferences = Some(NativePreferences::default());
                view.preferences_busy = true;
                view.named_workspaces = Some(NativeWorkspaceSet {
                    active_id: "one".into(), items: ["one", "two"].into_iter().map(|id| NativeWorkspace {
                        id: id.into(), name: id.into(), icon: "folder".into(), color: "#38BFD2".into(), url: None, layout: None,
                    }).collect(),
                });
                view
            });
            desktop = Some(view.clone());
            component::Root::new(cx.new(|_| Harness(view)), window, cx)
        });
        let desktop = desktop.unwrap();
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let icon = visual.debug_bounds("workspace-edit-icon-two").expect("workspace icon rendered");
        visual.simulate_click(icon.center(), Default::default());
        visual.run_until_parked();
        visual.read(|cx| {
            let view = desktop.read(cx);
            assert_eq!(view.workspace_editor.as_ref().map(|editor|editor.id.as_str()), Some("two"));
            assert_eq!(view.named_workspaces.as_ref().unwrap().active_id, "one");
        });
        assert!(visual.debug_bounds("workspace-editor-surface").is_some(), "editor popover is visible");
        let footer = visual.debug_bounds("workspace-editor-footer").expect("one shared action footer");
        let editor_bounds = visual.debug_bounds("workspace-editor-surface").unwrap();
        assert!(footer.left() >= editor_bounds.left() && footer.right() <= editor_bounds.right());
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.workspace_editor.as_ref().unwrap().icon_query.update(cx, |input, cx|
                input.set_value("rocket", window, cx));
        }));
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let picker = visual.debug_bounds("workspace-icon-picker-trigger").expect("full icon picker trigger");
        visual.simulate_click(picker.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let choice = visual.debug_bounds("workspace-icon-choice-rocket").expect("searched icon appears in popup");
        visual.simulate_click(choice.center(), Default::default());
        visual.run_until_parked();
        visual.read(|cx| {
            let view = desktop.read(cx);
            assert_eq!(view.workspace_editor.as_ref().unwrap().icon, "lucide:rocket");
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        let colour_trigger = visual.debug_bounds("workspace-colour-picker-trigger").expect("colour picker trigger");
        visual.simulate_click(colour_trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.read(|cx| assert!(desktop.read(cx).workspace_editor.as_ref().unwrap().color_picker.read(cx).is_open()));
        // An outside pointer dismisses the child colour popup first, preserving the draft.
        visual.simulate_click(point(gpui_kit::px(1.), gpui_kit::px(200.)), Default::default());
        visual.run_until_parked();
        visual.read(|cx| {
            let view = desktop.read(cx);
            assert!(view.workspace_editor.is_some());
            assert!(!view.workspace_editor.as_ref().unwrap().color_picker.read(cx).is_open());
        });
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.simulate_click(colour_trigger.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| { window.draw(cx).clear(cx); });
        visual.update(|window, cx| {
            let picker = desktop.read(cx).workspace_editor.as_ref().unwrap().color_picker.clone();
            picker.update(cx, |picker, cx| picker.select_color(rgb(0xEF709C).into(), window, cx));
        });
        visual.run_until_parked();
        visual.read(|cx| assert_eq!(desktop.read(cx).workspace_editor.as_ref().unwrap().color.read(cx).value().as_str(), "#EF709C"));
        visual.update(|window, cx| {
            let color = desktop.read(cx).workspace_editor.as_ref().unwrap().color.clone();
            color.update(cx, |input, cx| {
                input.set_value("#41C968", window, cx);
                cx.emit(super::InputEvent::Change);
            });
        });
        visual.run_until_parked();
        visual.read(|cx| {
            let color: Rgba = desktop.read(cx).workspace_editor.as_ref().unwrap().color_picker.read(cx).value().unwrap().into();
            assert!((color.r * 255. - 65.).abs() < 0.01);
            assert!((color.g * 255. - 201.).abs() < 0.01);
            assert!((color.b * 255. - 104.).abs() < 0.01);
        });
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.workspace_editor.as_ref().unwrap().url.update(cx, |input, cx|
                input.set_value("https://example.com/project", window, cx));
        }));
        visual.update(|window,cx|{window.draw(cx).clear(cx);});
        let save=visual.debug_bounds("workspace-editor-save").expect("visible Save button");
        let footer=visual.debug_bounds("workspace-editor-footer").unwrap();
        assert!(save.left()>=footer.left()&&save.right()<=footer.right()&&save.top()>=footer.top()&&save.bottom()<=footer.bottom());
        visual.simulate_click(save.center(),Default::default());visual.run_until_parked();
        visual.read(|cx|assert!(desktop.read(cx).workspace_editor.is_none(),"Save pointer commits and closes the actual editor"));
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            let saved = &view.named_workspaces.as_ref().unwrap().items[1];
            assert_eq!(saved.url.as_deref(), Some("https://example.com/project"));
            assert_eq!(saved.icon, "lucide:rocket");
            view.edit_named_workspace("two", window, cx);
            view.workspace_editor.as_ref().unwrap().url.update(cx, |input, cx|
                input.set_value("file:///tmp/private.html", window, cx));
            view.save_workspace_editor(cx);
            assert!(view.workspace_editor.as_ref().unwrap().error.is_some());
            assert_eq!(view.named_workspaces.as_ref().unwrap().items[1].url.as_deref(), Some("https://example.com/project"));
            view.workspace_editor.as_ref().unwrap().url.update(cx, |input, cx|
                input.set_value("", window, cx));
            view.save_workspace_editor(cx);
            assert_eq!(view.named_workspaces.as_ref().unwrap().items[1].url, None);
        }));
    }
    #[gpui_kit::test]
    fn design_system_workspace_editor_save_stays_visible_and_clickable_in_short_narrow_popovers(cx:&mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut desktop=None;
        let(_,visual)=cx.add_window_view(|window,cx|{
            let view=cx.new(|cx|{
                let mut view=Desktop::new(window,cx);
                view.preferences=Some(NativePreferences::default());
                // Block every startup-cache/preference write in this offline fixture.
                view.preferences_busy=true;
                view.named_workspaces=Some(NativeWorkspaceSet{active_id:"one".into(),items:["one","two"].into_iter().map(|id|NativeWorkspace{
                    id:id.into(),name:id.into(),icon:"folder".into(),color:"#38BFD2".into(),url:None,layout:None,
                }).collect()});view
            });desktop=Some(view.clone());
            component::Root::new(cx.new(|_|Harness(view)),window,cx)
        });
        let desktop=desktop.unwrap();
        for(width,height,font)in[(900.,700.,14.),(420.,460.,14.),(360.,420.,20.)]{
            visual.simulate_resize(size(gpui_kit::px(width),gpui_kit::px(height)));
            visual.update(|window,cx|{
                component::Theme::update(cx,|theme|theme.font_size=gpui_kit::px(font));
                desktop.update(cx,|view,cx|{
                    assert!(view.service.is_none(),"fixture never connects backend");
                    view.edit_named_workspace("two",window,cx);
                    view.workspace_editor.as_ref().unwrap().name.update(cx,|input,cx|input.set_value(format!("Saved at {width}"),window,cx));
                });window.draw(cx).clear(cx);
            });
            visual.run_until_parked();visual.update(|window,cx|{window.draw(cx).clear(cx);});
            let save=visual.debug_bounds("workspace-editor-save").expect("Save rendered at every viewport");
            let cancel=visual.debug_bounds("workspace-editor-cancel").expect("Cancel rendered at every viewport");
            let footer=visual.debug_bounds("workspace-editor-footer").unwrap();
            let fields=visual.debug_bounds("workspace-editor-fields").unwrap();
            let editor=visual.debug_bounds("workspace-editor-surface").unwrap();
            assert!(save.size.width>gpui_kit::px(0.)&&save.size.height>gpui_kit::px(0.));
            assert!(cancel.size.width>gpui_kit::px(0.)&&cancel.size.height>gpui_kit::px(0.));
            assert!(cancel.left()>=editor.left()&&cancel.right()<=editor.right()&&cancel.top()>=editor.top()&&cancel.bottom()<=editor.bottom(),"Cancel stays inside painted popup");
            assert!(cancel.left()>=gpui_kit::px(0.)&&cancel.right()<=gpui_kit::px(width)&&cancel.top()>=gpui_kit::px(0.)&&cancel.bottom()<=gpui_kit::px(height),"Cancel visible in window");
            assert!(save.left()>=editor.left()&&save.right()<=editor.right()&&save.top()>=editor.top()&&save.bottom()<=editor.bottom(),"Save stays inside editor: {save:?} / {editor:?}");
            assert!(save.left()>=gpui_kit::px(0.)&&save.right()<=gpui_kit::px(width)&&save.top()>=gpui_kit::px(0.)&&save.bottom()<=gpui_kit::px(height),"Save is actually visible in window: {save:?}");
            assert!(footer.top()>=fields.bottom(),"scrollable fields cannot overlap action footer");
            visual.simulate_click(save.center(),Default::default());visual.run_until_parked();
            visual.read(|cx|{
                let view=desktop.read(cx);
                assert!(view.workspace_editor.is_none(),"Save hit target commits the editor");
                assert_eq!(view.named_workspaces.as_ref().unwrap().items[1].name,format!("Saved at {width}"));
                assert!(view.preferences_busy&&view.service.is_none(),"save remains offline without persistence");
            });
        }
    }

}

#[cfg(test)]
mod sidebar_task_navigation_tests {
    use crate::{Desktop, Sidebar, docking::{DockWorkspace, PaneSpec}};
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, component, div,
    };
    use gpui_kit::component::dock::DockPlacement;
    use monitter_lib::experimental_desktop::{NativePreferences, NativeWorkspace, NativeWorkspaceSet};

    struct Content;
    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div().size_full() }
    }
    struct Harness(Entity<Desktop>);
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.read(cx).dock.clone()
        }
    }
    fn pane(id: &str, cx: &mut Context<DockWorkspace>) -> PaneSpec {
        PaneSpec { id: id.into(), title: id.into(), content: cx.new(|_| Content).into(),
            focus: cx.focus_handle(), lifecycle: None, closable: true }
    }
    fn fixture(cx: &mut TestAppContext) -> (Entity<Desktop>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let mut desktop = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = Desktop::new(window, cx);
                view.preferences = Some(NativePreferences::default());
                view.preferences_busy = false;
                view.preferences_restore = false;
                view.preferences_restoring = false;
                view.preferences_restore_suppressed = false;
                view
            });
            desktop = Some(view.clone());
            component::Root::new(cx.new(|_| Harness(view)), window, cx)
        });
        (desktop.unwrap(), visual)
    }
    fn workspaces(current: serde_json::Value, target: serde_json::Value) -> NativeWorkspaceSet {
        NativeWorkspaceSet { active_id: "current".into(), items: [("current", current), ("target", target)]
            .into_iter().map(|(id, layout)| NativeWorkspace { id: id.into(), name: id.into(),
                icon: "folder".into(), color: "#38BFD2".into(), url: None, layout: Some(layout) }).collect() }
    }

    fn install_workspaces(view: &mut Desktop, set: NativeWorkspaceSet) {
        let active = set.active_id.clone();
        view.workspace_window_assignments =
            crate::workspace_window_state::WorkspaceWindowState::new(
                &set.items
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        view.workspace_window_assignments
            .set_active_workspace(&view.current_workspace_window, &active)
            .unwrap();
        view.window.active_workspace_id = Some(active);
        view.workspace_window_restore_pending = false;
        view.named_workspaces = Some(set);
    }
    // A published snapshot normally prepares these panes in the destination's
    // new dock. This offline fixture supplies its placeholder content explicitly.
    fn restore_offline_target(
        view: &mut Desktop,
        layout: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Desktop>,
    ) {
        let ids = DockWorkspace::saved_ids(&layout).unwrap();
        view.window.dock.update(cx, |dock, cx| {
            for id in ids {
                dock.register(pane(&id, cx), cx).unwrap();
            }
            dock.restore_retaining(layout.clone(), window, cx).unwrap();
        });
        let target = view.window.workspace_switch.take().unwrap();
        view.window.active_workspace_id = Some(target.clone());
        view.named_workspaces.as_mut().unwrap().active_id = target.clone();
        view.workspace_window_assignments
            .set_active_workspace(&view.current_workspace_window, &target)
            .unwrap();
        view.window.preferences_restore = false;
        view.window.preferences_restoring = false;
        view.window.preferences_restore_suppressed = false;
        view.restore_named_workspace_focus(&layout, window, cx);
    }

    #[gpui_kit::test]
    fn workspace_notification_navigation_prefers_scoped_workspace_and_preserves_busy_guard(
        cx: &mut TestAppContext,
    ) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.dock.update(cx, |dock, cx| {
                dock.add(pane("chat:shared", cx), DockPlacement::Center, None, window, cx).unwrap();
            });
            let layout = view.dock.read(cx).serialize(cx).unwrap();
            install_workspaces(view, workspaces(layout.clone(), layout.clone()));
            let outgoing = view.window.dock.clone();
            view.snapshot = serde_json::json!({"agents":[{"id":"agent"}],"tasks":[{"id":"shared","agentId":"agent","updatedAt":1}],"messages":[],"channels":[],"approvalRequests":[]}).into();
            view.notification_model.sync(&view.snapshot);
            std::rc::Rc::make_mut(&mut view.snapshot)["messages"] = serde_json::json!([{"id":"answer","taskId":"shared","role":"assistant","text":"Done","createdAt":2,"phase":"final_answer","streamStatus":"complete"}]);
            view.notification_model.sync(&view.snapshot);
            view.preferences_busy = true;
            assert!(!view.navigate_workspace_notification("target", "shared".into(), window, cx));
            assert!(view.workspace_switch.is_none());
            view.preferences_busy = false;
            assert!(view.navigate_workspace_notification("target", "shared".into(), window, cx));
            assert_eq!(view.workspace_switch.as_deref(), Some("target"));
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("shared"));
            assert!(view.selected.is_none());
            assert_eq!(view.notification_model.entries(&view.snapshot).len(), 1, "queued navigation must not consume unread");
            assert!(!view.navigate_workspace_notification("current", "other".into(), window, cx));
            assert!(view.pending_notification_read.is_none(), "a later blocked review cancels the old read acknowledgment");
            view.fail_workspace_restore("Fixture restore failed".into(), window, cx);
            assert!(view.pending_notification_read.is_none());
            assert_eq!(view.notification_model.entries(&view.snapshot).len(), 1, "failed restore keeps unread");
            assert_eq!(view.window.active_workspace_id.as_deref(), Some("current"));
            assert_eq!(view.window.dock.entity_id(), outgoing.entity_id(), "failure returns the exact retained source dock");
            assert_eq!(view.window.dock.read(cx).serialize(cx).unwrap(), layout, "failure cannot mutate outgoing panes");
            assert_eq!(view.named_workspaces.as_ref().unwrap().items[1].layout.as_ref(), Some(&layout), "partial target never overwrites its saved layout");
            assert!(!view.workspace_view_cache.contains_key("target"), "failed target cannot masquerade as a restored cache entry");
            assert_eq!(view.preferences_error.as_deref(), Some("Fixture restore failed"));
            assert!(!view.preferences_blocked, "returning a preserved source permits an explicit retry");
            assert!(view.navigate_workspace_notification("target", "shared".into(), window, cx));
            assert_eq!(view.workspace_switch.as_deref(), Some("target"), "retry starts a fresh restore");
            restore_offline_target(view, layout.clone(), window, cx);
            assert!(view.notification_model.entries(&view.snapshot).is_empty(), "successful target focus acknowledges the same unread entry");
            assert!(view.pending_notification_read.is_none());
        }));
    }

    #[gpui_kit::test]
    fn failed_initial_restore_never_captures_partial_target(cx: &mut TestAppContext) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| {
            desktop.update(cx, |view, cx| {
                view.window.dock.update(cx, |dock, cx| {
                    dock.add(
                        pane("chat:partial", cx),
                        DockPlacement::Center,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                });
                let saved = workspaces(super::empty_layout(), super::empty_layout());
                install_workspaces(view, saved.clone());
                view.window.workspace_switch = Some("current".into());
                view.fail_workspace_restore("Fixture initial restore failed".into(), window, cx);
                assert!(view.preferences_blocked);
                assert!(
                    view.window.preferences_restoring && view.window.preferences_restore_suppressed
                );
                assert!(
                    !view.window.preferences_restore,
                    "failed startup cannot retry itself on every render"
                );
                view.capture_window_layouts(cx).unwrap();
                assert_eq!(
                    view.named_workspaces.as_ref(),
                    Some(&saved),
                    "global capture cannot save an incomplete initial target"
                );
                assert_eq!(
                    view.preferences_error.as_deref(),
                    Some("Fixture initial restore failed")
                );
            })
        });
    }

    #[gpui_kit::test]
    fn sidebar_task_handler_focuses_current_chat_without_adding_panes(cx: &mut TestAppContext) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.dock.update(cx, |dock, cx| {
                for id in ["chat:first", "chat:second"] {
                    dock.add(pane(id, cx), DockPlacement::Center, None, window, cx).unwrap();
                }
            });
            let before = view.dock.read(cx).serialize(cx).unwrap();
            for _ in 0..2 { view.navigate_sidebar_task("first".into(), window, cx); }
            assert_eq!(view.selected.as_deref(), Some("first"));
            assert_eq!(view.dock_current.as_deref(), Some("chat:first"));
            assert!(view.sidebar == Sidebar::Chats);
            assert_eq!(view.dock.read(cx).focused_pane_id(window, cx).as_deref(), Some("chat:first"));
            assert_eq!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap(),
                DockWorkspace::saved_ids(&before).unwrap());
            assert!(view.pending_sidebar_task.is_none());
            assert!(view.workspace_switch.is_none());
        }));
    }

    #[gpui_kit::test]
    fn sidebar_task_handler_switches_then_focuses_latest_click_after_restore(cx: &mut TestAppContext) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.dock.update(cx, |dock, cx| {
                dock.add(pane("chat:outgoing", cx), DockPlacement::Center, None, window, cx).unwrap();
                dock.register(pane("chat:first", cx), cx).unwrap();
                dock.register(pane("chat:latest", cx), cx).unwrap();
            });
            let current = view.dock.read(cx).serialize(cx).unwrap();
            let children = ["chat:first", "chat:latest"].into_iter().map(|id|
                serde_json::json!({"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":id}}}))
                .collect::<Vec<_>>();
            let mut target = serde_json::json!({"version":1,"focusedPaneId":"chat:first","center":{
                "panel_name":"TabPanel","children":children,"info":{"tabs":{"active_index":0}}}});
            crate::workspace_visibility::remember_focus(&mut target, Some("chat:first"));
            install_workspaces(view, workspaces(current.clone(), target.clone()));
            let outgoing = view.window.dock.clone();
            view.navigate_sidebar_task("first".into(), window, cx);
            assert_eq!(view.workspace_switch.as_deref(), Some("target"));
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("first"), "first click follows the new restore view before any later click");
            // No snapshot is published in this offline fixture, so the actual
            // handler has queued its restore and awaits the authoritative state.
            view.navigate_sidebar_task("latest".into(), window, cx);
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("latest"));
            assert!(view.selected.is_none(), "busy clicks cannot render a target in outgoing workspace");
            let retained = &view.workspace_view_cache["current"];
            assert_eq!(retained.dock.entity_id(), outgoing.entity_id(), "outgoing dock stays retained during restore");
            assert_eq!(retained.dock.read(cx).serialize(cx).unwrap(), current, "queued clicks cannot mutate outgoing layout");
            assert_ne!(view.window.dock.entity_id(), outgoing.entity_id(), "restore uses a separate destination dock");
            assert!(!view.window.dock.read(cx).contains("chat:outgoing"));
            assert_eq!(DockWorkspace::saved_ids(view.named_workspaces.as_ref().unwrap().items[0].layout.as_ref().unwrap()).unwrap(), ["chat:outgoing"]);
            restore_offline_target(view, target.clone(), window, cx);
            assert_eq!(outgoing.read(cx).serialize(cx).unwrap(), current, "successful restore also preserves original source layout");
            assert_eq!(view.selected.as_deref(), Some("latest"));
            assert_eq!(view.dock_current.as_deref(), Some("chat:latest"));
            assert_eq!(view.dock.read(cx).focused_pane_id(window, cx).as_deref(), Some("chat:latest"));
            assert!(view.pending_sidebar_task.is_none(), "latest requested pane supersedes remembered focus");
            assert_eq!(DockWorkspace::saved_ids(&view.dock.read(cx).serialize(cx).unwrap()).unwrap().len(), 2);
        }));
    }

    #[gpui_kit::test]
    fn sidebar_task_handler_focuses_and_acknowledges_cached_target(cx: &mut TestAppContext) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.window.dock.update(cx, |dock, cx| {
                dock.add(pane("chat:outgoing", cx), DockPlacement::Center, None, window, cx).unwrap();
            });
            let current = view.window.dock.read(cx).serialize(cx).unwrap();
            let children = ["chat:requested", "chat:remembered"].into_iter().map(|id|
                serde_json::json!({"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":id}}})).collect::<Vec<_>>();
            let mut target = serde_json::json!({"version":1,"center":{"panel_name":"TabPanel",
                "children":children,"info":{"tabs":{"active_index":1}}}});
            crate::workspace_visibility::remember_focus(&mut target, Some("chat:remembered"));
            install_workspaces(view, workspaces(current.clone(), target.clone()));
            view.switch_named_workspace("target".into(), window, cx);
            restore_offline_target(view, target, window, cx);
            assert_eq!(view.window.dock.read(cx).focused_pane_id(window, cx).as_deref(), Some("chat:remembered"));
            let retained_target = view.window.dock.clone();
            view.switch_named_workspace("current".into(), window, cx);
            assert_eq!(view.window.active_workspace_id.as_deref(), Some("current"));
            assert_eq!(view.workspace_view_cache["target"].dock.entity_id(), retained_target.entity_id());

            view.snapshot = serde_json::json!({"agents":[{"id":"agent"}],"tasks":[{"id":"requested","agentId":"agent","updatedAt":1}],"messages":[],"channels":[],"approvalRequests":[]}).into();
            view.notification_model.sync(&view.snapshot);
            std::rc::Rc::make_mut(&mut view.snapshot)["messages"] = serde_json::json!([{"id":"answer","taskId":"requested","role":"assistant","text":"Done","createdAt":2,"phase":"final_answer","streamStatus":"complete"}]);
            view.notification_model.sync(&view.snapshot);
            assert_eq!(view.notification_model.entries(&view.snapshot).len(), 1);
            view.window.pending_notification_read = Some("requested".into());
            view.navigate_sidebar_task("requested".into(), window, cx);
            assert_eq!(view.window.dock.entity_id(), retained_target.entity_id(), "navigation reuses the cached target dock");
            assert_eq!(view.window.active_workspace_id.as_deref(), Some("target"));
            assert!(view.window.workspace_switch.is_none(), "retained activation does not need an asynchronous restore");
            assert_eq!(view.window.selected.as_deref(), Some("requested"));
            assert_eq!(view.window.dock_current.as_deref(), Some("chat:requested"));
            assert_eq!(view.window.dock.read(cx).focused_pane_id(window, cx).as_deref(), Some("chat:requested"), "explicit navigation overrides the different remembered focus");
            assert!(view.window.pending_sidebar_task.is_none());
            assert!(view.window.pending_notification_read.is_none());
            assert!(view.notification_model.entries(&view.snapshot).is_empty(), "the explicitly reviewed unread task is acknowledged after actual focus");
            assert_eq!(view.workspace_view_cache["current"].dock.read(cx).serialize(cx).unwrap(), current, "cached navigation preserves the outgoing workspace");
        }));
    }

    #[gpui_kit::test]
    fn sidebar_task_handler_defers_missing_busy_targets_and_cancels_failed_restore(cx: &mut TestAppContext) {
        let (desktop, visual) = fixture(cx);
        visual.update(|window, cx| desktop.update(cx, |view, cx| {
            view.preferences_restoring = true;
            view.navigate_sidebar_task("missing".into(), window, cx);
            view.navigate_sidebar_task("latest".into(), window, cx);
            assert!(view.selected.is_none());
            assert!(!view.dock.read(cx).contains("chat:latest"));
            view.restore_named_workspace_focus(&super::empty_layout(), window, cx);
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("latest"), "a different incoming layout cannot consume intent");
            view.fail_workspace_restore("fixture restore failure".into(), window, cx);
            assert!(view.pending_sidebar_task.is_none());
            assert!(!view.dock.read(cx).contains("chat:latest"));
            view.preferences_blocked = false;
            view.preferences_busy = true;
            view.navigate_sidebar_task("missing".into(), window, cx);
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("missing"));
            view.finish_workspace_restore(serde_json::json!({"version":999}), Vec::new(), Vec::new(), 0, window, cx);
            assert!(view.pending_sidebar_task.is_none(), "validation failure also cancels deferred focus");
            assert!(view.preferences_blocked);
            assert!(view.selected.is_none());
            view.preferences_blocked = false;
            view.navigate_sidebar_task("queued".into(), window, cx);
            assert_eq!(view.pending_sidebar_task.as_deref(), Some("queued"));
            view.switch_named_workspace("manual-choice".into(), window, cx);
            assert!(view.pending_sidebar_task.is_none(), "manual workspace navigation cancels stale sidebar focus");
        }));
    }
}

#[cfg(test)]
mod workspace_icon_catalogue_tests {
    use ::core::prelude::v1::test;
    use super::*;
    #[test]
    fn every_renderable_icon_has_a_safe_persisted_key() {
        for choice in IconName::ALL.iter().copied() {
            let key = format!("lucide:{}", icon_key(choice));
            assert!(monitter_lib::experimental_desktop::valid_native_workspace_icon(&key), "{key}");
            assert_eq!(icon(&key), choice);
        }
        assert_eq!(icon("sparkles"), IconName::Star);
    }
}
